from contextlib import redirect_stdout
from io import StringIO
import json
from pathlib import Path
import tempfile
import unittest

from translation_engine.ner_cli import main
from translation_engine.providers import TranslationRequest


class FakeNerProvider:
    profile_id = "fake-provider-profile"
    config_id = "fake-config-no-secrets"

    def __init__(self) -> None:
        self.requests: list[TranslationRequest] = []

    def translate(self, request: TranslationRequest) -> str:
        self.requests.append(request)
        return """<think>private reasoning is ignored</think>
<NER_JSON>
```json
{"candidates": [
  {"entity": "Alice", "suggested_translation": "爱丽丝", "type": "character"},
  {"source": "Wonderland", "target": "仙境", "category": "location"},
],}
```
</NER_JSON>"""


class NerCliTests(unittest.TestCase):
    def test_independent_command_writes_review_candidates_without_merging_glossary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            project_root = Path(temporary_directory)
            source_path = project_root / "source" / "source.md"
            source_path.parent.mkdir(parents=True)
            source_path.write_text("A" * 6500, encoding="utf-8")
            glossary_path = project_root / "glossary" / "terms.csv"
            glossary_path.parent.mkdir(parents=True)
            original_glossary = (
                "source,translation,category,note\nExisting,既有,other,reviewed\n"
            )
            glossary_path.write_text(original_glossary, encoding="utf-8")
            provider = FakeNerProvider()
            stdout = StringIO()

            with redirect_stdout(stdout):
                exit_code = main(
                    [
                        "--project-root",
                        str(project_root),
                        "--provider-profile-id",
                        provider.profile_id,
                        "--provider-config-id",
                        provider.config_id,
                    ],
                    provider_factory=lambda profile_id, *, config_id: provider,
                )

            self.assertEqual(exit_code, 0)
            self.assertEqual(len(provider.requests), 1)
            request = provider.requests[0]
            self.assertEqual(len(request.text), 6000)
            for category in (
                "character",
                "location",
                "organization",
                "item",
                "title",
                "other",
            ):
                self.assertIn(category, request.system_instruction)
            candidate_path = project_root / "glossary" / "ner-candidates.json"
            document = json.loads(candidate_path.read_text(encoding="utf-8"))
            self.assertEqual(document["schema"], "translation-engine-ner-candidates-v1")
            self.assertEqual(document["reviewStatus"], "pending")
            self.assertEqual(
                document["candidates"],
                [
                    {
                        "source": "Alice",
                        "translation": "爱丽丝",
                        "category": "character",
                    },
                    {
                        "source": "Wonderland",
                        "translation": "仙境",
                        "category": "location",
                    },
                ],
            )
            self.assertEqual(glossary_path.read_text(encoding="utf-8"), original_glossary)
            report = json.loads(stdout.getvalue())
            self.assertEqual(report["candidateCount"], 2)
            self.assertEqual(report["artifact"]["path"], "glossary/ner-candidates.json")


if __name__ == "__main__":
    unittest.main()
