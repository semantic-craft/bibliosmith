"""Fenced code blocks in the EPUB builder (issue #121).

`build_epub.js` converts `chapters/final/*.md` with a line-based state machine.
Before this suite it had no fence branch, so a fenced block was not merely
flattened into a paragraph: its lines were fed to the heading, list and raw-HTML
rules, and `# comment` became a real `<h1>` in the finished book. These tests
pin both halves — the block renders as code, and nothing inside it is read as
structure.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile


REPO_ROOT = Path(__file__).resolve().parents[1]
SOURCE_SCRIPTS = REPO_ROOT / "tools" / "bibliosmith-launcher" / "source" / "scripts"


def build_book(chapter_markdown: str) -> tuple[str, str]:
    """Build a one-chapter book and return its chapter XHTML and stylesheet."""
    with tempfile.TemporaryDirectory() as temporary_directory:
        book_root = Path(temporary_directory) / "book"
        scripts = book_root / "scripts"
        final = book_root / "chapters" / "final"
        metadata = book_root / "metadata"
        scripts.mkdir(parents=True)
        final.mkdir(parents=True)
        metadata.mkdir(parents=True)
        shutil.copy(SOURCE_SCRIPTS / "build_epub.js", scripts / "build_epub.js")
        shutil.copy(SOURCE_SCRIPTS / "run_python.js", scripts / "run_python.js")
        (final / "chapter_001.md").write_text(chapter_markdown, encoding="utf-8")
        metadata.joinpath("source_manifest.json").write_text(
            json.dumps({"source_file_name": "Code Fixture.epub", "target_language": "zh-Hans"}),
            encoding="utf-8",
        )

        completed = subprocess.run(
            ["node", str(scripts / "build_epub.js")],
            check=False,
            capture_output=True,
            text=True,
        )
        assert completed.returncode == 0, completed.stderr

        with ZipFile(book_root / "output" / "reading" / "book.epub") as archive:
            return (
                archive.read("EPUB/chapter_001.xhtml").decode("utf-8"),
                archive.read("EPUB/styles/book.css").decode("utf-8"),
            )


class EpubBuilderCodeBlockTests(unittest.TestCase):
    def test_a_fenced_block_becomes_pre_code_with_its_line_breaks(self) -> None:
        chapter, _ = build_book(
            "# 第一章\n\n```\nfirst line\nsecond line\n```\n\n收尾段落。\n"
        )

        self.assertIn("<pre><code>first line\nsecond line</code></pre>", chapter)
        # The delimiters must not survive as body text, which is exactly what
        # the paragraph fallback used to do.
        self.assertNotIn("```", chapter)

    def test_a_hash_inside_a_block_is_not_promoted_to_a_heading(self) -> None:
        # The structural half of the bug: a comment line became a real <h1>,
        # which then reached the navigation document as a chapter landmark.
        chapter, _ = build_book("# 第一章\n\n```python\n# 这是注释\ncode()\n```\n")

        self.assertIn("# 这是注释", chapter)
        self.assertNotIn("<h1># 这是注释</h1>", chapter)
        self.assertEqual(chapter.count("<h1>"), 1)

    def test_list_and_raw_html_lines_inside_a_block_stay_literal(self) -> None:
        chapter, _ = build_book(
            '# 第一章\n\n```\n1. not a list item\n<div>not raw html</div>\n```\n'
        )

        self.assertNotIn('<p class="list-item">', chapter)
        self.assertIn("1. not a list item", chapter)
        # Escaped, not passed through: an unbalanced tag from a code sample
        # would otherwise break XHTML well-formedness and fail EPUBCheck.
        self.assertIn("&lt;div&gt;not raw html&lt;/div&gt;", chapter)

    def test_an_info_string_becomes_a_language_class(self) -> None:
        chapter, _ = build_book("# 第一章\n\n```Python\nprint(1)\n```\n")

        self.assertIn('<pre><code class="language-python">print(1)</code></pre>', chapter)

    def test_tilde_fences_and_blank_lines_inside_a_block_are_kept(self) -> None:
        # A blank line used to flush the paragraph buffer, cutting the block in
        # two; tildes were never recognised at all.
        chapter, _ = build_book("# 第一章\n\n~~~\nfirst\n\nthird\n~~~\n")

        self.assertIn("<pre><code>first\n\nthird</code></pre>", chapter)

    def test_a_longer_fence_closes_only_on_a_matching_run(self) -> None:
        chapter, _ = build_book("# 第一章\n\n````\n```\ninner\n```\n````\n")

        self.assertIn("<pre><code>```\ninner\n```</code></pre>", chapter)

    def test_an_unclosed_fence_runs_to_the_end_without_failing_the_build(self) -> None:
        # The chapter's final newline is inside the block, and stays there: the
        # conversion escapes and nothing else.
        chapter, _ = build_book("# 第一章\n\n```\nno closing fence\n")

        self.assertIn("<pre><code>no closing fence\n</code></pre>", chapter)

    def test_inline_code_outside_a_fence_still_works(self) -> None:
        chapter, _ = build_book("# 第一章\n\n段落里的 `inline()` 调用。\n")

        self.assertIn("<code>inline()</code>", chapter)
        self.assertNotIn("<pre>", chapter)

    def test_a_backtick_span_on_its_own_line_is_not_read_as_a_fence(self) -> None:
        chapter, _ = build_book("# 第一章\n\n``a `b` c`` 是一个行内代码段。\n")

        self.assertNotIn("<pre>", chapter)

    def test_the_stylesheet_wraps_long_code_lines(self) -> None:
        # An e-reader page cannot scroll sideways, so an unwrapped line is simply
        # cut off; pre-wrap is what makes a long line readable at all.
        _, stylesheet = build_book("# 第一章\n\n```\nx = 1\n```\n")

        self.assertIn("pre{", stylesheet)
        self.assertIn("white-space:pre-wrap", stylesheet)


class EpubBuilderCodeBlockReviewTests(unittest.TestCase):
    """Findings the automated review raised on PR #122."""

    def test_a_heading_inside_a_fence_does_not_name_the_chapter(self) -> None:
        # The body was already fence-aware, but the title came from a second
        # regex pass over the raw Markdown, so the sample's `# ` line became the
        # <title> and the navigation entry — a table of contents named after
        # code that is not a heading.
        chapter, _ = build_book(
            "```\n# code heading\n```\n\n# 真正的标题\n\n正文。\n"
        )

        self.assertIn("<title>真正的标题</title>", chapter)
        self.assertNotIn("<title># code heading</title>", chapter)

    def test_the_navigation_entry_uses_the_rendered_heading(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            book_root = Path(temporary_directory) / "book"
            scripts = book_root / "scripts"
            final = book_root / "chapters" / "final"
            metadata = book_root / "metadata"
            scripts.mkdir(parents=True)
            final.mkdir(parents=True)
            metadata.mkdir(parents=True)
            shutil.copy(SOURCE_SCRIPTS / "build_epub.js", scripts / "build_epub.js")
            shutil.copy(SOURCE_SCRIPTS / "run_python.js", scripts / "run_python.js")
            (final / "chapter_001.md").write_text(
                "```\n# code heading\n```\n\n# 真正的标题\n\n正文。\n", encoding="utf-8"
            )
            metadata.joinpath("source_manifest.json").write_text(
                json.dumps({"source_file_name": "N.epub", "target_language": "zh-Hans"}),
                encoding="utf-8",
            )
            completed = subprocess.run(
                ["node", str(scripts / "build_epub.js")],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)

            with ZipFile(book_root / "output" / "reading" / "book.epub") as archive:
                nav = archive.read("EPUB/nav.xhtml").decode("utf-8")

            self.assertIn("真正的标题", nav)
            self.assertNotIn("# code heading", nav)

    def test_trailing_whitespace_and_blank_lines_inside_a_block_survive(self) -> None:
        # An escape-only conversion has no business trimming the sample.
        chapter, _ = build_book("# 第一章\n\n```\nrow with space   \n\n\n```\n")

        self.assertIn("<pre><code>row with space   \n\n</code></pre>", chapter)


if __name__ == "__main__":
    unittest.main()
