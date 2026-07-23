import unittest

from translation_engine.sampling import (
    select_internal_blocks,
    truncate_at_sentence_boundary,
)


class SamplingTests(unittest.TestCase):
    def test_uniform_selection_excludes_first_and_last_blocks(self) -> None:
        blocks = ["front", "one", "two", "three", "four", "five", "back"]

        selected = select_internal_blocks(blocks, 3)

        self.assertEqual(selected, ["two", "three", "four"])
        self.assertNotIn("front", selected)
        self.assertNotIn("back", selected)

    def test_selection_returns_each_internal_block_once_when_requested_count_is_large(self) -> None:
        blocks = ["front", "one", "two", "back"]

        selected = select_internal_blocks(blocks, 8)

        self.assertEqual(selected, ["one", "two"])
        self.assertEqual(len(selected), len(set(selected)))

    def test_sentence_truncation_includes_the_sentence_that_crosses_budget(self) -> None:
        text = "First short. Second sentence crosses the budget! Third stays out."

        excerpt = truncate_at_sentence_boundary(text, 15)

        self.assertEqual(excerpt, "First short. Second sentence crosses the budget!")
        self.assertGreater(len(excerpt), 15)

    def test_sentence_truncation_preserves_pathological_text_without_boundaries(self) -> None:
        text = "a pathological block with no sentence boundary at all"

        excerpt = truncate_at_sentence_boundary(text, 8)

        self.assertEqual(excerpt, text)


if __name__ == "__main__":
    unittest.main()
