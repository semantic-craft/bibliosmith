"""The warning classifier and the marker vocabulary it emits."""

from __future__ import annotations

import logging
import unittest

from layout_pdf.warnings import (
    KIND_LARGE_PAGE,
    KIND_OTHER,
    WarningCollector,
    classify,
)


def record(message: str, level: int = logging.WARNING) -> logging.LogRecord:
    return logging.LogRecord(
        name="babeldoc.test",
        level=level,
        pathname=__file__,
        lineno=1,
        msg=message,
        args=None,
        exc_info=None,
    )


class ClassifyTests(unittest.TestCase):
    def test_recognises_the_large_page_warning(self) -> None:
        self.assertEqual(
            classify("page 41 is too large, maybe unable to translate"),
            KIND_LARGE_PAGE,
        )

    def test_matches_regardless_of_case(self) -> None:
        self.assertEqual(classify("Page 41 IS TOO LARGE"), KIND_LARGE_PAGE)

    def test_anything_unrecognised_falls_through_to_other(self) -> None:
        self.assertEqual(classify("Data-loss while decompressing"), KIND_OTHER)

    def test_a_message_merely_mentioning_a_page_is_not_a_large_page(self) -> None:
        self.assertEqual(classify("page 41 rendered without fonts"), KIND_OTHER)


class WarningCollectorTests(unittest.TestCase):
    def test_counts_by_kind(self) -> None:
        collector = WarningCollector()
        collector.emit(record("page 1 is too large, maybe unable to translate"))
        collector.emit(record("page 2 is too large, maybe unable to translate"))
        collector.emit(record("Data-loss while decompressing"))

        self.assertEqual(
            collector.markers(),
            [
                "BOOK_PIPELINE_MARKER warning=large_page count=2",
                "BOOK_PIPELINE_MARKER warning=other count=1",
            ],
        )

    def test_a_clean_run_emits_no_markers(self) -> None:
        self.assertEqual(WarningCollector().markers(), [])

    def test_info_records_are_ignored(self) -> None:
        collector = WarningCollector()
        collector.emit(record("nothing to see here", level=logging.INFO))
        self.assertEqual(collector.markers(), [])

    def test_markers_never_carry_the_message_text(self) -> None:
        # The Launcher's marker parser drops unknown fields, but a message that
        # interpolated a file path must not be built into a marker at all.
        collector = WarningCollector()
        collector.emit(record("could not open /library/storage/ABCD1234/secret.pdf"))
        for marker in collector.markers():
            self.assertNotIn("secret", marker)
            self.assertNotIn("/library", marker)


if __name__ == "__main__":
    unittest.main()
