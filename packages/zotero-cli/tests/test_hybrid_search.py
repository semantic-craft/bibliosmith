from __future__ import annotations

from pathlib import Path
import hashlib
import json
import sqlite3

from click.testing import CliRunner
import pytest

from zotero_cli.embed import EmbedConfig
from zotero_cli.search import index_markdown_item, query, query_fulltext, sync
import zotero_cli.search as search_module
import zotero_cli.vector_store as vector_store_module
from zotero_cli.vector_store import SQLiteVecStore, VectorStoreConfig
from zotero_cli.zotero_db import ZoteroItem
import zotero_cli.cli as zsearch_cli
import zotero_cli.mcp_server as mcp_server
import zotero_cli.zfulltext_cli as zfulltext_cli


class FixtureEmbedder:
    def __init__(self) -> None:
        self.cfg = EmbedConfig(model="fixture", dimensions=2, batch_size=32)

    def embed(self, texts: list[str]) -> list[list[float]]:
        return [[1.0, 0.0] for _ in texts]

    def embed_query(self, _query: str) -> list[float]:
        return [1.0, 0.0]

    def __enter__(self) -> "FixtureEmbedder":
        return self

    def __exit__(self, *_exc: object) -> None:
        return None


class FixtureRerankingEmbedder(FixtureEmbedder):
    def rerank(
        self,
        _query: str,
        documents: list[str],
        *,
        top_k: int,
    ) -> list[tuple[int, float]]:
        exact_index = next(
            index for index, document in enumerate(documents) if "Quuxzxyl" in document
        )
        return [(exact_index, 0.99)][:top_k]


def test_hybrid_query_recovers_exact_term_missed_by_vector_top_hit(
    tmp_path: Path,
) -> None:
    embedder = FixtureEmbedder()
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        store.upsert(
            keys=["VECTOR-1", "VECTOR-2", "VECTOR-3", "EXACT"],
            vectors=[[1.0, 0.0], [1.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            metadatas=[
                {"title": "Semantic neighbour", "creators": [], "tags": []},
                {"title": "Second neighbour", "creators": [], "tags": []},
                {"title": "Third neighbour", "creators": [], "tags": []},
                {"title": "Quuxzxyl doctrine", "creators": [], "tags": []},
            ],
            date_modified=["2026-01-01"] * 4,
        )

        vector_results = query(
            "Quuxzxyl",
            store,
            embedder,
            top_k=1,
            candidate_pool=1,
            mode="vector",
        )
        hybrid_results = query(
            "Quuxzxyl",
            store,
            embedder,
            top_k=1,
            candidate_pool=1,
            mode="hybrid",
        )

    assert [result["key"] for result in vector_results] != ["EXACT"]
    assert [result["key"] for result in hybrid_results] == ["EXACT"]


def test_hybrid_fulltext_recovers_exact_term_from_the_matching_chunk(
    tmp_path: Path,
) -> None:
    embedder = FixtureEmbedder()
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        vector_keys = [f"VECTOR-{index}#c0" for index in range(1, 7)]
        store.upsert(
            keys=[*vector_keys, "EXACT#c0"],
            vectors=[*[([1.0, 0.0])] * 6, [0.0, 1.0]],
            metadatas=[
                *[
                    {
                        "title": f"Semantic neighbour {index}",
                        "creators": [],
                        "tags": [],
                        "chunk_idx": 0,
                        "chunk_text": "ordinary passage",
                    }
                    for index in range(1, 7)
                ],
                {
                    "title": "Unrelated title",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 0,
                    "chunk_text": "The Quuxzxyl doctrine controls this question.",
                },
            ],
            date_modified=["2026-01-01"] * 7,
        )

        vector_results = query_fulltext(
            "Quuxzxyl",
            store,
            embedder,
            top_k=1,
            candidate_pool=1,
            context_chunks=0,
            mode="vector",
        )
        hybrid_results = query_fulltext(
            "Quuxzxyl",
            store,
            embedder,
            top_k=1,
            candidate_pool=1,
            context_chunks=0,
            mode="hybrid",
        )

    assert [result["key"] for result in vector_results] != ["EXACT"]
    assert [result["key"] for result in hybrid_results] == ["EXACT"]
    assert hybrid_results[0]["chunk_text"] == ("The Quuxzxyl doctrine controls this question.")


def test_keyword_query_matches_a_chinese_term_inside_longer_text(tmp_path: Path) -> None:
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        store.upsert(
            keys=["CJK"],
            vectors=[[0.0, 1.0]],
            metadatas=[
                {
                    "title": "论商业秘密保护的边界",
                    "creators": ["张三"],
                    "tags": [],
                }
            ],
            date_modified=["2026-01-01"],
        )

        results = query("商业秘密", store, None, top_k=1, mode="keyword")
        short_name_results = query("张三", store, None, top_k=1, mode="keyword")

    assert [result["key"] for result in results] == ["CJK"]
    assert [result["key"] for result in short_name_results] == ["CJK"]


def test_existing_vector_database_backfills_the_fts_index(tmp_path: Path) -> None:
    db_path = tmp_path / "vectors.sqlite"
    with SQLiteVecStore(VectorStoreConfig(db_path=db_path, dim=2)) as store:
        store.upsert(
            keys=["EXISTING"],
            vectors=[[0.0, 1.0]],
            metadatas=[{"title": "Quuxzxyl doctrine", "creators": [], "tags": []}],
            date_modified=["2026-01-01"],
        )

    with sqlite3.connect(db_path) as conn:
        conn.execute("DROP TABLE fts_items")

    with SQLiteVecStore(VectorStoreConfig(db_path=db_path, dim=2)) as store:
        results = query("Quuxzxyl", store, None, top_k=1, mode="keyword")

    assert [result["key"] for result in results] == ["EXISTING"]


def test_existing_fts_schema_is_rebuilt_when_the_tokenizer_changes(tmp_path: Path) -> None:
    db_path = tmp_path / "vectors.sqlite"
    with SQLiteVecStore(VectorStoreConfig(db_path=db_path, dim=2)) as store:
        store.upsert(
            keys=["EXISTING"],
            vectors=[[0.0, 1.0]],
            metadatas=[{"title": "论商业秘密保护的边界", "creators": [], "tags": []}],
            date_modified=["2026-01-01"],
        )

    with sqlite3.connect(db_path) as conn:
        conn.execute("DROP TABLE fts_items")
        conn.execute(
            """
            CREATE VIRTUAL TABLE fts_items
            USING fts5(
                key UNINDEXED,
                parent_key UNINDEXED,
                title,
                creators,
                abstract,
                chunk_text,
                tokenize='unicode61'
            )
            """
        )
        conn.execute(
            """
            INSERT INTO fts_items (
                rowid, key, parent_key, title, creators, abstract, chunk_text
            ) VALUES (1, 'EXISTING', 'EXISTING', '论商业秘密保护的边界', '', '', '')
            """
        )

    with SQLiteVecStore(VectorStoreConfig(db_path=db_path, dim=2)) as store:
        results = query("商业秘密", store, None, top_k=1, mode="keyword")

    assert [result["key"] for result in results] == ["EXISTING"]


def test_vector_mode_preserves_distance_order_and_parent_dedup(tmp_path: Path) -> None:
    embedder = FixtureEmbedder()
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        store.upsert(
            keys=["PARENT#c0", "PARENT", "OTHER"],
            vectors=[[1.0, 0.0], [0.8, 0.0], [0.5, 0.0]],
            metadatas=[
                {
                    "title": "Best parent chunk",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 0,
                    "chunk_text": "best passage",
                },
                {"title": "Parent metadata", "creators": [], "tags": []},
                {"title": "Other metadata", "creators": [], "tags": []},
            ],
            date_modified=["2026-01-01"] * 3,
        )

        results = query("fixture", store, embedder, top_k=3, mode="vector")

    assert [result["key"] for result in results] == ["PARENT", "OTHER"]
    assert [result["title"] for result in results] == ["Best parent chunk", "Other metadata"]
    assert results[0]["distance"] == pytest.approx(0.0)
    assert results[1]["distance"] == pytest.approx(0.5)


def test_fulltext_vector_mode_preserves_best_chunk_and_context(tmp_path: Path) -> None:
    embedder = FixtureEmbedder()
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        store.upsert(
            keys=["PARENT#c0", "PARENT#c1", "OTHER#c0"],
            vectors=[[1.0, 0.0], [0.8, 0.0], [0.5, 0.0]],
            metadatas=[
                {
                    "title": "Parent",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 0,
                    "chunk_text": "best passage",
                },
                {
                    "title": "Parent",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 1,
                    "chunk_text": "following passage",
                },
                {
                    "title": "Other",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 0,
                    "chunk_text": "other passage",
                },
            ],
            date_modified=["2026-01-01"] * 3,
        )

        results = query_fulltext(
            "fixture",
            store,
            embedder,
            top_k=2,
            candidate_pool=2,
            context_chunks=1,
            mode="vector",
        )

    assert [result["key"] for result in results] == ["PARENT", "OTHER"]
    assert results[0]["chunk_idx"] == 0
    assert results[0]["chunk_text"] == "best passage"
    assert results[0]["context_after"] == ["following passage"]
    assert results[0]["distance"] == pytest.approx(0.0)
    assert results[1]["distance"] == pytest.approx(0.5)


def test_fulltext_rerank_runs_on_the_fused_chunk_pool(tmp_path: Path) -> None:
    embedder = FixtureRerankingEmbedder()
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        store.upsert(
            keys=["VECTOR#c0", "EXACT#c0"],
            vectors=[[1.0, 0.0], [0.0, 1.0]],
            metadatas=[
                {
                    "title": "Semantic neighbour",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 0,
                    "chunk_text": "ordinary passage",
                },
                {
                    "title": "Unrelated title",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 0,
                    "chunk_text": "The Quuxzxyl doctrine controls this question.",
                },
            ],
            date_modified=["2026-01-01", "2026-01-01"],
        )

        results = query_fulltext(
            "Quuxzxyl",
            store,
            embedder,
            top_k=1,
            candidate_pool=2,
            context_chunks=0,
            mode="hybrid",
            rerank=True,
        )

    assert [result["key"] for result in results] == ["EXACT"]
    assert results[0]["rerank_score"] == 0.99


def test_generic_query_rerank_receives_keyword_matching_chunk_text(tmp_path: Path) -> None:
    embedder = FixtureRerankingEmbedder()
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        store.upsert(
            keys=["VECTOR", "EXACT#c0"],
            vectors=[[1.0, 0.0], [0.0, 1.0]],
            metadatas=[
                {"title": "Semantic neighbour", "creators": [], "tags": []},
                {
                    "title": "Unrelated title",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 0,
                    "chunk_text": "The Quuxzxyl doctrine controls this question.",
                },
            ],
            date_modified=["2026-01-01", "2026-01-01"],
        )

        results = query(
            "Quuxzxyl",
            store,
            embedder,
            top_k=1,
            candidate_pool=2,
            mode="hybrid",
            rerank=True,
        )

    assert [result["key"] for result in results] == ["EXACT"]
    assert results[0]["rerank_score"] == 0.99


def test_hybrid_fulltext_returns_the_keyword_matching_chunk_for_a_shared_parent(
    tmp_path: Path,
) -> None:
    embedder = FixtureEmbedder()
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        store.upsert(
            keys=["PARENT#c0", "PARENT#c1"],
            vectors=[[1.0, 0.0], [0.0, 1.0]],
            metadatas=[
                {
                    "title": "Fixture",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 0,
                    "chunk_text": "ordinary passage",
                },
                {
                    "title": "Fixture",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 1,
                    "chunk_text": "The Quuxzxyl doctrine controls this question.",
                },
            ],
            date_modified=["2026-01-01", "2026-01-01"],
        )

        results = query_fulltext(
            "Quuxzxyl",
            store,
            embedder,
            top_k=1,
            candidate_pool=2,
            context_chunks=0,
            mode="hybrid",
        )

    assert results[0]["chunk_idx"] == 1
    assert results[0]["chunk_text"] == ("The Quuxzxyl doctrine controls this question.")


def test_keyword_cli_does_not_initialize_an_embedding_provider(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db_path = tmp_path / "vectors.sqlite"
    with SQLiteVecStore(VectorStoreConfig(db_path=db_path, dim=2)) as store:
        store.upsert(
            keys=["EXACT"],
            vectors=[[0.0, 1.0]],
            metadatas=[{"title": "Quuxzxyl doctrine", "creators": [], "tags": []}],
            date_modified=["2026-01-01"],
        )

    monkeypatch.setattr(vector_store_module, "DEFAULT_DB_PATH", db_path)
    monkeypatch.setattr(
        zsearch_cli,
        "make_embedder",
        lambda **_kwargs: pytest.fail("keyword mode initialized the embedder"),
    )

    result = CliRunner().invoke(
        zsearch_cli.main,
        ["query", "Quuxzxyl", "--mode", "keyword", "--json"],
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["meta"]["schema_version"] == "zotero-cli-agent-v1"
    assert [item["key"] for item in payload["data"]] == ["EXACT"]


def test_keyword_fulltext_cli_does_not_initialize_an_embedding_provider(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db_path = tmp_path / "vectors.sqlite"
    with SQLiteVecStore(VectorStoreConfig(db_path=db_path, dim=2)) as store:
        store.upsert(
            keys=["EXACT#c0"],
            vectors=[[0.0, 1.0]],
            metadatas=[
                {
                    "title": "Unrelated title",
                    "creators": [],
                    "tags": [],
                    "chunk_idx": 0,
                    "chunk_text": "The Quuxzxyl doctrine controls this question.",
                }
            ],
            date_modified=["2026-01-01"],
        )

    monkeypatch.setattr(vector_store_module, "DEFAULT_DB_PATH", db_path)
    monkeypatch.setattr(
        zfulltext_cli,
        "make_embedder",
        lambda **_kwargs: pytest.fail("keyword mode initialized the embedder"),
    )

    result = CliRunner().invoke(
        zfulltext_cli.main,
        ["query", "Quuxzxyl", "--mode", "keyword", "--context", "0", "--json"],
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["meta"]["schema_version"] == "zotero-cli-agent-v1"
    assert [item["key"] for item in payload["data"]] == ["EXACT"]


def test_schema_advertises_hybrid_default_for_both_query_surfaces() -> None:
    result = CliRunner().invoke(zsearch_cli.main, ["schema"])

    assert result.exit_code == 0, result.output
    commands = json.loads(result.output)["data"]["commands"]
    by_path = {f"{command['entry_point']}.{command['path']}": command for command in commands}
    for path in ("zsearch.query", "zfulltext.query"):
        mode = next(
            parameter for parameter in by_path[path]["parameters"] if parameter["name"] == "mode"
        )
        assert mode["flags"] == ["--mode"]
        assert mode["default"] == "hybrid"


def test_keyword_mcp_query_does_not_initialize_an_embedding_provider(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db_path = tmp_path / "vectors.sqlite"
    with SQLiteVecStore(VectorStoreConfig(db_path=db_path, dim=2)) as store:
        store.upsert(
            keys=["EXACT"],
            vectors=[[0.0, 1.0]],
            metadatas=[{"title": "Quuxzxyl doctrine", "creators": [], "tags": []}],
            date_modified=["2026-01-01"],
        )

    monkeypatch.setattr(vector_store_module, "DEFAULT_DB_PATH", db_path)
    monkeypatch.setattr(
        mcp_server,
        "make_embedder",
        lambda **_kwargs: pytest.fail("keyword mode initialized the embedder"),
    )

    payload = json.loads(mcp_server._query_tool("Quuxzxyl", mode="keyword"))

    assert [item["key"] for item in payload] == ["EXACT"]


def test_sync_incrementally_replaces_keyword_rows_when_fulltext_changes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    embedder = FixtureEmbedder()
    item = ZoteroItem(
        key="PARENT",
        item_type="book",
        title="Fixture",
        abstract=None,
        date="2026",
        doi=None,
        url=None,
        venue=None,
        publisher=None,
        creators=(),
        tags=(),
        date_modified="unchanged-parent-date",
    )
    old_text = "The Oldterm doctrine controls this question."
    new_text = "The Newterm doctrine controls this question."
    artifacts = {item.key: (old_text, hashlib.sha256(old_text.encode()).hexdigest())}
    monkeypatch.setattr(search_module, "iter_items", lambda _db_path: [item])
    monkeypatch.setattr(
        search_module,
        "resolve_fulltext_artifacts",
        lambda _keys, _db_path: artifacts,
    )

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        sync(store, embedder, db_path=tmp_path / "zotero.sqlite")
        assert [
            result["key"]
            for result in query_fulltext("Oldterm", store, None, mode="keyword", context_chunks=0)
        ] == ["PARENT"]

        artifacts[item.key] = (
            new_text,
            hashlib.sha256(new_text.encode()).hexdigest(),
        )
        sync(store, embedder, db_path=tmp_path / "zotero.sqlite")
        old_results = query_fulltext("Oldterm", store, None, mode="keyword", context_chunks=0)
        new_results = query_fulltext("Newterm", store, None, mode="keyword", context_chunks=0)

    assert old_results == []
    assert [result["key"] for result in new_results] == ["PARENT"]


def test_full_sync_preserves_item_scoped_keyword_rows(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    embedder = FixtureEmbedder()
    markdown = tmp_path / "artifact.md"
    markdown.write_text(
        "# Fixture\n\nThe Pipelineonly doctrine controls this question.",
        encoding="utf-8",
    )
    sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()
    item = ZoteroItem(
        key="PIPELINE",
        item_type="book",
        title="Zotero copy",
        abstract=None,
        date="2026",
        doi=None,
        url=None,
        venue=None,
        publisher=None,
        creators=(),
        tags=(),
        date_modified="different-parent-date",
    )
    zotero_text = "The Zoteroonly doctrine must not replace the launcher index."
    monkeypatch.setattr(search_module, "iter_items", lambda _db_path: [item])
    monkeypatch.setattr(
        search_module,
        "resolve_fulltext_artifacts",
        lambda _keys, _db_path: {
            item.key: (zotero_text, hashlib.sha256(zotero_text.encode()).hexdigest())
        },
    )

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        index_markdown_item(
            store,
            embedder,
            parent_item_key="PIPELINE",
            markdown_path=markdown,
            expected_sha256=sha256,
            chunk_contract_version="zfulltext-chunk-v2",
            embedding_profile_id="fixture:2",
        )
        sync(
            store,
            embedder,
            db_path=tmp_path / "zotero.sqlite",
            full=True,
        )
        results = query_fulltext("Pipelineonly", store, None, mode="keyword", context_chunks=0)
        zotero_results = query_fulltext(
            "Zoteroonly", store, None, mode="keyword", context_chunks=0
        )
        stored = store.item_chunk_metadatas("PIPELINE")

    assert [result["key"] for result in results] == ["PIPELINE"]
    assert zotero_results == []
    assert [chunk["index_source"] for chunk in stored] == ["item_scoped"]
    assert [chunk["chunk_contract_version"] for chunk in stored] == ["zfulltext-chunk-v2"]


def test_full_sync_rebuilds_sync_managed_keyword_rows(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    embedder = FixtureEmbedder()
    item = ZoteroItem(
        key="PARENT",
        item_type="book",
        title="Fixture",
        abstract=None,
        date="2026",
        doi=None,
        url=None,
        venue=None,
        publisher=None,
        creators=(),
        tags=(),
        date_modified="parent-date",
    )
    old_text = "The Oldfullterm doctrine controls this question."
    new_text = "The Newfullterm doctrine controls this question."
    artifacts = {item.key: (old_text, hashlib.sha256(old_text.encode()).hexdigest())}
    monkeypatch.setattr(search_module, "iter_items", lambda _db_path: [item])
    monkeypatch.setattr(
        search_module,
        "resolve_fulltext_artifacts",
        lambda _keys, _db_path: artifacts,
    )

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        sync(store, embedder, db_path=tmp_path / "zotero.sqlite")
        artifacts[item.key] = (
            new_text,
            hashlib.sha256(new_text.encode()).hexdigest(),
        )
        stats = sync(
            store,
            embedder,
            db_path=tmp_path / "zotero.sqlite",
            full=True,
        )
        old_results = query_fulltext("Oldfullterm", store, None, mode="keyword", context_chunks=0)
        new_results = query_fulltext("Newfullterm", store, None, mode="keyword", context_chunks=0)

    assert stats["chunks"] == 1
    assert old_results == []
    assert [result["key"] for result in new_results] == ["PARENT"]


def test_zsearch_sync_full_preserves_item_scoped_indexes(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    full_flags: list[bool] = []

    class ItemScopedStore:
        def __init__(self, cfg: VectorStoreConfig) -> None:
            self.cfg = cfg

        def __enter__(self) -> "ItemScopedStore":
            return self

        def __exit__(self, *_exc: object) -> None:
            return None

        def has_item_scoped_chunks(self) -> bool:
            return True

        def drop(self, dim: int | None = None) -> None:
            pytest.fail(f"full sync dropped item-scoped rows at dimension {dim}")

    def fake_sync(
        _store: ItemScopedStore,
        _embedder: FixtureEmbedder,
        **kwargs: object,
    ) -> dict:
        full_flags.append(bool(kwargs["full"]))
        return {"total": 0, "embedded": 0, "skipped": 0, "chunks": 0}

    monkeypatch.setattr(zsearch_cli, "SQLiteVecStore", ItemScopedStore)
    monkeypatch.setattr(zsearch_cli, "make_embedder", lambda **_kwargs: FixtureEmbedder())
    monkeypatch.setattr(zsearch_cli, "do_sync", fake_sync)

    result = CliRunner().invoke(zsearch_cli.main, ["sync", "--full"])

    assert result.exit_code == 0, result.output
    assert full_flags == [True]
