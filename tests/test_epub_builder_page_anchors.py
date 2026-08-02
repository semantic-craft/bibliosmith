"""Page anchors in the monolingual EPUB builder (issue #108).

The PaddleOCR assembler writes a ``<!-- page: N -->`` anchor between pages so a
reviewer can map a translated passage back to a page of the original, and picked
a comment precisely so the marker would stay out of the prose. `build_epub.js`
recognises a fixed list of block tags as raw HTML and escapes everything else, so
an anchor matched nothing, fell through to the paragraph buffer and reached the
reader as a paragraph of literal ``<!-- page: N -->`` -- once per page, in every
book converted that way. The same defect was fixed in the bilingual builder
first; these tests pin the monolingual half.
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

# The shape `process_book` in packages/ocr/scripts/pdf_to_html_paddleocr.py
# actually assembles: each page is `f"\n{page_anchor(page_no)}\n\n{text}"` and
# the pages are joined with a blank line, so an anchor stands alone between two
# blank lines. The anchor's own text is pinned by
# packages/ocr/tests/test_paddle_page_anchors.py.
ANCHORED_CHAPTER = (
    "# 第一章\n"
    "\n"
    "\n"
    "<!-- page: 42 -->\n"
    "\n"
    "第四十二页的正文。\n"
    "\n"
    "\n"
    "<!-- page: 43 -->\n"
    "\n"
    "第四十三页的正文。\n"
)


def build_chapter(chapter_markdown: str) -> str:
    """Build a one-chapter book and return its chapter XHTML."""
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
            json.dumps(
                {"source_file_name": "Anchor Fixture.pdf", "target_language": "zh-Hans"}
            ),
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
            return archive.read("EPUB/chapter_001.xhtml").decode("utf-8")


def body_of(chapter: str) -> str:
    return chapter.split("<body>", 1)[1]


class EpubBuilderPageAnchorTests(unittest.TestCase):
    def test_page_anchors_never_reach_the_reader(self) -> None:
        chapter = build_chapter(ANCHORED_CHAPTER)

        # Neither as the escaped text the reader used to see, nor as a real
        # comment smuggled through unescaped: the anchor is gone entirely.
        self.assertNotIn("page:", chapter)
        self.assertNotIn("&lt;!--", chapter)
        self.assertNotIn("<!--", body_of(chapter))

    def test_the_prose_either_side_of_an_anchor_is_untouched(self) -> None:
        # Dropping the anchor must not merge the pages it separated, nor leave
        # the empty paragraph a "render it as nothing" fix would emit.
        body = body_of(build_chapter(ANCHORED_CHAPTER))

        self.assertIn("<p>第四十二页的正文。</p>", body)
        self.assertIn("<p>第四十三页的正文。</p>", body)
        self.assertEqual(body.count("<p>"), 2)
        self.assertNotIn("<p></p>", body)

    def test_a_comment_inside_a_paragraph_is_still_that_paragraph(self) -> None:
        # The over-eager mutation: strip comments from every paragraph rather
        # than drop paragraphs that are only comments, and this sentence loses
        # its middle.
        chapter = build_chapter("# 第一章\n\nA sentence <!-- an aside --> carrying it.\n")

        self.assertIn("A sentence &lt;!-- an aside --&gt; carrying it.", chapter)

    def test_a_comment_on_its_own_line_within_a_paragraph_stays(self) -> None:
        # The mutation a line-based state machine invites: skip the comment line
        # instead of testing the flushed paragraph, and the anchor is silently
        # cut out of the middle of a paragraph that has real content around it.
        # Only a paragraph that is *entirely* comments is the marker.
        body = body_of(build_chapter("# 第一章\n\n上半句。\n<!-- page: 42 -->\n下半句。\n"))

        self.assertIn("<p>上半句。 &lt;!-- page: 42 --&gt; 下半句。</p>", body)

    def test_a_real_sentence_between_two_comments_survives(self) -> None:
        # The false positive a `startswith("<!--") and endswith("-->")` test
        # would produce: a real sentence deleted along with the comments.
        chapter = build_chapter("# 第一章\n\n<!-- page: 1 --> 真正的句子 <!-- x -->\n")

        self.assertIn("真正的句子", chapter)

    def test_a_fenced_comment_is_a_code_sample_not_an_anchor(self) -> None:
        # A book about this pipeline quotes the anchor in a listing. Fences are
        # matched before the paragraph buffer, so the sample survives verbatim.
        chapter = build_chapter("# 第一章\n\n```html\n<!-- page: 42 -->\n```\n")

        self.assertIn(
            '<pre><code class="language-html">&lt;!-- page: 42 --&gt;</code></pre>',
            chapter,
        )

    def test_a_multi_line_comment_block_also_goes(self) -> None:
        body = body_of(build_chapter("# 第一章\n\n<!--\n  page: 42\n-->\n\n正文。\n"))

        self.assertNotIn("page:", body)
        self.assertEqual(body.count("<p>"), 1)

    def test_two_anchors_in_one_block_go_together(self) -> None:
        body = body_of(
            build_chapter("# 第一章\n\n<!-- page: 42 -->\n<!-- page: 43 -->\n\n正文。\n")
        )

        self.assertNotIn("page:", body)
        self.assertEqual(body.count("<p>"), 1)

    def test_an_unterminated_comment_is_left_alone(self) -> None:
        # Not a comment as far as any parser is concerned, so not this rule's
        # business to delete: it reaches the reader escaped, as before.
        chapter = build_chapter("# 第一章\n\n<!-- 没有收尾的注释\n")

        self.assertIn("&lt;!-- 没有收尾的注释", chapter)

    def test_a_run_on_comment_containing_a_heading_goes(self) -> None:
        # The line-based state machine used to hand each line of a run-on
        # comment to the heading, list and table rules before `isCommentOnly`
        # ever saw the whole thing: the delimiters reached the reader in pieces
        # and `# hidden` became a real <h1>. The bilingual builder splits on
        # blank lines, so this was one block there and was dropped correctly --
        # the two builders disagreed on the same Markdown.
        chapter = build_chapter("# 第一章\n\n<!--\n# hidden\n-->\n\n正文。\n")
        body = body_of(chapter)

        self.assertNotIn("hidden", body)
        self.assertEqual(body.count("<h1>"), 1)
        self.assertNotIn("&lt;!--", body)
        self.assertIn("<p>正文。</p>", body)

    def test_a_run_on_comment_containing_a_blank_line_goes(self) -> None:
        # A blank line inside a comment is the comment's, not a paragraph break.
        body = body_of(build_chapter("# 第一章\n\n<!--\n\n注释正文\n-->\n\n正文。\n"))

        self.assertNotIn("注释正文", body)
        self.assertEqual(body.count("<p>"), 1)

    def test_a_run_on_comment_containing_a_table_goes(self) -> None:
        body = body_of(
            build_chapter("# 第一章\n\n<!--\n| a | b |\n| --- | --- |\n| c | d |\n-->\n\n正文。\n")
        )

        self.assertNotIn("<table>", body)
        self.assertNotIn("&lt;!--", body)
        self.assertEqual(body.count("<p>"), 1)

    def test_a_run_on_comment_inside_a_paragraph_stays(self) -> None:
        # Same constraint as the single-line case: only a paragraph that is
        # entirely comments goes, so this one keeps its comment and its prose.
        body = body_of(
            build_chapter("# 第一章\n\n上半句。\n<!--\n夹在中间\n-->\n下半句。\n")
        )

        self.assertIn("<p>上半句。 &lt;!-- 夹在中间 --&gt; 下半句。</p>", body)

    def test_a_fenced_run_on_comment_is_still_a_code_sample(self) -> None:
        chapter = build_chapter("# 第一章\n\n```html\n<!--\n# 代码样例\n-->\n```\n")

        self.assertIn(
            '<pre><code class="language-html">&lt;!--\n# 代码样例\n--&gt;</code></pre>',
            chapter,
        )

    def test_an_unclosed_run_on_comment_leaves_the_rest_of_the_chapter_alone(self) -> None:
        # An unclosed fence runs to the end of the document because CommonMark
        # says so; nothing says that of a stray `<!--`. Consuming to the end
        # here would collapse every remaining paragraph into one over a typo,
        # so an opener with no closer falls through to the ordinary rules.
        body = body_of(
            build_chapter("# 第一章\n\n<!--\n没有收尾\n\n后面的正文。\n\n## 二级标题\n")
        )

        self.assertIn("<p>后面的正文。</p>", body)
        self.assertIn("<h2>二级标题</h2>", body)

    def test_a_line_that_merely_ends_with_an_open_comment_is_unchanged(self) -> None:
        # The run-on rule is deliberately narrow: the line has to *open* with
        # the comment. A heading that trails one is still a heading, exactly as
        # before -- widening this would quietly restructure real prose.
        body = body_of(build_chapter("# 第一章\n\n## 标题 <!--\n注释\n-->\n"))

        self.assertIn("<h2>标题 &lt;!--</h2>", body)

    def test_an_abrupt_closing_comment_is_left_alone(self) -> None:
        # `<!-->` carries its own `--` and `>`, so a closer searched for from
        # the start of the paragraph rather than from past the opener finds one
        # inside the opener itself and the paragraph disappears. The bilingual
        # builder's regex does not match this either; both leave it escaped.
        chapter = build_chapter("# 第一章\n\n<!-->\n")

        self.assertIn("&lt;!--&gt;", chapter)

    def test_an_anchor_beside_a_raw_html_line_still_goes(self) -> None:
        # The raw-HTML branch flushes the paragraph buffer itself, so an anchor
        # butted straight against an `<aside>` is flushed by that branch rather
        # than by a blank line -- and has to be dropped there too.
        body = body_of(
            build_chapter("# 第一章\n\n<!-- page: 42 -->\n<aside>边注</aside>\n")
        )

        self.assertNotIn("page:", body)
        self.assertIn("<aside>边注</aside>", body)

    def test_an_anchor_does_not_become_the_chapter_title(self) -> None:
        # A chapter file may open with an anchor, since the splitter cuts the
        # assembled Markdown wherever the headings fall.
        chapter = build_chapter("<!-- page: 42 -->\n\n# 真正的标题\n\n正文。\n")

        self.assertIn("<title>真正的标题</title>", chapter)


if __name__ == "__main__":
    unittest.main()
