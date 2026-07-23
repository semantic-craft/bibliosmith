from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sqlite3
from types import SimpleNamespace

from click.testing import CliRunner
import pytest

from zotero_cli.embed import EmbedConfig
import zotero_cli.search as search_module
from zotero_cli.search import index_markdown_item, query_fulltext, sync
from zotero_cli.vector_store import SQLiteVecStore, VectorStoreConfig
from zotero_cli.zotero_db import ZoteroItem
from zotero_cli.zfulltext_cli import main as zfulltext


class FixtureEmbedder:
    def __init__(self) -> None:
        self.cfg = EmbedConfig(model="fixture-embedding", dimensions=3, batch_size=2)
        self.calls: list[list[str]] = []

    def embed(self, texts: list[str]) -> list[list[float]]:
        self.calls.append(texts)
        return [[1.0, 0.0, 0.0] for _ in texts]

    def embed_query(self, _query: str) -> list[float]:
        return [1.0, 0.0, 0.0]

    def __enter__(self) -> FixtureEmbedder:
        return self

    def __exit__(self, *_exc: object) -> None:
        return None


class ContextFixtureEmbedder(FixtureEmbedder):
    def embed(self, texts: list[str]) -> list[list[float]]:
        self.calls.append(texts)
        return [[1.0, 0.0, 0.0] if "first-needle" in text else [0.0, 1.0, 0.0] for text in texts]


class WrongDimensionEmbedder(FixtureEmbedder):
    def embed(self, texts: list[str]) -> list[list[float]]:
        self.calls.append(texts)
        return [[1.0, 0.0] for _ in texts]


def test_item_index_makes_markdown_immediately_queryable(tmp_path: Path) -> None:
    markdown = tmp_path / "artifact.md"
    markdown.write_text(
        "# Fixture\n\nThis text contains the immediate-index-needle without a Zotero sidecar.\n",
        encoding="utf-8",
    )
    sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()
    embedder = FixtureEmbedder()

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=3)) as store:
        evidence = index_markdown_item(
            store,
            embedder,
            parent_item_key="PARENT123",
            markdown_path=markdown,
            expected_sha256=sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
            metadata={"title": "Fixture", "creators": [], "tags": []},
        )
        results = query_fulltext(
            "immediate-index-needle",
            store,
            embedder,
            top_k=1,
            db_path=tmp_path / "missing-zotero.sqlite",
            context_chunks=0,
        )

    assert {key: value for key, value in evidence.items() if key != "completedAt"} == {
        "parentItemKey": "PARENT123",
        "sourceSha256": sha256,
        "chunkCount": 1,
        "indexContractVersion": "zfulltext-item-index-v1",
        "chunkContractVersion": "zfulltext-chunk-v1",
        "embeddingProfileId": "fixture-embedding:3",
        "reused": False,
    }
    assert evidence["completedAt"].endswith("Z")
    assert results[0]["key"] == "PARENT123"
    assert "immediate-index-needle" in results[0]["chunk_text"]


def test_item_index_reuses_identical_contract_without_reembedding(tmp_path: Path) -> None:
    markdown = tmp_path / "artifact.md"
    markdown.write_text("# Fixture\n\nStable content for an idempotent index.\n", encoding="utf-8")
    sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()
    embedder = FixtureEmbedder()

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=3)) as store:
        first = index_markdown_item(
            store,
            embedder,
            parent_item_key="PARENT123",
            markdown_path=markdown,
            expected_sha256=sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
        )
        calls_after_first = len(embedder.calls)
        second = index_markdown_item(
            store,
            embedder,
            parent_item_key="PARENT123",
            markdown_path=markdown,
            expected_sha256=sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
        )

    assert first["reused"] is False
    assert second["reused"] is True
    assert len(embedder.calls) == calls_after_first


def test_item_index_replaces_old_chunks_when_markdown_hash_changes(tmp_path: Path) -> None:
    markdown = tmp_path / "artifact.md"
    markdown.write_text("# Fixture\n\nold-index-needle\n", encoding="utf-8")
    old_sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()
    embedder = FixtureEmbedder()

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=3)) as store:
        index_markdown_item(
            store,
            embedder,
            parent_item_key="PARENT123",
            markdown_path=markdown,
            expected_sha256=old_sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
        )
        markdown.write_text("# Fixture\n\nnew-index-needle\n", encoding="utf-8")
        new_sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()
        evidence = index_markdown_item(
            store,
            embedder,
            parent_item_key="PARENT123",
            markdown_path=markdown,
            expected_sha256=new_sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
        )
        stored = store.item_chunk_metadatas("PARENT123")

    assert evidence["reused"] is False
    assert evidence["sourceSha256"] == new_sha256
    assert [item["source_sha256"] for item in stored] == [new_sha256]
    assert "new-index-needle" in stored[0]["chunk_text"]
    assert "old-index-needle" not in stored[0]["chunk_text"]


def test_item_index_keeps_previous_chunks_when_replacement_write_fails(tmp_path: Path) -> None:
    markdown = tmp_path / "artifact.md"
    markdown.write_text("# Fixture\n\nstable-old-index\n", encoding="utf-8")
    old_sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=3)) as store:
        index_markdown_item(
            store,
            FixtureEmbedder(),
            parent_item_key="PARENT123",
            markdown_path=markdown,
            expected_sha256=old_sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
        )
        markdown.write_text("# Fixture\n\nfailed-new-index\n", encoding="utf-8")
        new_sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()

        with pytest.raises(Exception):
            index_markdown_item(
                store,
                WrongDimensionEmbedder(),
                parent_item_key="PARENT123",
                markdown_path=markdown,
                expected_sha256=new_sha256,
                chunk_contract_version="zfulltext-chunk-v1",
                embedding_profile_id="fixture-embedding:2",
            )

        stored = store.item_chunk_metadatas("PARENT123")

    assert [item["source_sha256"] for item in stored] == [old_sha256]
    assert "stable-old-index" in stored[0]["chunk_text"]


def test_zfulltext_exposes_item_scoped_index_contract() -> None:
    result = CliRunner().invoke(zfulltext, ["index", "--help"])

    assert result.exit_code == 0
    for option in (
        "--parent-item-key",
        "--markdown",
        "--sha256",
        "--chunk-contract-version",
        "--embedding-profile-id",
    ):
        assert option in result.output


def test_zfulltext_profile_reports_active_non_secret_identity(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import zotero_cli.zfulltext_cli as zfulltext_cli

    class FakeProfileStore:
        def __init__(self, _cfg) -> None:
            self.cfg = SimpleNamespace(dim=3)

        def __enter__(self) -> "FakeProfileStore":
            return self

        def __exit__(self, *_exc: object) -> None:
            return None

    monkeypatch.setattr(zfulltext_cli, "SQLiteVecStore", FakeProfileStore)
    monkeypatch.setattr(
        zfulltext_cli,
        "resolve_embedder_config",
        lambda *_args, dimensions=None, **_kwargs: (
            "fixture",
            EmbedConfig(model="fixture-embedding", dimensions=dimensions or 3),
        ),
    )

    result = CliRunner().invoke(zfulltext_cli.main, ["profile"])

    assert result.exit_code == 0
    assert json.loads(result.output) == {"embeddingProfileId": "fixture-embedding:3"}


def test_zfulltext_index_rejects_a_profile_that_is_not_active(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import zotero_cli.zfulltext_cli as zfulltext_cli

    class FakeProfileStore:
        def __init__(self, _cfg) -> None:
            self.cfg = SimpleNamespace(dim=3)

        def __enter__(self) -> "FakeProfileStore":
            return self

        def __exit__(self, *_exc: object) -> None:
            return None

    markdown = tmp_path / "artifact.md"
    markdown.write_text("# Fixture\n", encoding="utf-8")
    sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()
    monkeypatch.setattr(zfulltext_cli, "SQLiteVecStore", FakeProfileStore)
    monkeypatch.setattr(zfulltext_cli, "make_embedder", lambda **_kwargs: FixtureEmbedder())

    result = CliRunner().invoke(
        zfulltext_cli.main,
        [
            "index",
            "--parent-item-key",
            "PARENT123",
            "--markdown",
            str(markdown),
            "--sha256",
            sha256,
            "--chunk-contract-version",
            "zfulltext-chunk-v1",
            "--embedding-profile-id",
            "different-profile:3",
        ],
    )

    assert result.exit_code == 1
    assert "embedding profile mismatch" in result.output


def test_item_index_query_returns_context_from_the_local_index(tmp_path: Path) -> None:
    markdown = tmp_path / "artifact.md"
    markdown.write_text(
        "first-needle " + ("a" * 3500) + "\n\nsecond-context " + ("b" * 1200) + "\n",
        encoding="utf-8",
    )
    sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()
    embedder = ContextFixtureEmbedder()

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=3)) as store:
        index_markdown_item(
            store,
            embedder,
            parent_item_key="PARENT123",
            markdown_path=markdown,
            expected_sha256=sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
        )
        results = query_fulltext(
            "first-needle",
            store,
            embedder,
            top_k=1,
            db_path=tmp_path / "missing-zotero.sqlite",
            context_chunks=1,
        )

    assert "first-needle" in results[0]["chunk_text"]
    assert any("second-context" in chunk for chunk in results[0]["context_after"])


def test_sync_replaces_changed_fulltext_when_parent_date_is_unchanged(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    markdown = tmp_path / "artifact.md"
    old_text = "# Fixture\n\nold-sync-needle"
    markdown.write_text(old_text, encoding="utf-8")
    old_sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()
    embedder = FixtureEmbedder()
    item = ZoteroItem(
        key="PARENT123",
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

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=3)) as store:
        index_markdown_item(
            store,
            embedder,
            parent_item_key=item.key,
            markdown_path=markdown,
            expected_sha256=old_sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
        )
        store.upsert(
            keys=[item.key],
            vectors=[[1.0, 0.0, 0.0]],
            metadatas=[{"title": item.title}],
            date_modified=[item.date_modified],
        )
        new_text = "# Fixture\n\nnew-sync-needle"
        new_sha256 = hashlib.sha256(new_text.encode("utf-8")).hexdigest()
        monkeypatch.setattr(search_module, "iter_items", lambda _db_path: [item])
        monkeypatch.setattr(
            search_module,
            "resolve_fulltext_artifact",
            lambda _key, _db_path: (new_text, new_sha256),
        )

        stats = sync(store, embedder, db_path=tmp_path / "zotero.sqlite")
        stored = store.item_chunk_metadatas(item.key)

    assert stats["embedded"] == 0
    assert stats["chunks"] == 1
    assert [chunk["source_sha256"] for chunk in stored] == [new_sha256]
    assert "new-sync-needle" in stored[0]["chunk_text"]
    assert "old-sync-needle" not in stored[0]["chunk_text"]


def test_full_sync_preserves_item_scoped_chunks_before_zotero_storage_catches_up(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    markdown = tmp_path / "artifact.md"
    markdown.write_text(
        "# Fixture\n\n" + ("pipeline-only-needle " * 30),
        encoding="utf-8",
    )
    sha256 = hashlib.sha256(markdown.read_bytes()).hexdigest()
    embedder = FixtureEmbedder()
    item = ZoteroItem(
        key="PARENT123",
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

    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=3)) as store:
        index_markdown_item(
            store,
            embedder,
            parent_item_key=item.key,
            markdown_path=markdown,
            expected_sha256=sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
        )
        monkeypatch.setattr(search_module, "iter_items", lambda _db_path: [item])
        monkeypatch.setattr(search_module, "resolve_fulltext_artifact", lambda _key, _db_path: None)

        sync(store, embedder, db_path=tmp_path / "zotero.sqlite", full=True)
        stored = store.item_chunk_metadatas(item.key)

    assert [chunk["source_sha256"] for chunk in stored] == [sha256]
    assert "pipeline-only-needle" in stored[0]["chunk_text"]


def test_sync_reuses_crlf_item_index_with_raw_artifact_sha256(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from zotero_cli.fulltext import resolve_fulltext_artifact
    import zotero_cli.fulltext as fulltext_module

    zotero_db = tmp_path / "zotero.sqlite"
    with sqlite3.connect(zotero_db) as conn:
        conn.executescript(
            """
            CREATE TABLE items (itemID INTEGER PRIMARY KEY, key TEXT NOT NULL);
            CREATE TABLE itemAttachments (
                itemID INTEGER PRIMARY KEY,
                parentItemID INTEGER NOT NULL,
                path TEXT NOT NULL
            );
            INSERT INTO items (itemID, key) VALUES (1, 'PARENT123'), (2, 'ATTACH123');
            INSERT INTO itemAttachments (itemID, parentItemID, path)
            VALUES (2, 1, 'storage:artifact.md');
            """
        )
    storage_root = tmp_path / "storage"
    attachment_dir = storage_root / "ATTACH123"
    attachment_dir.mkdir(parents=True)
    markdown = attachment_dir / "artifact.md"
    raw_markdown = b"# Fixture\r\n\r\ncrlf-identity-needle\r\n"
    markdown.write_bytes(raw_markdown)
    raw_sha256 = hashlib.sha256(raw_markdown).hexdigest()
    monkeypatch.setattr(fulltext_module, "ZOTERO_STORAGE", storage_root)

    resolved = resolve_fulltext_artifact("PARENT123", zotero_db)

    assert resolved == ("# Fixture\n\ncrlf-identity-needle\n", raw_sha256)

    embedder = FixtureEmbedder()
    item = ZoteroItem(
        key="PARENT123",
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
    with SQLiteVecStore(VectorStoreConfig(db_path=tmp_path / "vectors.sqlite", dim=3)) as store:
        index_markdown_item(
            store,
            embedder,
            parent_item_key=item.key,
            markdown_path=markdown,
            expected_sha256=raw_sha256,
            chunk_contract_version="zfulltext-chunk-v1",
            embedding_profile_id="fixture-embedding:3",
        )
        store.upsert(
            keys=[item.key],
            vectors=[[1.0, 0.0, 0.0]],
            metadatas=[{"title": item.title}],
            date_modified=[item.date_modified],
        )
        monkeypatch.setattr(search_module, "iter_items", lambda _db_path: [item])

        stats = sync(store, embedder, db_path=zotero_db)
        stored = store.item_chunk_metadatas(item.key)

    assert stats["chunks"] == 0
    assert [chunk["source_sha256"] for chunk in stored] == [raw_sha256]
