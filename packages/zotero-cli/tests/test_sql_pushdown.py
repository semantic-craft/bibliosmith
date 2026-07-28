from __future__ import annotations

from pathlib import Path
import sqlite3

import pytest

from zotero_cli.embed import EmbedConfig
import zotero_cli.fulltext as fulltext_module
from zotero_cli.search import query, query_fulltext, sync
from zotero_cli.vector_store import SQLiteVecStore, VectorStoreConfig
import zotero_cli.zotero_db as zotero_db


class FixtureEmbedder:
    def __init__(self) -> None:
        self.cfg = EmbedConfig(model="fixture", dimensions=2, batch_size=32)

    def embed(self, texts: list[str]) -> list[list[float]]:
        return [[1.0, 0.0] for _ in texts]

    def embed_query(self, _query: str) -> list[float]:
        return [1.0, 0.0]


def _zotero_fixture(tmp_path: Path) -> Path:
    db_path = tmp_path / "zotero.sqlite"
    with sqlite3.connect(db_path) as conn:
        conn.executescript(
            """
            CREATE TABLE itemTypes (
                itemTypeID INTEGER PRIMARY KEY,
                typeName TEXT NOT NULL
            );
            CREATE TABLE items (
                itemID INTEGER PRIMARY KEY,
                key TEXT NOT NULL,
                itemTypeID INTEGER NOT NULL,
                dateModified TEXT NOT NULL
            );
            CREATE TABLE deletedItems (itemID INTEGER PRIMARY KEY);
            CREATE TABLE fields (
                fieldID INTEGER PRIMARY KEY,
                fieldName TEXT NOT NULL
            );
            CREATE TABLE itemDataValues (
                valueID INTEGER PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE itemData (
                itemID INTEGER NOT NULL,
                fieldID INTEGER NOT NULL,
                valueID INTEGER NOT NULL
            );
            CREATE TABLE creators (
                creatorID INTEGER PRIMARY KEY,
                firstName TEXT,
                lastName TEXT
            );
            CREATE TABLE itemCreators (
                itemID INTEGER NOT NULL,
                creatorID INTEGER NOT NULL,
                orderIndex INTEGER NOT NULL
            );
            CREATE TABLE tags (
                tagID INTEGER PRIMARY KEY,
                name TEXT NOT NULL
            );
            CREATE TABLE itemTags (
                itemID INTEGER NOT NULL,
                tagID INTEGER NOT NULL
            );
            CREATE TABLE itemAttachments (
                itemID INTEGER PRIMARY KEY,
                parentItemID INTEGER NOT NULL,
                path TEXT
            );

            INSERT INTO itemTypes VALUES (1, 'book'), (2, 'journalArticle'), (3, 'attachment');
            INSERT INTO fields VALUES
                (1, 'title'),
                (2, 'abstractNote'),
                (3, 'date'),
                (4, 'DOI'),
                (5, 'url'),
                (6, 'publicationTitle'),
                (7, 'publisher');
            INSERT INTO items VALUES
                (1, 'OLD', 1, '2026-01-01'),
                (2, 'CREATOR', 2, '2026-03-01'),
                (3, 'TITLE', 1, '2026-02-01'),
                (4, 'ATTACH', 3, '2026-03-02');
            INSERT INTO itemDataValues VALUES
                (1, 'Alpha'),
                (2, 'ordinary abstract'),
                (3, 'published 2021'),
                (4, 'No keyword here'),
                (5, 'revised 2024'),
                (6, 'Needle title'),
                (7, 'A 100% literal_match'),
                (8, 'circa 1985, revised');
            INSERT INTO itemData VALUES
                (1, 1, 1), (1, 2, 2), (1, 3, 3),
                (2, 1, 4), (2, 3, 5),
                (3, 1, 6), (3, 2, 7), (3, 3, 8);
            INSERT INTO creators VALUES
                (1, 'Old', 'Author'),
                (2, 'Ada', 'Needle'),
                (3, 'Title', 'Author');
            INSERT INTO itemCreators VALUES
                (1, 1, 0), (2, 2, 0), (3, 3, 0);
            INSERT INTO tags VALUES (1, 'Common'), (2, 'Rare');
            INSERT INTO itemTags VALUES (1, 1), (2, 2), (3, 2);
            INSERT INTO itemAttachments VALUES (4, 3, 'storage:artifact.md');
            """
        )
    return db_path


def test_recent_and_grep_match_existing_order_without_calling_iter_items(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    db_path = _zotero_fixture(tmp_path)
    all_items = zotero_db.iter_items(db_path)
    expected_recent = all_items[:2]
    legacy_grep = [
        item
        for item in all_items
        if "needle" in " ".join(filter(None, [item.title, item.abstract])).lower()
    ]

    monkeypatch.setattr(
        zotero_db,
        "iter_items",
        lambda *_args, **_kwargs: pytest.fail("read-path query fell back to a full scan"),
    )

    recent = zotero_db.recent_items(db_path, n=2)
    grep = zotero_db.grep_items("Needle", db_path, limit=10)

    assert recent == expected_recent
    assert [item.key for item in grep] == ["CREATOR", "TITLE"]
    assert {item.key for item in legacy_grep} <= {item.key for item in grep}
    assert [item.key for item in zotero_db.grep_items("% literal_", db_path)] == ["TITLE"]


def test_item_filter_keys_preserve_type_year_and_exact_tag_semantics(tmp_path: Path) -> None:
    db_path = _zotero_fixture(tmp_path)
    with sqlite3.connect(db_path) as conn:
        conn.execute("INSERT INTO items VALUES (5, 'NODATE', 1, '2026-04-01')")

    assert zotero_db.filtered_item_keys(
        db_path,
        item_type="book",
        year=(1980, 1990),
        tag="Rare",
    ) == {"TITLE"}
    assert zotero_db.filtered_item_keys(db_path, tag="rare") == set()
    assert zotero_db.filtered_item_keys(db_path, year=(2025, None)) == set()
    assert zotero_db.filtered_item_keys(db_path, year=(None, None)) == {
        "OLD",
        "CREATOR",
        "TITLE",
    }


def _populate_crowded_store(store: SQLiteVecStore) -> None:
    keys: list[str] = []
    vectors: list[list[float]] = []
    metadatas: list[dict] = []
    date_modified: list[str] = []
    for index in range(205):
        key = f"D{index:03d}"
        metadata = {
            "item_type": "book",
            "date": "2024",
            "tags": ["Other"],
            "title": key,
        }
        for stored_key, chunk_idx in ((key, None), (f"{key}#c0", 0)):
            keys.append(stored_key)
            vectors.append([1.0, 0.0])
            metadatas.append(
                metadata
                if chunk_idx is None
                else {
                    **metadata,
                    "chunk_idx": chunk_idx,
                    "is_chunk": True,
                    "chunk_text": f"distractor {key}",
                }
            )
            date_modified.append("2026-01-01")
    for key in ("CREATOR", "TITLE"):
        metadata = {
            "item_type": "book" if key == "TITLE" else "journalArticle",
            "date": "1985" if key == "TITLE" else "2024",
            "tags": ["Rare"],
            "title": key,
        }
        for stored_key, chunk_idx in ((key, None), (f"{key}#c0", 0)):
            keys.append(stored_key)
            vectors.append([0.0, 1.0])
            metadatas.append(
                metadata
                if chunk_idx is None
                else {
                    **metadata,
                    "chunk_idx": chunk_idx,
                    "is_chunk": True,
                    "chunk_text": f"rare {key}",
                }
            )
            date_modified.append("2026-01-01")
    store.upsert(keys, vectors, metadatas, date_modified)


def test_selective_tag_prefilters_vector_and_fulltext_queries(tmp_path: Path) -> None:
    db_path = _zotero_fixture(tmp_path)
    embedder = FixtureEmbedder()
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        _populate_crowded_store(store)

        metadata_results = query(
            "needle",
            store,
            embedder,
            top_k=2,
            tag="Rare",
            candidate_pool=1,
            db_path=db_path,
        )
        fulltext_results = query_fulltext(
            "needle",
            store,
            embedder,
            top_k=2,
            tag="Rare",
            candidate_pool=1,
            db_path=db_path,
            context_chunks=0,
        )

    assert {result["key"] for result in metadata_results} == {"CREATOR", "TITLE"}
    assert {result["key"] for result in fulltext_results} == {"CREATOR", "TITLE"}


def test_sync_uses_constant_zotero_connection_count(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    db_path = _zotero_fixture(tmp_path)
    storage_root = tmp_path / "storage"
    attachment_dir = storage_root / "ATTACH"
    attachment_dir.mkdir(parents=True)
    (attachment_dir / "artifact.md").write_text("full text", encoding="utf-8")
    monkeypatch.setattr(fulltext_module, "ZOTERO_STORAGE", storage_root)
    embedder = FixtureEmbedder()

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=2)) as store:
        connection_count = 0
        original_connect = sqlite3.connect

        def counted_connect(*args: object, **kwargs: object) -> sqlite3.Connection:
            nonlocal connection_count
            connection_count += 1
            return original_connect(*args, **kwargs)

        monkeypatch.setattr(sqlite3, "connect", counted_connect)
        stats = sync(store, embedder, db_path=db_path)

    assert stats["total"] == 3
    assert stats["chunks"] == 1
    assert connection_count == 2
