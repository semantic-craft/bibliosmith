"""Generate real extractor outputs for the Rust publication-contract suite.

The four fixtures run the production EPUB, direct-PDF, PaddleOCR, and MinerU
assemblers.  Only paid/network engine boundaries are replaced with deterministic
responses; publication evidence is always written by production code.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest import mock
from zipfile import ZIP_DEFLATED, ZipFile

import fitz
from pypdf import PdfWriter


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = PACKAGE_ROOT / "scripts"
sys.path.insert(0, str(PACKAGE_ROOT))
sys.path.insert(0, str(SCRIPTS))

import mineru  # noqa: E402
from epub_to_markdown import extract_book  # noqa: E402


def load_paddle_converter():  # type: ignore[no-untyped-def]
    module_name = "publication_contract_paddle_converter"
    spec = importlib.util.spec_from_file_location(
        module_name, SCRIPTS / "pdf_to_html_paddleocr.py"
    )
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import PaddleOCR converter")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


paddle = load_paddle_converter()


class Progress:
    def update_item(self, *_args: object, **_kwargs: object) -> None:
        pass

    def touch(self, *_args: object, **_kwargs: object) -> None:
        pass


def write_pdf(path: Path, *, structured: bool) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if not structured:
        writer = PdfWriter()
        writer.add_blank_page(width=612, height=792)
        with path.open("wb") as handle:
            writer.write(handle)
        return
    document = fitz.open()
    page = document.new_page()
    page.insert_text((72, 72), "Chapter One", fontsize=24)
    page.insert_textbox(
        fitz.Rect(72, 100, 520, 165),
        "A durable publication paragraph contains enough ordinary words to remain readable "
        "after direct extraction and cites the source note[^pdf-1].",
        fontsize=12,
    )
    page.insert_text((72, 190), "Section A", fontsize=18)
    page.insert_textbox(
        fitz.Rect(72, 215, 520, 280),
        "The nested section preserves hierarchy across producer, handoff, and split stages.",
        fontsize=12,
    )
    page.insert_textbox(
        fitz.Rect(72, 310, 520, 365),
        "[^pdf-1]: Direct PDF note retained by the publication evidence contract.",
        fontsize=10,
    )
    document.save(path)
    document.close()


def write_epub(path: Path) -> None:
    container = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"""
    package = """<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Contract Fixture</dc:title><dc:creator>Fixture Author</dc:creator><dc:identifier id="uid">contract-fixture</dc:identifier><dc:language>en</dc:language></metadata>
<manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="notes" href="notes.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="chapter"/><itemref idref="notes"/></spine></package>"""
    nav = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="chapter.xhtml#chapter">Chapter One</a><ol><li><a href="chapter.xhtml#section-a">Section A</a></li></ol></li></ol></nav></body></html>"""
    chapter = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><h1 id="chapter">Chapter One</h1><p>A durable publication paragraph cites the source note<sup><a epub:type="noteref" href="notes.xhtml#fn1">1</a></sup>.</p><h2 id="section-a">Section A</h2><p>The nested section preserves hierarchy across every stage.</p></body></html>"""
    notes = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><aside epub:type="footnote" id="fn1"><p>EPUB note retained by the publication evidence contract.</p></aside></body></html>"""
    with ZipFile(path, "w", ZIP_DEFLATED) as archive:
        archive.writestr("mimetype", "application/epub+zip")
        archive.writestr("META-INF/container.xml", container)
        archive.writestr("OEBPS/content.opf", package)
        archive.writestr("OEBPS/nav.xhtml", nav)
        archive.writestr("OEBPS/chapter.xhtml", chapter)
        archive.writestr("OEBPS/notes.xhtml", notes)


def paddle_config():  # type: ignore[no-untyped-def]
    return paddle.Config(
        baidu_token="fixture-token",
        baidu_job_url="https://example.invalid/jobs",
        baidu_model="PaddleOCR-VL",
        max_ocr_pages_per_job=12,
        baidu_max_upload_mb=49,
        request_timeout=30,
        poll_seconds=0,
        workers=1,
    )


class FakePaddleClient:
    def __init__(self, _config: object) -> None:
        pass

    def submit_job(self, _chunk_path: Path, batch_id: str) -> str:
        return f"job-{batch_id}"

    def poll_json_url(
        self, job_id: str, _deadline: float, on_progress=None
    ) -> str:  # type: ignore[no-untyped-def]
        if on_progress is not None:
            on_progress(1, 1)
        return f"https://example.invalid/{job_id}.jsonl"

    def download_jsonl(self, _json_url: str) -> str:
        markdown = (
            "# Chapter One\n\n"
            "A durable OCR paragraph cites a source note[^ocr-1].\n\n"
            "## Section A\n\n"
            "The nested OCR section preserves hierarchy across every stage.\n\n"
            "[^ocr-1]: PaddleOCR note retained by the publication evidence contract."
        )
        return json.dumps(
            {
                "result": {
                    "layoutParsingResults": [
                        {"markdown": {"text": markdown, "images": {}}}
                    ]
                }
            }
        )


def fake_chunk_specs(
    _source: Path, page_numbers: list[int], chunk_dir: Path, _max_bytes: int
) -> list[tuple[list[int], Path]]:
    chunk_path = chunk_dir / "pages-0001-0001.pdf"
    chunk_path.write_bytes(b"%PDF deterministic Paddle fixture")
    return [(page_numbers, chunk_path)]


def route_record(name: str, source: Path, markdown: Path) -> dict[str, object]:
    evidence = markdown.with_suffix(".publication.json")
    if not source.is_file() or not markdown.is_file() or not evidence.is_file():
        raise RuntimeError(f"Producer {name} did not emit its complete handoff contract")
    return {
        "name": name,
        "sourcePath": str(source.resolve()),
        "markdownPath": str(markdown.resolve()),
        "evidencePath": str(evidence.resolve()),
        "expectedRoles": ["bodymatter", "bodymatter"],
        "expectedKinds": ["chapter", "section"],
    }


def generate(output: Path) -> list[dict[str, object]]:
    output.mkdir(parents=True, exist_ok=True)
    routes: list[dict[str, object]] = []

    epub_source = output / "epub" / "Contract Fixture.epub"
    epub_source.parent.mkdir(parents=True)
    write_epub(epub_source)
    epub_result = extract_book(epub_source, output / "epub" / "produced")
    routes.append(route_record("epub", epub_source, epub_result.markdown_path))

    direct_source = output / "direct_pdf" / "Direct Contract.pdf"
    write_pdf(direct_source, structured=True)
    direct_html = paddle.extract_book_text(
        direct_source, output / "direct_pdf" / "produced", Progress()
    )
    routes.append(route_record("direct_pdf", direct_source, direct_html.with_suffix(".md")))

    paddle_source = output / "paddle" / "Paddle Contract.pdf"
    write_pdf(paddle_source, structured=False)
    paddle_output = output / "paddle" / "produced"
    paddle_temp = paddle_output / ".temp"
    paddle_temp.mkdir(parents=True)
    with (
        mock.patch.object(paddle, "pdf_page_count", return_value=1),
        mock.patch.object(paddle, "make_chunk_specs", side_effect=fake_chunk_specs),
        mock.patch.object(paddle, "BaiduOCRClient", FakePaddleClient),
    ):
        paddle_html = paddle.process_book(
            paddle_source,
            paddle_output,
            paddle_config(),
            paddle_temp,
            Progress(),
            route=paddle.ROUTE_REMOTE_PADDLEOCR,
        )
    routes.append(route_record("paddle", paddle_source, paddle_html.with_suffix(".md")))

    mineru_source = output / "mineru" / "MinerU Contract.pdf"
    write_pdf(mineru_source, structured=False)
    mineru_output = output / "mineru" / "produced"
    data_id = "mineru-contract"
    part_path = mineru_output / data_id / "parts" / "0001" / "extracted" / "part.md"
    part_path.parent.mkdir(parents=True)
    part_path.write_text(
        "# Chapter One\n\n"
        "A durable MinerU paragraph cites a source note[^mineru-1].\n\n"
        "## Section A\n\n"
        "The nested MinerU section preserves hierarchy across every stage.\n\n"
        "[^mineru-1]: MinerU note retained by the publication evidence contract.\n",
        encoding="utf-8",
    )
    work_item = mineru.WorkItem(
        source=str(mineru_source),
        name=mineru_source.name,
        data_id=data_id,
        local_path=mineru_source,
        source_data_id=data_id,
        source_name=mineru_source.name,
        source_pages=1,
        selected_pages=(1,),
    )
    mineru.merge_downloaded_parts(
        SimpleNamespace(
            output_dir=str(mineru_output),
            no_download=False,
            no_wait=False,
            model_version="vlm",
        ),
        [work_item],
        [mineru.DownloadedPart(item=work_item, markdown_path=part_path)],
    )
    routes.append(route_record("mineru", mineru_source, mineru_output / data_id / "full.md"))
    return routes


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    routes = generate(args.output.resolve())
    index = args.output.resolve() / "producer-contract-index.json"
    index.write_text(
        json.dumps(
            {"schema": "producer-publication-contract-fixtures-v1", "routes": routes},
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    print(index)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
