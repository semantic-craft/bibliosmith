"""Acceptance contract for signed and notarized macOS releases (issue #63)."""

from __future__ import annotations

import json
import os
import stat
import subprocess
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release-launcher.yml"
TAURI_CONFIG = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "src-tauri"
    / "tauri.conf.json"
)
VERIFIER = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "scripts"
    / "verify-macos-release.sh"
)
README_EN = REPO_ROOT / "README.md"
README_ZH = REPO_ROOT / "README.zh-CN.md"
LAUNCHER_README_ZH = (
    REPO_ROOT / "tools" / "bibliosmith-launcher" / "source" / "README.zh-CN.md"
)


def test_release_uses_secret_backed_developer_id_signing_and_notarization() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")
    config = json.loads(TAURI_CONFIG.read_text(encoding="utf-8"))

    assert config["bundle"]["macOS"].get("signingIdentity") == "Developer ID Application"

    required_secrets = (
        "APPLE_CERTIFICATE",
        "APPLE_CERTIFICATE_PASSWORD",
        "KEYCHAIN_PASSWORD",
        "APPLE_ID",
        "APPLE_PASSWORD",
        "APPLE_TEAM_ID",
    )
    for name in required_secrets:
        assert f"${{{{ secrets.{name} }}}}" in workflow

    assert "security create-keychain" in workflow
    assert "security import" in workflow
    assert "security set-key-partition-list" in workflow
    assert "APPLE_SIGNING_IDENTITY" in workflow
    assert workflow.index("security import") < workflow.index("npx tauri build --bundles dmg")
    assert 'trap \'rm -f "$certificate_path"\' EXIT' in workflow
    assert "if: ${{ always() }}" in workflow
    assert 'security delete-keychain "$RUNNER_TEMP/bibliosmith-signing.keychain-db"' in workflow


def _write_command(path: Path, body: str) -> None:
    path.write_text(f"#!/bin/sh\n{body}\n", encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


def _run_verifier(spctl_output: str) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        app = root / "BiblioSmith Launcher.app"
        app.mkdir()
        dmg = root / "BiblioSmith Launcher.dmg"
        dmg.write_bytes(b"test-dmg")
        command_log = root / "commands.log"

        _write_command(
            bin_dir / "codesign",
            'printf \'codesign %s\\n\' "$*" >> "$COMMAND_LOG"',
        )
        _write_command(
            bin_dir / "spctl",
            'printf \'spctl %s\\n\' "$*" >> "$COMMAND_LOG"\n'
            'printf \'%s\\n\' "$SPCTL_OUTPUT" >&2',
        )
        _write_command(
            bin_dir / "xcrun",
            'printf \'xcrun %s\\n\' "$*" >> "$COMMAND_LOG"',
        )
        _write_command(
            bin_dir / "hdiutil",
            'printf \'hdiutil %s\\n\' "$*" >> "$COMMAND_LOG"\n'
            'if [ "$1" = "attach" ]; then\n'
            '  for argument in "$@"; do mount_point="$argument"; done\n'
            '  mkdir -p "$mount_point/BiblioSmith Launcher.app"\n'
            'elif [ "$1" = "detach" ]; then\n'
            '  find "$2" -depth -delete\n'
            'fi',
        )

        env = os.environ.copy()
        env.update(
            {
                "PATH": f"{bin_dir}{os.pathsep}{env['PATH']}",
                "COMMAND_LOG": str(command_log),
                "SPCTL_OUTPUT": spctl_output,
            }
        )
        completed = subprocess.run(
            ["bash", str(VERIFIER), str(app), str(dmg)],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )
        commands = command_log.read_text(encoding="utf-8").splitlines() if command_log.exists() else []
        return completed, commands


def test_release_verifier_accepts_a_notarized_developer_id_app() -> None:
    completed, commands = _run_verifier("accepted\nsource=Notarized Developer ID")

    assert completed.returncode == 0, completed.stderr
    assert [command.split()[0] for command in commands] == [
        "codesign",
        "spctl",
        "xcrun",
        "hdiutil",
        "xcrun",
        "hdiutil",
        "codesign",
        "spctl",
        "xcrun",
        "hdiutil",
    ]
    assert "--verify --deep --strict --verbose=2" in commands[0]
    assert "-a -vvv -t install" in commands[1]
    assert commands[2].startswith("xcrun stapler validate ")
    assert commands[3].startswith("hdiutil verify ")
    assert commands[4].startswith("xcrun stapler validate ")
    assert commands[5].startswith("hdiutil attach ")
    assert commands[6].startswith("codesign --verify ")
    assert commands[7].startswith("spctl -a -vvv -t install ")
    assert commands[8].startswith("xcrun stapler validate ")
    assert commands[9].startswith("hdiutil detach ")


def test_release_verifier_rejects_a_non_notarized_gatekeeper_source() -> None:
    completed, commands = _run_verifier("accepted\nsource=Developer ID")

    assert completed.returncode != 0
    assert "Notarized Developer ID" in completed.stderr
    assert [command.split()[0] for command in commands] == ["codesign", "spctl"]


def test_release_workflow_verifies_the_app_before_publishing() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")

    verifier = "./scripts/verify-macos-release.sh"
    notarytool = "xcrun notarytool submit"
    staple = "xcrun stapler staple"
    assert verifier in workflow
    assert "--wait" in workflow[workflow.index(notarytool) : workflow.index(staple)]
    assert workflow.index("npx tauri build --bundles dmg") < workflow.index(notarytool)
    assert workflow.index(notarytool) < workflow.index(staple)
    assert workflow.index(staple) < workflow.index(verifier)
    assert workflow.index(verifier) < workflow.index('gh release create "$RELEASE_TAG"')
    assert f'{verifier} "${{apps[0]}}" "${{dmgs[0]}}"' in workflow


def test_install_docs_describe_a_direct_notarized_first_launch() -> None:
    english = README_EN.read_text(encoding="utf-8")
    chinese = README_ZH.read_text(encoding="utf-8")
    launcher_chinese = LAUNCHER_README_ZH.read_text(encoding="utf-8")

    combined = f"{english}\n{chinese}\n{launcher_chinese}"
    assert "xattr -dr com.apple.quarantine" not in combined
    assert "Open Anyway" not in english
    assert "仍要打开" not in chinese
    assert "Notarized Developer ID" in english
    assert "Developer ID" in chinese and "公证" in chinese
    assert "Developer ID" in launcher_chinese and "公证" in launcher_chinese
    for local_build_doc in (english, chinese, launcher_chinese):
        assert "npx tauri build --bundles dmg --no-sign" in local_build_doc
