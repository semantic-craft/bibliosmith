from pathlib import Path
import re
import tempfile
import unittest

from translation_engine.engine import run_manifest
from translation_engine.placeholders import PLACEHOLDER_PATTERN
from translation_engine.providers import TranslationRequest
from tests.fixtures import build_run_fixture


class GlossaryFollowingProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.instructions: list[str] = []

    def translate(self, request: TranslationRequest) -> str:
        self.instructions.append(request.system_instruction)
        translated = request.text
        for source, target in re.findall(
            r"^- (.+?) -> (.+?)(?: \[.*\])?$", request.system_instruction, re.MULTILINE
        ):
            for variant in source.split("|"):
                translated = translated.replace(variant, target)
        return translated


class GlossaryIgnoringProvider:
    """Returns the source untouched: structurally valid, terminologically blind.

    The double above applies the glossary by construction, so no test using it
    could ever have caught a model that ignores the glossary entirely -- which
    is the one failure the output check exists to make visible.
    """

    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def translate(self, request: TranslationRequest) -> str:
        return request.text


class GlossarySubstitutingProvider:
    """Translates, but reaches for a synonym instead of the required term."""

    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self, substitutions: dict[str, str]) -> None:
        self.substitutions = substitutions

    def translate(self, request: TranslationRequest) -> str:
        translated = request.text
        for source, replacement in self.substitutions.items():
            translated = translated.replace(source, replacement)
        return translated


class StructureBreakingProvider:
    """Adds a paragraph, so every chunk fails validation and falls back to source."""

    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def translate(self, request: TranslationRequest) -> str:
        return f"{request.text}\n\nAdded paragraph."


class GlossaryEnforcementTests(unittest.TestCase):
    def test_placeholder_example_is_stable_and_coexists_with_glossary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Bridge one.\n\nBridge two.\n",
                max_tokens=14,
                glossary_text=(
                    "source,translation,category,note\n"
                    "Bridge,桥,location,\n"
                ),
            )
            provider = GlossaryFollowingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertEqual(len(provider.instructions), 2)
            example = (
                "# EXAMPLE: PLACEHOLDER PRESERVATION\n"
                "```text\n"
                "Source: At dawn, we crossed ⟦PH_000000⟧the bridge⟦PH_000001⟧.\n"
                "Translation: 黎明时，我们穿过了⟦PH_000000⟧那座桥⟦PH_000001⟧。\n"
                "```"
            )
            source_placeholders = PLACEHOLDER_PATTERN.findall(
                "At dawn, we crossed ⟦PH_000000⟧the bridge⟦PH_000001⟧."
            )
            translated_placeholders = PLACEHOLDER_PATTERN.findall(
                "黎明时，我们穿过了⟦PH_000000⟧那座桥⟦PH_000001⟧。"
            )
            self.assertEqual(
                source_placeholders,
                ["⟦PH_000000⟧", "⟦PH_000001⟧"],
            )
            self.assertEqual(translated_placeholders, source_placeholders)
            for instruction in provider.instructions:
                self.assertIn(example, instruction)
                self.assertIn("# GLOSSARY - REQUIRED TRANSLATIONS", instruction)

    def test_book_glossary_is_filtered_per_chunk_and_enforced_consistently(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text=(
                    "Fan keeps Secret.\n\n"
                    "Fantasy waits.\n\n"
                    "Fan keeps Secret.\n"
                ),
                max_tokens=18,
                glossary_text=(
                    "source,translation,category,note\n"
                    "Fan,风扇,item,\n"
                    "Secret,秘密,other,\n"
                ),
            )
            provider = GlossaryFollowingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertEqual(len(provider.instructions), 3)
            self.assertIn("# GLOSSARY - REQUIRED TRANSLATIONS", provider.instructions[0])
            self.assertIn("- Fan -> 风扇 [item]", provider.instructions[0])
            self.assertIn("- Secret -> 秘密 [other]", provider.instructions[0])
            self.assertNotIn("# GLOSSARY", provider.instructions[1])
            # Same glossary block on the repeated chunk; the later chunk also
            # carries the previous chunk's translation tail as CONTEXT.
            self.assertTrue(
                provider.instructions[2].startswith(provider.instructions[0])
            )
            translated = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(
                translated,
                "风扇 keeps 秘密.\n\nFantasy waits.\n\n风扇 keeps 秘密.\n",
            )

    def test_filter_uses_cjk_substrings_latin_boundaries_and_variants(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Fantasy records secrets in 神经网络。\n",
                max_tokens=100,
                glossary_text=(
                    "source,translation,category,note\n"
                    "Fan,风扇,item,\n"
                    "secret|secrets,商业秘密,other,\n"
                    "网络,网络,other,\n"
                ),
            )
            provider = GlossaryFollowingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            instruction = provider.instructions[0]
            self.assertNotIn("- Fan ->", instruction)
            self.assertIn("- secret|secrets -> 商业秘密 [other]", instruction)
            self.assertIn("- 网络 -> 网络 [other]", instruction)

    def test_filter_caps_entries_by_frequency_then_longest_variant(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            entries = [
                ("Low", "低"),
                ("LongerLow", "较长低频"),
                *((f"Term{index:02}", f"术语{index:02}") for index in range(48)),
                ("High", "高频"),
            ]
            source_text = " ".join(source for source, _ in entries) + " High High\n"
            glossary_text = "source,translation,category,note\n" + "".join(
                f"{source},{translation},other,\n" for source, translation in entries
            )
            manifest_path = build_run_fixture(
                project_root,
                source_text=source_text,
                max_tokens=1000,
                glossary_text=glossary_text,
            )
            provider = GlossaryFollowingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            glossary_lines = [
                line
                for line in provider.instructions[0].splitlines()
                if line.startswith("- ")
            ]
            self.assertEqual(len(glossary_lines), 50)
            self.assertIn("- High -> 高频 [other]", glossary_lines)
            self.assertIn("- LongerLow -> 较长低频 [other]", glossary_lines)
            self.assertNotIn("- Low -> 低 [other]", glossary_lines)

    def test_changed_glossary_requires_a_new_prepared_task_hash(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text="Existing term.\n",
                max_tokens=100,
                glossary_text=(
                    "source,translation,category,note\nExisting,既有,other,\n"
                ),
            )
            (project_root / "glossary" / "terms.csv").write_text(
                "source,translation,category,note\nExisting,新译名,other,reviewed\n",
                encoding="utf-8",
            )
            provider = GlossaryFollowingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "failed")
            self.assertEqual(
                report["units"][0]["error"]["code"], "glossary_hash_mismatch"
            )
            self.assertEqual(provider.instructions, [])


class GlossaryOutputCheckTests(unittest.TestCase):
    """Enforcement used to stop at the prompt: a model was told the terms and
    nothing ever looked at what came back. These cover the looking."""

    def _run(self, provider, **fixture_kwargs):
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(project_root, **fixture_kwargs)
            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )
            translated_path = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            )
            translated = (
                translated_path.read_text(encoding="utf-8")
                if translated_path.exists()
                else None
            )
            return report["units"][0], translated

    def test_ignored_glossary_is_reported_with_the_terms_that_drifted(self) -> None:
        unit, _ = self._run(
            GlossaryIgnoringProvider(),
            source_text="Fan keeps Secret.\n",
            max_tokens=100,
            glossary_text=(
                "source,translation,category,note\n"
                "Fan,风扇,item,\n"
                "Secret,秘密,other,\n"
            ),
        )

        self.assertEqual(unit["status"], "completed")
        self.assertEqual(unit["metrics"]["glossaryViolationCount"], 2)
        self.assertEqual(
            sorted(
                (violation["source"], violation["translation"])
                for violation in unit["glossaryViolations"]
            ),
            [("Fan", "风扇"), ("Secret", "秘密")],
        )

    def test_a_synonym_in_place_of_the_required_term_is_reported(self) -> None:
        unit, translated = self._run(
            GlossarySubstitutingProvider({"Fan": "电扇"}),
            source_text="Fan keeps turning.\n",
            max_tokens=100,
            glossary_text="source,translation,category,note\nFan,风扇,item,\n",
        )

        self.assertEqual(unit["metrics"]["glossaryViolationCount"], 1)
        self.assertEqual(unit["glossaryViolations"][0]["translation"], "风扇")
        # The wrong term is left exactly where the model put it.
        self.assertEqual(translated, "电扇 keeps turning.\n")

    def test_following_the_glossary_reports_nothing(self) -> None:
        unit, _ = self._run(
            GlossaryFollowingProvider(),
            source_text="Fan keeps Secret.\n",
            max_tokens=100,
            glossary_text=(
                "source,translation,category,note\n"
                "Fan,风扇,item,\n"
                "Secret,秘密,other,\n"
            ),
        )

        self.assertEqual(unit["metrics"]["glossaryViolationCount"], 0)
        self.assertNotIn("glossaryViolations", unit)

    def test_terms_absent_from_the_source_are_never_demanded(self) -> None:
        """A glossary entry the chunk does not use is not injected, so it cannot
        be violated -- otherwise every book would report its whole glossary."""
        unit, _ = self._run(
            GlossaryIgnoringProvider(),
            source_text="Nothing relevant here.\n",
            max_tokens=100,
            glossary_text="source,translation,category,note\nFan,风扇,item,\n",
        )

        self.assertEqual(unit["metrics"]["glossaryViolationCount"], 0)
        self.assertNotIn("glossaryViolations", unit)

    def test_one_term_missed_in_many_chunks_counts_once(self) -> None:
        """The count should describe the terminology, not the chapter length."""
        unit, _ = self._run(
            GlossaryIgnoringProvider(),
            source_text="Fan one.\n\nFan two.\n\nFan three.\n",
            max_tokens=12,
            glossary_text="source,translation,category,note\nFan,风扇,item,\n",
        )

        self.assertGreater(unit["metrics"]["chunkCount"], 1)
        self.assertEqual(unit["metrics"]["glossaryViolationCount"], 1)

    def test_chunks_that_fell_back_to_source_are_not_counted_as_drift(self) -> None:
        """An untranslated chunk is already reported as a source fallback.
        Counting its terms again would bury real drift under the same event."""
        unit, _ = self._run(
            StructureBreakingProvider(),
            source_text="Fan keeps Secret.\n",
            max_tokens=100,
            glossary_text=(
                "source,translation,category,note\n"
                "Fan,风扇,item,\n"
                "Secret,秘密,other,\n"
            ),
        )

        self.assertEqual(unit["status"], "failed")
        self.assertGreater(unit["metrics"]["sourceFallbackCount"], 0)
        self.assertEqual(unit["metrics"]["glossaryViolationCount"], 0)
        self.assertNotIn("glossaryViolations", unit)

    def test_the_check_never_degrades_or_rewrites(self) -> None:
        """Reported, never enforced: Chinese compounds make hard rejection a bad
        trade, so a violation must not cost the user a completed chapter."""
        unit, translated = self._run(
            GlossaryIgnoringProvider(),
            source_text="Fan keeps Secret.\n",
            max_tokens=100,
            glossary_text="source,translation,category,note\nFan,风扇,item,\n",
        )

        self.assertEqual(unit["status"], "completed")
        self.assertEqual(unit["artifact"]["complete"], True)
        self.assertEqual(unit["metrics"]["alignedFallbackCount"], 0)
        self.assertEqual(unit["metrics"]["sourceFallbackCount"], 0)
        self.assertNotIn("error", unit)
        self.assertEqual(translated, "Fan keeps Secret.\n")


if __name__ == "__main__":
    unittest.main()
