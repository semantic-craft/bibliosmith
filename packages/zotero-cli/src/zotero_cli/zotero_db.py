"""Read items from the local Zotero SQLite database.

We open the database in read-only mode so the running Zotero app keeps the
write lock. Item-type names that are not bibliographically interesting
(``attachment``, ``note``, ``annotation``) are skipped at iteration time.
"""

from __future__ import annotations

import hashlib
import shutil
import sqlite3
import stat as stat_module
import tempfile
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator

DEFAULT_DB_PATH = Path.home() / "Zotero" / "zotero.sqlite"
ZOTERO_STORAGE = Path.home() / "Zotero" / "storage"

# Item types we never want in the semantic-search index.
SKIP_TYPES = frozenset({"attachment", "note", "annotation"})

# Field IDs we care about. Zotero ships a stable mapping; we read it once at
# query time rather than hardcoding numeric IDs.
INDEX_FIELDS = ("title", "abstractNote", "date", "DOI", "url", "publicationTitle", "publisher")
SNAPSHOT_STABILITY_READS = 4


@dataclass(frozen=True)
class ZoteroItem:
    """A single Zotero item indexable for semantic search."""

    key: str
    item_type: str
    title: str | None
    abstract: str | None
    date: str | None
    doi: str | None
    url: str | None
    venue: str | None
    publisher: str | None
    creators: tuple[str, ...]
    tags: tuple[str, ...]
    date_modified: str

    def embedding_text(self) -> str:
        """Concatenated text used as the embedding payload."""
        parts: list[str] = []
        if self.title:
            parts.append(self.title)
        if self.creators:
            parts.append("Authors: " + "; ".join(self.creators))
        if self.venue:
            parts.append(f"In: {self.venue}")
        if self.date:
            parts.append(f"Date: {self.date}")
        if self.tags:
            parts.append("Tags: " + ", ".join(self.tags))
        if self.abstract:
            parts.append(self.abstract)
        return "\n".join(parts)


def _connect_readonly(db_path: Path) -> sqlite3.Connection:
    uri = f"file:{db_path}?mode=ro&immutable=1"
    return sqlite3.connect(uri, uri=True)


def _connect_snapshot(db_path: Path) -> sqlite3.Connection:
    """Open a read-only connection to an already-stable snapshot copy."""
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.execute("PRAGMA query_only = ON")
    return conn


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _materialize_sqlite_snapshot(db_path: Path, directory: Path, index: int) -> Path:
    """Copy and recover one SQLite image without taking a lock on the live DB."""
    staged = directory / f"source-{index}.sqlite"
    normalized = directory / f"snapshot-{index}.sqlite"
    try:
        shutil.copyfile(db_path, staged)
        for suffix in ("-wal", "-journal"):
            source_sidecar = Path(f"{db_path}{suffix}")
            staged_sidecar = Path(f"{staged}{suffix}")
            try:
                shutil.copyfile(source_sidecar, staged_sidecar)
            except FileNotFoundError:
                pass

        with sqlite3.connect(staged) as source, sqlite3.connect(normalized) as target:
            if source.execute("PRAGMA quick_check").fetchone() != ("ok",):
                raise sqlite3.DatabaseError("snapshot integrity check failed")
            source.backup(target)
            if target.execute("PRAGMA quick_check").fetchone() != ("ok",):
                raise sqlite3.DatabaseError("normalized snapshot integrity check failed")
    except (OSError, sqlite3.DatabaseError) as error:
        raise RuntimeError(
            "Could not obtain a consistent read-only Zotero snapshot; retry discovery."
        ) from error
    finally:
        staged.unlink(missing_ok=True)
        Path(f"{staged}-wal").unlink(missing_ok=True)
        Path(f"{staged}-shm").unlink(missing_ok=True)
        Path(f"{staged}-journal").unlink(missing_ok=True)
    return normalized


@contextmanager
def _stable_snapshot_database(db_path: Path) -> Iterator[Path]:
    """Yield a recovered DB only after two adjacent physical reads agree."""
    with tempfile.TemporaryDirectory(prefix="zotero-collection-snapshot-") as temp_dir:
        directory = Path(temp_dir)
        previous_hash: str | None = None
        previous_path: Path | None = None
        for index in range(SNAPSHOT_STABILITY_READS):
            current_path = _materialize_sqlite_snapshot(db_path, directory, index)
            current_hash = _sha256_file(current_path)
            if current_hash == previous_hash:
                if previous_path is not None:
                    previous_path.unlink(missing_ok=True)
                yield current_path
                return
            if previous_path is not None:
                previous_path.unlink(missing_ok=True)
            previous_hash = current_hash
            previous_path = current_path
        raise RuntimeError(
            "Zotero changed throughout snapshot capture; retry collection discovery."
        )


def _load_field_ids(conn: sqlite3.Connection) -> dict[str, int]:
    rows = conn.execute("SELECT fieldID, fieldName FROM fields").fetchall()
    return {name: fid for fid, name in rows}


def _load_creators_by_item(conn: sqlite3.Connection) -> dict[int, list[str]]:
    rows = conn.execute(
        """
        SELECT ic.itemID, c.firstName, c.lastName
        FROM itemCreators ic
        JOIN creators c ON c.creatorID = ic.creatorID
        ORDER BY ic.itemID, ic.orderIndex
        """
    ).fetchall()
    out: dict[int, list[str]] = {}
    for item_id, first, last in rows:
        name = (last or "").strip()
        if first:
            name = f"{name}, {first.strip()}" if name else first.strip()
        if name:
            out.setdefault(item_id, []).append(name)
    return out


def _load_tags_by_item(conn: sqlite3.Connection) -> dict[int, list[str]]:
    rows = conn.execute(
        """
        SELECT it.itemID, t.name
        FROM itemTags it
        JOIN tags t ON t.tagID = it.tagID
        ORDER BY it.itemID
        """
    ).fetchall()
    out: dict[int, list[str]] = {}
    for item_id, name in rows:
        if name:
            out.setdefault(item_id, []).append(name)
    return out


def iter_items(db_path: Path | None = None) -> list[ZoteroItem]:
    """Return all bibliographically-indexable items from the local Zotero DB.

    Args:
        db_path: Path to ``zotero.sqlite``. Defaults to ``~/Zotero/zotero.sqlite``.

    Returns:
        List of ZoteroItem records, deleted items excluded.

    Raises:
        FileNotFoundError: If db_path does not exist.
    """
    db_path = db_path or DEFAULT_DB_PATH
    if not db_path.exists():
        raise FileNotFoundError(f"Zotero DB not found at {db_path}")

    with _connect_readonly(db_path) as conn:
        field_ids = _load_field_ids(conn)
        creators = _load_creators_by_item(conn)
        tags = _load_tags_by_item(conn)

        skip_clause = ",".join(f"'{t}'" for t in SKIP_TYPES)
        rows = conn.execute(
            f"""
            SELECT i.itemID, i.key, it.typeName, i.dateModified
            FROM items i
            JOIN itemTypes it ON i.itemTypeID = it.itemTypeID
            WHERE i.itemID NOT IN (SELECT itemID FROM deletedItems)
              AND it.typeName NOT IN ({skip_clause})
            ORDER BY i.dateModified DESC
            """
        ).fetchall()

        # Bulk-load all itemData for these items into one lookup pass.
        item_ids = [r[0] for r in rows]
        if not item_ids:
            return []
        placeholders = ",".join(["?"] * len(item_ids))
        wanted_field_ids = tuple(field_ids[f] for f in INDEX_FIELDS if f in field_ids)
        field_placeholders = ",".join(["?"] * len(wanted_field_ids))
        data_rows = conn.execute(
            f"""
            SELECT id.itemID, id.fieldID, idv.value
            FROM itemData id
            JOIN itemDataValues idv ON idv.valueID = id.valueID
            WHERE id.itemID IN ({placeholders})
              AND id.fieldID IN ({field_placeholders})
            """,
            (*item_ids, *wanted_field_ids),
        ).fetchall()

    field_name_by_id = {fid: name for name, fid in field_ids.items()}
    item_data: dict[int, dict[str, str]] = {}
    for item_id, field_id, value in data_rows:
        fname = field_name_by_id.get(field_id)
        if fname:
            item_data.setdefault(item_id, {})[fname] = value

    items: list[ZoteroItem] = []
    for item_id, key, type_name, date_modified in rows:
        d = item_data.get(item_id, {})
        items.append(
            ZoteroItem(
                key=key,
                item_type=type_name,
                title=d.get("title"),
                abstract=d.get("abstractNote"),
                date=d.get("date"),
                doi=d.get("DOI"),
                url=d.get("url"),
                venue=d.get("publicationTitle"),
                publisher=d.get("publisher"),
                creators=tuple(creators.get(item_id, [])),
                tags=tuple(tags.get(item_id, [])),
                date_modified=date_modified,
            )
        )
    return items


def get_item(key: str, db_path: Path | None = None) -> ZoteroItem | None:
    """Look up a single item by Zotero key. Returns None if not found."""
    db_path = db_path or DEFAULT_DB_PATH
    if not db_path.exists():
        raise FileNotFoundError(f"Zotero DB not found at {db_path}")
    for it in iter_items(db_path):
        if it.key == key:
            return it
    return None


@dataclass(frozen=True)
class CollectionRow:
    """A Zotero collection."""

    key: str
    name: str
    parent_key: str | None
    item_count: int


@dataclass(frozen=True)
class CollectionSnapshotIdentity:
    """Canonical identity of one direct-membership collection snapshot."""

    key: str
    name: str
    version: int


@dataclass(frozen=True)
class CollectionSnapshotMember:
    """One direct collection member and one of its attachment outcomes."""

    collection_key: str
    parent_item_key: str
    parent_item_type: str
    parent_item_version: int
    parent_date_modified: str
    title: str | None
    attachment_key: str | None
    attachment_version: int | None
    attachment_date_modified: str | None
    content_type: str | None
    link_mode: int | None
    storage_path: str | None
    attachment_path: str | None
    path_exists: bool
    file_size: int | None
    file_mtime_ns: int | None
    eligibility: str
    reason: str | None


@dataclass(frozen=True)
class CollectionSnapshot:
    """A deterministic, non-recursive Zotero collection read snapshot."""

    schema_version: str
    collection: CollectionSnapshotIdentity
    members: tuple[CollectionSnapshotMember, ...]


def snapshot_collection(
    collection_key: str,
    *,
    db_path: Path | None = None,
    storage_root: Path | None = None,
) -> CollectionSnapshot:
    """Read one collection and its direct parent-to-attachment relationships once."""
    db_path = db_path or DEFAULT_DB_PATH
    storage_root = storage_root or ZOTERO_STORAGE
    if not db_path.exists():
        raise FileNotFoundError(f"Zotero DB not found at {db_path}")

    with _stable_snapshot_database(db_path) as snapshot_db_path:
        conn = _connect_snapshot(snapshot_db_path)
        try:
            snapshot_rows = conn.execute(
                """
                SELECT
                    c.key,
                    c.collectionName,
                    c.version,
                    direct.parentKey,
                    direct.parentType,
                    direct.parentVersion,
                    direct.parentDateModified,
                    direct.title,
                    direct.attachmentKey,
                    direct.attachmentVersion,
                    direct.attachmentDateModified,
                    direct.contentType,
                    direct.linkMode,
                    direct.path
                FROM collections c
                LEFT JOIN (
                    SELECT
                        ci.collectionID,
                        parent.key AS parentKey,
                        parent_type.typeName AS parentType,
                        parent.version AS parentVersion,
                        parent.dateModified AS parentDateModified,
                        (
                            SELECT idv.value
                            FROM itemData id
                            JOIN fields f ON f.fieldID = id.fieldID
                            JOIN itemDataValues idv ON idv.valueID = id.valueID
                            WHERE id.itemID = parent.itemID AND f.fieldName = 'title'
                            LIMIT 1
                        ) AS title,
                        attachment.key AS attachmentKey,
                        attachment.version AS attachmentVersion,
                        attachment.dateModified AS attachmentDateModified,
                        ia.contentType,
                        ia.linkMode,
                        ia.path
                    FROM collectionItems ci
                    JOIN items parent ON parent.itemID = ci.itemID
                    JOIN itemTypes parent_type ON parent_type.itemTypeID = parent.itemTypeID
                    LEFT JOIN itemAttachments ia
                      ON ia.parentItemID = parent.itemID
                     AND ia.itemID NOT IN (SELECT itemID FROM deletedItems)
                    LEFT JOIN items attachment ON attachment.itemID = ia.itemID
                    WHERE parent.itemID NOT IN (SELECT itemID FROM deletedItems)
                ) AS direct ON direct.collectionID = c.collectionID
                WHERE c.key = ?
                  AND c.collectionID NOT IN (SELECT collectionID FROM deletedCollections)
                ORDER BY direct.parentKey, direct.attachmentKey
                """,
                (collection_key,),
            ).fetchall()
        finally:
            conn.close()

    if not snapshot_rows:
        raise ValueError(f"Zotero collection not found: {collection_key}")
    canonical_key, name, collection_version = snapshot_rows[0][:3]
    rows = [row[3:] for row in snapshot_rows if row[3] is not None]

    members = tuple(
        _snapshot_member(row, collection_key=canonical_key, storage_root=storage_root)
        for row in rows
    )
    return CollectionSnapshot(
        schema_version="zotero-collection-snapshot-v1",
        collection=CollectionSnapshotIdentity(
            key=canonical_key,
            name=name,
            version=int(collection_version),
        ),
        members=members,
    )


def _snapshot_member(
    row: tuple,
    *,
    collection_key: str,
    storage_root: Path,
) -> CollectionSnapshotMember:
    (
        parent_key,
        parent_type,
        parent_version,
        parent_modified,
        title,
        attachment_key,
        attachment_version,
        attachment_modified,
        content_type,
        link_mode,
        storage_path,
    ) = row
    attachment_path = _resolve_attachment_path(
        attachment_key,
        storage_path,
        link_mode=link_mode,
        storage_root=storage_root,
    )
    file_stat = None
    if attachment_path is not None:
        try:
            observed = attachment_path.stat()
        except OSError:
            pass
        else:
            if stat_module.S_ISREG(observed.st_mode):
                file_stat = observed
    path_exists = file_stat is not None

    if attachment_key is None:
        eligibility = "no_attachment"
        reason = "Collection member has no file attachment."
    elif not _is_pdf_attachment(content_type, storage_path):
        eligibility = "unsupported_content_type"
        reason = f"Unsupported attachment content type: {content_type or 'unknown'}."
    elif attachment_path is None:
        eligibility = "unresolved_path"
        reason = "PDF attachment path cannot be resolved locally."
    elif not path_exists:
        eligibility = "missing_file"
        reason = "PDF attachment file is missing."
    else:
        eligibility = "eligible_pdf"
        reason = None

    return CollectionSnapshotMember(
        collection_key=collection_key,
        parent_item_key=parent_key,
        parent_item_type=parent_type,
        parent_item_version=int(parent_version),
        parent_date_modified=parent_modified,
        title=title,
        attachment_key=attachment_key,
        attachment_version=(
            int(attachment_version) if attachment_version is not None else None
        ),
        attachment_date_modified=attachment_modified,
        content_type=content_type,
        link_mode=int(link_mode) if link_mode is not None else None,
        storage_path=storage_path,
        attachment_path=str(attachment_path) if attachment_path is not None else None,
        path_exists=path_exists,
        file_size=file_stat.st_size if file_stat is not None else None,
        file_mtime_ns=file_stat.st_mtime_ns if file_stat is not None else None,
        eligibility=eligibility,
        reason=reason,
    )


def _resolve_attachment_path(
    attachment_key: str | None,
    storage_path: str | None,
    *,
    link_mode: int | None,
    storage_root: Path,
) -> Path | None:
    if attachment_key is None or not storage_path:
        return None
    if storage_path.startswith("storage:"):
        if link_mode not in {0, 1}:
            return None
        relative = storage_path.removeprefix("storage:")
        attachment_root = (storage_root / attachment_key).resolve(strict=False)
        candidate = (attachment_root / relative).resolve(strict=False)
        if candidate == attachment_root or not candidate.is_relative_to(attachment_root):
            return None
        return candidate
    candidate = Path(storage_path)
    return candidate if link_mode == 2 and candidate.is_absolute() else None


def _is_pdf_attachment(content_type: str | None, storage_path: str | None) -> bool:
    normalized_content_type = (content_type or "").strip().lower()
    if normalized_content_type:
        return normalized_content_type == "application/pdf"
    return (storage_path or "").lower().endswith(".pdf")


def collection_snapshot_payload(snapshot: CollectionSnapshot) -> dict:
    """Serialize a collection snapshot using the stable pipeline JSON contract."""
    return {
        "schemaVersion": snapshot.schema_version,
        "collection": {
            "key": snapshot.collection.key,
            "name": snapshot.collection.name,
            "version": snapshot.collection.version,
        },
        "members": [
            {
                "collectionKey": member.collection_key,
                "parentItemKey": member.parent_item_key,
                "parentItemType": member.parent_item_type,
                "parentItemVersion": member.parent_item_version,
                "parentDateModified": member.parent_date_modified,
                "title": member.title,
                "attachmentKey": member.attachment_key,
                "attachmentVersion": member.attachment_version,
                "attachmentDateModified": member.attachment_date_modified,
                "contentType": member.content_type,
                "linkMode": member.link_mode,
                "storagePath": member.storage_path,
                "attachmentPath": member.attachment_path,
                "pathExists": member.path_exists,
                "fileSize": member.file_size,
                "fileMtimeNs": member.file_mtime_ns,
                "eligibility": member.eligibility,
                "reason": member.reason,
            }
            for member in snapshot.members
        ],
    }


def list_collections(db_path: Path | None = None) -> list[CollectionRow]:
    """List all non-deleted collections, with item counts."""
    db_path = db_path or DEFAULT_DB_PATH
    with _connect_readonly(db_path) as conn:
        rows = conn.execute(
            """
            SELECT c.key, c.collectionName, p.key, COUNT(ci.itemID)
            FROM collections c
            LEFT JOIN collections p ON p.collectionID = c.parentCollectionID
            LEFT JOIN collectionItems ci ON ci.collectionID = c.collectionID
            WHERE c.collectionID NOT IN (SELECT collectionID FROM deletedCollections)
            GROUP BY c.collectionID
            ORDER BY c.collectionName
            """
        ).fetchall()
    return [
        CollectionRow(key=k, name=n, parent_key=pk, item_count=cnt)
        for k, n, pk, cnt in rows
    ]


def list_items_in_collection(
    coll_key: str, db_path: Path | None = None
) -> list[ZoteroItem]:
    """Return indexable items in a collection by collection key."""
    db_path = db_path or DEFAULT_DB_PATH
    with _connect_readonly(db_path) as conn:
        row = conn.execute(
            "SELECT collectionID FROM collections WHERE key = ?", (coll_key,)
        ).fetchone()
        if not row:
            return []
        coll_id = row[0]
        item_keys = {
            r[0]
            for r in conn.execute(
                """
                SELECT i.key FROM collectionItems ci
                JOIN items i ON i.itemID = ci.itemID
                WHERE ci.collectionID = ?
                """,
                (coll_id,),
            ).fetchall()
        }
    return [it for it in iter_items(db_path) if it.key in item_keys]


def list_tags(db_path: Path | None = None, limit: int = 50) -> list[tuple[str, int]]:
    """Return ``[(tag_name, item_count), ...]`` ordered by count desc."""
    db_path = db_path or DEFAULT_DB_PATH
    with _connect_readonly(db_path) as conn:
        rows = conn.execute(
            """
            SELECT t.name, COUNT(*) AS c
            FROM tags t
            JOIN itemTags it ON it.tagID = t.tagID
            JOIN items i ON i.itemID = it.itemID
            WHERE i.itemID NOT IN (SELECT itemID FROM deletedItems)
            GROUP BY t.tagID
            ORDER BY c DESC, t.name
            LIMIT ?
            """,
            (limit,),
        ).fetchall()
    return [(name, count) for name, count in rows]


def recent_items(
    db_path: Path | None = None, n: int = 20
) -> list[ZoteroItem]:
    """Return the n most-recently-modified indexable items."""
    items = iter_items(db_path)  # already ordered by dateModified DESC
    return items[:n]


def grep_items(
    query: str, db_path: Path | None = None, limit: int = 30
) -> list[ZoteroItem]:
    """SQL LIKE search over title + abstract. Case-insensitive substring match."""
    items = iter_items(db_path)
    q = query.lower()
    out: list[ZoteroItem] = []
    for it in items:
        hay = " ".join(filter(None, [it.title, it.abstract])).lower()
        if q in hay:
            out.append(it)
            if len(out) >= limit:
                break
    return out


@dataclass(frozen=True)
class NoteRow:
    """A Zotero note (top-level or attached to an item)."""

    note_key: str
    parent_key: str | None
    title: str | None
    body_html: str
    date_modified: str


def notes_for_item(
    item_key: str, db_path: Path | None = None
) -> list[NoteRow]:
    """List child notes for a Zotero item."""
    db_path = db_path or DEFAULT_DB_PATH
    with _connect_readonly(db_path) as conn:
        parent = conn.execute(
            "SELECT itemID FROM items WHERE key = ?", (item_key,)
        ).fetchone()
        if not parent:
            return []
        rows = conn.execute(
            """
            SELECT i.key, parent.key, n.title, n.note, i.dateModified
            FROM itemNotes n
            JOIN items i ON i.itemID = n.itemID
            LEFT JOIN items parent ON parent.itemID = n.parentItemID
            WHERE n.parentItemID = ?
              AND i.itemID NOT IN (SELECT itemID FROM deletedItems)
            ORDER BY i.dateModified DESC
            """,
            (parent[0],),
        ).fetchall()
    return [
        NoteRow(note_key=k, parent_key=pk, title=t, body_html=body or "", date_modified=dm)
        for k, pk, t, body, dm in rows
    ]
