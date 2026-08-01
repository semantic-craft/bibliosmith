from pathlib import Path
import tempfile
import unittest

from translation_engine.engine import (
    _candidate_is_acceptable,
    _candidate_avoids_previous_echo,
    _candidate_preserves_structure,
    _normalize_candidate_structure,
    _repetition_regions_align,
    _restore_chunks,
    run_manifest,
)
from translation_engine.placeholders import (
    protect_chunk_structure,
    protect_markdown,
    protect_markdown_for_chunking,
)
from translation_engine.profiles import ZH_HANS
from translation_engine.providers import TranslationRequest
from tests.fixtures import build_run_fixture


class AddsUnprotectedStructureProvider:
    profile_id = "structure-noise-provider"
    config_id = "offline"

    def translate(self, request: TranslationRequest) -> str:
        return (
            request.text.replace("Heading", "# 标题\n\n")
            .replace("Paragraph one.", "第一段。\n\n")
            .replace("Paragraph two.", "第二段。")
        )


class SplitsInsideProtectedParagraphProvider:
    profile_id = "structure-noise-provider"
    config_id = "offline"

    def __init__(self) -> None:
        self.calls = 0

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        return request.text.replace("Middle sentence.", "Middle\n\nsentence.")


class SplitsSingleParagraphProvider:
    profile_id = "structure-noise-provider"
    config_id = "offline"

    def __init__(self) -> None:
        self.calls = 0

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        return request.text.replace("Middle sentence.", "Middle\n\nsentence.")


class AddsVisualBreakThenRecoversProvider:
    profile_id = "structure-noise-provider"
    config_id = "offline"

    def __init__(self) -> None:
        self.calls = 0

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        if self.calls == 1:
            return r"第一句。\n第二句。"
        return "第一句。第二句。"


class AddsRepeatedTextThenRecoversProvider:
    profile_id = "content-noise-provider"
    config_id = "offline"

    def __init__(self) -> None:
        self.calls = 0

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        if self.calls == 1:
            return "商业秘密的经济价值需要单独评估。中间内容。商业秘密的经济价值需要单独评估。"
        return "商业秘密的经济价值需要单独评估。"


class EchoesPreviousChunkThenRecoversProvider:
    profile_id = "cross-chunk-echo-provider"
    config_id = "offline"

    def __init__(self) -> None:
        self.calls = 0

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        previous = "上一段的完整译文包含商业秘密经济价值分析。"
        if self.calls == 1:
            return previous
        if self.calls == 2:
            return f"{previous}当前段讨论损害赔偿。"
        return "当前段讨论损害赔偿。"


class StructurePreservationTests(unittest.TestCase):
    def test_chunk_boundary_whitespace_is_rebuilt_from_source(self) -> None:
        missing_boundary = [
            protect_markdown("First paragraph.\n"),
            protect_markdown("\nSecond paragraph.\n"),
        ]
        extra_boundary = [
            protect_markdown("First line.\n"),
            protect_markdown("Second line.\n"),
        ]

        self.assertEqual(
            _restore_chunks(missing_boundary, ["第一段。", "第二段。"]),
            "第一段。\n\n第二段。\n",
        )
        self.assertEqual(
            _restore_chunks(extra_boundary, ["第一行。\n\n", "第二行。"]),
            "第一行。\n第二行。\n",
        )
        self.assertEqual(
            _restore_chunks(
                [protect_markdown("Hello "), protect_markdown("world.")],
                ["你好", "世界。"],
            ),
            "你好世界。",
        )

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

    def test_markdown_heading_labels_are_protected_verbatim(self) -> None:
        source = (
            "## II. Wirtschaftlicher Wert\n\n"
            "### 3. Zwischenergebnis\n\n"
            "#### aa) Nutzungswert\n"
        )

        protected = protect_markdown(source)

        self.assertNotIn("II.", protected.text)
        self.assertNotIn("3.", protected.text)
        self.assertNotIn("aa)", protected.text)
        translated = (
            protected.text.replace("Wirtschaftlicher Wert", "经济价值")
            .replace("Zwischenergebnis", "中间结论")
            .replace("Nutzungswert", "使用价值")
        )
        self.assertEqual(
            protected.restore(translated),
            "## II. 经济价值\n\n### 3. 中间结论\n\n#### aa) 使用价值\n",
        )

        inline = protect_markdown_for_chunking(source)
        chunk = protect_chunk_structure(inline.text, inline)
        self.assertNotIn("II.", chunk.text)
        self.assertNotIn("3.", chunk.text)
        self.assertNotIn("aa)", chunk.text)

    def test_html_table_tags_round_trip_as_chunk_structure_placeholders(self) -> None:
        source = "<table><tr><td>Abkommen</td><td>Agreement</td></tr></table>"
        inline = protect_markdown_for_chunking(source)
        protected = protect_chunk_structure(inline.text, inline)

        self.assertNotIn("<table>", protected.text)
        self.assertNotIn("<td>", protected.text)
        translated = protected.text.replace("Abkommen", "协定").replace(
            "Agreement", "协议"
        )
        self.assertEqual(
            protected.restore(translated),
            "<table><tr><td>协定</td><td>协议</td></tr></table>",
        )

    def test_simplified_chinese_profile_requires_exact_markdown_structure(self) -> None:
        instruction = ZH_HANS.build_system_instruction(
            source_text="source", task_manifest={}
        )

        self.assertIn("Preserve every protected placeholder exactly", instruction)
        self.assertIn("Do not add, remove, merge, or split headings or paragraphs", instruction)

    def test_model_added_block_structure_is_removed_before_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text=(
                    "# Heading\n\nParagraph one.\n\nParagraph two.\n"
                ),
                max_tokens=200,
            )

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: (
                    AddsUnprotectedStructureProvider()
                ),
            )

            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")

        self.assertEqual(
            report["summary"], {"total": 1, "completed": 1, "failed": 0}
        )
        self.assertEqual(
            translated,
            "# 标题\n\n第一段。\n\n第二段。\n",
        )

    def test_direct_candidate_removes_an_extra_break_inside_a_protected_paragraph(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="First.\n\nMiddle sentence.\n\nThird.\n",
                max_tokens=200,
            )
            provider = SplitsInsideProtectedParagraphProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            unit = report["units"][0]
            self.assertEqual(unit["status"], "completed")
            self.assertEqual(unit["metrics"]["alignedFallbackCount"], 0)
            self.assertEqual(unit["metrics"]["sourceFallbackCount"], 0)
            self.assertEqual(provider.calls, 1)
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(
                translated,
                "First.\n\nMiddle sentence.\n\nThird.\n",
            )

    def test_direct_candidate_removes_an_extra_break_inside_a_single_paragraph(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="First. Middle sentence. Third.\n",
                max_tokens=200,
            )
            provider = SplitsSingleParagraphProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            unit = report["units"][0]
            self.assertEqual(unit["status"], "completed")
            self.assertEqual(unit["metrics"]["alignedFallbackCount"], 0)
            self.assertEqual(unit["metrics"]["sourceFallbackCount"], 0)
            self.assertEqual(provider.calls, 1)
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(translated, "First. Middle sentence. Third.\n")

    def test_default_prompt_forbids_visual_line_break_escapes(self) -> None:
        instruction = ZH_HANS.build_system_instruction(
            source_text="First line.\nSecond line.\n",
            task_manifest={},
        )

        self.assertIn("never spell a line break", instruction)
        self.assertIn("never insert HTML <br> tags", instruction)
        self.assertIn("Translate each source segment exactly once", instruction)
        self.assertIn("Never repeat or echo", instruction)
        self.assertIn("bibliography title text", instruction)

    def test_candidate_normalization_preserves_literal_content(self) -> None:
        protected = protect_markdown("Plain source.\n")
        candidate = r"Path C:\new\file and First<br>Second."

        self.assertEqual(
            _normalize_candidate_structure(protected, candidate),
            candidate,
        )

    def test_candidate_rejects_only_model_added_visual_break_tokens(self) -> None:
        literal_source = protect_markdown(r"Path C:\new\file.")
        html_break_source = protect_markdown("First<br>Second<BR />Third.")
        plain_source = protect_markdown("Plain source.")

        self.assertTrue(
            _candidate_preserves_structure(literal_source, r"路径 C:\new\file。")
        )
        self.assertTrue(
            _candidate_preserves_structure(
                html_break_source,
                "第一句<br>第二句<BR />第三句。",
            )
        )
        self.assertFalse(
            _candidate_preserves_structure(plain_source, r"第一句。\n第二句。")
        )
        self.assertFalse(
            _candidate_preserves_structure(plain_source, "第一句。<br>第二句。")
        )

    def test_default_translation_retries_model_added_visual_break(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="First sentence. Second sentence.\n",
                max_tokens=200,
            )
            provider = AddsVisualBreakThenRecoversProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")

        self.assertEqual(
            report["summary"], {"total": 1, "completed": 1, "failed": 0}
        )
        self.assertEqual(provider.calls, 2)
        self.assertEqual(translated, "第一句。第二句。\n")

    def test_candidate_rejects_model_added_repetition(self) -> None:
        protected = protect_markdown("Trade secret value needs a separate assessment.")

        self.assertFalse(
            _candidate_is_acceptable(
                protected,
                "商业秘密的经济价值需要单独评估。中间内容。商业秘密的经济价值需要单独评估。",
            )
        )
        self.assertFalse(
            _candidate_is_acceptable(protected, "专家证言专家证言可用于证明。")
        )
        self.assertFalse(
            _candidate_is_acceptable(protected, "商业秘密" * 8)
        )
        self.assertFalse(
            _candidate_is_acceptable(protected, "商业秘密" * 10)
        )

    def test_candidate_allows_repetition_present_in_source(self) -> None:
        protected = protect_markdown(
            "Trade secret value needs a separate assessment. "
            "Trade secret value needs a separate assessment."
        )

        self.assertTrue(
            _candidate_is_acceptable(
                protected,
                "商业秘密的经济价值需要单独评估。商业秘密的经济价值需要单独评估。",
            )
        )

    def test_candidate_rejects_extra_repetition_when_source_already_repeats(self) -> None:
        protected = protect_markdown(
            "The source repeats this sentence. The source repeats this sentence."
        )

        self.assertFalse(
            _candidate_is_acceptable(
                protected,
                "这是源文中本来就重复的合法完整句子。"
                "这是源文中本来就重复的合法完整句子。"
                "商业秘密的经济价值需要单独评估。中间内容。"
                "商业秘密的经济价值需要单独评估。",
            )
        )

    def test_candidate_rejects_repetition_added_at_a_different_position(self) -> None:
        protected = protect_markdown(
            "The source repeats this complete sentence near the beginning. "
            "The source repeats this complete sentence near the beginning. "
            "The remaining source is unique and continues for several clauses."
        )

        self.assertFalse(
            _candidate_is_acceptable(
                protected,
                "源文开头的内容只出现一次，随后是互不重复的中间内容。"
                "最后却新增了本不应重复的完整句子。过渡文字。"
                "最后却新增了本不应重复的完整句子。",
            )
        )

    def test_shifted_repetition_regions_use_an_ordered_matching(self) -> None:
        self.assertTrue(
            _repetition_regions_align(
                source_regions=(0.026, 0.196),
                candidate_regions=(0.180, 0.380),
            )
        )
        self.assertTrue(
            _repetition_regions_align(
                source_regions=(0.102023, 0.240173, 0.402890),
                candidate_regions=(0.092043, 0.239905, 0.605107),
            )
        )

    def test_candidate_rejects_a_long_unchanged_source_passage(self) -> None:
        source = (
            "Geschäftsgeheimnisse besitzen einen wirtschaftlichen Wert, wenn ihre "
            "Geheimhaltung einen Wettbewerbsvorteil erhält. Diese Voraussetzung muss "
            "anhand der konkreten Nutzungsmöglichkeiten und der drohenden Nachteile "
            "beurteilt werden. Eine bloß abstrakte Behauptung genügt hierfür nicht."
        )

        self.assertFalse(_candidate_is_acceptable(protect_markdown(source), source))

    def test_candidate_rejects_a_long_unchanged_russian_passage(self) -> None:
        source = (
            "Коммерческая тайна обладает экономической ценностью, когда ее "
            "секретность обеспечивает конкурентное преимущество и возможность "
            "практического использования. "
        ) * 4

        self.assertFalse(_candidate_is_acceptable(protect_markdown(source), source))

    def test_candidate_rejects_a_long_unchanged_japanese_passage(self) -> None:
        source = (
            "営業秘密は、その秘密性が競争上の優位性と具体的な利用可能性をもたらす場合に、"
            "経済的価値を有すると評価されます。単なる抽象的な主張だけでは十分ではありません。"
        ) * 4

        self.assertFalse(_candidate_is_acceptable(protect_markdown(source), source))

    def test_candidate_allows_short_traceability_metadata(self) -> None:
        source = (
            "ISBN 978-3-631-92029-9 https://www.peterlang.com "
            "Zhang Xianwei, gakalone@gmail.com"
        )

        self.assertTrue(_candidate_is_acceptable(protect_markdown(source), source))

    def test_candidate_allows_a_long_bare_url(self) -> None:
        source = "https://example.com/" + ("traceability-identifier/" * 12)

        self.assertTrue(_candidate_is_acceptable(protect_markdown(source), source))

    def test_candidate_allows_target_language_source_text(self) -> None:
        source = "商业秘密的经济价值需要结合具体利用可能性和潜在损害加以判断。" * 10

        self.assertTrue(_candidate_is_acceptable(protect_markdown(source), source))

    def test_default_translation_retries_model_added_repetition(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Trade secret value needs a separate assessment.\n",
                max_tokens=200,
            )
            provider = AddsRepeatedTextThenRecoversProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")

        self.assertEqual(
            report["summary"], {"total": 1, "completed": 1, "failed": 0}
        )
        self.assertEqual(provider.calls, 2)
        self.assertEqual(translated, "商业秘密的经济价值需要单独评估。\n")

    def test_candidate_rejects_echo_from_previous_chunk(self) -> None:
        self.assertFalse(
            _candidate_avoids_previous_echo(
                previous_source="Valuation principles govern the earlier discussion.",
                current_source="Damages are calculated from the claimant's loss.",
                previous_candidate="上一段的完整译文包含商业秘密经济价值分析。",
                candidate="上一段的完整译文包含商业秘密经济价值分析。当前段讨论损害赔偿。",
            )
        )

    def test_candidate_allows_cross_chunk_text_repeated_by_source(self) -> None:
        source_shared = "9783631920381 - Zhang Xianwei - Nicht zum Wiederverkauf."
        target_shared = "9783631920381 - 张贤伟 - 不得转售。"

        self.assertTrue(
            _candidate_avoids_previous_echo(
                previous_source=f"Previous page. {source_shared}",
                current_source=f"{source_shared} Current page.",
                previous_candidate=f"上一页。{target_shared}",
                candidate=f"{target_shared} 当前页。",
            )
        )

    def test_candidate_allows_a_legal_phrase_shared_inside_both_chunks(self) -> None:
        shared = "商业秘密经济价值应结合具体利用可能性判断"

        self.assertTrue(
            _candidate_avoids_previous_echo(
                previous_source="Earlier text cites the shared legal standard, then ends.",
                current_source="The next paragraph introduces damages before citing it.",
                previous_candidate=f"上一段引用{shared}，然后讨论举证责任。",
                candidate=f"本段先讨论损害赔偿，随后再次说明{shared}。",
            )
        )

    def test_unrelated_source_overlap_does_not_hide_a_real_boundary_echo(self) -> None:
        shared_source = "a shared source phrase that appears away from the boundary"
        echoed = "上一段结尾的完整译文被错误复制到了下一段"

        self.assertFalse(
            _candidate_avoids_previous_echo(
                previous_source=f"{shared_source}; previous source ends elsewhere.",
                current_source=f"Current source starts elsewhere; {shared_source}.",
                previous_candidate=f"上一段正文。{echoed}",
                candidate=f"{echoed}本段原本应从损害赔偿开始。",
            )
        )

    def test_short_source_boundary_overlap_does_not_excuse_a_long_echo(self) -> None:
        echoed = "上一段结尾的完整译文被复制到这里"

        self.assertFalse(
            _candidate_avoids_previous_echo(
                previous_source="Previous page ends with Page label",
                current_source="Page label starts the next page",
                previous_candidate=f"上一段正文。{echoed}",
                candidate=f"{echoed}本段正文。",
            )
        )

    def test_long_source_boundary_overlap_does_not_excuse_a_short_unrelated_echo(
        self,
    ) -> None:
        source_overlap = "A deliberately long repeated source boundary phrase. " * 2
        echoed = "上一段结尾的无关译文被复制到这里"

        self.assertFalse(
            _candidate_avoids_previous_echo(
                previous_source=f"Previous source. {source_overlap}",
                current_source=f"{source_overlap}Current source.",
                previous_candidate=f"上一段正文。{echoed}",
                candidate=f"{echoed}本段正文。",
            )
        )

    def test_shared_identifier_does_not_excuse_an_extra_sentence_echo(self) -> None:
        shared = "9783631920381"
        echoed = "上一段结尾的一整句错误译文被复制到了当前段开头"

        self.assertFalse(
            _candidate_avoids_previous_echo(
                previous_source=f"Previous source. {shared}",
                current_source=f"{shared} Current source.",
                previous_candidate=f"上一段。{shared} - {echoed}",
                candidate=f"{shared} - {echoed}本段正文。",
            )
        )

    def test_default_translation_retries_previous_chunk_echo(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text=(
                    "Valuation principles govern the earlier discussion.\n"
                    "Damages are calculated from the claimant's loss.\n"
                ),
                max_tokens=54,
            )
            provider = EchoesPreviousChunkThenRecoversProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")

        self.assertEqual(
            report["summary"], {"total": 1, "completed": 1, "failed": 0}
        )
        self.assertEqual(provider.calls, 3)
        self.assertEqual(
            translated,
            "上一段的完整译文包含商业秘密经济价值分析。\n当前段讨论损害赔偿。\n",
        )

if __name__ == "__main__":
    unittest.main()
