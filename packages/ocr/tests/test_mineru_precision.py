from __future__ import annotations

import importlib.util
import hashlib
from io import BytesIO, StringIO
import json
import os
from pathlib import Path
import sys
import unittest
from unittest import mock
import zipfile

from pypdf import PdfReader, PdfWriter


PACKAGE_ROOT = Path(__file__).resolve().parents[1]


def load_mineru_module():  # type: ignore[no-untyped-def]
    module_name = "ocr_mineru_precision_test"
    spec = importlib.util.spec_from_file_location(module_name, PACKAGE_ROOT / "mineru.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import MinerU client")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


mineru = load_mineru_module()


class FakeResponse:
    def __init__(self, payload: dict, status_code: int = 200, content: bytes = b"") -> None:
        self._payload = payload
        self.status_code = status_code
        self.text = ""
        self.content = content

    def json(self) -> dict:
        return self._payload

    def raise_for_status(self) -> None:
        if self.status_code >= 400:
            raise RuntimeError(f"HTTP {self.status_code}")


class SingleUrlSession:
    def __init__(self) -> None:
        self.posts: list[dict] = []

    def post(self, url: str, **kwargs: object) -> FakeResponse:
        self.posts.append({"url": url, **kwargs})
        return FakeResponse({"code": 0, "msg": "ok", "data": {"task_id": "task-html"}})


class FailedSingleUrlSession(SingleUrlSession):
    def get(self, url: str, **kwargs: object) -> FakeResponse:
        del url, kwargs
        return FakeResponse(
            {
                "code": 0,
                "msg": "ok",
                "data": {"task_id": "task-failed", "state": "failed", "err_msg": "quota exhausted"},
            }
        )


class LocalBatchSession:
    def __init__(self) -> None:
        self.posts: list[dict] = []

    def post(self, url: str, **kwargs: object) -> FakeResponse:
        self.posts.append({"url": url, **kwargs})
        index = len(self.posts)
        files = kwargs["json"]["files"]
        return FakeResponse(
            {
                "code": 0,
                "msg": "ok",
                "data": {
                    "batch_id": f"batch-{index}",
                    "file_urls": [f"https://upload.test/{index}/{offset}" for offset in range(len(files))],
                },
            }
        )


def result_zip(markdown: str, *, include_image: bool = False) -> bytes:
    buffer = BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr("full.md", markdown)
        archive.writestr("content_list.json", "[]\n")
        if include_image:
            archive.writestr("images/figure.png", b"png fixture")
    return buffer.getvalue()


def write_blank_pdf(path: Path, pages: int = 1) -> None:
    writer = PdfWriter()
    for _ in range(pages):
        writer.add_blank_page(width=72, height=72)
    with path.open("wb") as handle:
        writer.write(handle)


class SplitPdfSession(LocalBatchSession):
    def get(self, url: str, **kwargs: object) -> FakeResponse:
        del url, kwargs
        entries = self.posts[0]["json"]["files"]
        results = [
            {
                "data_id": entry["data_id"],
                "file_name": entry["name"],
                "state": "done",
                "err_msg": "",
                "full_zip_url": f"https://result.test/{index}.zip",
            }
            for index, entry in enumerate(entries, start=1)
        ]
        return FakeResponse(
            {"code": 0, "msg": "ok", "data": {"batch_id": "batch-1", "extract_result": results}}
        )


class IncrementalBatchSession(LocalBatchSession):
    def __init__(self) -> None:
        super().__init__()
        self.get_calls = 0

    def get(self, url: str, **kwargs: object) -> FakeResponse:
        del url, kwargs
        self.get_calls += 1
        entries = self.posts[0]["json"]["files"]
        visible = entries[:1] if self.get_calls == 1 else entries
        results = [
            {
                "data_id": entry["data_id"],
                "file_name": entry["name"],
                "state": "done",
                "err_msg": "",
                "full_zip_url": f"https://result.test/{index}.zip",
            }
            for index, entry in enumerate(visible, start=1)
        ]
        return FakeResponse(
            {"code": 0, "msg": "ok", "data": {"batch_id": "batch-1", "extract_result": results}}
        )


class FailedBatchSession(LocalBatchSession):
    def get(self, url: str, **kwargs: object) -> FakeResponse:
        del url, kwargs
        entry = self.posts[0]["json"]["files"][0]
        return FakeResponse(
            {
                "code": 0,
                "msg": "ok",
                "data": {
                    "batch_id": "batch-1",
                    "extract_result": [
                        {
                            "data_id": entry["data_id"],
                            "file_name": entry["name"],
                            "state": "failed",
                            "err_msg": "page limit",
                        }
                    ],
                },
            }
        )


class MinerUPrecisionCliTests(unittest.TestCase):
    # The scratch-directory filter used to be matched against the absolute path,
    # so scanning any folder living under a directory named tmp — /tmp on Linux,
    # or a plain ~/tmp/books — silently found nothing and failed preflight with
    # "No supported local files or URLs found".
    def test_a_scanned_root_under_tmp_still_finds_its_files(self) -> None:
        from tempfile import TemporaryDirectory

        with TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory) / "tmp" / "books"
            root.mkdir(parents=True)
            write_blank_pdf(root / "book.pdf")
            # A scratch directory *inside* the scanned tree is still skipped.
            (root / "tmp").mkdir()
            write_blank_pdf(root / "tmp" / "scratch.pdf")

            found = mineru.iter_local_files(root)

        self.assertEqual([path.name for path in found], ["book.pdf"])

    def test_cli_marks_progress_failed_when_precision_extract_fails(self) -> None:
        with (
            mock.patch.object(
                mineru, "main", side_effect=mineru.MinerUError("authentication failed")
            ),
            mock.patch.object(mineru.OPERATION_PROGRESS, "touch") as touch,
            mock.patch("sys.stderr", new_callable=StringIO),
        ):
            result = mineru.cli([])

        self.assertEqual(result, 1)
        touch.assert_called_once_with("failed")

    def test_local_pdf_finishes_with_known_page_total(self) -> None:
        progress = mock.Mock()
        progress.total = 450
        item = mineru.WorkItem(
            source="book.pdf",
            name="book.pdf",
            data_id="book",
            local_path=Path("book.pdf"),
            source_pages=450,
        )
        with (
            mock.patch.object(mineru, "collect_items", return_value=([item], [])),
            mock.patch.object(mineru, "process_batches"),
            mock.patch.object(mineru, "OPERATION_PROGRESS", progress),
            mock.patch.dict(os.environ, {"MINERU_API_TOKEN": "test-token"}, clear=False),
        ):
            result = mineru.main(["book.pdf"])

        self.assertEqual(result, 0)
        progress.start.assert_called_once_with("uploading", total=450)
        progress.update.assert_called_once_with(
            completed=450, total=450, phase="completed"
        )

    def test_submit_only_run_is_not_reported_as_completed(self) -> None:
        progress = mock.Mock()
        item = mineru.WorkItem(
            source="book.pdf",
            name="book.pdf",
            data_id="book",
            local_path=Path("book.pdf"),
            source_pages=450,
        )
        with (
            mock.patch.object(mineru, "collect_items", return_value=([item], [])),
            mock.patch.object(mineru, "process_batches"),
            mock.patch.object(mineru, "OPERATION_PROGRESS", progress),
            mock.patch.dict(os.environ, {"MINERU_API_TOKEN": "test-token"}, clear=False),
        ):
            result = mineru.main(["book.pdf", "--no-wait"])

        self.assertEqual(result, 0)
        progress.touch.assert_called_once_with("submitted")
        progress.update.assert_not_called()

    def test_single_html_url_uses_precision_task_with_mineru_html_model(self) -> None:
        session = SingleUrlSession()
        with (
            mock.patch.object(mineru.requests, "Session", return_value=session),
            mock.patch.dict(os.environ, {"MINERU_API_TOKEN": "test-token"}, clear=False),
        ):
            result = mineru.main(
                [
                    "https://example.test/paper.html",
                    "--no-wait",
                    "--page-ranges",
                    "1-2",
                ]
            )

        self.assertEqual(result, 0)
        self.assertEqual(len(session.posts), 1)
        request = session.posts[0]
        self.assertEqual(request["url"], "https://mineru.net/api/v4/extract/task")
        self.assertEqual(request["json"]["model_version"], "MinerU-HTML")
        self.assertNotIn("is_ocr", request["json"])
        self.assertNotIn("enable_table", request["json"])
        self.assertNotIn("enable_formula", request["json"])
        self.assertNotIn("page_ranges", request["json"])

    def test_failed_single_precision_task_is_a_failed_cli_run(self) -> None:
        from tempfile import TemporaryDirectory

        session = FailedSingleUrlSession()
        with TemporaryDirectory() as temporary_directory:
            with (
                mock.patch.object(mineru.requests, "Session", return_value=session),
                mock.patch.dict(os.environ, {"MINERU_API_TOKEN": "test-token"}, clear=False),
            ):
                with self.assertRaisesRegex(mineru.MinerUError, "quota exhausted"):
                    mineru.main(
                        [
                            "https://example.test/paper.pdf",
                            "--output-dir",
                            temporary_directory,
                            "--no-download",
                            "--poll-seconds",
                            "0",
                        ]
                    )

    def test_mixed_local_batch_uses_precision_uploads_grouped_by_required_model(self) -> None:
        from tempfile import TemporaryDirectory

        session = LocalBatchSession()
        uploads: list[dict] = []

        def put(url: str, **kwargs: object) -> FakeResponse:
            uploads.append({"url": url, **kwargs})
            kwargs["data"].read()
            return FakeResponse({}, status_code=200)

        with TemporaryDirectory() as temporary_directory:
            source_dir = Path(temporary_directory) / "sources"
            source_dir.mkdir()
            write_blank_pdf(source_dir / "01_中文 文档.pdf")
            (source_dir / "02_page.html").write_text("<main>hello</main>", encoding="utf-8")

            with (
                mock.patch.object(mineru.requests, "Session", return_value=session),
                mock.patch.object(mineru.requests, "put", side_effect=put),
                mock.patch.dict(os.environ, {"MINERU_API_TOKEN": "test-token"}, clear=False),
            ):
                result = mineru.main(
                    [
                        str(source_dir),
                        "--no-wait",
                        "--no-cache",
                        "--cache-tolerance",
                        "1",
                    ]
                )

        self.assertEqual(result, 0)
        self.assertEqual(len(session.posts), 2)
        self.assertEqual(
            {request["json"]["model_version"] for request in session.posts},
            {"vlm", "MinerU-HTML"},
        )
        self.assertTrue(
            all(request["url"] == "https://mineru.net/api/v4/file-urls/batch" for request in session.posts)
        )
        self.assertTrue(all("no_cache" not in request["json"] for request in session.posts))
        self.assertTrue(all("cache_tolerance" not in request["json"] for request in session.posts))
        entries = [entry for request in session.posts for entry in request["json"]["files"]]
        self.assertEqual(len(entries), 2)
        self.assertTrue(all(mineru.re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", entry["data_id"]) for entry in entries))
        html_request = next(
            request for request in session.posts if request["json"]["model_version"] == "MinerU-HTML"
        )
        self.assertNotIn("is_ocr", html_request["json"]["files"][0])
        self.assertEqual(len(uploads), 2)
        self.assertTrue(all("headers" not in upload for upload in uploads))

    def test_201_page_pdf_is_physically_split_uploaded_and_reassembled_in_page_order(self) -> None:
        from tempfile import TemporaryDirectory

        session = SplitPdfSession()
        progress = mock.Mock()
        progress.total = 201
        uploaded_pdfs: list[bytes] = []

        def put(url: str, **kwargs: object) -> FakeResponse:
            del url
            uploaded_pdfs.append(kwargs["data"].read())
            return FakeResponse({}, status_code=200)

        def get(url: str, **kwargs: object) -> FakeResponse:
            del kwargs
            part_number = int(Path(url).stem)
            markdown = (
                "# Chapter\n\nClaim[^mineru-1].\n\n[^mineru-1]: MinerU note.\n\n"
                "![figure](images/figure.png)\n\nchunk-1"
                if part_number == 1
                else "chunk-2"
            )
            return FakeResponse({}, content=result_zip(markdown, include_image=part_number == 1))

        with TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            source_pdf = root / "long-book.pdf"
            write_blank_pdf(source_pdf, pages=201)
            output_dir = root / "output"

            with (
                mock.patch.object(mineru.requests, "Session", return_value=session),
                mock.patch.object(mineru.requests, "put", side_effect=put),
                mock.patch.object(mineru.requests, "get", side_effect=get),
                mock.patch.object(mineru, "OPERATION_PROGRESS", progress),
                mock.patch.dict(os.environ, {"MINERU_API_TOKEN": "test-token"}, clear=False),
            ):
                result = mineru.main(
                    [
                        str(source_pdf),
                        "--output-dir",
                        str(output_dir),
                        "--poll-seconds",
                        "0",
                    ]
                )

            self.assertEqual(result, 0)
            self.assertEqual([len(PdfReader(BytesIO(payload)).pages) for payload in uploaded_pdfs], [200, 1])
            progress.update_item.assert_has_calls(
                [
                    mock.call(mock.ANY, 200, "extracting", total=200),
                    mock.call(mock.ANY, 1, "extracting", total=1),
                ]
            )
            full_markdown = list(output_dir.rglob("full.md"))
            self.assertEqual(len(full_markdown), 1)
            merged = full_markdown[0].read_text(encoding="utf-8")
            self.assertLess(merged.index("chunk-1"), merged.index("chunk-2"))
            self.assertIn("parts/0001/extracted/images/figure.png", merged)
            manifest_path = full_markdown[0].with_name("mineru_manifest.json")
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            self.assertEqual(manifest["source_pages"], 201)
            self.assertEqual([part["page_count"] for part in manifest["parts"]], [200, 1])
            evidence_path = full_markdown[0].with_suffix(".publication.json")
            evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
            self.assertEqual(evidence["schema"], "publication-extraction-evidence-v2")
            self.assertEqual(len(evidence["sourceDocuments"]), 2)
            self.assertEqual(evidence["notes"][0]["id"], "note_001")
            self.assertEqual(evidence["notes"][0]["sourceLabel"], "mineru-1")
            self.assertEqual(
                evidence["notes"][0]["referenceIds"],
                ["noteref_note_001_001"],
            )
            self.assertEqual(evidence["notes"][0]["anomalies"], [])
            for document in evidence["sourceDocuments"]:
                persisted = full_markdown[0].parent / document["path"]
                self.assertTrue(persisted.is_file())
                self.assertEqual(
                    hashlib.sha256(persisted.read_bytes()).hexdigest(),
                    document["sha256"],
                )

    def test_batch_poll_waits_until_every_submitted_file_has_a_terminal_result(self) -> None:
        from tempfile import TemporaryDirectory

        session = IncrementalBatchSession()

        def put(url: str, **kwargs: object) -> FakeResponse:
            del url
            kwargs["data"].read()
            return FakeResponse({}, status_code=200)

        with TemporaryDirectory() as temporary_directory:
            source_dir = Path(temporary_directory) / "sources"
            source_dir.mkdir()
            write_blank_pdf(source_dir / "one.pdf")
            write_blank_pdf(source_dir / "two.pdf")
            with (
                mock.patch.object(mineru.requests, "Session", return_value=session),
                mock.patch.object(mineru.requests, "put", side_effect=put),
                mock.patch.dict(os.environ, {"MINERU_API_TOKEN": "test-token"}, clear=False),
            ):
                result = mineru.main(
                    [
                        str(source_dir),
                        "--output-dir",
                        str(Path(temporary_directory) / "output"),
                        "--no-download",
                        "--poll-seconds",
                        "0",
                    ]
                )

        self.assertEqual(result, 0)
        self.assertEqual(session.get_calls, 2)

    def test_failed_batch_item_is_a_failed_cli_run_even_without_downloading(self) -> None:
        from tempfile import TemporaryDirectory

        session = FailedBatchSession()

        def put(url: str, **kwargs: object) -> FakeResponse:
            del url
            kwargs["data"].read()
            return FakeResponse({}, status_code=200)

        with TemporaryDirectory() as temporary_directory:
            source_pdf = Path(temporary_directory) / "paper.pdf"
            write_blank_pdf(source_pdf)
            with (
                mock.patch.object(mineru.requests, "Session", return_value=session),
                mock.patch.object(mineru.requests, "put", side_effect=put),
                mock.patch.dict(os.environ, {"MINERU_API_TOKEN": "test-token"}, clear=False),
            ):
                with self.assertRaisesRegex(mineru.MinerUError, "page limit"):
                    mineru.main(
                        [
                            str(source_pdf),
                            "--output-dir",
                            str(Path(temporary_directory) / "output"),
                            "--no-download",
                            "--poll-seconds",
                            "0",
                        ]
                    )


if __name__ == "__main__":
    unittest.main()
