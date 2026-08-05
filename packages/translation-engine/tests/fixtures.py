import hashlib
import json
from pathlib import Path

from translation_engine.prompt_packs import revision_content_sha256


def _prompt_pack_revision(
    *,
    pack_id: str,
    display_name: str,
    stages: list[dict[str, str]],
) -> dict[str, object]:
    revision: dict[str, object] = {
        "schema": "translation-prompt-pack-revision-v1",
        "packId": pack_id,
        "revisionId": "test-1",
        "displayName": display_name,
        "executor": "programmatic",
        "sourceLanguage": "auto",
        "targetLanguage": "zh-Hans",
        "stages": stages,
    }
    revision["contentSha256"] = revision_content_sha256(revision)
    return revision


STRUCTURE_FIDELITY_PROMPT_PACK = _prompt_pack_revision(
    pack_id="builtin.structure-fidelity",
    display_name="结构保真翻译",
    stages=[
        {
            "stageId": "translate",
            "template": "Translate the current block faithfully into Simplified Chinese.",
        }
    ],
)

FOUR_DIMENSION_PROMPT_PACK = _prompt_pack_revision(
    pack_id="builtin.four-dimension-refinement",
    display_name="四维反思精修",
    stages=[
        {"stageId": "translate", "template": "Create a faithful first translation."},
        {
            "stageId": "reflect",
            "template": "Reflect on accuracy, fluency, style, and terminology.",
        },
        {
            "stageId": "improve",
            "template": "Improve the draft using the four-dimension reflection.",
        },
    ],
)


def build_run_fixture(
    project_root: Path,
    *,
    source_text: str,
    max_tokens: int,
    glossary_text: str = "source,translation,category,note\n",
    second_pass_enabled: bool | None = None,
    text_cleanup: bool | None = None,
    prompt_pack: object | None = None,
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
            "schema": "local-reading-source-map-v1",
            "chapters": [
                {
                    "id": "chapter_001",
                    "chapterSourcePath": "chapters/src/chapter_001.md",
                    "chapterSourceSha256": source_sha256,
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
            "schema": "local-reading-translation-task-v1",
            "taskPolicyVersion": "task-policy-v1",
            "chapterId": "chapter_001",
            "targetLanguage": "zh-Hans",
            "sourceChapterPath": "chapters/src/chapter_001.md",
            "sourceChapterSha256": source_sha256,
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
    manifest["promptPack"] = prompt_pack or (
        FOUR_DIMENSION_PROMPT_PACK
        if second_pass_enabled is True
        else STRUCTURE_FIDELITY_PROMPT_PACK
    )
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
            "promptPack": STRUCTURE_FIDELITY_PROMPT_PACK,
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
    prompt_pack: object | None = None,
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
        "promptPack": prompt_pack or STRUCTURE_FIDELITY_PROMPT_PACK,
        "units": units,
    }
    if text_cleanup is not None:
        manifest["textCleanup"] = text_cleanup
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
                "chapterSourcePath": source_relative,
                "chapterSourceSha256": source_sha256,
            }
        )

        task_relative = f"qa/tasks/{chapter_id}.json"
        task_path = project_root / task_relative
        task_path.parent.mkdir(parents=True, exist_ok=True)
        _write_json(
            task_path,
            {
                "schema": "local-reading-translation-task-v1",
                "taskPolicyVersion": "task-policy-v1",
                "chapterId": chapter_id,
                "targetLanguage": "zh-Hans",
                "sourceChapterPath": source_relative,
                "sourceChapterSha256": source_sha256,
                "glossaryPath": "glossary/terms.csv",
                "glossarySha256": glossary_sha256,
            },
        )
        units.append({"taskManifestPath": task_relative})

    source_map_path = project_root / "metadata" / "source_map.json"
    source_map_path.parent.mkdir(parents=True, exist_ok=True)
    _write_json(
        source_map_path,
        {"schema": "local-reading-source-map-v1", "chapters": chapters},
    )
    return units


def _write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
