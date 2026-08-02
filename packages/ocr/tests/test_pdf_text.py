"""Tests for the hybrid pdf-inspector / PyMuPDF extractor.

The real-corpus cases at the bottom are the point of this file. Hand-built
fixtures are all born-digital and all parse, so they are blind to the four ways
pdf-inspector behaves on this library: Chinese downloads with a broken file
trailer, CID fonts it cannot decode, scan-backed layouts it gives up on, and
documents it parses while dropping much of the text. Those tests skip when
~/Zotero/storage is not present, which is the case on CI.
"""

from __future__ import annotations

import dataclasses
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import fitz


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
REAL_CORPUS = Path.home() / "Zotero" / "storage"
PRIVATE_USE = ""


def load_pdf_text_module():  # type: ignore[no-untyped-def]
    module_name = "ocr_pdf_text_test"
    spec = importlib.util.spec_from_file_location(module_name, PACKAGE_ROOT / "pdf_text.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("Cannot import pdf_text")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


pdf_text = load_pdf_text_module()


def write_pdf(path: Path, page_texts: list[str]) -> Path:
    doc = fitz.open()
    for text in page_texts:
        page = doc.new_page()
        if text:
            page.insert_text((72, 300), text, fontsize=16)
    doc.save(path)
    doc.close()
    return path


def nonspace(text: str) -> int:
    return sum(1 for ch in text if not ch.isspace())


class StubPage:
    """Stands in for pdf_inspector.PageMarkdown, a Rust extension type."""

    def __init__(self, page: int, markdown: str) -> None:
        self.page = page
        self.markdown = markdown


class StubResult:
    """Stands in for pdf_inspector.PagesExtractionResult.

    Takes the document as one string and splits it back into pages on the form
    feed, so a test that does not care about pages can still pass one string.
    """

    def __init__(self, markdown: str, page_count: int = 3) -> None:
        texts = markdown.split("\f")
        self.pages = [StubPage(index, text) for index, text in enumerate(texts)]
        self.page_count = page_count


def stub_pages(*page_texts: str) -> StubResult:
    return StubResult("\f".join(page_texts))


def refuse_to_parse(*_args, **_kwargs):
    raise ValueError("PDF parsing error: couldn't parse input: invalid file trailer")


#: Two chapters that open a page and one section that does not, with enough
#: text on each page to beat what PyMuPDF reads off the stand-in PDF.
LEVELLED_STUB = stub_pages(
    "# 1 The First Chapter\n\nThe opening paragraph of the first chapter runs here.",
    "The argument continues at some length.\n\n# 2. A Section\n\nMore of the argument.",
    "# 2 The Second Chapter\n\nThe opening paragraph of the second chapter runs here.",
)


class HybridChainTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))

    def test_born_digital_pdf_is_extracted_by_pdf_inspector(self) -> None:
        pdf = write_pdf(self.tmp / "born.pdf", ["Alpha the quick brown fox", "Beta the lazy dog"])

        result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR)
        self.assertEqual(result.fallback_reason, "")
        self.assertIn("Alpha the quick brown fox", result.markdown)
        self.assertIn("Beta the lazy dog", result.markdown)
        self.assertEqual(result.page_count, 2)
        self.assertEqual(result.chars, nonspace(result.markdown))

    def test_page_selection_is_one_indexed(self) -> None:
        pdf = write_pdf(
            self.tmp / "three.pdf", ["PageOneMarker", "PageTwoMarker", "PageThreeMarker"]
        )

        result = pdf_text.extract_markdown(pdf, pages=[2])

        self.assertIn("PageTwoMarker", result.markdown)
        self.assertNotIn("PageOneMarker", result.markdown)
        self.assertNotIn("PageThreeMarker", result.markdown)

    def test_a_whole_document_has_its_heading_levels_normalised(self) -> None:
        pdf = write_pdf(self.tmp / "book.pdf", ["one", "two", "three"])

        with mock.patch.object(
            pdf_text.pdf_inspector, "extract_pages_markdown", return_value=LEVELLED_STUB
        ):
            result = pdf_text.extract_markdown(pdf)

        self.assertIn("\n## 2. A Section", result.markdown)

    def test_a_page_range_of_the_same_document_is_not_normalised(self) -> None:
        """The same three pages, asked for as a range rather than as the book.

        Which headings are chapters is read off the whole document, so a caller
        that asked for part of it is handed the levels pdf-inspector gave.
        """
        pdf = write_pdf(self.tmp / "book.pdf", ["one", "two", "three", "four"])

        with mock.patch.object(
            pdf_text.pdf_inspector, "extract_pages_markdown", return_value=LEVELLED_STUB
        ):
            result = pdf_text.extract_markdown(pdf, pages=[1, 2, 3])

        self.assertIn("\n# 2. A Section", result.markdown)
        self.assertNotIn("## 2. A Section", result.markdown)

    def test_pdf_without_a_text_layer_falls_back_to_pymupdf(self) -> None:
        pdf = write_pdf(self.tmp / "blank.pdf", ["", ""])

        result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertEqual(result.fallback_reason, "empty_markdown")

    def test_mojibake_markdown_falls_back_to_pymupdf(self) -> None:
        pdf = write_pdf(self.tmp / "clean.pdf", ["Readable text the extractor can use"])
        mojibake = "文" * 1000 + PRIVATE_USE * 60

        with mock.patch.object(
            pdf_text.pdf_inspector, "extract_pages_markdown", return_value=StubResult(mojibake)
        ):
            result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertIn("dirty_text_layer", result.fallback_reason)
        self.assertIn("private_use_ratio", result.fallback_reason)
        self.assertIn("Readable text the extractor can use", result.markdown)

    def test_mojibake_thresholds_come_from_the_caller(self) -> None:
        pdf = write_pdf(self.tmp / "clean.pdf", ["Readable text the extractor can use"])
        mojibake = "文" * 1000 + PRIVATE_USE * 60

        with mock.patch.object(
            pdf_text.pdf_inspector, "extract_pages_markdown", return_value=StubResult(mojibake)
        ):
            result = pdf_text.extract_markdown(
                pdf, dirty_text=pdf_text.DirtyTextConfig(dirty_text_guard=False)
            )

        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR)
        self.assertEqual(result.markdown, mojibake)

    def test_markdown_holding_less_text_than_pymupdf_falls_back(self) -> None:
        pdf = write_pdf(
            self.tmp / "long.pdf",
            ["Paragraph one of the document body", "Paragraph two of the document body"],
        )

        with mock.patch.object(
            pdf_text.pdf_inspector, "extract_pages_markdown", return_value=StubResult("# Paragraph one")
        ):
            result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertIn("less_text_than_pymupdf", result.fallback_reason)
        self.assertIn("Paragraph two of the document body", result.markdown)

    def test_parse_failure_is_repaired_and_retried_before_falling_back(self) -> None:
        pdf = write_pdf(self.tmp / "trailer.pdf", ["Recovered"])
        calls: list[str] = []

        def refuse(_path, _pages=None):
            calls.append("extract_pages_markdown")
            refuse_to_parse()

        def accept(_data, _pages=None):
            calls.append("extract_pages_markdown_bytes")
            return StubResult("# Recovered after the repair save", page_count=1)

        with mock.patch.object(
            pdf_text.pdf_inspector, "extract_pages_markdown", refuse
        ), mock.patch.object(pdf_text.pdf_inspector, "extract_pages_markdown_bytes", accept):
            result = pdf_text.extract_markdown(pdf)

        self.assertEqual(calls, ["extract_pages_markdown", "extract_pages_markdown_bytes"])
        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR_REPAIRED)
        self.assertIn("invalid file trailer", result.fallback_reason)
        self.assertEqual(result.markdown, "# Recovered after the repair save")

    def test_parse_failure_the_repair_cannot_fix_falls_back_to_pymupdf(self) -> None:
        pdf = write_pdf(self.tmp / "trailer.pdf", ["Still readable by PyMuPDF"])

        with mock.patch.object(
            pdf_text.pdf_inspector, "extract_pages_markdown", refuse_to_parse
        ), mock.patch.object(pdf_text.pdf_inspector, "extract_pages_markdown_bytes", refuse_to_parse):
            result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertIn("repair_failed", result.fallback_reason)
        self.assertIn("Still readable by PyMuPDF", result.markdown)

    def test_pymupdf_fallback_marks_pages_with_comments_not_headings(self) -> None:
        pdf = write_pdf(self.tmp / "blank.pdf", ["", ""])

        result = pdf_text.extract_markdown(pdf)

        self.assertIn("<!-- page: 1 -->", result.markdown)
        self.assertIn("<!-- page: 2 -->", result.markdown)
        self.assertNotIn("## Page", result.markdown)

    def test_file_neither_engine_can_read_raises(self) -> None:
        not_a_pdf = self.tmp / "notes.pdf"
        not_a_pdf.write_text('{"itemType": "book"}', encoding="utf-8")

        with self.assertRaises(pdf_text.PdfTextError):
            pdf_text.extract_markdown(not_a_pdf)


class OcrRoutingBoundaryTests(unittest.TestCase):
    """The result must not hand a caller pdf-inspector's per-page OCR verdict.

    Measured on this corpus it flags hundreds of pages that PyMuPDF reads
    without trouble, so a caller acting on it would buy paid OCR for nothing.
    """

    def test_result_carries_only_markdown_and_provenance(self) -> None:
        names = {field.name for field in dataclasses.fields(pdf_text.PdfTextResult)}

        self.assertEqual(
            names,
            {
                "markdown",
                "engine",
                "fallback_reason",
                "chars",
                "page_count",
                "page_chars",
                "running_heads",
            },
        )

    def test_the_module_offers_a_single_entry_point(self) -> None:
        functions = {
            name
            for name, value in vars(pdf_text).items()
            if not name.startswith("_")
            and callable(value)
            and not isinstance(value, type)
            and getattr(value, "__module__", "") == pdf_text.__name__
        }

        self.assertEqual(functions, {"extract_markdown"})


HEAD = "EDITORS’ INTRODUCTION"
FOOT = "THOMSON REUTERS"


def book_page(number: int, body: str, *, head: bool = True, foot: bool = True) -> str:
    """One page of a six-page book, with the running head printed as a heading.

    The page number moves from the front of the head to the back on facing
    pages, the way a book prints it, so the fixture exercises the part of
    detection that has to see through it.
    """
    lines = []
    if head:
        lines.append(f"## {number} {HEAD}" if number % 2 == 0 else f"## {HEAD} {number}")
    lines.append(body)
    if foot:
        lines.append(FOOT)
    return "\n\n".join(lines)


def book(*, head: bool = True, foot: bool = True) -> list[tuple[int, str]]:
    """Six pages. Page 3 carries a real heading that repeats mid-page later."""
    bodies = [
        "# A Real Chapter Title\n\nThe opening paragraph sets out the problem.",
        "A second page arguing that the received view cannot be right.",
        "Objections are considered next.\n\n## Further Reading\n\nWorks on the received view.",
        "The fourth page turns to the consequences of rejecting it.",
        "A rival account is set out here.\n\n## Further Reading\n\nWorks on the rival account.",
        "The argument is drawn together.\n\n## Further Reading\n\nWorks on both accounts.",
    ]
    return [(i + 1, book_page(i + 1, body, head=head, foot=foot)) for i, body in enumerate(bodies)]


TITLE = "Law as a Malleable Artifact"


def self_headed_chapter(*, title_hashes: str = "#") -> list[tuple[int, str]]:
    """A chapter whose title is printed again as the running head of every recto.

    The shape `2018_Law as an Artifact.pdf` prints on pages 46 to 60: the title
    opens the chapter set large and unnumbered by the folio, and the same words
    head each recto after it, set smaller and carrying the page number. Both
    reduce to `law as a malleable artifact`, so counting cannot tell them apart.
    """
    pages = [(1, f"{title_hashes} 2 {TITLE}\n\nThe chapter opens on its own title page.")]
    for page in range(2, 10):
        body = f"The argument runs on over page {page}."
        head = f"## {TITLE} {page + 29}\n\n" if page % 2 else ""
        pages.append((page, f"{head}{body}"))
    return pages


class RunningHeadTests(unittest.TestCase):
    """pdf-inspector sizes headings by font, so a running head becomes one.

    Left in, they are the whole table of contents of the finished book: one
    267-page volume produced 1055 headings this way.
    """

    def strip(self, pages):  # type: ignore[no-untyped-def]
        trimmed, heads = pdf_text._strip_running_heads(pages)
        return "\n\n".join(text for _, text in trimmed), heads

    def test_a_running_head_is_removed_from_every_page(self) -> None:
        markdown, heads = self.strip(book())

        self.assertNotIn(HEAD, markdown)
        self.assertIn("editors’ introduction", heads)

    def test_a_running_foot_is_removed_too(self) -> None:
        markdown, heads = self.strip(book())

        self.assertNotIn(FOOT, markdown)
        self.assertIn("thomson reuters", heads)

    def test_a_heading_that_repeats_mid_page_is_kept(self) -> None:
        """`Further Reading` opens a section on 39 pages of one real book.

        Repetition alone would delete it. Position is what tells the two
        apart, and this is the case that says so.
        """
        markdown, heads = self.strip(book())

        self.assertEqual(markdown.count("## Further Reading"), 3)
        self.assertNotIn("further reading", heads)

    def test_the_body_and_the_real_chapter_heading_survive(self) -> None:
        markdown, _ = self.strip(book())

        self.assertIn("# A Real Chapter Title", markdown)
        self.assertIn("The opening paragraph sets out the problem.", markdown)
        self.assertIn("The fourth page turns to the consequences of rejecting it.", markdown)

    def test_a_book_that_never_had_a_running_head_is_left_alone(self) -> None:
        """The mutation the ticket asks for: delete the heads from the input.

        Nothing else about the document changes, so anything removed here is
        collateral damage. The paired assertions above — where the very same
        bodies do lose their heads — are what stop this from passing for the
        trivial reason that the code removes nothing at all.
        """
        clean = book(head=False, foot=False)

        trimmed, heads = pdf_text._strip_running_heads(clean)

        self.assertEqual(trimmed, clean)
        self.assertEqual(heads, ())

    def test_a_document_too_short_to_judge_is_left_alone(self) -> None:
        pages = [(1, "## A Title\n\nBody."), (2, "## A Title\n\nBody."), (3, "## A Title\n\nBody.")]

        trimmed, heads = pdf_text._strip_running_heads(pages)

        self.assertEqual(trimmed, pages)
        self.assertEqual(heads, ())

    def test_a_bare_page_number_does_not_hide_the_head_behind_it(self) -> None:
        """Some pages print the folio on its own line above the head."""
        bodies = [
            "The opening of the argument.",
            "A distinction is drawn.",
            "An objection is raised.",
            "The objection is answered.",
            "A second objection follows.",
            "The chapter closes.",
        ]
        pages = [(i + 1, f"{i + 1}\n\n## {HEAD}\n\n{body}") for i, body in enumerate(bodies)]

        markdown, heads = self.strip(pages)

        self.assertNotIn(HEAD, markdown)
        self.assertIn("The chapter closes.", markdown)
        self.assertIn("editors’ introduction", heads)

    def test_numbered_chapter_titles_are_not_mistaken_for_one_head(self) -> None:
        """`Chapter 1` and `Chapter 2` are the same string once the number goes.

        They open a page each, so position alone would take every one of them
        and leave the book with no chapter titles at all. What saves them is
        that they are scattered down the book rather than printed on every
        page of a run.
        """
        pages = []
        for chapter in range(1, 5):
            pages.append((len(pages) + 1, f"# Chapter {chapter}\n\nThe chapter opens."))
            for step in range(4):
                pages.append(
                    (len(pages) + 1, f"Chapter {chapter} argues its {step}th point at length.")
                )

        markdown, heads = self.strip(pages)

        self.assertEqual(heads, ())
        for chapter in range(1, 5):
            self.assertIn(f"# Chapter {chapter}", markdown)

    def test_a_chapter_title_that_is_also_its_own_running_head_survives(self) -> None:
        """The one case position and density both get wrong.

        A chapter title is at the edge of the page that opens the chapter, and
        it is printed on every page of the run its own head spans, so both
        earlier tests read it as furniture. Five of the twelve chapters of
        `2018_Law as an Artifact.pdf` lost their names this way. Size is what
        is left: the book sets the title larger than the head.
        """
        markdown, heads = self.strip(self_headed_chapter())

        self.assertIn(f"# 2 {TITLE}", markdown)
        self.assertNotIn(f"## {TITLE}", markdown)
        self.assertIn("law as a malleable artifact", heads)

    def test_only_the_outsized_printing_is_spared_not_the_first_one(self) -> None:
        """Sparing whichever printing comes first would leave a stray head.

        Most running heads have no outsized printing at all — 381 of the 497
        runs over a 74-book sample — and on those, `first one wins` keeps a
        piece of furniture and rescues nothing. So the title has to earn its
        place by being set larger, not by coming first.
        """
        markdown, _ = self.strip(self_headed_chapter(title_hashes="##"))

        self.assertNotIn(TITLE, markdown)

    def test_a_head_the_parser_sized_unevenly_keeps_none_of_its_printings(self) -> None:
        """`## Foreword vii` on four pages and body text on four others.

        One real book prints its foreword head that way. The larger half is
        not a title, just the half pdf-inspector happened to size — reading it
        as one would put four pieces of furniture back into the book, so a
        size has to be used exactly once before it counts.
        """
        folios = ["vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv"]
        pages = [
            (
                page,
                f"{'## Foreword' if page % 2 else '*Foreword*'} {folio}\n\n"
                f"The foreword continues over page {page}.",
            )
            for page, folio in enumerate(folios, start=1)
        ]

        markdown, heads = self.strip(pages)

        self.assertNotIn("Foreword", markdown)
        self.assertIn("foreword", heads)


class RunningHeadChainTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))

    def test_removing_heads_does_not_send_the_document_back_to_pymupdf(self) -> None:
        """The text-loss guard has to judge the document as parsed.

        Measure the trimmed text against PyMuPDF instead and a book with a
        head on all 629 pages looks like it lost text, so a perfectly good
        structured extraction gets thrown away for the flat dump.
        """
        pdf = write_pdf(self.tmp / "headed.pdf", ["Body one", "Body two"])
        pages = [text for _, text in book()]

        with mock.patch.object(
            pdf_text.pdf_inspector, "extract_pages_markdown", return_value=stub_pages(*pages)
        ):
            result = pdf_text.extract_markdown(pdf)

        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR)
        self.assertEqual(result.fallback_reason, "")
        self.assertNotIn(HEAD, result.markdown)
        self.assertIn("# A Real Chapter Title", result.markdown)
        self.assertEqual(result.chars, nonspace(result.markdown))

    def test_the_removed_heads_are_reported_to_the_caller(self) -> None:
        pdf = write_pdf(self.tmp / "headed.pdf", ["Body one", "Body two"])
        pages = [text for _, text in book()]

        with mock.patch.object(
            pdf_text.pdf_inspector, "extract_pages_markdown", return_value=stub_pages(*pages)
        ):
            result = pdf_text.extract_markdown(pdf)

        self.assertIn("editors’ introduction", result.running_heads)
        self.assertIn("thomson reuters", result.running_heads)

    def test_per_page_characters_are_reported_for_the_sidecar(self) -> None:
        pdf = write_pdf(self.tmp / "two.pdf", ["Alpha the quick brown fox", "Beta the lazy dog"])

        result = pdf_text.extract_markdown(pdf)

        self.assertEqual([page for page, _ in result.page_chars], [1, 2])
        self.assertTrue(all(chars > 0 for _, chars in result.page_chars))


def levelled_book(*, flat: bool = True) -> list[tuple[int, str]]:
    """Six pages whose sections are set at the level the chapters sit at.

    `flat=False` is the same book with its levels already right: every body,
    every title and every page break is identical, and the only difference is
    the depth of the four section headings. That is what makes the pair worth
    having — the same input, once needing the fix and once not.
    """
    section = "#" if flat else "##"
    return [
        (
            1,
            f"# 1 The Artifact Thesis\n\nThe opening paragraph states it.\n\n"
            f"{section} 2. Preliminaries\n\nSome ground has to be cleared first.",
        ),
        (
            2,
            f"The clearing continues here.\n\n{section} 3.2 Hans Kelsen\n\n"
            f"Kelsen's version of the claim.",
        ),
        (
            3,
            f"# 2 A Second Chapter\n\nA rival account is set out.\n\n"
            f"{section} The Received View\n\nWhat the rival account denies.",
        ),
        (
            4,
            f"{section} 4. A Section Printed at the Top of a Page\n\n"
            f"The account is defended at length across a full page of prose.",
        ),
        (
            5,
            f"# 3 A Third Chapter\n\nThe argument turns.\n\n"
            f"{section} Further Reading\n\nWorks bearing on the turn.",
        ),
        (6, "# 4 A Fourth Chapter\n\nThe argument is drawn together and closed."),
    ]


class HeadingLevelTests(unittest.TestCase):
    """The shallowest heading level is what the launcher cuts chapters at.

    Anything sitting there that is not a chapter becomes one: a 293-page book
    with 12 chapters produced 71 headings at that level, 52 of them its own
    sections and 3 of them the second halves of titles that wrapped.
    """

    def normalize(self, pages):  # type: ignore[no-untyped-def]
        """Every case here is a whole book; the page range has its own test."""
        return pdf_text._normalize_heading_levels(pages, whole_document=True)

    def levels(self, pages):  # type: ignore[no-untyped-def]
        found: dict[str, int] = {}
        for _, markdown in pages:
            for line in markdown.splitlines():
                heading = pdf_text._heading(line)
                if heading is not None:
                    found[heading[1]] = heading[0]
        return found

    def test_a_section_below_a_chapter_is_demoted(self) -> None:
        levels = self.levels(self.normalize(levelled_book()))

        self.assertEqual(levels["2. Preliminaries"], 2)
        self.assertEqual(levels["3.2 Hans Kelsen"], 2)

    def test_an_unnumbered_section_is_demoted_too(self) -> None:
        """One companion volume numbers none of its sections."""
        levels = self.levels(self.normalize(levelled_book()))

        self.assertEqual(levels["The Received View"], 2)
        self.assertEqual(levels["Further Reading"], 2)

    def test_a_numbered_section_falling_at_the_top_of_a_page_is_demoted(self) -> None:
        """The one case the page it opens cannot settle on its own.

        Sections run on from the text above them, so opening a page is what
        marks a chapter — until a section happens to start one anyway, and
        only the number it carries still says what it is.
        """
        levels = self.levels(self.normalize(levelled_book()))

        self.assertEqual(levels["4. A Section Printed at the Top of a Page"], 2)

    def test_the_chapters_stay_where_they_are(self) -> None:
        levels = self.levels(self.normalize(levelled_book()))

        for title in ("1 The Artifact Thesis", "2 A Second Chapter", "3 A Third Chapter"):
            self.assertEqual(levels[title], 1, title)

    def test_a_book_whose_levels_are_already_right_is_left_alone(self) -> None:
        """The mutation: the same book with nothing wrong with it.

        Anything moved here is collateral damage. The tests above — where these
        very bodies do lose a level — are what stop this from passing for the
        trivial reason that the code moves nothing at all.
        """
        correct = levelled_book(flat=False)

        self.assertEqual(self.normalize(correct), correct)

    def test_a_book_that_numbers_its_chapters_keeps_them(self) -> None:
        """`1.` is a chapter here, not a section, and nothing else is competing.

        The mid-page heading is what gives the case its teeth: it has to be
        demoted, so the book cannot pass by being left alone wholesale. Read
        the dotted number on its own and the three chapters go down with it.
        """
        pages = [
            (1, "# 1. Introduction\n\nWhat the book is about.\n\n# A Note on Sources"),
            (2, "# 2. Method\n\nHow the question was approached."),
            (3, "# 3. Results\n\nWhat came of it."),
        ]

        levels = self.levels(self.normalize(pages))

        self.assertEqual(levels["A Note on Sources"], 2)
        for title in ("1. Introduction", "2. Method", "3. Results"):
            self.assertEqual(levels[title], 1, title)

    def test_a_book_whose_headings_never_open_a_page_is_left_alone(self) -> None:
        """Nothing here says which of these is a chapter, so nothing moves.

        Demoting the level wholesale would only hand the splitter the level
        below as the new shallowest one, leaving the book cut exactly where it
        was cut before.
        """
        pages = [
            (1, "Body text opens the page.\n\n# 2. A Section"),
            (2, "More body text.\n\n# 3. Another Section"),
        ]

        self.assertEqual(self.normalize(pages), pages)

    def test_one_heading_at_the_top_of_a_page_does_not_make_the_rest_sections(self) -> None:
        """A paper's sections are peers; one of them opening a page is luck.

        Read literally, the page break here says this document has a single
        chapter with every other heading nested under it. One chapter is the
        same cut as no chapter, so the level is left as the peers it is.
        """
        pages = [
            (1, "Body text opens the page.\n\n# Method\n\nHow it was done."),
            (2, "More body.\n\n# Results\n\nWhat came of it."),
            (3, "# Discussion\n\nWhat it means."),
            (4, "More body.\n\n# Conclusion\n\nWhat follows."),
        ]

        self.assertEqual(self.normalize(pages), pages)

    def test_a_heading_printed_before_the_first_chapter_is_left_alone(self) -> None:
        """A section belongs to a chapter, so it cannot come before all of them.

        This is also the shape a `pages=` range produces when it opens in the
        middle of a chapter, and the reason a heading before the first chapter
        is not read as a section of one.
        """
        pages = [
            (1, "Body text opens the page.\n\n# Foreword\n\nWhat the book owes."),
            (2, "# 1 The First Chapter\n\nThe opening paragraph."),
            (3, "The argument continues.\n\n# 2. A Section\n\nMore of it."),
            (4, "# 2 The Second Chapter\n\nThe second opening paragraph."),
        ]

        levels = self.levels(self.normalize(pages))

        self.assertEqual(levels["Foreword"], 1)
        self.assertEqual(levels["2. A Section"], 2)

    def test_a_page_range_is_not_levelled(self) -> None:
        """Which headings are chapters is a fact about the whole book.

        Levelling a window onto it would make the depth a heading came out at
        depend on the range the caller asked for.
        """
        pages = levelled_book()

        self.assertEqual(
            pdf_text._normalize_heading_levels(pages, whole_document=False), pages
        )

    def test_a_page_range_still_has_its_wrapped_titles_rejoined(self) -> None:
        """Both printed lines are inside the range, so both are visible."""
        pages = [(7, "# 9 Law Is an Institution, an Artifact,\n\n# and a Practice\n\nBody.")]

        levelled = pdf_text._normalize_heading_levels(pages, whole_document=False)

        self.assertIn("9 Law Is an Institution, an Artifact, and a Practice", self.levels(levelled))

    def test_a_folio_above_a_chapter_title_does_not_hide_it(self) -> None:
        """Some pages print the folio on its own line above the title."""
        pages = [
            (1, "# 1 The First Chapter\n\nThe opening paragraph."),
            (2, "The argument continues.\n\n# 2. A Section\n\nMore of it."),
            (3, "43\n\n# 2 The Second Chapter\n\nThe second opening paragraph."),
        ]

        levels = self.levels(self.normalize(pages))

        self.assertEqual(levels["2 The Second Chapter"], 1)
        self.assertEqual(levels["2. A Section"], 2)

    def test_a_deeper_level_is_not_touched(self) -> None:
        pages = [
            (1, "# 1 A Chapter\n\nBody.\n\n### 1.1.1 A Sub-Sub-Section\n\nMore body."),
            (2, "Body.\n\n# 2. A Section\n\nMore body."),
        ]

        levels = self.levels(self.normalize(pages))

        self.assertEqual(levels["1.1.1 A Sub-Sub-Section"], 3)

    def test_a_demoted_section_takes_its_own_subsections_down_with_it(self) -> None:
        """The half-right outline font sizing produces all the time.

        `2. Preliminaries` is a section printed at the chapter level while its
        own `Hans Kelsen` came out correctly deeper. Move the parent alone and
        the two end up peers, which is a worse outline than the one this pass
        was handed.
        """
        pages = [
            (1, "# 1 A Chapter\n\nBody.\n\n# 2. Preliminaries\n\nGround to clear."),
            (2, "## Hans Kelsen\n\nHis version.\n\n### The Basic Norm\n\nMore."),
            (3, "# 2 A Second Chapter\n\nBody.\n\n## A Section Of Its Own\n\nMore."),
        ]

        levels = self.levels(self.normalize(pages))

        self.assertEqual(levels["2. Preliminaries"], 2)
        self.assertEqual(levels["Hans Kelsen"], 3)
        self.assertEqual(levels["The Basic Norm"], 4)
        # The second chapter stays put, so nothing under it moves either.
        self.assertEqual(levels["2 A Second Chapter"], 1)
        self.assertEqual(levels["A Section Of Its Own"], 2)

    def test_a_subsection_already_at_the_bottom_stays_there(self) -> None:
        """Six hashes is as deep as Markdown goes; a seventh is body text.

        Flattening the last two levels of one branch costs an outline entry its
        depth. Writing `#######` would cost it its place in the outline
        altogether, and would take the line out of `_heading()`'s reach.
        """
        pages = [
            (1, "# 1 A Chapter\n\nBody.\n\n# 2. Preliminaries\n\nGround to clear."),
            (2, "##### Deep\n\nBody.\n\n###### Deeper\n\nMore body."),
            (3, "# 2 A Second Chapter\n\nBody."),
        ]

        levels = self.levels(self.normalize(pages))

        self.assertEqual(levels["Deep"], 6)
        self.assertEqual(levels["Deeper"], 6)


class WrappedHeadingTests(unittest.TestCase):
    """A title set over two printed lines comes back as two headings.

    Each half is then a chapter boundary of its own, and the second half is
    what names the chapter it opens: `Character of Legal Institutions`.
    """

    def titles(self, pages):  # type: ignore[no-untyped-def]
        return [
            heading[1]
            for _, markdown in pages
            for heading in (pdf_text._heading(line) for line in markdown.splitlines())
            if heading is not None
        ]

    def merge(self, *headings: str):  # type: ignore[no-untyped-def]
        body = "\n\n".join(headings) + "\n\nThe chapter opens here."
        return self.titles(pdf_text._merge_wrapped_headings([(1, body)]))

    def test_a_title_broken_on_a_dash_is_rejoined(self) -> None:
        self.assertEqual(
            self.merge("# 5 On the Artifactual— and Natural—", "# Character of Legal Institutions"),
            ["5 On the Artifactual— and Natural— Character of Legal Institutions"],
        )

    def test_a_title_broken_on_a_comma_is_rejoined(self) -> None:
        self.assertEqual(
            self.merge("# 9 Law Is an Institution, an Artifact,", "# and a Practice"),
            ["9 Law Is an Institution, an Artifact, and a Practice"],
        )

    def test_a_title_broken_after_a_dangling_word_is_rejoined(self) -> None:
        self.assertEqual(
            self.merge("# THE ROUTLEDGE COMPANION TO", "# PHILOSOPHY OF LAW"),
            ["THE ROUTLEDGE COMPANION TO PHILOSOPHY OF LAW"],
        )

    def test_a_title_broken_before_a_lower_case_of_is_rejoined(self) -> None:
        """No title of its own begins with a lower-case word."""
        self.assertEqual(
            self.merge("# 1 Problems and Promises", "# of Comparative Law"),
            ["1 Problems and Promises of Comparative Law"],
        )

    def test_a_chapter_title_beginning_with_of_is_left_as_two(self) -> None:
        """`Of Laws in General` is a chapter, and this library prints part
        dividers at the level of the chapters under them.

        Read the word alone and the two structural boundaries become one, with
        the chapter's text swallowed by whatever came before the divider. The
        case the word is set in is what says which of the two this is.
        """
        self.assertEqual(
            self.merge("# PART I", "# Of Laws in General"),
            ["PART I", "Of Laws in General"],
        )

    def test_a_capitalised_continuation_is_left_as_two_even_when_it_wrapped(self) -> None:
        """The price of the rule above, paid in the cheap direction.

        `THEORIES ABOUT THE NATURE` / `OF LAW` really is one title over two
        lines, and a book that sets its headings in capitals prints the same
        `OF` either way, so nothing in it distinguishes this from the divider
        above. A missed merge leaves the tail as a subsection of the chapter it
        belongs to; a wrong one destroys a chapter.
        """
        self.assertEqual(
            self.merge("# THEORIES ABOUT THE NATURE", "# OF LAW"),
            ["THEORIES ABOUT THE NATURE", "OF LAW"],
        )

    def test_a_divider_above_a_chapter_title_is_left_as_two(self) -> None:
        """The shape the merge must not touch: both halves are real headings.

        `EVIDENCE` over `IS IT FINALLY TIME TO PUT ...` reads exactly like a
        wrap until you know the book, which is why only breaks that cannot be
        anything else are joined.
        """
        self.assertEqual(
            self.merge("# PART I METHODOLOGY", "# 1 Legal Positivism about the Artifact Law"),
            ["PART I METHODOLOGY", "1 Legal Positivism about the Artifact Law"],
        )

    def test_a_title_beginning_with_the_is_left_as_two(self) -> None:
        """`THE` opens plenty of real chapters, so it cannot signal a wrap."""
        self.assertEqual(
            self.merge("# MORAL OBLIGATIONS TO LAW", "# THE MORAL OBLIGATION TO OBEY"),
            ["MORAL OBLIGATIONS TO LAW", "THE MORAL OBLIGATION TO OBEY"],
        )

    def test_headings_with_a_paragraph_between_them_are_left_as_two(self) -> None:
        pages = [(1, "# THE ROUTLEDGE COMPANION TO\n\nA paragraph.\n\n# PHILOSOPHY OF LAW")]

        self.assertEqual(
            self.titles(pdf_text._merge_wrapped_headings(pages)),
            ["THE ROUTLEDGE COMPANION TO", "PHILOSOPHY OF LAW"],
        )

    def test_headings_of_different_levels_are_left_as_two(self) -> None:
        self.assertEqual(
            self.merge("# THE ROUTLEDGE COMPANION TO", "## PHILOSOPHY OF LAW"),
            ["THE ROUTLEDGE COMPANION TO", "PHILOSOPHY OF LAW"],
        )


@unittest.skipUnless(REAL_CORPUS.is_dir(), "no ~/Zotero/storage on this machine")
class RealCorpusTests(unittest.TestCase):
    def extract(self, pattern: str):  # type: ignore[no-untyped-def]
        matches = sorted(REAL_CORPUS.glob(f"*/{pattern}"))
        if not matches:
            self.skipTest(f"no book matching {pattern} in the local library")
        return pdf_text.extract_markdown(matches[0])

    def test_born_digital_article_gains_structure(self) -> None:
        """The 88% of the corpus this whole module exists for."""
        result = self.extract("Purtova_Newell_2026_Against_Data_Fixation.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR)
        self.assertEqual(result.fallback_reason, "")
        self.assertGreater(len([line for line in result.markdown.splitlines() if line.startswith("#")]), 10)

    def test_cid_font_book_keeps_its_text(self) -> None:
        """pdf-inspector decodes this one's fonts to `!"#$%&'()` and gives up."""
        result = self.extract("*广松涉_2013_资本论的哲学.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertEqual(result.fallback_reason, "empty_markdown")
        self.assertGreater(result.chars, 300_000)

    def test_scan_backed_layout_book_keeps_its_text(self) -> None:
        """Its text layer decodes cleanly; pdf-inspector's layout stage bails."""
        result = self.extract("Esser_1972_Vorverst*richterlicher Entschei.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertEqual(result.fallback_reason, "empty_markdown")
        self.assertGreater(result.chars, 500_000)

    def test_broken_trailer_the_repair_save_rescues(self) -> None:
        """Half the broken-trailer files come back after PyMuPDF rewrites them."""
        result = self.extract("杜颖_2017_网络交易平台上的知识产权恶意投诉及其应对.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_INSPECTOR_REPAIRED)
        self.assertIn("invalid file trailer", result.fallback_reason)

    def test_broken_trailer_the_repair_save_cannot_rescue(self) -> None:
        """The other half keeps today's PyMuPDF-only behaviour rather than failing."""
        result = self.extract("叶名怡_2018_论个人信息权的基本范畴.pdf")

        self.assertEqual(result.engine, pdf_text.ENGINE_PYMUPDF)
        self.assertIn("invalid file trailer", result.fallback_reason)
        self.assertGreater(result.chars, 10_000)

    def test_chapters_named_after_their_own_running_head_keep_their_titles(self) -> None:
        """Five of this book's twelve chapters lost their names to head removal.

        The seven that survived did so by accident — a running head shortened
        from the title, or a title pdf-inspector broke over two lines — which
        is why the fixture above cannot stand in for this: the shapes that
        matter here are the ones a hand-written book never produces.
        """
        result = self.extract("2018_Law as an Artifact.pdf")
        titles = " ".join(result.markdown.split())

        for chapter in (
            "# 2 Law as a Malleable Artifact",
            "# 3 Law, Fiction, and Reality",
            "# 4 Law, Morality, Art, the Works",
            "# 8 Obligations from Artifacts",
            "# 11 A Strange Kind of Artifact",
        ):
            self.assertIn(chapter, titles)
        # The furniture those titles share a form with still goes.
        self.assertNotIn("## Law as a Malleable Artifact 31", titles)
        self.assertNotIn("## Law, Fiction, and Reality 45", titles)

    def test_over_flagged_book_is_still_extracted_in_full(self) -> None:
        """pdf-inspector wants OCR on 304 of its 346 pages; every page has text."""
        result = self.extract("阿马蒂亚·森_2013_以自由看待发展.pdf")

        self.assertGreater(result.chars, 300_000)
        self.assertFalse(hasattr(result, "pages_needing_ocr"))

    def chapter_level_headings(self, pattern: str) -> list[str]:
        """The headings the launcher would cut this book into chapters at.

        These books set their titles with non-breaking spaces, which say
        nothing about the level and only make the expected values unreadable.
        """
        result = self.extract(pattern)
        headings = [
            heading
            for heading in (
                pdf_text._heading(line) for line in result.markdown.splitlines()
            )
            if heading is not None
        ]
        if not headings:
            self.skipTest(f"{pattern} came back without headings")
        shallowest = min(level for level, _ in headings)
        return [
            title.replace("\xa0", " ") for level, title in headings if level == shallowest
        ]

    def test_a_books_own_sections_do_not_become_its_chapters(self) -> None:
        """12 chapters, 4 part dividers and 3 pieces of matter — 19 in all.

        Before this module levelled them, 71 of its headings sat at the level
        the launcher cuts at: every `2.`, `3.2` section of every chapter, plus
        the tail of each title that wrapped.
        """
        titles = self.chapter_level_headings("2018_Law as an Artifact.pdf")

        self.assertLess(len(titles), 3 * 19)
        self.assertIn("1 Legal Positivism about the Artifact Law", titles)
        self.assertIn("PART I METHODOLOGY", titles)
        self.assertNotIn("3.2 Hans Kelsen", titles)
        self.assertNotIn("2. Legal Positivism, Some Preliminaries", titles)
        # Printed at the top of page 39, so its number is the only thing left
        # that says it is a section.
        self.assertNotIn("6. The Case of Dworkin-Lite", titles)

    def test_a_wrapped_title_is_one_chapter_under_its_whole_name(self) -> None:
        """Printed over two lines, so pdf-inspector reports it as two headings."""
        titles = self.chapter_level_headings("2018_Law as an Artifact.pdf")

        self.assertIn(
            "5 On the Artifactual— and Natural— Character of Legal Institutions", titles
        )
        self.assertNotIn("Character of Legal Institutions", titles)

    def heading_levels(self, pattern: str) -> dict[str, int]:
        result = self.extract(pattern)
        return {
            heading[1].replace("\xa0", " "): heading[0]
            for heading in (
                pdf_text._heading(line) for line in result.markdown.splitlines()
            )
            if heading is not None
        }

    def test_a_book_whose_chapters_do_not_open_a_page_keeps_them(self) -> None:
        """A print-out with no page breaks between its chapters.

        Exactly one heading in these 254 pages falls at the top of one, so
        reading the page break as chapter evidence turns eight headings that
        say `Chapter N:` in so many words into sections of a ninth. Two
        chapters have to survive the test before any heading goes down.
        """
        titles = self.chapter_level_headings(
            "Tombal_2022_Imposing Data Sharing Among Private Actors*.pdf"
        )

        for chapter in range(2, 10):
            self.assertTrue(
                any(title.startswith(f"Chapter {chapter}:") for title in titles),
                f"Chapter {chapter} is no longer a chapter",
            )

    def test_a_demoted_section_keeps_its_own_subsections_under_it(self) -> None:
        """A German legal outline, half of it levelled correctly to begin with.

        `Einleitung` and `B. Know-how-Ausspähung durch „Reverse Engineering“`
        are sections printed at the chapter level, and pdf-inspector already
        put `A. Problemstellung` and `II. Beweggründe` a level below them.
        Moving the parents alone flattened each pair into peers.
        """
        levels = self.heading_levels(
            "Kochmann_2009_Schutz des know-how gegen ausspähende produktanalysen.pdf"
        )
        for section, subsection in (
            ("Einleitung", "A. Problemstellung"),
            (
                "B. Know-how-Ausspähung durch „Reverse Engineering“",
                "II. Beweggründe für „Reverse Engineering“",
            ),
        ):
            self.assertIn(section, levels)
            self.assertIn(subsection, levels)
            self.assertLess(levels[section], levels[subsection], subsection)

    def test_a_companion_volume_that_numbers_no_section_is_levelled_too(self) -> None:
        """43 chapters. Its sections carry no number to demote them by, so the
        page each heading opens is the only thing separating the two."""
        titles = self.chapter_level_headings(
            "Marmor_2014_The Routledge Companion to Philosophy of Law.pdf"
        )

        self.assertLess(len(titles), 3 * 43)
        self.assertIn("THE NATURE OF LAW", titles)
        self.assertNotIn("The Conditions of Legal Validity", titles)


if __name__ == "__main__":
    unittest.main()
