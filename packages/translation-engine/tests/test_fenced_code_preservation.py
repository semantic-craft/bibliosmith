from pathlib import Path
import tempfile
import unittest

from translation_engine.engine import run_manifest
from translation_engine.placeholders import (
    protect_chunk_structure,
    protect_markdown_for_chunking,
)
from translation_engine.providers import TranslationRequest
from tests.fixtures import build_run_fixture


class CapturingProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.requests: list[TranslationRequest] = []

    def translate(self, request: TranslationRequest) -> str:
        self.requests.append(request)
        return request.text


class FencedCodePreservationTests(unittest.TestCase):
    def test_large_fenced_code_is_hidden_from_provider_and_round_trips(self) -> None:
        fenced_body = (
            "def hidden():\n"
            "    return '`' * 400\n\n"
            "# heading-like code\n"
            + "print('private code')\n" * 50
        )
        source = (
            "# Intro\n\n"
            "Before.\n\n"
            "```python title=demo\n"
            f"{fenced_body}"
            "```\n\n"
            "## After\n\n"
            "After.\n"
        )

        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            manifest_path = build_run_fixture(
                project_root,
                source_text=source,
                max_tokens=60,
            )
            provider = CapturingProvider()

            report = run_manifest(
                manifest_path,
                provider_factory=lambda profile_id, *, config_id: provider,
            )

            self.assertEqual(report["units"][0]["status"], "completed")
            self.assertTrue(provider.requests)
            for protected_line in (
                "def hidden():",
                "return '`' * 400",
                "# heading-like code",
                "print('private code')",
            ):
                self.assertTrue(
                    all(
                        protected_line not in request.text
                        for request in provider.requests
                    )
                )
            self.assertEqual(
                sum("⟦PH_000000⟧" in request.text for request in provider.requests),
                1,
            )
            output = (
                project_root / "chapters" / "translated" / "chapter_001.md"
            ).read_text(encoding="utf-8")
            self.assertEqual(output, source)

    def test_tilde_fence_with_info_string_is_one_structure_safe_atom(self) -> None:
        fenced_block = (
            "~~~rust linenos\n"
            "fn main() {\n"
            '    println!("``` inside");\n'
            "}\n\n"
            "# heading-like code\n"
            "~~~"
        )
        source = f"Before.\n\n{fenced_block}\n\nAfter.\n"

        protected = protect_markdown_for_chunking(source)

        self.assertEqual(
            protected.replacements,
            (("⟦PH_000000⟧", fenced_block),),
        )
        self.assertEqual(protected.restore(protected.text), source)
        structured = protect_chunk_structure(protected.text, protected)
        self.assertEqual(structured.restore(structured.text), source)
        self.assertEqual(
            [
                original
                for _, original in structured.replacements
                if "heading-like code" in original
            ],
            [fenced_block],
        )

    def test_unclosed_fence_is_protected_through_end_of_file(self) -> None:
        fenced_tail = "```python\ndef truncated():\n    return 1\n\n# still code\n"
        source = f"Intro.\n\n{fenced_tail}"

        protected = protect_markdown_for_chunking(source)

        self.assertEqual(protected.text, "Intro.\n\n⟦PH_000000⟧")
        self.assertEqual(
            protected.replacements,
            (("⟦PH_000000⟧", fenced_tail),),
        )
        self.assertEqual(protected.restore(protected.text), source)

    def test_unclosed_fence_opener_at_end_of_file_is_protected(self) -> None:
        source = "Intro.\n\n~~~text"

        protected = protect_markdown_for_chunking(source)

        self.assertEqual(protected.text, "Intro.\n\n⟦PH_000000⟧")
        self.assertEqual(
            protected.replacements,
            (("⟦PH_000000⟧", "~~~text"),),
        )
        self.assertEqual(protected.restore(protected.text), source)


if __name__ == "__main__":
    unittest.main()
