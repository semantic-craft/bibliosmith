"""High-level search / sync orchestration."""

from __future__ import annotations

from dataclasses import asdict
from datetime import datetime, timezone
import hashlib
from pathlib import Path
from typing import Callable

from .embed import EmbedderProtocol
from .fulltext import chunk_text, resolve_fulltext, resolve_fulltext_artifact
from .vector_store import SQLiteVecStore
from .zotero_db import ZoteroItem, iter_items


ITEM_INDEX_CONTRACT_VERSION = "zfulltext-item-index-v1"
CHUNK_CONTRACT_VERSION = "zfulltext-chunk-v2"


def _to_metadata(item: ZoteroItem, *, chunk_idx: int | None = None) -> dict:
    """Subset of ZoteroItem we persist alongside its vector."""
    d = asdict(item)
    d["creators"] = list(item.creators)
    d["tags"] = list(item.tags)
    if chunk_idx is not None:
        d["chunk_idx"] = chunk_idx
        d["is_chunk"] = True
    return d


def sync(
    store: SQLiteVecStore,
    embedder: EmbedderProtocol,
    *,
    db_path: Path | None = None,
    full: bool = False,
    progress: Callable[[int, int], None] | None = None,
) -> dict:
    """Sync vector store from Zotero local DB.

    Two-pass embedding:
      1. Metadata pass — title/abstract/authors (one vector per item).
      2. Fulltext pass — .md content chunked (multiple vectors per item).

    Args:
        store: Vector store backend.
        embedder: Embedding client.
        db_path: Optional override for ``zotero.sqlite`` path.
        full: If True, clear sync-managed vectors while preserving item-scoped chunks.
        progress: Optional callback ``(seen, total)`` invoked per batch.

    Returns:
        Stats dict with keys ``total``, ``embedded``, ``skipped``, ``chunks``.
    """
    items = iter_items(db_path)
    total = len(items)

    if full:
        store.clear_for_full_sync_preserving_item_scoped_chunks()
    existing = store.existing_keys()

    # Items needing embed: new key OR dateModified changed.
    todo: list[ZoteroItem] = [
        it for it in items if existing.get(it.key) != it.date_modified
    ]
    skipped = total - len(todo)

    # --- Pass 1: metadata vectors ---
    embedded = 0
    batch_size = embedder.cfg.batch_size
    for i in range(0, len(todo), batch_size):
        batch = todo[i : i + batch_size]
        texts = [it.embedding_text() for it in batch]
        vecs = embedder.embed(texts)
        store.upsert(
            keys=[it.key for it in batch],
            vectors=vecs,
            metadatas=[_to_metadata(it) for it in batch],
            date_modified=[it.date_modified for it in batch],
        )
        embedded += len(batch)
        if progress is not None:
            progress(embedded, len(todo))

    # --- Pass 2: fulltext chunk vectors ---
    # Fulltext identity is independent from the bibliographic parent's
    # dateModified value because a Markdown attachment can arrive later.
    chunk_count = 0
    embedding_profile_id = f"{embedder.cfg.model}:{embedder.cfg.dimensions}"
    for it in items:
        resolved = resolve_fulltext_artifact(it.key, db_path)
        if resolved is None:
            store.remove_sync_managed_item_chunks(it.key)
            continue
        text, source_sha256 = resolved
        if not text.strip():
            store.remove_sync_managed_item_chunks(it.key)
            continue
        chunks = chunk_text(text)
        existing_chunks = store.item_chunk_metadatas(it.key)
        if _chunks_match_identity(
            existing_chunks,
            chunks,
            source_sha256=source_sha256,
            chunk_contract_version=CHUNK_CONTRACT_VERSION,
            embedding_profile_id=embedding_profile_id,
        ):
            continue
        metadatas = [
            {
                **_to_metadata(it, chunk_idx=index),
                "total_chunks": len(chunks),
                "chunk_text": chunk,
                "source_sha256": source_sha256,
                "index_contract_version": ITEM_INDEX_CONTRACT_VERSION,
                "chunk_contract_version": CHUNK_CONTRACT_VERSION,
                "embedding_profile_id": embedding_profile_id,
                "index_source": "zotero_sync",
            }
            for index, chunk in enumerate(chunks)
        ]
        vectors: list[list[float]] = []
        for start in range(0, len(chunks), batch_size):
            vectors.extend(embedder.embed(chunks[start : start + batch_size]))
        store.replace_item_chunks(
            it.key,
            keys=[f"{it.key}#c{index}" for index in range(len(chunks))],
            vectors=vectors,
            metadatas=metadatas,
            date_modified=[source_sha256] * len(chunks),
        )
        chunk_count += len(chunks)
        if progress is not None:
            progress(embedded + chunk_count, embedded + chunk_count)

    return {"total": total, "embedded": embedded, "skipped": skipped, "chunks": chunk_count}


def index_markdown_item(
    store: SQLiteVecStore,
    embedder: EmbedderProtocol,
    *,
    parent_item_key: str,
    markdown_path: Path,
    expected_sha256: str,
    chunk_contract_version: str,
    embedding_profile_id: str,
    metadata: dict | None = None,
) -> dict:
    """Index one verified Markdown artifact without scanning Zotero storage."""
    actual_sha256 = hashlib.sha256(markdown_path.read_bytes()).hexdigest()
    if actual_sha256 != expected_sha256:
        raise ValueError("Markdown SHA-256 does not match the requested item index input")
    text = markdown_path.read_text(encoding="utf-8")
    if not text.strip():
        raise ValueError("Markdown artifact is empty")
    chunks = chunk_text(text)
    if not chunks:
        raise ValueError("Markdown artifact produced no full-text chunks")

    existing = store.item_chunk_metadatas(parent_item_key)
    same_identity = _chunks_match_identity(
        existing,
        chunks,
        source_sha256=actual_sha256,
        chunk_contract_version=chunk_contract_version,
        embedding_profile_id=embedding_profile_id,
    )
    if same_identity:
        return _item_index_evidence(
            parent_item_key,
            actual_sha256,
            len(chunks),
            chunk_contract_version,
            embedding_profile_id,
            reused=True,
        )

    base_metadata = dict(metadata or {})
    base_metadata.setdefault("title", parent_item_key)
    base_metadata.setdefault("creators", [])
    base_metadata.setdefault("tags", [])
    chunk_metadatas = [
        {
            **base_metadata,
            "chunk_idx": index,
            "total_chunks": len(chunks),
            "is_chunk": True,
            "chunk_text": chunk,
            "source_sha256": actual_sha256,
            "index_contract_version": ITEM_INDEX_CONTRACT_VERSION,
            "chunk_contract_version": chunk_contract_version,
            "embedding_profile_id": embedding_profile_id,
            "index_source": "item_scoped",
        }
        for index, chunk in enumerate(chunks)
    ]
    vectors: list[list[float]] = []
    for start in range(0, len(chunks), embedder.cfg.batch_size):
        vectors.extend(embedder.embed(chunks[start : start + embedder.cfg.batch_size]))

    store.replace_item_chunks(
        parent_item_key,
        keys=[f"{parent_item_key}#c{index}" for index in range(len(chunks))],
        vectors=vectors,
        metadatas=chunk_metadatas,
        date_modified=[actual_sha256] * len(chunks),
    )
    return _item_index_evidence(
        parent_item_key,
        actual_sha256,
        len(chunks),
        chunk_contract_version,
        embedding_profile_id,
        reused=False,
    )


def _chunks_match_identity(
    existing: list[dict],
    chunks: list[str],
    *,
    source_sha256: str,
    chunk_contract_version: str,
    embedding_profile_id: str,
) -> bool:
    return len(existing) == len(chunks) and all(
        item.get("source_sha256") == source_sha256
        and item.get("index_contract_version") == ITEM_INDEX_CONTRACT_VERSION
        and item.get("chunk_contract_version") == chunk_contract_version
        and item.get("embedding_profile_id") == embedding_profile_id
        and item.get("chunk_idx") == index
        and item.get("chunk_text") == chunks[index]
        for index, item in enumerate(existing)
    )


def _item_index_evidence(
    parent_item_key: str,
    source_sha256: str,
    chunk_count: int,
    chunk_contract_version: str,
    embedding_profile_id: str,
    *,
    reused: bool,
) -> dict:
    return {
        "parentItemKey": parent_item_key,
        "sourceSha256": source_sha256,
        "chunkCount": chunk_count,
        "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
        "chunkContractVersion": chunk_contract_version,
        "embeddingProfileId": embedding_profile_id,
        "completedAt": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "reused": reused,
    }


def _matches_filters(
    meta: dict,
    *,
    item_type: str | None,
    year: tuple[int | None, int | None] | None,
    tag: str | None,
) -> bool:
    if item_type and meta.get("item_type") != item_type:
        return False
    if tag and tag not in (meta.get("tags") or []):
        return False
    if year:
        lo, hi = year
        date = (meta.get("date") or "").strip()
        # Zotero stores dates in many formats; pull the first 4-digit year.
        import re
        m = re.search(r"\b(19|20)\d{2}\b", date)
        if not m:
            return False
        y = int(m.group(0))
        if lo is not None and y < lo:
            return False
        if hi is not None and y > hi:
            return False
    return True


def _parent_key(key: str) -> str:
    """Strip chunk suffix: 'ABC123#c0' → 'ABC123'."""
    return key.split("#")[0]


def query(
    text: str,
    store: SQLiteVecStore,
    embedder: EmbedderProtocol,
    *,
    top_k: int = 10,
    item_type: str | None = None,
    year: tuple[int | None, int | None] | None = None,
    tag: str | None = None,
    rerank: bool = False,
    candidate_pool: int = 50,
) -> list[dict]:
    """Top-K semantic search. Supports metadata filters and optional reranking.

    Results are deduplicated by parent item key — if both a metadata vector
    and a fulltext chunk match, the best distance wins.
    """
    qv = embedder.embed_query(text)
    can_rerank = rerank and hasattr(embedder, "rerank")
    pool = max(top_k * 3, candidate_pool) if (can_rerank or item_type or year or tag) else top_k
    raw = store.query(qv, top_k=pool)

    # Deduplicate by parent key — keep best distance per item
    seen: dict[str, dict] = {}
    for key, dist, meta in raw:
        if not _matches_filters(meta, item_type=item_type, year=year, tag=tag):
            continue
        pk = _parent_key(key)
        entry = {"key": pk, "distance": dist, **meta}
        if pk not in seen or dist < seen[pk]["distance"]:
            seen[pk] = entry
    enriched = sorted(seen.values(), key=lambda r: r["distance"])

    if can_rerank and enriched:
        docs = [
            f"{r.get('title') or ''} :: {(r.get('abstract') or '')[:500]}"
            for r in enriched
        ]
        ranked = embedder.rerank(text, docs, top_k=top_k)  # type: ignore[attr-defined]
        return [{**enriched[i], "rerank_score": s} for i, s in ranked]

    return enriched[:top_k]


def query_fulltext(
    text: str,
    store: SQLiteVecStore,
    embedder: EmbedderProtocol,
    *,
    top_k: int = 10,
    item_type: str | None = None,
    year: tuple[int | None, int | None] | None = None,
    tag: str | None = None,
    rerank: bool = False,
    candidate_pool: int = 200,
    db_path: Path | None = None,
    context_chunks: int = 2,
) -> list[dict]:
    """Semantic search restricted to fulltext chunks, returning matched text.

    Each result includes ``chunk_text`` (the matched chunk) and optionally
    surrounding chunks via ``context_before`` / ``context_after``.
    """
    qv = embedder.embed_query(text)
    can_rerank = rerank and hasattr(embedder, "rerank")
    pool = max(top_k * 5, candidate_pool)
    raw = store.query(qv, top_k=pool)

    # Only keep chunk vectors (key contains #c)
    chunk_hits: list[dict] = []
    for key, dist, meta in raw:
        if "#c" not in key:
            continue
        if not _matches_filters(meta, item_type=item_type, year=year, tag=tag):
            continue
        chunk_hits.append({"raw_key": key, "distance": dist, **meta})

    # Deduplicate: keep best chunk per parent item
    seen: dict[str, dict] = {}
    for hit in chunk_hits:
        pk = _parent_key(hit["raw_key"])
        if pk not in seen or hit["distance"] < seen[pk]["distance"]:
            seen[pk] = {**hit, "key": pk}
    enriched = sorted(seen.values(), key=lambda r: r["distance"])

    if can_rerank and enriched:
        docs = [r.get("title", "") for r in enriched]
        ranked = embedder.rerank(text, docs, top_k=top_k)  # type: ignore[attr-defined]
        enriched = [{**enriched[i], "rerank_score": s} for i, s in ranked]
    else:
        enriched = enriched[:top_k]

    # Resolve chunk text + context for each result
    for r in enriched:
        item_key = r["key"]
        chunk_idx = r.get("chunk_idx", 0)
        stored_chunk_text = r.get("chunk_text")
        if stored_chunk_text:
            stored_chunks = store.item_chunk_metadatas(item_key)
            stored_texts = [item.get("chunk_text", "") for item in stored_chunks]
            r["chunk_text"] = stored_chunk_text
            lo = max(0, chunk_idx - context_chunks)
            hi = min(len(stored_texts), chunk_idx + context_chunks + 1)
            r["context_before"] = stored_texts[lo:chunk_idx]
            r["context_after"] = stored_texts[chunk_idx + 1 : hi]
            r["total_chunks"] = len(stored_texts)
        else:
            fulltext = resolve_fulltext(item_key, db_path)
            if not fulltext:
                r["chunk_text"] = ""
                r["context_before"] = []
                r["context_after"] = []
                r["total_chunks"] = 0
                r.pop("raw_key", None)
                continue
            chunks = chunk_text(fulltext)
            r["chunk_text"] = chunks[chunk_idx] if chunk_idx < len(chunks) else ""
            lo = max(0, chunk_idx - context_chunks)
            hi = min(len(chunks), chunk_idx + context_chunks + 1)
            r["context_before"] = chunks[lo:chunk_idx]
            r["context_after"] = chunks[chunk_idx + 1 : hi]
            r["total_chunks"] = len(chunks)
        r.pop("raw_key", None)

    return enriched


def get_item_chunks(
    item_key: str,
    db_path: Path | None = None,
) -> list[dict]:
    """Return all fulltext chunks for a single item."""
    fulltext = resolve_fulltext(item_key, db_path)
    if not fulltext:
        return []
    chunks = chunk_text(fulltext)
    return [{"chunk_idx": i, "text": c} for i, c in enumerate(chunks)]


def _resolve_snippet(
    parent_key: str,
    chunk_idx: int,
    db_path: Path | None,
    cache: dict[str, list[str]],
    fulltext_fn: Callable[[str, Path | None], str | None] = resolve_fulltext,
    chunk_fn: Callable[[str], list[str]] = chunk_text,
) -> str | None:
    """Return the original text of chunk `chunk_idx` for `parent_key`.

    Re-derives chunks with the same deterministic ``chunk_text`` used at index
    time, caching per parent so repeated hits in one query read the .md once.
    Returns None if the full text is missing or the index is out of range.

    WARNING: chunk_idx values are only valid while chunk_text's parameters/logic
    are unchanged. If chunking changes, stored chunk_idx values go stale until
    `zsearch sync --full` rebuilds the index.
    """
    if parent_key not in cache:
        text = fulltext_fn(parent_key, db_path)
        cache[parent_key] = chunk_fn(text) if text else []
    chunks = cache[parent_key]
    if 0 <= chunk_idx < len(chunks):
        return chunks[chunk_idx]
    return None


def query_chunks(
    text: str,
    store: SQLiteVecStore,
    embedder: EmbedderProtocol,
    *,
    top_k: int = 10,
    db_path: Path | None = None,
    candidate_pool: int = 50,
    fulltext_fn: Callable[[str, Path | None], str | None] = resolve_fulltext,
    chunk_fn: Callable[[str], list[str]] = chunk_text,
) -> list[dict]:
    """Chunk-level semantic search returning matched passages with snippet text.

    Unlike ``query``, this does NOT dedup by parent item and keeps chunk
    granularity. Only fulltext-chunk hits (key like ``KEY#c3``) are returned;
    metadata-only vectors are skipped. Each result carries the original snippet
    text resolved from the item's ``.md`` full text.
    """
    qv = embedder.embed_query(text)
    pool = max(top_k * 3, candidate_pool)
    raw = store.query(qv, top_k=pool)

    cache: dict[str, list[str]] = {}
    results: list[dict] = []
    for key, dist, meta in raw:
        if "#c" not in key:
            continue  # metadata-only vector, no chunk text
        chunk_idx = meta.get("chunk_idx")
        if chunk_idx is None:
            continue
        parent_key = _parent_key(key)
        snippet = _resolve_snippet(
            parent_key, chunk_idx, db_path, cache, fulltext_fn, chunk_fn
        )
        if snippet is None:
            continue
        results.append(
            {
                "key": parent_key,
                "chunk_idx": chunk_idx,
                "distance": dist,
                "snippet": snippet,
                "title": meta.get("title"),
                "creators": meta.get("creators") or [],
                "date": meta.get("date"),
                "venue": meta.get("venue"),
                "doi": meta.get("doi"),
            }
        )
        if len(results) >= top_k:
            break
    return results
