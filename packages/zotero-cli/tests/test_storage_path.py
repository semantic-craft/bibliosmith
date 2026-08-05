from pathlib import Path

from zotero_cli.vector_store import default_db_path


def test_app_can_route_the_rebuildable_vector_index_into_its_cache(
    monkeypatch, tmp_path: Path
) -> None:  # type: ignore[no-untyped-def]
    cache_db = tmp_path / "Library" / "Caches" / "BiblioSmith" / "zotero" / "vectors.sqlite"
    monkeypatch.setenv("BIBLIOSMITH_ZOTERO_INDEX_PATH", str(cache_db))

    assert default_db_path() == cache_db
