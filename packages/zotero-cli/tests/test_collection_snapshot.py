from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sqlite3

from click.testing import CliRunner

import zotero_cli.zotero_db as zotero_db
from zotero_cli.cli import main as zsearch


def _fixture(tmp_path: Path) -> tuple[Path, Path]:
    db_path = tmp_path / "zotero.sqlite"
    with sqlite3.connect(db_path) as conn:
        conn.executescript(
            """
            CREATE TABLE collections (
                collectionID INTEGER PRIMARY KEY,
                key TEXT NOT NULL,
                collectionName TEXT NOT NULL,
                parentCollectionID INTEGER,
                version INTEGER NOT NULL
            );
            CREATE TABLE deletedCollections (collectionID INTEGER PRIMARY KEY);
            CREATE TABLE collectionItems (collectionID INTEGER NOT NULL, itemID INTEGER NOT NULL);
            CREATE TABLE itemTypes (itemTypeID INTEGER PRIMARY KEY, typeName TEXT NOT NULL);
            CREATE TABLE items (
                itemID INTEGER PRIMARY KEY,
                key TEXT NOT NULL,
                itemTypeID INTEGER NOT NULL,
                dateModified TEXT NOT NULL,
                version INTEGER NOT NULL
            );
            CREATE TABLE deletedItems (itemID INTEGER PRIMARY KEY);
            CREATE TABLE itemAttachments (
                itemID INTEGER PRIMARY KEY,
                parentItemID INTEGER NOT NULL,
                linkMode INTEGER NOT NULL,
                contentType TEXT,
                path TEXT
            );
            CREATE TABLE fields (fieldID INTEGER PRIMARY KEY, fieldName TEXT NOT NULL);
            CREATE TABLE itemDataValues (valueID INTEGER PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE itemData (itemID INTEGER NOT NULL, fieldID INTEGER NOT NULL, valueID INTEGER NOT NULL);

            INSERT INTO collections VALUES (1, 'COLL1', 'Direct collection', NULL, 11);
            INSERT INTO collections VALUES (2, 'SUB1', 'Nested collection', 1, 3);
            INSERT INTO itemTypes VALUES (1, 'book'), (2, 'attachment');
            INSERT INTO fields VALUES (1, 'title');

            INSERT INTO items VALUES
                (10, 'PARENT1', 1, '2026-07-15 10:00:00', 7),
                (11, 'PARENT2', 1, '2026-07-15 10:01:00', 8),
                (12, 'PARENT3', 1, '2026-07-15 10:02:00', 9),
                (13, 'NESTED1', 1, '2026-07-15 10:03:00', 10),
                (20, 'PDFOK', 2, '2026-07-15 11:00:00', 21),
                (21, 'TEXTOther', 2, '2026-07-15 11:01:00', 22),
                (22, 'PDFMISSING', 2, '2026-07-15 11:02:00', 23),
                (23, 'NESTEDPDF', 2, '2026-07-15 11:03:00', 24);
            INSERT INTO collectionItems VALUES (1, 10), (1, 11), (1, 12), (2, 13);
            INSERT INTO itemAttachments VALUES
                (20, 10, 0, 'application/pdf', 'storage:ok.pdf'),
                (21, 10, 0, 'text/plain', 'storage:notes.txt'),
                (22, 11, 0, 'application/pdf', 'storage:missing.pdf'),
                (23, 13, 0, 'application/pdf', 'storage:nested.pdf');

            INSERT INTO itemDataValues VALUES
                (1, 'Parent One'), (2, 'Parent Two'), (3, 'Parent Three'), (4, 'Nested Parent');
            INSERT INTO itemData VALUES (10, 1, 1), (11, 1, 2), (12, 1, 3), (13, 1, 4);
            """
        )

    storage_root = tmp_path / "storage"
    for key, filename, content in (
        ("PDFOK", "ok.pdf", b"%PDF fixture"),
        ("TEXTOther", "notes.txt", b"notes"),
        ("NESTEDPDF", "nested.pdf", b"%PDF nested"),
    ):
        directory = storage_root / key
        directory.mkdir(parents=True)
        (directory / filename).write_bytes(content)
    return db_path, storage_root


def test_collection_snapshot_is_readonly_direct_ordered_and_explainable(
    tmp_path: Path, monkeypatch
) -> None:  # type: ignore[no-untyped-def]
    db_path, storage_root = _fixture(tmp_path)
    before = hashlib.sha256(db_path.read_bytes()).hexdigest()
    connect_calls = 0
    original_connect = zotero_db._connect_snapshot

    def counted_connect(path: Path):  # type: ignore[no-untyped-def]
        nonlocal connect_calls
        connect_calls += 1
        return original_connect(path)

    monkeypatch.setattr(zotero_db, "_connect_snapshot", counted_connect)

    snapshot = zotero_db.snapshot_collection("COLL1", db_path=db_path, storage_root=storage_root)

    assert connect_calls == 1
    assert hashlib.sha256(db_path.read_bytes()).hexdigest() == before
    assert snapshot.collection.key == "COLL1"
    assert snapshot.collection.name == "Direct collection"
    assert snapshot.collection.version == 11
    assert [member.attachment_key for member in snapshot.members] == [
        "PDFOK",
        "TEXTOther",
        "PDFMISSING",
        None,
    ]
    assert [member.eligibility for member in snapshot.members] == [
        "eligible_pdf",
        "unsupported_content_type",
        "missing_file",
        "no_attachment",
    ]
    assert snapshot.members[0].path_exists is True
    assert snapshot.members[0].file_size == len(b"%PDF fixture")
    assert snapshot.members[0].file_mtime_ns is not None
    assert snapshot.members[2].reason == "PDF attachment file is missing."
    assert all(member.parent_item_key != "NESTED1" for member in snapshot.members)


def test_collection_snapshot_cli_is_deterministic_json(tmp_path: Path, monkeypatch) -> None:  # type: ignore[no-untyped-def]
    db_path, storage_root = _fixture(tmp_path)
    monkeypatch.setattr(zotero_db, "ZOTERO_STORAGE", storage_root)

    first = CliRunner().invoke(zsearch, ["collection-snapshot", "COLL1", "--db", str(db_path)])
    second = CliRunner().invoke(zsearch, ["collection-snapshot", "COLL1", "--db", str(db_path)])

    assert first.exit_code == 0
    assert second.exit_code == 0
    assert first.output == second.output
    payload = json.loads(first.output)
    assert payload["schemaVersion"] == "zotero-collection-snapshot-v1"
    assert payload["collection"]["key"] == "COLL1"
    assert {member["collectionKey"] for member in payload["members"]} == {"COLL1"}
    assert [member["parentItemKey"] for member in payload["members"]] == [
        "PARENT1",
        "PARENT1",
        "PARENT2",
        "PARENT3",
    ]
    assert "NESTED1" not in first.output


def test_collection_snapshot_remains_readable_during_zotero_exclusive_lock(
    tmp_path: Path,
) -> None:
    db_path, storage_root = _fixture(tmp_path)
    with sqlite3.connect(db_path) as setup:
        setup.execute("CREATE TABLE writePressure (id INTEGER PRIMARY KEY, payload TEXT)")
        setup.executemany(
            "INSERT INTO writePressure (payload) VALUES (?)",
            [("before" * 700,)] * 512,
        )
    writer = sqlite3.connect(db_path)
    writer.execute("PRAGMA cache_size = 4")
    writer.execute("PRAGMA locking_mode = EXCLUSIVE")
    writer.execute("BEGIN EXCLUSIVE")
    writer.execute("UPDATE collections SET version = 12 WHERE key = 'COLL1'")
    writer.execute("DELETE FROM collectionItems WHERE collectionID = 1 AND itemID = 12")
    writer.execute("UPDATE writePressure SET payload = replace(payload, 'before', 'after!')")
    try:
        snapshot = zotero_db.snapshot_collection(
            "COLL1", db_path=db_path, storage_root=storage_root
        )
    finally:
        writer.rollback()
        writer.close()

    assert snapshot.collection.key == "COLL1"
    assert snapshot.collection.version == 11
    assert {member.parent_item_key for member in snapshot.members} == {
        "PARENT1",
        "PARENT2",
        "PARENT3",
    }
    assert [member.eligibility for member in snapshot.members] == [
        "eligible_pdf",
        "unsupported_content_type",
        "missing_file",
        "no_attachment",
    ]


def test_collection_snapshot_reads_identity_and_membership_in_one_statement(
    tmp_path: Path, monkeypatch
) -> None:  # type: ignore[no-untyped-def]
    db_path, storage_root = _fixture(tmp_path)
    original_connect = zotero_db._connect_snapshot
    select_count = 0

    class CountingConnection:
        def __init__(self, path: Path) -> None:
            self._connection = original_connect(path)

        def execute(self, sql: str, parameters=()):  # type: ignore[no-untyped-def]
            nonlocal select_count
            if sql.lstrip().upper().startswith("SELECT"):
                select_count += 1
            return self._connection.execute(sql, parameters)

        def close(self) -> None:
            self._connection.close()

    monkeypatch.setattr(zotero_db, "_connect_snapshot", CountingConnection)

    snapshot = zotero_db.snapshot_collection(
        "COLL1", db_path=db_path, storage_root=storage_root
    )

    assert select_count == 1
    assert snapshot.collection.version == 11
    assert {member.parent_item_key for member in snapshot.members} == {
        "PARENT1",
        "PARENT2",
        "PARENT3",
    }


def test_collection_snapshot_retries_until_adjacent_physical_reads_agree(
    tmp_path: Path, monkeypatch
) -> None:  # type: ignore[no-untyped-def]
    db_path, storage_root = _fixture(tmp_path)
    original_copyfile = zotero_db.shutil.copyfile
    main_copy_count = 0

    def copy_and_commit(source: Path, destination: Path):  # type: ignore[no-untyped-def]
        nonlocal main_copy_count
        result = original_copyfile(source, destination)
        if Path(source) == db_path:
            main_copy_count += 1
            if main_copy_count == 1:
                with sqlite3.connect(db_path) as writer:
                    writer.execute("UPDATE collections SET version = 12 WHERE key = 'COLL1'")
        return result

    monkeypatch.setattr(zotero_db.shutil, "copyfile", copy_and_commit)

    snapshot = zotero_db.snapshot_collection(
        "COLL1", db_path=db_path, storage_root=storage_root
    )

    assert main_copy_count == 3
    assert snapshot.collection.version == 12


def test_collection_snapshot_rejects_path_escape_and_wrong_link_mode(tmp_path: Path) -> None:
    db_path, storage_root = _fixture(tmp_path)
    outside = tmp_path / "escape.pdf"
    outside.write_bytes(b"%PDF outside")

    with sqlite3.connect(db_path) as conn:
        conn.execute(
            "UPDATE itemAttachments SET path = ? WHERE itemID = 22",
            ("storage:../../escape.pdf",),
        )
    escaped = zotero_db.snapshot_collection("COLL1", db_path=db_path, storage_root=storage_root)
    escaped_member = next(
        member for member in escaped.members if member.attachment_key == "PDFMISSING"
    )
    assert escaped_member.eligibility == "unresolved_path"
    assert escaped_member.attachment_path is None

    with sqlite3.connect(db_path) as conn:
        conn.execute(
            "UPDATE itemAttachments SET path = ?, linkMode = 0 WHERE itemID = 22",
            (str(outside),),
        )
    wrong_mode = zotero_db.snapshot_collection("COLL1", db_path=db_path, storage_root=storage_root)
    wrong_mode_member = next(
        member for member in wrong_mode.members if member.attachment_key == "PDFMISSING"
    )
    assert wrong_mode_member.eligibility == "unresolved_path"
    assert wrong_mode_member.attachment_path is None

    with sqlite3.connect(db_path) as conn:
        conn.execute("UPDATE itemAttachments SET linkMode = 2 WHERE itemID = 22")
    linked = zotero_db.snapshot_collection("COLL1", db_path=db_path, storage_root=storage_root)
    linked_member = next(
        member for member in linked.members if member.attachment_key == "PDFMISSING"
    )
    assert linked_member.eligibility == "eligible_pdf"
    assert linked_member.attachment_path == str(outside)


def test_collection_snapshot_collects_file_evidence(tmp_path: Path) -> None:
    # Verifies file evidence (eligibility + size) for a present PDF attachment.
    # This once also asserted the file was stat()ed exactly once, but
    # _resolve_attachment_path calls .resolve() for symlink-traversal safety, and
    # .resolve() stats the leaf a platform-dependent number of times (fewer on
    # macOS than on Linux), so that exact-count assertion was not portable. The
    # security-relevant resolution is intentional and left unchanged.
    db_path, storage_root = _fixture(tmp_path)

    snapshot = zotero_db.snapshot_collection("COLL1", db_path=db_path, storage_root=storage_root)

    eligible = next(member for member in snapshot.members if member.attachment_key == "PDFOK")
    assert eligible.eligibility == "eligible_pdf"
    assert eligible.file_size == len(b"%PDF fixture")
