"""Resolve full-text content for Zotero items.

Priority: .md file in attachment storage > skip (PDF parsing is out of scope).
Chunking splits long documents into overlapping segments for embedding.
"""

from __future__ import annotations

import hashlib
import sqlite3
from pathlib import Path

from .zotero_db import DEFAULT_DB_PATH

ZOTERO_STORAGE = Path.home() / "Zotero" / "storage"
CHUNK_SIZE = 4000  # characters per chunk (~2000 CJK chars)
CHUNK_OVERLAP = 300


def _get_attachment_keys_by_parent(db_path: Path) -> dict[str, list[str]]:
    """Return all parent-to-storage-attachment keys in one read transaction."""
    uri = f"file:{db_path}?mode=ro&immutable=1"
    with sqlite3.connect(uri, uri=True) as conn:
        rows = conn.execute(
            """
            SELECT parent.key, i.key
            FROM itemAttachments ia
            JOIN items i ON i.itemID = ia.itemID
            JOIN items parent ON parent.itemID = ia.parentItemID
            WHERE ia.path LIKE 'storage:%'
            ORDER BY ia.parentItemID, ia.itemID
            """
        ).fetchall()
    by_parent: dict[str, list[str]] = {}
    for parent_key, attachment_key in rows:
        by_parent.setdefault(parent_key, []).append(attachment_key)
    return by_parent


def _resolve_attachment_keys(attachment_keys: list[str]) -> tuple[str, str] | None:
    for attachment_key in attachment_keys:
        storage_dir = ZOTERO_STORAGE / attachment_key
        if not storage_dir.is_dir():
            continue
        md_files = list(storage_dir.glob("*.md"))
        if md_files:
            # Pick the largest .md file (most likely the full content)
            md_file = max(md_files, key=lambda file: file.stat().st_size)
            try:
                raw = md_file.read_bytes()
                text = raw.decode("utf-8").replace("\r\n", "\n").replace("\r", "\n")
                return text, hashlib.sha256(raw).hexdigest()
            except (OSError, UnicodeDecodeError):
                continue
    return None


def resolve_fulltext_artifacts(
    parent_keys: list[str], db_path: Path | None = None
) -> dict[str, tuple[str, str]]:
    """Resolve Markdown artifacts for many parents with one SQLite connection."""
    requested = set(parent_keys)
    if not requested:
        return {}
    attachment_keys = _get_attachment_keys_by_parent(db_path or DEFAULT_DB_PATH)
    resolved: dict[str, tuple[str, str]] = {}
    for parent_key, keys in attachment_keys.items():
        if parent_key not in requested:
            continue
        artifact = _resolve_attachment_keys(keys)
        if artifact is not None:
            resolved[parent_key] = artifact
    return resolved


def resolve_fulltext_artifact(
    parent_key: str, db_path: Path | None = None
) -> tuple[str, str] | None:
    """Return normalized Markdown text and its raw-file SHA-256 identity.

    Args:
        parent_key: The Zotero item key (parent, not attachment).
        db_path: Override path to zotero.sqlite.

    Returns:
        ``(text, sha256)`` if an .md file is found, else None.
    """
    return resolve_fulltext_artifacts([parent_key], db_path).get(parent_key)


def resolve_fulltext(parent_key: str, db_path: Path | None = None) -> str | None:
    """Return normalized full-text content for an item, if available."""
    resolved = resolve_fulltext_artifact(parent_key, db_path)
    return resolved[0] if resolved is not None else None


def chunk_text(
    text: str,
    chunk_size: int = CHUNK_SIZE,
    overlap: int = CHUNK_OVERLAP,
) -> list[str]:
    """Split all text into overlapping chunks by paragraph boundaries."""
    paragraphs = text.split("\n\n")
    chunks: list[str] = []
    current = ""

    for para in paragraphs:
        para = para.strip()
        if not para:
            continue
        if len(current) + len(para) + 2 <= chunk_size:
            current = f"{current}\n\n{para}" if current else para
        else:
            if current:
                chunks.append(current)
            if len(para) > chunk_size:
                for i in range(0, len(para), chunk_size - overlap):
                    chunks.append(para[i : i + chunk_size])
            else:
                current = para
                continue
            current = ""

    if current:
        chunks.append(current)

    # Apply overlap
    if overlap > 0 and len(chunks) > 1:
        overlapped: list[str] = [chunks[0]]
        for i in range(1, len(chunks)):
            prev_tail = chunks[i - 1][-overlap:]
            overlapped.append(prev_tail + chunks[i])
        chunks = overlapped

    return chunks
