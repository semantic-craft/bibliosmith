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

Pages are extracted one at a time rather than as one document-wide blob. Two
of the passes below need to know which page a line was printed on:
`_strip_running_heads()` to tell furniture from a heading that repeats, and
`_demote_subsections()` to tell a chapter from a section — see their docstrings
for why. It also keeps pdf-inspector's heading levels stable, because the
per-page call derives its font statistics from the whole document instead of
from whichever pages the caller asked for.
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
    parsed_markdown = _join_page_text(parsed)
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
    markdown = _join_pages(
        _normalize_heading_levels(
            trimmed, whole_document=_covers_the_document(page_list, fallback.page_count)
        )
    )
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


#: An ATX heading, split into its hashes and its title.
_HEADING = re.compile(r"^(#{1,6})(?:[ \t]+(.*))?$")
#: A section number printed on a heading: `2.`, `3.2`, `3.2.2`. A chapter in
#: the same book is numbered `5` — bare, no dot — which is what makes the dot
#: worth reading, and why one is required for the number to count as a
#: section's rather than a chapter's.
_SECTION_NUMBER = re.compile(r"^\d{1,3}(?:(?:\.\d{1,3})*\.|(?:\.\d{1,3})+)(?=\s|$)")
#: Words a title does not begin with *in lower case*, so a heading opening with
#: one is the tail of a title that wrapped onto a second printed line. Case is
#: half the test and not decoration: `Of Laws in General` and `And Justice for
#: All` are chapter titles of their own, and only the lower-case `of` of
#: `# The Artifactual Nature` / `# of Legal Institutions` says that the line
#: before it is unfinished. `the` and `a` are absent for a different reason:
#: plenty of real chapters start with them in any case.
_CONTINUES = frozenset({"of", "and", "or", "nor", "but"})
#: Words no title line ends on, read from the other side of the same break.
_DANGLES = frozenset(
    {
        "a", "an", "the", "of", "and", "or", "nor", "but", "to", "in", "on",
        "for", "with", "as", "at", "from", "by", "into", "over", "under",
        "between", "about", "against", "toward", "towards", "its", "their",
    }
)
#: A line break inside a title lands on one of these when it does not land
#: between words.
_BROKEN_TAIL = re.compile(r"[-‐-―,;:]$")
_TITLE_WORDS = re.compile(r"[^\W\d_]+")
#: How many chapters a book has to be read as having before its remaining
#: headings are read as their sections. One chapter is the same cut as none,
#: so a level that yields fewer than this yields no chapter structure at all.
_MIN_CHAPTERS = 2


def _heading(line: str) -> tuple[int, str] | None:
    match = _HEADING.match(line)
    if match is None:
        return None
    return len(match.group(1)), (match.group(2) or "").strip()


def _wraps_onto(first: str, second: str) -> bool:
    """Whether two headings are one title broken across two printed lines.

    Only the shape of the break is read, never the words themselves, and only
    breaks that cannot be anything else count. A wrong merge destroys a real
    chapter title, while a missed one costs a single spare table-of-contents
    entry, so every unclear case is left alone: `EVIDENCE` followed by
    `IS IT FINALLY TIME TO PUT ...` is a section divider above a chapter title
    and reads exactly like a wrap until you know the book.

    The last of the three tests is the one that has to read the second heading,
    and a word alone will not do it: `# PART I` followed by `# Of Laws in
    General` is a part divider above a chapter, and books in this library do
    print the two at one level. What makes the difference is the case the word
    is printed in — `of Legal Institutions` continues a line, `Of Laws in
    General` opens one — so a capitalised `Of` is left as the title it is.
    """
    if not first or not second:
        return False
    if _BROKEN_TAIL.search(first.rstrip()):
        return True
    words = _TITLE_WORDS.findall(first)
    if words and words[-1].casefold() in _DANGLES:
        return True
    tail = _TITLE_WORDS.findall(second)
    return bool(tail) and tail[0].islower() and tail[0] in _CONTINUES


def _merge_wrapped_headings(pages: list[tuple[int, str]]) -> list[tuple[int, str]]:
    """Rejoin a heading pdf-inspector reported as two, one per printed line.

    A title set over two lines is two headings of the same level with nothing
    between them, and the launcher's splitter reads each as a chapter boundary:
    `# 5 On the Artifactual— and Natural—` and `# Character of Legal
    Institutions` become two chapters, the second of them named after the
    second half of one title.
    """
    merged: list[tuple[int, str]] = []
    for page_no, markdown in pages:
        lines = markdown.splitlines()
        kept: list[str] = []
        joined = False
        index = 0
        while index < len(lines):
            here = _heading(lines[index])
            if here is None:
                kept.append(lines[index])
                index += 1
                continue
            level, title = here
            after = index + 1
            while after < len(lines) and not lines[after].strip():
                after += 1
            following = _heading(lines[after]) if after < len(lines) else None
            if (
                following is not None
                and following[0] == level
                and _wraps_onto(title, following[1])
            ):
                kept.append(f"{'#' * level} {title} {following[1]}".rstrip())
                joined = True
                index = after + 1
                continue
            kept.append(lines[index])
            index += 1
        merged.append((page_no, "\n".join(kept) if joined else markdown))
    return merged


def _opens_page(lines: list[str], index: int) -> bool:
    """Whether this line is the first thing printed on its page.

    A folio sitting on its own line above the title is stepped over, the same
    way `_edge_lines()` steps over it.
    """
    for position, line in enumerate(lines):
        stripped = line.strip()
        if not stripped or _BARE_NUMBER.match(stripped):
            continue
        return position == index
    return False


@dataclass(frozen=True)
class _Heading:
    """One ATX heading, placed on the page and in the line it was printed on."""

    page: int
    line: int
    level: int
    title: str
    opens_page: bool


def _document_headings(pages: list[tuple[int, str]]) -> list[_Heading]:
    """Every heading in the document, in reading order."""
    found: list[_Heading] = []
    for page_index, (_, markdown) in enumerate(pages):
        lines = markdown.splitlines()
        for line_index, line in enumerate(lines):
            heading = _heading(line)
            if heading is None:
                continue
            found.append(
                _Heading(
                    page=page_index,
                    line=line_index,
                    level=heading[0],
                    title=heading[1],
                    opens_page=_opens_page(lines, line_index),
                )
            )
    return found


def _demote_subsections(pages: list[tuple[int, str]]) -> list[tuple[int, str]]:
    """Push a chapter's own sections below the level the chapters sit at.

    pdf-inspector sizes headings by font, and in a book whose section titles
    are set at or near the size of its chapter titles the two land on the same
    level: `# 1 Legal Positivism about the Artifact Law` is a chapter while
    `# 2. Legal Positivism, Some Preliminaries` and `# 3.2 Hans Kelsen` are
    sections inside it. `split_source_markdown()` cuts a chapter at every
    heading of the shallowest level present, so each of those sections becomes
    a chapter of the finished book — 71 of them in a book with 12.

    Two things say a heading is a chapter rather than a section, and a heading
    has to satisfy both:

    * It starts a page. Chapters open on a fresh page and sections run on from
      the text above them. This is the load-bearing test, because it reads the
      book's layout rather than its typographic conventions, and it is the only
      one of the two that works on a book whose sections are unnumbered —
      `The Conditions of Legal Validity` and `<u>Further Reading</u>` are
      sections of one companion volume and carry no number to demote them by.
    * It is not numbered like a section. `6. The Case of Dworkin-Lite` happens
      to fall at the top of a page and is still a section.

    A section that goes down takes its own subsections with it. Font-sized
    headings routinely come back half right — `# Sources of Law` misplaced at
    the chapter level with a correctly deeper `## Custom` under it — and moving
    the parent alone would leave the two at the same level, which is a worse
    outline than the one this pass was given. Everything deeper than the
    demoted heading follows it down to the next heading at the chapter level.

    Where a page begins is weak evidence on its own, and three guards say when
    it is not evidence at all. Two of them keep either test from taking the
    whole level, which would achieve nothing in any case — the splitter would
    simply read the level below as the new shallowest one: nothing is demoted
    unless some heading opens a page, and the dotted test is dropped as soon as
    no undotted chapter is left to compare against, so a book whose chapters
    really are numbered `1.`, `2.` keeps them.

    The third answers the case those two do not reach, which the corpus shows
    is the common one: a document whose headings are all peers, one of which
    happens to fall at the top of a page. A watermarked thesis yields 450
    headings at one level and one that opens a page, an EU study 313 and three
    — read literally, a 240-page book with a single chapter and 449 sections
    inside it. So `_MIN_CHAPTERS` headings have to survive the two tests before
    any heading goes down. One chapter is the same cut as none, and buying it
    at the price of every other heading in the book is the trade the two guards
    above already refuse.

    What the third guard does not cover, the shape of the outline does: a
    section belongs to a chapter, so headings printed before the first one are
    left where they are rather than demoted under nothing. That is what a book
    whose front matter is set at the chapter level needs, and it is also what
    keeps a `pages=` range that opens in the middle of a chapter from reading
    its first heading as a section of a chapter that is not in the range.
    """
    headings = _document_headings(pages)
    if not headings:
        return pages
    primary = min(head.level for head in headings)
    if primary >= 6:
        return pages

    openers = [head for head in headings if head.level == primary and head.opens_page]
    if not openers:
        return pages
    # Only a book that numbers its sections but not its chapters can be read
    # this way; where every heading that opens a page is numbered, the number
    # is the chapter's, not a section's.
    read_numbers = any(not _numbered_like_a_section(head) for head in openers)
    chapters = {
        head
        for head in openers
        if not (read_numbers and _numbered_like_a_section(head))
    }
    if len(chapters) < _MIN_CHAPTERS:
        return pages

    demoted: list[_Heading] = []
    inside_a_section = False
    started = False
    for head in headings:
        if head.level == primary:
            started = started or head in chapters
            inside_a_section = started and head not in chapters
            if inside_a_section:
                demoted.append(head)
        elif inside_a_section:
            demoted.append(head)
    return _lower(pages, demoted)


def _numbered_like_a_section(head: _Heading) -> bool:
    return _SECTION_NUMBER.match(head.title) is not None


def _lower(pages: list[tuple[int, str]], demoted: list[_Heading]) -> list[tuple[int, str]]:
    """Add a hash to each of these headings, leaving every other line alone.

    A heading already printed at level 6 stays there: a seventh hash is no
    heading at all in Markdown, so the alternative to flattening the bottom of
    one branch is dropping a line out of the outline entirely.
    """
    split: dict[int, list[str]] = {}
    for head in demoted:
        if head.level >= 6:
            continue
        lines = split.setdefault(head.page, pages[head.page][1].splitlines())
        lines[head.line] = "#" + lines[head.line]
    if not split:
        return pages
    lowered: list[tuple[int, str]] = []
    for page_index, page in enumerate(pages):
        lines = split.get(page_index)
        lowered.append(page if lines is None else (page[0], "\n".join(lines)))
    return lowered


def _normalize_heading_levels(
    pages: list[tuple[int, str]], *, whole_document: bool
) -> list[tuple[int, str]]:
    """Make the shallowest heading level mean `chapter` and nothing else.

    Both passes exist because that level is what the launcher's splitter cuts
    the book into chapters at; see the two docstrings below for what each one
    finds sitting there that is not a chapter.

    Only one of them runs on a `pages=` range. That a title was printed over
    two lines is visible in those two lines and nowhere else, so rejoining it
    needs no more of the book than the caller asked for. Which headings are
    chapters is a fact about the whole book — where its pages break, and what
    else is set at the same size the length of it — and a window onto twenty
    pages cannot settle it. Reading one anyway would make the level a heading
    came out at depend on the range that was requested.
    """
    merged = _merge_wrapped_headings(pages)
    return _demote_subsections(merged) if whole_document else merged


def _covers_the_document(pages: list[int] | None, page_count: int) -> bool:
    """Whether the caller asked for the whole book rather than a range of it.

    The worker always passes a list, so `None` is not what says the document is
    whole; a list holding every page is. A page count of zero means PyMuPDF
    could not read the file, and the caller is about to be handed its fallback
    anyway.
    """
    if pages is None:
        return True
    return page_count > 0 and set(pages) == set(range(1, page_count + 1))


def _join_pages(pages: list[tuple[int, str]]) -> str:
    return "\n\n".join(
        f"<!-- page: {page_no} -->\n\n{markdown}"
        for page_no, markdown in pages
        if markdown.strip()
    ).strip()


def _join_page_text(pages: list[tuple[int, str]]) -> str:
    """Join pages for quality measurement without counting provenance anchors."""
    return "\n\n".join(markdown for _, markdown in pages if markdown.strip()).strip()


def _repaired_bytes(path: Path) -> bytes:
    with fitz.open(path) as doc:
        return doc.tobytes(garbage=3, clean=True)


def _nonspace(text: str) -> int:
    return zotero_llm_worker.count_nonspace(text)


def _join(prefix: str, reason: str) -> str:
    return f"{prefix}; {reason}" if prefix else reason
