import unittest

from translation_engine.placeholders import protect_markdown
from translation_engine.profiles import ZH_HANS


class StructurePreservationTests(unittest.TestCase):
    def test_yaml_front_matter_is_preserved_verbatim(self) -> None:
        source = "---\nprivate_path: /local/book.pdf\nroute: pdf-text\n---\n\n# Heading\n"

        protected = protect_markdown(source)

        self.assertNotIn("private_path", protected.text)
        translated = protected.text.replace("Heading", "标题")
        self.assertEqual(
            protected.restore(translated),
            "---\nprivate_path: /local/book.pdf\nroute: pdf-text\n---\n\n# 标题\n",
        )

    def test_markdown_headings_and_paragraph_boundaries_round_trip_as_placeholders(self) -> None:
        source = "# Heading\n\nParagraph one.\n\n## Subheading\n\nParagraph two.\n"

        protected = protect_markdown(source)

        self.assertNotIn("# ", protected.text)
        self.assertNotIn("## ", protected.text)
        self.assertNotIn("\n\n", protected.text)
        translated = (
            protected.text.replace("Heading", "标题")
            .replace("Paragraph one.", "第一段。")
            .replace("Subheading", "小标题")
            .replace("Paragraph two.", "第二段。")
        )
        self.assertEqual(
            protected.restore(translated),
            "# 标题\n\n第一段。\n\n## 小标题\n\n第二段。\n",
        )

    def test_simplified_chinese_profile_requires_exact_markdown_structure(self) -> None:
        instruction = ZH_HANS.build_system_instruction(
            source_text="source", task_manifest={}
        )

        self.assertIn("Preserve every protected placeholder exactly", instruction)
        self.assertIn("Do not add, remove, merge, or split headings or paragraphs", instruction)


if __name__ == "__main__":
    unittest.main()
