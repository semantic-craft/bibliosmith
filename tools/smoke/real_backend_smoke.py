#!/usr/bin/env python3
"""Real-backend smoke: drive translation-engine against a live provider.

Builds a throwaway one-chapter project (source map + task manifest + glossary +
run manifest), points it at a registered real provider profile, and runs the
engine. Credentials are read from the repository-root .env (same key_env names
as packages/translation-engine/.../providers.toml).

This isolates "does the real provider/registry/key/glossary path work?" from the
Tauri launcher. Run it once a real key is present in the root .env:

    uv run --package translation-engine python tools/smoke/real_backend_smoke.py
    uv run --package translation-engine python tools/smoke/real_backend_smoke.py \
        --provider-profile-id gemini-native --config-id gemini-default
    uv run --package translation-engine python tools/smoke/real_backend_smoke.py --second-pass

This remains a manual smoke rather than an automated gate. Verified live runs
are recorded in ``docs/runbooks/real-backend-smoke.md``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

SOURCE_TEXT = (
    "The lighthouse keeper watched the storm gather over the northern sea.\n\n"
    "Each night he climbed the spiral stair and lit the lamp, trusting the "
    "old mechanism to hold against the wind.\n\n"
    "By morning the gulls returned, and the harbor woke to a quiet, "
    "salt-bright calm.\n"
)

# A tiny glossary to prove per-chunk injection reaches the live model.
GLOSSARY_TEXT = (
    "source,translation,category,note\n"
    "lighthouse,灯塔,item,\n"
    "harbor,港湾,place,\n"
)


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def load_root_env() -> list[str]:
    """Inject repo-root .env into os.environ (real env wins). Returns keys set."""
    env_path = REPO_ROOT / ".env"
    injected: list[str] = []
    if not env_path.is_file():
        return injected
    for raw in env_path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        if key and key not in os.environ:
            os.environ[key] = value
            injected.append(key)
    return injected


def build_project(root: Path, *, provider_profile_id: str, config_id: str,
                  second_pass: bool) -> Path:
    source_sha = _sha256(SOURCE_TEXT.encode())
    source_path = root / "chapters" / "src" / "chapter_001.md"
    source_path.parent.mkdir(parents=True, exist_ok=True)
    source_path.write_text(SOURCE_TEXT, encoding="utf-8")

    _write_json(
        root / "metadata" / "source_map.json",
        {
            "schema": "local-reading-source-map-v2",
            "translationUnits": [
                {
                    "id": "chapter_001",
                    "sourceUnitPath": "chapters/src/chapter_001.md",
                    "sourceUnitSha256": source_sha,
                }
            ],
        },
    )

    glossary_path = root / "glossary" / "terms.csv"
    glossary_path.parent.mkdir(parents=True, exist_ok=True)
    glossary_path.write_text(GLOSSARY_TEXT, encoding="utf-8")
    glossary_sha = _sha256(GLOSSARY_TEXT.encode())

    _write_json(
        root / "qa" / "tasks" / "chapter_001.json",
        {
            "schema": "local-reading-translation-task-v2",
            "taskPolicyVersion": "task-policy-v1",
            "unitId": "chapter_001",
            "targetLanguage": "zh-Hans",
            "sourceUnitPath": "chapters/src/chapter_001.md",
            "sourceUnitSha256": source_sha,
            "glossaryPath": "glossary/terms.csv",
            "glossarySha256": glossary_sha,
        },
    )

    manifest = {
        "schema": "translation-engine-run-v1",
        "projectRoot": str(root),
        "sourceMapPath": "metadata/source_map.json",
        "sourceLanguage": "en",
        "targetLanguage": "zh-Hans",
        "providerProfileId": provider_profile_id,
        "providerConfigId": config_id,
        "translationPolicyVersion": "translation-policy-v1",
        "maxTokens": 400,
        "units": [{"taskManifestPath": "qa/tasks/chapter_001.json"}],
    }
    if second_pass:
        manifest["secondPassEnabled"] = True
    manifest_path = root / "translation-run.json"
    _write_json(manifest_path, manifest)
    return manifest_path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--provider-profile-id", default="openai-compatible")
    parser.add_argument("--config-id", default="openai-default")
    parser.add_argument("--second-pass", action="store_true",
                        help="also exercise the #62 windowed reflection pass")
    parser.add_argument("--keep", action="store_true",
                        help="keep the temp project dir for inspection")
    args = parser.parse_args(argv)

    injected = load_root_env()
    print(f"[env] injected from root .env: {injected or '(none — relying on shell env)'}")

    # Import after env is loaded; engine reads os.environ at provider construction.
    from translation_engine.engine import run_manifest
    from translation_engine.providers import load_provider_registry

    try:
        registry = load_provider_registry()
    except Exception as error:  # noqa: BLE001 - smoke surfaces any registry issue
        print(f"[registry] FAILED to load providers.toml: {error}")
        return 3
    key = (args.provider_profile_id, args.config_id)
    if key not in registry:
        print(f"[registry] {key} not in registry; known: {sorted(registry)}")
        return 3
    key_env = registry[key].key_env
    if not os.environ.get(key_env):
        print(f"[key] {key_env} is empty. Put a real key in {REPO_ROOT / '.env'} "
              f"(comma-separated for multiple) and retry.")
        return 4
    print(f"[key] {key_env} present ({len(os.environ[key_env].split(','))} key(s))")

    tmp = Path(tempfile.mkdtemp(prefix="lrt-smoke-"))
    manifest_path = build_project(
        tmp,
        provider_profile_id=args.provider_profile_id,
        config_id=args.config_id,
        second_pass=args.second_pass,
    )
    print(f"[run] {args.provider_profile_id}/{args.config_id} "
          f"second_pass={args.second_pass} project={tmp}")

    report = run_manifest(manifest_path)
    print("[report] " + json.dumps(report["summary"]))
    for unit in report["units"]:
        print(f"[unit] {unit['unitId']}: {unit['status']}"
              + (f" error={unit.get('error')}" if unit.get("error") else ""))

    translated = tmp / "chapters" / "translated" / "chapter_001.md"
    if translated.is_file():
        print("\n===== translated chapter_001.md =====")
        print(translated.read_text(encoding="utf-8"))
    if args.second_pass:
        reflection = tmp / "qa" / "reflection" / "chapter_001.reflection.md"
        if reflection.is_file():
            print("===== reflection critique (excerpt) =====")
            print(reflection.read_text(encoding="utf-8")[:800])

    if args.keep:
        print(f"[keep] project retained at {tmp}")
    else:
        shutil.rmtree(tmp)
    print("\nEyeball check: is the Chinese fluent, and are 灯塔/港湾 used for "
          "lighthouse/harbor?")
    return 0 if report["summary"]["failed"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
