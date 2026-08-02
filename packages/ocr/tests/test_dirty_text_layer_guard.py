"""Guard tests for text_layer_quality().

Fixture shapes are taken from the ~/Zotero/storage measurements in #138 rather
than invented, because the failure this guards against is invisible to clean
synthetic text: Chinese academic PDFs legitimately carry a high fullwidth-ASCII
ratio, and the guard used to read that as mojibake.
"""

from dataclasses import replace
from pathlib import Path
import sys
import unittest


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from zotero_llm_worker import (  # noqa: E402
    count_nonspace,
    get_config,
    text_layer_quality,
)


def to_fullwidth(text: str) -> str:
    return "".join(
        chr(ord(ch) + 0xFEE0) if "!" <= ch <= "~" else ("　" if ch == " " else ch)
        for ch in text
    )


class DirtyTextLayerGuardTests(unittest.TestCase):
    def config(self, **overrides):
        base = replace(
            get_config(),
            dirty_text_guard=True,
            dirty_text_min_chars=1000,
            dirty_text_private_use_ratio=0.005,
        )
        return replace(base, **overrides) if overrides else base

    def judge(self, text: str, config=None):
        config = config or self.config()
        return text_layer_quality(text, count_nonspace(text), config)

    def test_chinese_journal_fullwidth_typesetting_is_not_dirty(self) -> None:
        """#140: the shape of 曾俊_2020 / 王锡锌_2021 — CJK body plus fullwidth Latin.

        Measured fullwidth-ASCII ratio on these books runs 0.15-0.46 with zero
        private-use characters, and every one of them reads correctly. The old
        fullwidth branch blocked 31 such books and caught nothing this check misses.
        """
        body = "数据界权的关系进路与个人信息保护的规范结构在此展开讨论。" * 20
        references = to_fullwidth("L'Oreal v. eBay, Upload Monitoring, SJTU Law Review, 2011. ") * 12
        text = body + references

        nonspace = [ch for ch in text if not ch.isspace()]
        fullwidth = sum(1 for ch in nonspace if 0xFF01 <= ord(ch) <= 0xFF5E)
        self.assertGreater(fullwidth / len(nonspace), 0.15, "fixture must reproduce the real ratio")
        self.assertGreaterEqual(count_nonspace(text), 1000, "fixture must clear dirty_text_min_chars")

        degraded, reason = self.judge(text)

        self.assertFalse(degraded, f"legitimate fullwidth typesetting must not block: {reason}")
        self.assertEqual("", reason)

    def test_private_use_punctuation_soup_is_dirty(self) -> None:
        """The shape of 高秦伟_2019: an OCR text layer with punctuation in U+E5xx."""
        text = ("个人信息概念之反思和重塑\ue5d2\ue5cf立法与实践的理论起点\ue5e5" * 40) + "补充正文" * 200

        degraded, reason = self.judge(text)

        self.assertTrue(degraded)
        self.assertIn("private_use_ratio", reason)

    def test_private_use_digits_are_dirty_without_any_cjk(self) -> None:
        """The shape of Gordon_2003: oldstyle digits mapped into U+F73x.

        Pure English, so this also pins that the check does not depend on a CJK ratio.
        """
        text = ("Copyright \uf732\uf730\uf730\uf733 by the author, page \uf736\uf731\uf738. " * 60) + "x" * 400

        degraded, reason = self.judge(text)

        self.assertTrue(degraded)
        self.assertIn("private_use_ratio", reason)

    def test_private_use_below_threshold_is_clean(self) -> None:
        text = "ordinary body text that reads fine " * 60 + "\ue5d2"

        degraded, reason = self.judge(text)

        self.assertFalse(degraded, reason)

    def test_short_sample_is_not_evaluated(self) -> None:
        """Below dirty_text_min_chars the sample is too small to judge."""
        text = "\ue5d2\ue5cf\ue5e5" * 20

        degraded, reason = self.judge(text)

        self.assertFalse(degraded, reason)

    def test_guard_off_never_blocks(self) -> None:
        text = ("\ue5d2\ue5cf" * 400) + "正文" * 400

        degraded, reason = self.judge(text, self.config(dirty_text_guard=False))

        self.assertFalse(degraded, reason)
        self.assertTrue(self.judge(text)[0], "same text must block while the guard is on")


if __name__ == "__main__":
    unittest.main()
