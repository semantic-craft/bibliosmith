#!/usr/bin/env python3
"""Structured Markdown extraction for born-digital PDFs.

pdf-inspector recovers headings, tables, lists and links that a flat PyMuPDF
text dump has no way to express. On the live Zotero corpus it also fails
outright on about one PDF in eight — mostly Chinese CNKI-style downloads whose
file trailer it rejects — returns empty Markdown for documents whose CID fonts
or scan-backed layout it gives up on, and on close to a third of the corpus
quietly drops text it did parse, in the worst case a hundred thousand
characters of it. PyMuPDF reads all of those.

So the two run as a chain rather than as alternatives. PyMuPDF extracts the
document either way; pdf-inspector's Markdown is kept only when it parsed, came
back legible, and carries at least as many non-space characters as PyMuPDF got.
Otherwise the PyMuPDF text is what the caller receives. A document can gain
structure this way but never lose text — the guarantee is by construction, not
by measurement.

Deliberately not part of the result: pdf-inspector's per-page OCR flags. On
this corpus they are badly over-eager — one book flags 304 of its 346 pages
while PyMuPDF reads text on every single one — so letting them reach a caller
would mean paying for OCR on hundreds of perfectly extractable pages. Whether a
PDF has a usable text layer stays a question about PyMuPDF's character count,
which is what the callers already ask.
"""

from __future__ import annotations

import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

import fitz  # PyMuPDF
import pdf_inspector

_WORKER_SCRIPTS = Path(__file__).resolve().parent / "scripts"
if str(_WORKER_SCRIPTS) not in sys.path:
    sys.path.insert(0, str(_WORKER_SCRIPTS))
# Imported as a module, not by name: the worker is this module's caller, so
# binding its functions at call time is what keeps the pair importable in
# either order.
import zotero_llm_worker  # noqa: E402


ENGINE_INSPECTOR = "pdf-inspector"
ENGINE_INSPECTOR_REPAIRED = "pdf-inspector-repaired"
ENGINE_PYMUPDF = "pymupdf"


class PdfTextError(Exception):
    pass


@dataclass(frozen=True)
class DirtyTextConfig:
    """The four settings `text_layer_quality()` reads off the worker's Config.

    The defaults mirror `build_config()`; a caller holding a real Config passes
    that in instead, so the mojibake thresholds stay a single decision.
    """

    dirty_text_guard: bool = True
    dirty_text_min_chars: int = 1000
    dirty_text_fullwidth_alnum_ratio: float = 0.03
    dirty_text_private_use_ratio: float = 0.005


@dataclass(frozen=True)
class PdfTextResult:
    markdown: str
    engine: str
    #: Why the winning engine is not a plain first-attempt pdf-inspector run.
    #: Empty exactly when it is.
    fallback_reason: str
    chars: int
    page_count: int


def extract_markdown(
    pdf_path: Path | str,
    *,
    pages: Sequence[int] | None = None,
    dirty_text: DirtyTextConfig | None = None,
) -> PdfTextResult:
    """Convert a PDF to Markdown, falling back to PyMuPDF when needed.

    `pages` is a 1-indexed page list, the same convention the worker and
    pdf-inspector both use; None means the whole document.
    """
    path = Path(pdf_path)
    config = dirty_text if dirty_text is not None else DirtyTextConfig()
    page_list = list(pages) if pages is not None else None
    fallback = _PyMuPdfText.read(path, page_list)

    engine = ENGINE_INSPECTOR
    reason = ""
    try:
        result = pdf_inspector.process_pdf(str(path), page_list)
    except Exception as parse_error:
        # Rewriting the file with PyMuPDF and letting pdf-inspector have one
        # more go rescues about half of the broken-trailer files; the rest keep
        # today's PyMuPDF-only behaviour.
        reason = f"parse_error={parse_error}"
        try:
            result = pdf_inspector.process_pdf_bytes(_repaired_bytes(path), page_list)
        except Exception as repair_error:
            return fallback.into_result(f"{reason}; repair_failed={repair_error}")
        engine = ENGINE_INSPECTOR_REPAIRED

    markdown = result.markdown or ""
    chars = _nonspace(markdown)
    if chars == 0:
        return fallback.into_result(_join(reason, "empty_markdown"))
    degraded, why = zotero_llm_worker.text_layer_quality(markdown, chars, config)
    if degraded:
        return fallback.into_result(_join(reason, f"dirty_text_layer: {why}"))
    if fallback.chars is not None and chars < fallback.chars:
        return fallback.into_result(
            _join(reason, f"less_text_than_pymupdf: {chars}<{fallback.chars}")
        )
    return PdfTextResult(
        markdown=markdown,
        engine=engine,
        fallback_reason=reason,
        chars=chars,
        page_count=int(result.page_count),
    )


@dataclass(frozen=True)
class _PyMuPdfText:
    """What PyMuPDF makes of the document: the baseline and the fallback in one.

    Read once per call, because both jobs need the same page text — deciding
    whether pdf-inspector kept enough of it, and standing in when it did not.
    """

    path: Path
    pages: tuple[tuple[int, str], ...]
    page_count: int
    chars: int | None
    error: str

    @classmethod
    def read(cls, path: Path, pages: list[int] | None) -> _PyMuPdfText:
        extracted: list[tuple[int, str]] = []
        try:
            with fitz.open(path) as doc:
                page_count = int(doc.page_count)
                wanted = pages if pages is not None else range(1, page_count + 1)
                for page_no in wanted:
                    text = zotero_llm_worker.normalize_text(
                        doc.load_page(page_no - 1).get_text("text", sort=True)
                    )
                    extracted.append((page_no, text))
        except Exception as exc:
            return cls(
                path=path,
                pages=(),
                page_count=0,
                chars=None,
                error=f"{type(exc).__name__}: {exc}",
            )
        return cls(
            path=path,
            pages=tuple(extracted),
            page_count=page_count,
            chars=sum(_nonspace(text) for _, text in extracted),
            error="",
        )

    def into_result(self, reason: str) -> PdfTextResult:
        if self.chars is None:
            raise PdfTextError(
                f"{self.path.name}: pdf-inspector was unusable ({reason}) "
                f"and PyMuPDF could not read it either: {self.error}"
            )
        markdown = self.markdown()
        return PdfTextResult(
            markdown=markdown,
            engine=ENGINE_PYMUPDF,
            fallback_reason=reason,
            chars=_nonspace(markdown),
            page_count=self.page_count,
        )

    def markdown(self) -> str:
        """The flat text dump, page numbers kept as comments rather than headings.

        `<!-- page: N -->` is the same invisible anchor the PaddleOCR assembler
        emits: a reviewer can still map a passage back to a page of the
        original, and nothing printable reaches the EPUB's table of contents.
        """
        blocks: list[str] = []
        for page_no, text in self.pages:
            blocks.append(f"<!-- page: {page_no} -->")
            if text:
                blocks.append(text)
        return "\n\n".join(blocks).strip() + "\n"


def _repaired_bytes(path: Path) -> bytes:
    with fitz.open(path) as doc:
        return doc.tobytes(garbage=3, clean=True)


def _nonspace(text: str) -> int:
    return zotero_llm_worker.count_nonspace(text)


def _join(prefix: str, reason: str) -> str:
    return f"{prefix}; {reason}" if prefix else reason
