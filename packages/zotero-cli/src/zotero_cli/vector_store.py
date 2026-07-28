"""sqlite-vec backed vector store. We own the lifecycle."""

from __future__ import annotations

import json
import sqlite3
import struct
from dataclasses import dataclass
from pathlib import Path

import sqlite_vec

DEFAULT_DB_PATH = Path.home() / ".local" / "share" / "zotero-cli" / "vectors.sqlite"
FTS_SCHEMA_SQL = """
CREATE VIRTUAL TABLE fts_items
USING fts5(
    key UNINDEXED,
    parent_key UNINDEXED,
    title,
    creators,
    abstract,
    chunk_text,
    tokenize='trigram'
)
"""


@dataclass(frozen=True)
class VectorStoreConfig:
    """Configuration for the vector store."""

    db_path: Path
    dim: int = 1024


def _serialize_vec(vec: list[float]) -> bytes:
    """Pack a float32 vector for sqlite-vec."""
    return struct.pack(f"{len(vec)}f", *vec)


def _normalize_ddl(sql: str) -> str:
    """Normalize SQLite DDL for exact sidecar-schema reconciliation."""
    return "".join(sql.lower().split())


class SQLiteVecStore:
    """Single-file vector store using the sqlite-vec extension.

    Lifecycle is explicit: we never auto-reset on schema mismatch. If the user
    changes embedding dimension, ``rebuild()`` is the only path that drops data.
    """

    def __init__(self, cfg: VectorStoreConfig | None = None) -> None:
        self.cfg = cfg or VectorStoreConfig(db_path=DEFAULT_DB_PATH)
        self.cfg.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._conn = sqlite3.connect(str(self.cfg.db_path))
        self._conn.enable_load_extension(True)
        sqlite_vec.load(self._conn)
        self._conn.enable_load_extension(False)
        self._init_schema()
        self._detect_actual_dim()

    def _detect_actual_dim(self) -> None:
        """Override cfg.dim with the actual dimension from the vec_items schema.

        Handles the case where the store was created by a different config
        (e.g. `info` command using default 1024, while the embedder uses 3072).
        The virtual table's DDL is the source of truth.
        """
        row = self._conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='vec_items'"
        ).fetchone()
        if row is None:
            return
        import re
        m = re.search(r"embedding\s+float\[(\d+)\]", row[0])
        if m:
            actual = int(m.group(1))
            if actual != self.cfg.dim:
                object.__setattr__(self.cfg, "dim", actual)

    def __enter__(self) -> "SQLiteVecStore":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def close(self) -> None:
        self._conn.close()

    def _init_schema(self) -> None:
        cur = self._conn.cursor()
        cur.execute(
            f"""
            CREATE VIRTUAL TABLE IF NOT EXISTS vec_items
            USING vec0(embedding float[{self.cfg.dim}])
            """
        )
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS items (
                rowid INTEGER PRIMARY KEY,
                key TEXT UNIQUE NOT NULL,
                metadata_json TEXT NOT NULL,
                date_modified TEXT NOT NULL
            )
            """
        )
        cur.execute("CREATE INDEX IF NOT EXISTS idx_items_key ON items(key)")
        fts_schema = cur.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='fts_items'"
        ).fetchone()
        rebuild_fts = fts_schema is None
        if fts_schema is not None and _normalize_ddl(fts_schema[0]) != _normalize_ddl(FTS_SCHEMA_SQL):
            cur.execute("DROP TABLE fts_items")
            rebuild_fts = True
        if rebuild_fts:
            cur.execute(FTS_SCHEMA_SQL)
        item_count = cur.execute("SELECT COUNT(*) FROM items").fetchone()[0]
        fts_count = cur.execute("SELECT COUNT(*) FROM fts_items").fetchone()[0]
        if rebuild_fts or item_count != fts_count:
            cur.execute("DELETE FROM fts_items")
            for rowid, key, metadata_json in cur.execute(
                "SELECT rowid, key, metadata_json FROM items"
            ).fetchall():
                self._insert_fts_row(cur, rowid, key, json.loads(metadata_json))
        self._conn.commit()

    @staticmethod
    def _insert_fts_row(
        cur: sqlite3.Cursor,
        rowid: int,
        key: str,
        metadata: dict,
    ) -> None:
        creators = metadata.get("creators") or []
        cur.execute(
            """
            INSERT INTO fts_items (
                rowid, key, parent_key, title, creators, abstract, chunk_text
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            """,
            (
                rowid,
                key,
                key.split("#c", 1)[0],
                metadata.get("title") or "",
                " ".join(creators),
                metadata.get("abstract") or "",
                metadata.get("chunk_text") or "",
            ),
        )

    def upsert(
        self,
        keys: list[str],
        vectors: list[list[float]],
        metadatas: list[dict],
        date_modified: list[str],
    ) -> None:
        """Upsert vectors + metadata keyed by Zotero item key."""
        if not (len(keys) == len(vectors) == len(metadatas) == len(date_modified)):
            raise ValueError("keys/vectors/metadatas/date_modified length mismatch")
        cur = self._conn.cursor()
        self._upsert_with_cursor(cur, keys, vectors, metadatas, date_modified)
        self._conn.commit()

    def _upsert_with_cursor(
        self,
        cur: sqlite3.Cursor,
        keys: list[str],
        vectors: list[list[float]],
        metadatas: list[dict],
        date_modified: list[str],
    ) -> None:
        for key, vec, meta, dm in zip(keys, vectors, metadatas, date_modified):
            cur.execute(
                """
                INSERT INTO items (key, metadata_json, date_modified)
                VALUES (?, ?, ?)
                ON CONFLICT(key) DO UPDATE SET
                    metadata_json = excluded.metadata_json,
                    date_modified = excluded.date_modified
                RETURNING rowid
                """,
                (key, json.dumps(meta, ensure_ascii=False), dm),
            )
            rowid = cur.fetchone()[0]
            cur.execute("DELETE FROM vec_items WHERE rowid = ?", (rowid,))
            cur.execute(
                "INSERT INTO vec_items (rowid, embedding) VALUES (?, ?)",
                (rowid, _serialize_vec(vec)),
            )
            cur.execute("DELETE FROM fts_items WHERE rowid = ?", (rowid,))
            self._insert_fts_row(cur, rowid, key, meta)

    def query(
        self,
        vector: list[float],
        top_k: int = 10,
        *,
        allowed_parent_keys: set[str] | None = None,
    ) -> list[tuple[str, float, dict]]:
        """Top-K nearest neighbors, optionally prefiltered by parent item key."""
        if allowed_parent_keys is not None and not allowed_parent_keys:
            return []
        cur = self._conn.cursor()
        if allowed_parent_keys is None:
            rows = cur.execute(
                """
                SELECT i.key, v.distance, i.metadata_json
                FROM vec_items v
                JOIN items i ON i.rowid = v.rowid
                WHERE v.embedding MATCH ?
                  AND k = ?
                ORDER BY v.distance
                """,
                (_serialize_vec(vector), top_k),
            ).fetchall()
        else:
            rows = cur.execute(
                """
                SELECT i.key, v.distance, i.metadata_json
                FROM vec_items v
                JOIN items i ON i.rowid = v.rowid
                WHERE v.embedding MATCH ?
                  AND k = ?
                  AND v.rowid IN (
                      SELECT candidate.rowid
                      FROM items candidate
                      JOIN json_each(?) allowed
                        ON allowed.value = CASE
                            WHEN instr(candidate.key, '#c') > 0
                            THEN substr(candidate.key, 1, instr(candidate.key, '#c') - 1)
                            ELSE candidate.key
                        END
                  )
                ORDER BY v.distance
                """,
                (
                    _serialize_vec(vector),
                    top_k,
                    json.dumps(sorted(allowed_parent_keys)),
                ),
            ).fetchall()
        return [(key, dist, json.loads(meta)) for key, dist, meta in rows]

    def keyword_query(
        self,
        text: str,
        top_k: int = 10,
        *,
        allowed_parent_keys: set[str] | None = None,
        chunks_only: bool = False,
    ) -> list[tuple[str, float, dict]]:
        """Return literal FTS5 phrase matches ordered by BM25 relevance."""
        literal = text.strip()
        if not literal or (allowed_parent_keys is not None and not allowed_parent_keys):
            return []
        if len(literal) < 3:
            return self._short_keyword_query(
                literal,
                top_k=top_k,
                allowed_parent_keys=allowed_parent_keys,
                chunks_only=chunks_only,
            )
        quoted_literal = '"' + literal.replace('"', '""') + '"'
        match_expression = f"chunk_text : {quoted_literal}" if chunks_only else quoted_literal
        predicates = ["fts_items MATCH ?"]
        params: list[object] = [match_expression]
        if chunks_only:
            predicates.append("instr(i.key, '#c') > 0")
        if allowed_parent_keys is not None:
            predicates.append("fts_items.parent_key IN (SELECT value FROM json_each(?))")
            params.append(json.dumps(sorted(allowed_parent_keys)))
        params.append(top_k)
        rows = self._conn.execute(
            f"""
            SELECT i.key,
                   bm25(fts_items, 0.0, 0.0, 8.0, 4.0, 2.0, 1.0) AS score,
                   i.metadata_json
            FROM fts_items
            JOIN items i ON i.rowid = fts_items.rowid
            WHERE {" AND ".join(predicates)}
            ORDER BY score
            LIMIT ?
            """,
            params,
        ).fetchall()
        return [(key, score, json.loads(meta)) for key, score, meta in rows]

    def _short_keyword_query(
        self,
        literal: str,
        *,
        top_k: int,
        allowed_parent_keys: set[str] | None,
        chunks_only: bool,
    ) -> list[tuple[str, float, dict]]:
        """Handle one- and two-character literals that trigram cannot tokenize."""
        weighted_columns = (
            (("chunk_text", 1.0),)
            if chunks_only
            else (
                ("title", 8.0),
                ("creators", 4.0),
                ("abstract", 2.0),
                ("chunk_text", 1.0),
            )
        )
        score_parts = [
            (
                f"CASE WHEN instr(lower(coalesce(fts_items.{column}, '')), lower(?)) > 0 "
                f"THEN {weight} ELSE 0 END"
            )
            for column, weight in weighted_columns
        ]
        match_parts = [
            f"instr(lower(coalesce(fts_items.{column}, '')), lower(?)) > 0"
            for column, _weight in weighted_columns
        ]
        predicates = [f"({' OR '.join(match_parts)})"]
        params: list[object] = []
        params.extend([literal] * (len(score_parts) + len(match_parts)))
        if chunks_only:
            predicates.append("instr(i.key, '#c') > 0")
        if allowed_parent_keys is not None:
            predicates.append("fts_items.parent_key IN (SELECT value FROM json_each(?))")
            params.append(json.dumps(sorted(allowed_parent_keys)))
        params.append(top_k)
        rows = self._conn.execute(
            f"""
            SELECT i.key,
                   -({" + ".join(score_parts)}) AS score,
                   i.metadata_json
            FROM fts_items
            JOIN items i ON i.rowid = fts_items.rowid
            WHERE {" AND ".join(predicates)}
            ORDER BY score
            LIMIT ?
            """,
            params,
        ).fetchall()
        return [(key, score, json.loads(meta)) for key, score, meta in rows]

    def count(self) -> int:
        return self._conn.execute("SELECT COUNT(*) FROM items").fetchone()[0]

    def existing_keys(self) -> dict[str, str]:
        """Return ``{key: date_modified}`` of all stored items."""
        return {
            k: dm
            for k, dm in self._conn.execute(
                "SELECT key, date_modified FROM items"
            ).fetchall()
        }

    def replace_item_chunks(
        self,
        parent_item_key: str,
        keys: list[str],
        vectors: list[list[float]],
        metadatas: list[dict],
        date_modified: list[str],
    ) -> None:
        """Atomically replace all full-text chunks for one Zotero parent item."""
        if not (len(keys) == len(vectors) == len(metadatas) == len(date_modified)):
            raise ValueError("keys/vectors/metadatas/date_modified length mismatch")
        prefix = f"{parent_item_key}#c"
        with self._conn:
            cur = self._conn.cursor()
            rows = cur.execute(
                "SELECT rowid FROM items WHERE substr(key, 1, ?) = ?",
                (len(prefix), prefix),
            ).fetchall()
            for (rowid,) in rows:
                cur.execute("DELETE FROM vec_items WHERE rowid = ?", (rowid,))
                cur.execute("DELETE FROM fts_items WHERE rowid = ?", (rowid,))
            cur.execute(
                "DELETE FROM items WHERE substr(key, 1, ?) = ?",
                (len(prefix), prefix),
            )
            self._upsert_with_cursor(cur, keys, vectors, metadatas, date_modified)

    def item_chunk_metadatas(self, parent_item_key: str) -> list[dict]:
        """Return stored chunk metadata for one parent, ordered by chunk index."""
        prefix = f"{parent_item_key}#c"
        rows = self._conn.execute(
            "SELECT key, metadata_json FROM items WHERE substr(key, 1, ?) = ?",
            (len(prefix), prefix),
        ).fetchall()
        parsed = [(key, json.loads(metadata)) for key, metadata in rows]
        parsed.sort(key=lambda row: int(row[0].rsplit("#c", 1)[1]))
        return [metadata for _, metadata in parsed]

    @staticmethod
    def _is_item_scoped_metadata(metadata: dict) -> bool:
        return metadata.get("index_source") == "item_scoped" or (
            not metadata.get("index_source")
            and bool(metadata.get("source_sha256"))
            and bool(metadata.get("index_contract_version"))
            and bool(metadata.get("embedding_profile_id"))
        )

    def has_item_scoped_chunks(self, parent_item_key: str | None = None) -> bool:
        """Return whether the store, or one parent, has pipeline-owned chunks."""
        if parent_item_key is None:
            rows = self._conn.execute(
                "SELECT metadata_json FROM items WHERE instr(key, '#c') > 0"
            ).fetchall()
        else:
            prefix = f"{parent_item_key}#c"
            rows = self._conn.execute(
                "SELECT metadata_json FROM items WHERE substr(key, 1, ?) = ?",
                (len(prefix), prefix),
            ).fetchall()
        return any(self._is_item_scoped_metadata(json.loads(metadata)) for (metadata,) in rows)

    def clear_for_full_sync_preserving_item_scoped_chunks(self) -> None:
        """Clear sync-owned rows without erasing pipeline item indexes."""
        rows = self._conn.execute("SELECT rowid, metadata_json FROM items").fetchall()
        rowids = [
            rowid
            for rowid, metadata in rows
            if not self._is_item_scoped_metadata(json.loads(metadata))
        ]
        self._delete_rowids(rowids)

    def remove_sync_managed_item_chunks(self, parent_item_key: str) -> None:
        """Remove stale sync chunks while retaining pipeline-owned chunks."""
        prefix = f"{parent_item_key}#c"
        rows = self._conn.execute(
            "SELECT rowid, metadata_json FROM items WHERE substr(key, 1, ?) = ?",
            (len(prefix), prefix),
        ).fetchall()
        rowids = [
            rowid
            for rowid, metadata in rows
            if not self._is_item_scoped_metadata(json.loads(metadata))
        ]
        self._delete_rowids(rowids)

    def _delete_rowids(self, rowids: list[int]) -> None:
        with self._conn:
            cur = self._conn.cursor()
            for rowid in rowids:
                cur.execute("DELETE FROM vec_items WHERE rowid = ?", (rowid,))
                cur.execute("DELETE FROM fts_items WHERE rowid = ?", (rowid,))
                cur.execute("DELETE FROM items WHERE rowid = ?", (rowid,))

    def drop(self, dim: int | None = None) -> None:
        """Drop all tables and recreate empty schema."""
        if dim is not None and dim != self.cfg.dim:
            object.__setattr__(self.cfg, "dim", dim)
        cur = self._conn.cursor()
        cur.execute("DROP TABLE IF EXISTS vec_items")
        cur.execute("DROP TABLE IF EXISTS fts_items")
        cur.execute("DROP TABLE IF EXISTS items")
        self._conn.commit()
        self._init_schema()
