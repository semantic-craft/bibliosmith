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

Pages are extracted one at a time rather than as one document-wide blob. That
is what makes `_strip_running_heads()` possible — see its docstring for why a
book's running heads have to go — and it also keeps pdf-inspector's heading
levels stable, because the per-page call derives its font statistics from the
whole document instead of from whichever pages the caller asked for.
"""

from __future__ import annotations

import re
import sys
from collections import defaultdict
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
    #: `(page, non-space characters)` per requested page, always measured on
    #: PyMuPDF's text so the number means the same thing whichever engine won.
    #: The worker's sidecar publishes it and downstream readers rely on it.
    page_chars: tuple[tuple[int, int], ...] = ()
    #: The running heads `_strip_running_heads()` removed, most repeated first.
    #: Empty when the document has none or when PyMuPDF supplied the text.
    running_heads: tuple[str, ...] = ()


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
    # pdf-inspector numbers pages from zero here, while this module, the worker
    # and `process_pdf` all number them from one.
    zero_based = None if page_list is None else [page_no - 1 for page_no in page_list]

    engine = ENGINE_INSPECTOR
    reason = ""
    try:
        extracted = pdf_inspector.extract_pages_markdown(str(path), zero_based)
    except Exception as parse_error:
        # Rewriting the file with PyMuPDF and letting pdf-inspector have one
        # more go rescues about half of the broken-trailer files; the rest keep
        # today's PyMuPDF-only behaviour.
        reason = f"parse_error={parse_error}"
        try:
            extracted = pdf_inspector.extract_pages_markdown_bytes(
                _repaired_bytes(path), zero_based
            )
        except Exception as repair_error:
            return fallback.into_result(f"{reason}; repair_failed={repair_error}")
        engine = ENGINE_INSPECTOR_REPAIRED

    parsed = [(page.page + 1, page.markdown or "") for page in extracted.pages]
    # Every check below reads the text as pdf-inspector parsed it, before any
    # running head is dropped. Measuring the trimmed document instead would let
    # a book with a head on all 629 pages look like it lost text and send a
    # perfectly good extraction back to the flat PyMuPDF dump.
    parsed_markdown = _join_pages(parsed)
    chars = _nonspace(parsed_markdown)
    if chars == 0:
        return fallback.into_result(_join(reason, "empty_markdown"))
    degraded, why = zotero_llm_worker.text_layer_quality(parsed_markdown, chars, config)
    if degraded:
        return fallback.into_result(_join(reason, f"dirty_text_layer: {why}"))
    if fallback.chars is not None and chars < fallback.chars:
        return fallback.into_result(
            _join(reason, f"less_text_than_pymupdf: {chars}<{fallback.chars}")
        )

    trimmed, heads = _strip_running_heads(parsed)
    markdown = _join_pages(trimmed)
    return PdfTextResult(
        markdown=markdown,
        engine=engine,
        fallback_reason=reason,
        chars=_nonspace(markdown),
        page_count=fallback.page_count or len(parsed),
        page_chars=fallback.page_chars(),
        running_heads=heads,
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
            page_chars=self.page_chars(),
        )

    def page_chars(self) -> tuple[tuple[int, int], ...]:
        return tuple((page_no, _nonspace(text)) for page_no, text in self.pages)

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


# A page number as it is printed in a running head: arabic, or a roman numeral
# in the front matter. The lookahead and the strict alternation are what keep
# `civil` and `did` from reading as numerals; `mix` still does, which is why a
# token is only ever stripped when enough of the line survives it.
_ROMAN = r"(?=[ivxlcdmIVXLCDM])[mM]{0,4}(?:[cC][mMdD]|[dD]?[cC]{0,3})(?:[xX][cClL]|[lL]?[xX]{0,3})(?:[iI][xXvV]|[vV]?[iI]{0,3})"
_NUMBER = rf"(?:\d{{1,4}}|{_ROMAN})"
_BARE_NUMBER = re.compile(rf"^\W*{_NUMBER}\W*$")
_LEADING_NUMBER = re.compile(rf"^\W*{_NUMBER}\b\W+")
_TRAILING_NUMBER = re.compile(rf"\W+\b{_NUMBER}\W*$")

#: A running head has to turn up at the edge of this many pages before it is
#: treated as one. Three is enough to clear a chapter title that happens to
#: open a page, and low enough to catch a head over a four-page article.
_MIN_REPEATS = 3
#: Below this the "repeats across pages" question has no useful answer.
_MIN_PAGES = 4
#: Running heads are short. The cap keeps a body paragraph out of reach.
_MAX_HEAD_CHARS = 120
#: How much of the run it spans a head has to be printed on. Furniture is on
#: every page of its section, or every other page when it alternates with the
#: facing one, so real heads clear this easily. It is what keeps `Chapter 1`,
#: `Chapter 2`, `Chapter 3` — which look identical once the number comes off,
#: and each of which opens a page — from being read as one repeated head and
#: deleting every chapter title in the book.
_MIN_SPAN_DENSITY = 0.5
#: What survives stripping the page number off, so a stripped head still reads
#: as a head rather than as the empty string.
_MIN_STEM_CHARS = 3
#: What a line that is no heading at all counts as when the sizes one form is
#: printed at are compared. Deeper than any real level, because a form set in
#: body type is furniture whatever else the book does with it.
_NOT_A_HEADING = 7

#: An ATX heading, for reading the level off the hashes. A row of hashes with
#: no space after it is not one, so `#hashtag` stays body text.
_HEADING_HASHES = re.compile(r"^(#{1,6})(?:\s|$)")


def _normalize_head(line: str) -> str:
    """The form of a line that ignores what changes from page to page.

    `## viii Editors' Introduction` and `## Editors' Introduction ix` are the
    same running head printed on a verso and a recto, so both have to reduce to
    `editors' introduction` before they can be counted together.
    """
    stem = line.lstrip("#").replace("\xa0", " ").replace("*", "").strip()
    stem = re.sub(r"\s+", " ", stem)
    for pattern in (_LEADING_NUMBER, _TRAILING_NUMBER):
        shorter = pattern.sub("", stem)
        if len(shorter) >= _MIN_STEM_CHARS:
            stem = shorter
    return stem.casefold().strip(" .,:;-—–_|")


def _edge_lines(markdown: str) -> list[tuple[int, str]]:
    """The topmost and bottommost real lines of a page, with their indices.

    A bare page number is scaffolding sitting in front of the head rather than
    the head itself, so it is stepped over on the way in from either end.
    """
    lines = markdown.splitlines()
    live = [
        index
        for index, line in enumerate(lines)
        if line.strip() and not _BARE_NUMBER.match(line.strip())
    ]
    if not live:
        return []
    edges = {live[0], live[-1]}
    return [(i, lines[i].strip()) for i in sorted(edges)]


def _span_density(on_pages: set[int]) -> float:
    """How solidly a line fills the run of pages it appears on."""
    return len(on_pages) / (max(on_pages) - min(on_pages) + 1)


def _printed_level(line: str) -> int:
    """How large a line is set, as the heading level pdf-inspector gave it."""
    match = _HEADING_HASHES.match(line)
    return len(match.group(1)) if match else _NOT_A_HEADING


def _title_level(printed_at: list[int]) -> int | None:
    """The size that marks one printing of a form as the chapter's own title.

    A chapter title that is also printed as that chapter's running head reduces
    to the same form as its furniture, so counting alone cannot tell them
    apart: `Law as a Malleable Artifact` opens chapter 2 on page 46 and heads
    the seven rectos after it. What tells them apart is that the book sets the
    title larger — `# 2 Law as a Malleable Artifact` against
    `## Law as a Malleable Artifact 31` — and pdf-inspector, which sizes
    headings by font, preserves the difference as a heading level.

    Only a size that is used *once* counts. A head the parser sized unevenly —
    `## Foreword vii` on five pages and italic body text on six — has no
    outsized printing, just an inconsistent one, and reading its larger half as
    a title would put five pieces of furniture back into the book.
    """
    shallowest = min(printed_at)
    if printed_at.count(shallowest) != 1:
        return None
    return shallowest if any(level > shallowest for level in printed_at) else None


def _strip_running_heads(
    pages: list[tuple[int, str]],
) -> tuple[list[tuple[int, str]], tuple[str, ...]]:
    """Drop the book's running heads and feet from the pages they repeat on.

    pdf-inspector sizes headings by font, and a running head is set in a font
    the body does not use, so it comes back as a heading on every page it is
    printed on: one 267-page book yields 1055 of them. Left in, they become the
    entire table of contents of the finished EPUB, and — because a head sits
    between the last line of one page and the first of the next — they also
    read as a name dropped into the middle of a paragraph.

    Repetition alone cannot be the test. `Further Reading` opens a section on
    39 pages of one companion volume and is a real heading every time. What
    separates the two is position: a running head is the first or last thing on
    its page, and a real heading that repeats is somewhere in between. So only
    the edges of a page are ever considered, and only a form that turns up
    there on `_MIN_REPEATS` pages is removed.

    Position is not enough on its own either, because the page number has to be
    normalised away before two printings of one head can be counted together —
    and that same normalisation makes `Chapter 1` and `Chapter 2` identical.
    Density is what separates those: furniture is printed on every page of its
    run, while chapter titles are scattered the length of the book.

    None of that separates a chapter title from its *own* running head, which
    is the same words in the same place on the page that opens the chapter. So
    one printing of a head is spared: the one the book set larger than all the
    rest. See `_title_level()`.
    """
    if len(pages) < _MIN_PAGES:
        return pages, ()

    edges = [_edge_lines(markdown) for _, markdown in pages]
    seen: dict[str, set[int]] = defaultdict(set)
    printed_at: dict[str, list[int]] = defaultdict(list)
    for (page_no, _), page_edges in zip(pages, edges, strict=True):
        for _, line in page_edges:
            if len(line) <= _MAX_HEAD_CHARS:
                key = _normalize_head(line)
                if key:
                    seen[key].add(page_no)
                    printed_at[key].append(_printed_level(line))

    heads = {
        key
        for key, on_pages in seen.items()
        if len(on_pages) >= _MIN_REPEATS and _span_density(on_pages) >= _MIN_SPAN_DENSITY
    }
    if not heads:
        return pages, ()
    titles = {key: _title_level(printed_at[key]) for key in heads}

    trimmed: list[tuple[int, str]] = []
    for (page_no, markdown), page_edges in zip(pages, edges, strict=True):
        drop: set[int] = set()
        for index, line in page_edges:
            if len(line) > _MAX_HEAD_CHARS:
                continue
            key = _normalize_head(line)
            if key in heads and _printed_level(line) != titles[key]:
                drop.add(index)
        if not drop:
            trimmed.append((page_no, markdown))
            continue
        kept = [line for i, line in enumerate(markdown.splitlines()) if i not in drop]
        trimmed.append((page_no, "\n".join(kept).strip()))
    ordered = sorted(heads, key=lambda key: (-len(seen[key]), key))
    return trimmed, tuple(ordered)


def _join_pages(pages: list[tuple[int, str]]) -> str:
    return "\n\n".join(markdown for _, markdown in pages if markdown.strip()).strip()


def _repaired_bytes(path: Path) -> bytes:
    with fitz.open(path) as doc:
        return doc.tobytes(garbage=3, clean=True)


def _nonspace(text: str) -> int:
    return zotero_llm_worker.count_nonspace(text)


def _join(prefix: str, reason: str) -> str:
    return f"{prefix}; {reason}" if prefix else reason
