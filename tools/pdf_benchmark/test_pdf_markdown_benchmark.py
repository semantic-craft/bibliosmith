"""Unit tests for the PDF→Markdown benchmark harness.

These are stdlib-only and touch no PDF: they pin the metric definitions, which
are where a benchmark quietly stops measuring what it claims to. The one test
that needs the production worker skips itself when PyMuPDF and `requests` are
not installed in the running environment, so the repository-wide suite can run
it without depending on the OCR package.
"""

from __future__ import annotations

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from pdf_markdown_benchmark import (
    STATUS_EMPTY,
    STATUS_ERROR,
    STATUS_OK,
    EngineOutcome,
    FileRecord,
    MarkdownMetrics,
    cjk_ratio,
    file_magic,
    home_relative,
    load_worker_module,
    measure_markdown,
    render_page_scaffold_markdown,
    select_corpus,
    sub_sample,
    summarize,
)


class MarkdownMetricsTests(unittest.TestCase):
    def test_page_scaffolding_is_never_counted_as_a_real_heading(self):
        markdown = "## Page 1\n\nbody text\n\n## Page 2\n\nmore text\n"
        metrics = measure_markdown(markdown)
        self.assertEqual(metrics.scaffolding_headings, 2)
        self.assertEqual(metrics.real_headings, 0)

    def test_recovered_headings_are_counted_as_real(self):
        markdown = "# The Book\n\n## Chapter One\n\n#### viii Editors' Introduction\n"
        metrics = measure_markdown(markdown)
        self.assertEqual(metrics.real_headings, 3)
        self.assertEqual(metrics.scaffolding_headings, 0)

    def test_scaffolding_matches_any_level_and_case(self):
        markdown = "# Page 3\n\n###### page 4\n\n## Pages 5\n"
        metrics = measure_markdown(markdown)
        self.assertEqual(metrics.scaffolding_headings, 2)
        self.assertEqual(metrics.real_headings, 1)

    def test_table_delimiter_rows_are_not_content_rows(self):
        markdown = "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n"
        self.assertEqual(measure_markdown(markdown).table_rows, 3)

    def test_links_exclude_images_and_include_autolinks(self):
        markdown = "See [the spec](https://example.invalid/spec) ![cover](img.png) <https://example.invalid/a>\n"
        self.assertEqual(measure_markdown(markdown).links, 2)

    def test_fenced_code_is_counted_once_and_shields_its_contents(self):
        markdown = "text\n\n```python\n# not a heading\n| not | a table |\n```\n\n## Chapter\n"
        metrics = measure_markdown(markdown)
        self.assertEqual(metrics.code_blocks, 1)
        self.assertEqual(metrics.real_headings, 1)
        self.assertEqual(metrics.table_rows, 0)

    def test_nonspace_chars_ignores_every_kind_of_whitespace(self):
        self.assertEqual(measure_markdown("a b\tc\nd\r\n").nonspace_chars, 4)

    def test_metrics_add_componentwise(self):
        left = MarkdownMetrics(1, 2, 3, 4, 5, 6)
        right = MarkdownMetrics(10, 20, 30, 40, 50, 60)
        self.assertEqual(left + right, MarkdownMetrics(11, 22, 33, 44, 55, 66))


class LanguageSignalTests(unittest.TestCase):
    def test_chinese_title_is_detected(self):
        self.assertGreater(cjk_ratio("王利明_2013_论个人信息权的法律保护"), 0.15)

    def test_latin_title_is_not(self):
        self.assertEqual(cjk_ratio("Lessig_2008_Remix"), 0.0)

    def test_empty_text_does_not_divide_by_zero(self):
        self.assertEqual(cjk_ratio("   \n\t"), 0.0)


class FileMagicTests(unittest.TestCase):
    def setUp(self):
        self._tmp = TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def test_pdf_json_and_other_are_told_apart(self):
        (self.root / "real.pdf").write_bytes(b"%PDF-1.7\n...")
        (self.root / "stored.pdf").write_bytes(b'\n  {"error": "not found"}')
        (self.root / "junk.pdf").write_bytes(b"<html>nope</html>")
        self.assertEqual(file_magic(self.root / "real.pdf"), "pdf")
        self.assertEqual(file_magic(self.root / "stored.pdf"), "json")
        self.assertEqual(file_magic(self.root / "junk.pdf"), "other")

    def test_missing_file_is_unreadable_rather_than_a_crash(self):
        self.assertEqual(file_magic(self.root / "absent.pdf"), "unreadable")


class CorpusSelectionTests(unittest.TestCase):
    def setUp(self):
        self._tmp = TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        for index in range(20):
            item = self.root / f"KEY{index:02d}"
            item.mkdir()
            (item / f"book{index:02d}.pdf").write_bytes(b"%PDF-1.7")

    def select(self, **kwargs):
        options = {
            "root": self.root,
            "glob": "*/*.pdf",
            "stride": 1,
            "offset": 0,
            "limit": None,
            "file_list": None,
        }
        options.update(kwargs)
        return select_corpus(**options)

    def test_stride_sampling_is_deterministic_and_sorted(self):
        first = self.select(stride=7)
        second = self.select(stride=7)
        self.assertEqual(first, second)
        self.assertEqual([path.name for path in first], ["book00.pdf", "book07.pdf", "book14.pdf"])

    def test_offset_shifts_the_sample(self):
        self.assertEqual([path.name for path in self.select(stride=7, offset=1)], ["book01.pdf", "book08.pdf", "book15.pdf"])

    def test_limit_applies_after_striding(self):
        self.assertEqual([path.name for path in self.select(stride=7, limit=2)], ["book00.pdf", "book07.pdf"])

    def test_file_list_overrides_sampling_and_skips_comments(self):
        listing = self.root / "corpus.txt"
        listing.write_text(
            f"# a comment\n{self.root / 'KEY03' / 'book03.pdf'}\n\n{self.root / 'KEY05' / 'book05.pdf'}\n",
            encoding="utf-8",
        )
        selected = self.select(file_list=listing)
        self.assertEqual([path.name for path in selected], ["book03.pdf", "book05.pdf"])

    def test_extraction_sub_sample_keeps_the_first_of_each_stride(self):
        paths = [Path(f"{index}.pdf") for index in range(10)]
        self.assertEqual([path.name for path in sub_sample(paths, stride=4, limit=None)], ["0.pdf", "4.pdf", "8.pdf"])
        self.assertEqual([path.name for path in sub_sample(paths, stride=4, limit=2)], ["0.pdf", "4.pdf"])


def _record(name, *, magic="pdf", chinese=False, page_count=None, classify=None, extract=None):
    record = FileRecord(path=Path("/corpus") / name, magic=magic, size_bytes=1024)
    record.chinese = chinese
    record.page_count = page_count
    record.classify = classify or {}
    record.extract = extract or {}
    return record


def _extracted(status, *, seconds=1.0, metrics=None, title=None):
    return EngineOutcome(status, seconds, detail={"title": title}, metrics=metrics or MarkdownMetrics())


class SummaryTests(unittest.TestCase):
    """The aggregation is where a benchmark quietly starts flattering itself."""

    def test_failure_rate_is_over_real_pdfs_not_over_every_file(self):
        records = [
            _record("a.pdf", classify={"cand": EngineOutcome(STATUS_OK, 0.1)}),
            _record("b.pdf", classify={"cand": EngineOutcome(STATUS_ERROR, 0.1, "invalid file trailer")}),
            _record("stored.pdf", magic="json", classify={"cand": EngineOutcome(STATUS_ERROR, 0.1, "not a PDF")}),
        ]
        engines = summarize(records, ["cand"])["classification"]["engines"]["cand"]
        self.assertEqual(engines["real_pdf_failures"], 1)
        self.assertEqual(engines["real_pdf_failure_rate"], 0.5)
        self.assertEqual(engines["non_pdf_rejected"], 1)

    def test_chinese_failures_are_counted_separately(self):
        records = [
            _record("知网.pdf", chinese=True, classify={"cand": EngineOutcome(STATUS_ERROR, 0.1, "trailer")}),
            _record("Lessig.pdf", classify={"cand": EngineOutcome(STATUS_ERROR, 0.1, "trailer")}),
        ]
        engines = summarize(records, ["cand"])["classification"]["engines"]["cand"]
        self.assertEqual(engines["real_pdf_failures"], 2)
        self.assertEqual(engines["real_pdf_failures_chinese"], 1)

    def test_paired_subset_excludes_files_only_one_engine_handled(self):
        both = _record(
            "both.pdf",
            page_count=100,
            extract={
                "ours": _extracted(STATUS_OK, metrics=MarkdownMetrics(0, 100, 0, 0, 0, 5000)),
                "cand": _extracted(STATUS_OK, metrics=MarkdownMetrics(40, 0, 12, 3, 0, 4900), title="A Book"),
            },
        )
        one_sided = _record(
            "cid-font.pdf",
            page_count=480,
            extract={
                "ours": _extracted(STATUS_OK, metrics=MarkdownMetrics(0, 480, 0, 0, 0, 357000)),
                "cand": _extracted(STATUS_EMPTY, metrics=MarkdownMetrics()),
            },
        )
        extraction = summarize([both, one_sided], ["ours", "cand"])["extraction"]
        self.assertEqual(extraction["attempted"], 2)
        self.assertEqual(extraction["paired"]["files"], 1)
        self.assertEqual(extraction["paired"]["pages"], 100)
        # The one-sided book's 357k characters must not inflate our column in a
        # table whose whole point is that the two columns describe one corpus.
        self.assertEqual(extraction["paired"]["engines"]["ours"]["nonspace_chars"], 5000)
        self.assertEqual(extraction["paired"]["engines"]["ours"]["scaffolding_headings"], 100)
        self.assertEqual(extraction["paired"]["engines"]["ours"]["real_headings"], 0)
        self.assertEqual(extraction["paired"]["engines"]["cand"]["titles_recovered"], 1)
        self.assertEqual(extraction["engines"]["cand"]["empty"], 1)

    def test_no_extraction_pass_leaves_the_extraction_section_out(self):
        summary = summarize([_record("a.pdf", classify={"ours": EngineOutcome(STATUS_OK, 0.1)})], ["ours"])
        self.assertNotIn("extraction", summary)


class HomeRelativeTests(unittest.TestCase):
    def test_home_prefix_is_replaced(self):
        self.assertEqual(home_relative(str(Path.home() / "Zotero" / "storage")), "~/Zotero/storage")

    def test_other_paths_are_untouched(self):
        self.assertEqual(home_relative("/opt/corpus"), "/opt/corpus")


class ProductionShapeTests(unittest.TestCase):
    """The benchmark must render what the shipped route renders.

    `render_page_scaffold_markdown` deliberately drops the Zotero front matter
    and the `# {title}` line, because neither comes from the PDF. Everything
    below that has to stay byte-identical to `render_markdown`, or the harness
    starts measuring a document the pipeline never produces.
    """

    def test_page_body_matches_the_worker_renderer(self):
        try:
            worker = load_worker_module()
        except Exception as exc:  # pragma: no cover - environment-dependent
            self.skipTest(f"zotero_llm_worker is not importable here: {exc}")
        pages = [(1, "first page text"), (2, ""), (3, "third page text")]
        produced = worker.render_markdown(
            title="A Book",
            metadata={"route": "pdf-text", "page_count": 3},
            pages=pages,
        )
        self.assertTrue(
            produced.endswith(render_page_scaffold_markdown(pages)),
            "benchmark page rendering drifted from zotero_llm_worker.render_markdown",
        )

    def test_empty_pages_keep_the_worker_placeholder(self):
        markdown = render_page_scaffold_markdown([(1, "")])
        self.assertIn("[no extractable text]", markdown)
        self.assertEqual(measure_markdown(markdown).scaffolding_headings, 1)


if __name__ == "__main__":
    unittest.main()
