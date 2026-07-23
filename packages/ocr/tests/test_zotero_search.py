from pathlib import Path
import sys
import tempfile
import unittest

SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from zotero_llm_worker import Attachment, ZoteroLocalClient, build_parser  # noqa: E402


class QueryArgumentTests(unittest.TestCase):
    def test_query_is_a_recognised_argument(self) -> None:
        args = build_parser().parse_args(["--query", "wirtschaftliche Geschäftsgeheimnisse"])
        self.assertEqual(args.query, "wirtschaftliche Geschäftsgeheimnisse")

    def test_query_defaults_to_none(self) -> None:
        args = build_parser().parse_args([])
        self.assertIsNone(args.query)


class StubbedClient(ZoteroLocalClient):
    """A ZoteroLocalClient whose only API seam (.get) is scripted, so the
    title-search logic is tested without a running Zotero server."""

    def __init__(self, storage_root: Path, responses: dict[str, object]) -> None:
        super().__init__("http://127.0.0.1:23119/api/users/0", storage_root, timeout=5)
        self._responses = responses
        self.requested_params: list[dict[str, object]] = []

    def get(self, path: str, **params: object):  # type: ignore[override]
        self.requested_params.append({"path": path, **params})
        return self._responses[path]


def _pdf_child(key: str, storage_root: Path) -> dict:
    (storage_root / key).mkdir(parents=True, exist_ok=True)
    (storage_root / key / "paper.pdf").write_bytes(b"%PDF-1.4")
    return {
        "data": {
            "key": key,
            "contentType": "application/pdf",
            "filename": "paper.pdf",
            "title": "Full Text PDF",
        }
    }


class SearchPdfAttachmentsTests(unittest.TestCase):
    def test_matches_a_parent_by_title_and_returns_its_pdf_child(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            storage_root = Path(temporary_directory)
            client = StubbedClient(
                storage_root,
                {
                    "items": [
                        {
                            "data": {
                                "key": "PARENT01",
                                "itemType": "book",
                                "title": "Der wirtschaftliche Wert von Geschäftsgeheimnissen",
                                "creators": [{"lastName": "Autor"}],
                                "date": "2019",
                            }
                        }
                    ],
                    "items/PARENT01/children": [_pdf_child("ATTACH01", storage_root)],
                },
            )

            hits = list(client.search_pdf_attachments("Geschäftsgeheimnisse", limit=20))

            self.assertEqual(len(hits), 1)
            hit = hits[0]
            self.assertIsInstance(hit, Attachment)
            self.assertEqual(hit.key, "ATTACH01")
            self.assertEqual(hit.parent_key, "PARENT01")
            self.assertEqual(
                hit.parent_title, "Der wirtschaftliche Wert von Geschäftsgeheimnissen"
            )
            self.assertEqual(hit.parent_item_type, "book")

    def test_search_uses_zotero_quick_search_not_a_local_index(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            storage_root = Path(temporary_directory)
            client = StubbedClient(storage_root, {"items": []})

            list(client.search_pdf_attachments("some title", limit=5))

            self.assertEqual(
                client.requested_params[0],
                {
                    "path": "items",
                    "q": "some title",
                    "qmode": "titleCreatorYear",
                    "itemType": "-attachment",
                    "limit": 5,
                    "format": "json",
                },
            )

    def test_a_matched_item_with_no_pdf_child_yields_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            storage_root = Path(temporary_directory)
            client = StubbedClient(
                storage_root,
                {
                    "items": [{"data": {"key": "PARENT01", "title": "No PDF here"}}],
                    "items/PARENT01/children": [
                        {"data": {"key": "NOTE1", "contentType": ""}}
                    ],
                },
            )

            self.assertEqual(list(client.search_pdf_attachments("x")), [])

    def test_a_missing_file_on_disk_is_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            storage_root = Path(temporary_directory)
            # Not writing the file to disk: attachment_path resolves a path that
            # does not exist, which must be filtered out rather than yielded.
            client = StubbedClient(
                storage_root,
                {
                    "items": [{"data": {"key": "PARENT01", "title": "Ghost"}}],
                    "items/PARENT01/children": [
                        {
                            "data": {
                                "key": "GHOST01",
                                "contentType": "application/pdf",
                                "filename": "missing.pdf",
                            }
                        }
                    ],
                },
            )

            self.assertEqual(list(client.search_pdf_attachments("x")), [])

    def test_limit_caps_results_across_multiple_matched_items(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            storage_root = Path(temporary_directory)
            client = StubbedClient(
                storage_root,
                {
                    "items": [
                        {"data": {"key": "P1", "title": "One"}},
                        {"data": {"key": "P2", "title": "Two"}},
                    ],
                    "items/P1/children": [_pdf_child("A1", storage_root)],
                    "items/P2/children": [_pdf_child("A2", storage_root)],
                },
            )

            hits = list(client.search_pdf_attachments("x", limit=1))

            self.assertEqual(len(hits), 1)


if __name__ == "__main__":
    unittest.main()
