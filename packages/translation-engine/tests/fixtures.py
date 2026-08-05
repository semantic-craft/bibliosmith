import hashlib
import json
from pathlib import Path


def build_run_fixture(
    project_root: Path,
    *,
    source_text: str,
    max_tokens: int,
    glossary_text: str = "source,translation,category,note\n",
    second_pass_enabled: bool | None = None,
    text_cleanup: bool | None = None,
    custom_instructions: object | None = None,
) -> Path:
    source_sha256 = hashlib.sha256(source_text.encode()).hexdigest()
    source_path = project_root / "chapters" / "src" / "chapter_001.md"
    source_path.parent.mkdir(parents=True)
    source_path.write_text(source_text, encoding="utf-8")

    source_map_path = project_root / "metadata" / "source_map.json"
    source_map_path.parent.mkdir(parents=True)
    _write_json(
        source_map_path,
        {
            "schema": "local-reading-source-map-v2",
            "translationUnits": [
                {
                    "id": "chapter_001",
                    "sourceUnitPath": "chapters/src/chapter_001.md",
                    "sourceUnitSha256": source_sha256,
                }
            ],
        },
    )

    glossary_path = project_root / "glossary" / "terms.csv"
    glossary_path.parent.mkdir(parents=True)
    glossary_path.write_text(glossary_text, encoding="utf-8")
    glossary_sha256 = hashlib.sha256(glossary_text.encode()).hexdigest()

    task_path = project_root / "qa" / "tasks" / "chapter_001.json"
    task_path.parent.mkdir(parents=True)
    _write_json(
        task_path,
        {
            "schema": "local-reading-translation-task-v2",
            "taskPolicyVersion": "task-policy-v1",
            "unitId": "chapter_001",
            "targetLanguage": "zh-Hans",
            "sourceUnitPath": "chapters/src/chapter_001.md",
            "sourceUnitSha256": source_sha256,
            "glossaryPath": "glossary/terms.csv",
            "glossarySha256": glossary_sha256,
        },
    )

    manifest_path = project_root / "translation-run.json"
    manifest = {
        "schema": "translation-engine-run-v1",
        "projectRoot": str(project_root),
        "sourceMapPath": "metadata/source_map.json",
        "sourceLanguage": "auto",
        "targetLanguage": "zh-Hans",
        "providerProfileId": "fake-provider-profile",
        "providerConfigId": "fake-config-no-secrets",
        "translationPolicyVersion": "translation-policy-v1",
        "maxTokens": max_tokens,
        "units": [{"taskManifestPath": "qa/tasks/chapter_001.json"}],
    }
    if second_pass_enabled is not None:
        manifest["secondPassEnabled"] = second_pass_enabled
    if text_cleanup is not None:
        manifest["textCleanup"] = text_cleanup
    if custom_instructions is not None:
        manifest["customInstructions"] = custom_instructions
    _write_json(manifest_path, manifest)
    return manifest_path


def build_multi_unit_run_fixture(
    project_root: Path,
    *,
    source_texts: list[str],
    max_tokens: int,
) -> Path:
    """A run manifest over several chapters, for anything units do to each other.

    `build_run_fixture` stays single-unit because most engine behaviour is
    per-chapter; concurrency, dispatch, and report ordering are the exceptions.
    """
    units = _write_units(project_root, source_texts)
    manifest_path = project_root / "translation-run.json"
    _write_json(
        manifest_path,
        {
            "schema": "translation-engine-run-v1",
            "projectRoot": str(project_root),
            "sourceMapPath": "metadata/source_map.json",
            "sourceLanguage": "auto",
            "targetLanguage": "zh-Hans",
            "providerProfileId": "fake-provider-profile",
            "providerConfigId": "fake-config-no-secrets",
            "translationPolicyVersion": "translation-policy-v1",
            "maxTokens": max_tokens,
            "units": units,
        },
    )
    return manifest_path


def build_sample_fixture(
    project_root: Path,
    *,
    source_texts: list[str],
    sample_count: int,
    character_budget: int,
    text_cleanup: bool | None = None,
    custom_instructions: object | None = None,
) -> Path:
    units = _write_units(project_root, source_texts)

    manifest_path = project_root / "translation-sample.json"
    manifest: dict[str, object] = {
        "schema": "translation-engine-sample-v1",
        "projectRoot": str(project_root),
        "sourceMapPath": "metadata/source_map.json",
        "sourceLanguage": "auto",
        "targetLanguage": "zh-Hans",
        "providerProfileId": "fake-provider-profile",
        "providerConfigId": "fake-config-no-secrets",
        "sampleCount": sample_count,
        "characterBudget": character_budget,
        "placeholderRetries": 1,
        "units": units,
    }
    if text_cleanup is not None:
        manifest["textCleanup"] = text_cleanup
    if custom_instructions is not None:
        manifest["customInstructions"] = custom_instructions
    _write_json(manifest_path, manifest)
    return manifest_path


def _write_units(project_root: Path, source_texts: list[str]) -> list[dict[str, str]]:
    """Write one chapter, task manifest, and source-map entry per source text."""
    glossary_text = "source,translation,category,note\n"
    glossary_path = project_root / "glossary" / "terms.csv"
    glossary_path.parent.mkdir(parents=True, exist_ok=True)
    glossary_path.write_text(glossary_text, encoding="utf-8")
    glossary_sha256 = hashlib.sha256(glossary_text.encode()).hexdigest()

    chapters = []
    units = []
    for index, source_text in enumerate(source_texts, start=1):
        chapter_id = f"chapter_{index:03d}"
        source_relative = f"chapters/src/{chapter_id}.md"
        source_path = project_root / source_relative
        source_path.parent.mkdir(parents=True, exist_ok=True)
        source_path.write_text(source_text, encoding="utf-8")
        source_sha256 = hashlib.sha256(source_text.encode()).hexdigest()
        chapters.append(
            {
                "id": chapter_id,
                "sourceUnitPath": source_relative,
                "sourceUnitSha256": source_sha256,
            }
        )

        task_relative = f"qa/tasks/{chapter_id}.json"
        task_path = project_root / task_relative
        task_path.parent.mkdir(parents=True, exist_ok=True)
        _write_json(
            task_path,
            {
                "schema": "local-reading-translation-task-v2",
                "taskPolicyVersion": "task-policy-v1",
                "unitId": chapter_id,
                "targetLanguage": "zh-Hans",
                "sourceUnitPath": source_relative,
                "sourceUnitSha256": source_sha256,
                "glossaryPath": "glossary/terms.csv",
                "glossarySha256": glossary_sha256,
            },
        )
        units.append({"taskManifestPath": task_relative})

    source_map_path = project_root / "metadata" / "source_map.json"
    source_map_path.parent.mkdir(parents=True, exist_ok=True)
    _write_json(
        source_map_path,
        {"schema": "local-reading-source-map-v2", "translationUnits": chapters},
    )
    return units


def _write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
