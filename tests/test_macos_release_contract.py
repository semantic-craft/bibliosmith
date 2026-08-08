"""Acceptance contract for signed and notarized macOS releases (issue #63)."""

from __future__ import annotations

import json
import os
import plistlib
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
LAUNCHER_PACKAGE = (
    REPO_ROOT / "tools" / "bibliosmith-launcher" / "source" / "package.json"
)
VERIFIER = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "scripts"
    / "verify-macos-release.sh"
)
RUNTIME_SIGNER = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "scripts"
    / "sign-bundle-runtime-macos.mjs"
)
BROWSER_ENTITLEMENTS = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "scripts"
    / "browser-runtime.entitlements.plist"
)
APPLE_PASSWORD_SECRET_SETTER = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "scripts"
    / "set-apple-password-secret-macos.sh"
)
UPDATER_VERIFIER = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "scripts"
    / "verify-macos-updater-bundle.sh"
)
UPDATE_MANIFEST_BUILDER = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "scripts"
    / "build-update-manifest.sh"
)
UPDATE_ENDPOINT_VERIFIER = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "scripts"
    / "verify-update-endpoint.sh"
)
LAUNCHER_CAPABILITIES = (
    REPO_ROOT
    / "tools"
    / "bibliosmith-launcher"
    / "source"
    / "src-tauri"
    / "capabilities"
    / "default.json"
)
# The app bundle is built alongside the DMG because the updater bundle is a
# tarball of it, and the bundler only produces updater artifacts for app
# bundles it actually built.
BUILD_COMMAND = "npx tauri build --bundles app dmg"
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
    assert workflow.index("security import") < workflow.index(BUILD_COMMAND)
    assert 'trap \'rm -f "$certificate_path"\' EXIT' in workflow
    assert "if: ${{ always() }}" in workflow
    assert 'security delete-keychain "$RUNNER_TEMP/bibliosmith-signing.keychain-db"' in workflow


def test_tauri_build_signs_prepared_runtime_before_bundling() -> None:
    package = json.loads(LAUNCHER_PACKAGE.read_text(encoding="utf-8"))
    scripts = package["scripts"]

    assert scripts["bundle:sign-macos"] == "node scripts/sign-bundle-runtime-macos.mjs"
    assert scripts["build:tauri"] == (
        "npm run bundle:prepare && npm run bundle:sign-macos && npm run build"
    )


def _write_command(path: Path, body: str) -> None:
    path.write_text(f"#!/bin/sh\n{body}\n", encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


def test_runtime_signer_signs_every_macho_and_rebinds_browser_digest() -> None:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        runtime = root / "runtime"
        browser = (
            runtime
            / "vendor"
            / "playwright-core"
            / ".local-browsers"
            / "chromium-test"
            / "chrome-headless-shell"
        )
        library = browser.parent / "libGLESv2.dylib"
        plain_text = browser.parent / "resources.pak"
        manifest = runtime / "vendor" / "playwright-core" / "browser-manifest.json"
        for candidate, payload in (
            (browser, b"browser"),
            (library, b"library"),
            (plain_text, b"plain"),
        ):
            candidate.parent.mkdir(parents=True, exist_ok=True)
            candidate.write_bytes(payload)
        manifest.write_text(
            json.dumps(
                {
                    "schema": "bibliosmith-browser-runtime-v1",
                    "relativePath": str(browser.relative_to(runtime)),
                    "version": "test",
                    "sha256": "pre-sign-digest",
                    "playwrightCoreVersion": "test",
                }
            ),
            encoding="utf-8",
        )
        command_log = root / "commands.log"
        _write_command(
            bin_dir / "file",
            'for candidate do :; done\n'
            'case "$candidate" in\n'
            '  *chrome-headless-shell|*.dylib) printf "Mach-O 64-bit arm64\\n" ;;\n'
            '  *) printf "data\\n" ;;\n'
            'esac',
        )
        _write_command(
            bin_dir / "codesign",
            'printf \'codesign %s\\n\' "$*" >> "$COMMAND_LOG"\n'
            'if [ "$1" = "--force" ]; then\n'
            '  for candidate do :; done\n'
            '  printf \'signed\' >> "$candidate"\n'
            'fi',
        )
        env = os.environ.copy()
        env.update(
            {
                "PATH": f"{bin_dir}{os.pathsep}{env['PATH']}",
                "APPLE_SIGNING_IDENTITY": "Developer ID Application: Test",
                "COMMAND_LOG": str(command_log),
            }
        )

        completed = subprocess.run(
            ["node", str(RUNTIME_SIGNER), str(runtime)],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )

        assert completed.returncode == 0, completed.stderr
        commands = command_log.read_text(encoding="utf-8").splitlines()
        signing_commands = [command for command in commands if " --force " in f" {command} "]
        assert len(signing_commands) == 2
        for candidate in (browser, library):
            command = next(command for command in signing_commands if str(candidate) in command)
            assert "--options runtime" in command
            assert "--timestamp" in command
            assert "--sign Developer ID Application: Test" in command
        browser_command = next(
            command for command in signing_commands if str(browser) in command
        )
        library_command = next(
            command for command in signing_commands if str(library) in command
        )
        assert f"--entitlements {BROWSER_ENTITLEMENTS}" in browser_command
        assert "--entitlements" not in library_command
        assert all(str(plain_text) not in command for command in commands)
        rebound = json.loads(manifest.read_text(encoding="utf-8"))
        import hashlib

        assert rebound["sha256"] == hashlib.sha256(browser.read_bytes()).hexdigest()


def test_browser_runtime_entitlements_grant_only_jit_execution() -> None:
    with BROWSER_ENTITLEMENTS.open("rb") as stream:
        entitlements = plistlib.load(stream)

    assert entitlements == {"com.apple.security.cs.allow-jit": True}


def _run_verifier(
    spctl_output: str,
) -> tuple[subprocess.CompletedProcess[str], list[str], str]:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        fake_runtime = root / "runtime"
        required_runtime_files = (
            "pyproject.toml",
            "uv.lock",
            "bundle-input.json",
            "sidecar-manifest.json",
            "tools/bibliosmith-launcher/source/scripts/build_bilingual_epub.py",
            "tools/bibliosmith-launcher/source/scripts/build_epub.cjs",
            "tools/bibliosmith-launcher/source/scripts/run_python.cjs",
            "packages/translation-engine/src/translation_engine/__main__.py",
            "packages/layout-pdf/src/layout_pdf/__main__.py",
            "packages/ocr/mineru.py",
            "packages/zotero-cli/src/zotero_cli/cli.py",
            "packages/digest/bibliosmith_digest/core.py",
            "licenses/node/LICENSE",
            "licenses/uv/LICENSE-MIT",
            "licenses/uv/LICENSE-APACHE",
            "vendor/epubchecker/vendors/test/epubcheck.jar",
            "vendor/playwright-core/browser-manifest.json",
            "vendor/playwright-core/.local-browsers/test/chrome-headless-shell",
        )
        for relative_path in required_runtime_files:
            fixture = fake_runtime / relative_path
            fixture.parent.mkdir(parents=True, exist_ok=True)
            fixture.write_text("test fixture\n", encoding="utf-8")

        fake_node = root / "node"
        _write_command(
            fake_node,
            'if [ "$1" = "--version" ]; then\n'
            '  printf \'v22.23.2\\n\'\n'
            'elif [ "$1" = "--jitless" ] && [ "$#" -eq 3 ]; then\n'
            '  printf \'node-js-ok\'\n'
            'elif [ "$1" = "--jitless" ] && [ "$#" -eq 4 ]; then\n'
            '  printf \'uv 0.11.8 (test)\'\n'
            'elif [ "$1" = "--jitless" ] && [ "$#" -eq 5 ] && [ "$5" = "browser-runtime-smoke" ]; then\n'
            '  printf \'called\\n\' > "$BROWSER_SMOKE_LOG"\n'
            '  printf \'browser-runtime-ok\'\n'
            'else\n'
            '  exit 1\n'
            'fi',
        )
        fake_uv = root / "uv"
        _write_command(fake_uv, "printf 'uv 0.11.8 (test)\\n'")
        dmg = root / "BiblioSmith Launcher.dmg"
        dmg.write_bytes(b"test-dmg")
        command_log = root / "commands.log"
        browser_smoke_log = root / "browser-smoke.log"

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
            '  app="$mount_point/BiblioSmith Launcher.app"\n'
            '  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/bibliosmith-runtime"\n'
            '  cp "$FAKE_NODE" "$app/Contents/MacOS/node"\n'
            '  cp "$FAKE_UV" "$app/Contents/MacOS/uv"\n'
            '  cp -R "$FAKE_RUNTIME/." "$app/Contents/Resources/bibliosmith-runtime/"\n'
            'elif [ "$1" = "detach" ]; then\n'
            '  find "$2" -depth -delete\n'
            'fi',
        )

        env = os.environ.copy()
        env.update(
            {
                "PATH": f"{bin_dir}{os.pathsep}{env['PATH']}",
                "COMMAND_LOG": str(command_log),
                "BROWSER_SMOKE_LOG": str(browser_smoke_log),
                "FAKE_NODE": str(fake_node),
                "FAKE_RUNTIME": str(fake_runtime),
                "FAKE_UV": str(fake_uv),
                "SPCTL_OUTPUT": spctl_output,
            }
        )
        completed = subprocess.run(
            ["bash", str(VERIFIER), str(dmg)],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )
        commands = command_log.read_text(encoding="utf-8").splitlines() if command_log.exists() else []
        browser_smoke = (
            browser_smoke_log.read_text(encoding="utf-8")
            if browser_smoke_log.exists()
            else ""
        )
        return completed, commands, browser_smoke


def test_release_verifier_accepts_a_notarized_developer_id_app() -> None:
    completed, commands, _ = _run_verifier("accepted\nsource=Notarized Developer ID")

    assert completed.returncode == 0, completed.stderr
    assert [command.split()[0] for command in commands] == [
        "hdiutil",
        "xcrun",
        "hdiutil",
        "codesign",
        "codesign",
        "codesign",
        "codesign",
        "spctl",
        "xcrun",
        "hdiutil",
    ]
    assert commands[0].startswith("hdiutil verify ")
    assert commands[1].startswith("xcrun stapler validate ")
    assert commands[2].startswith("hdiutil attach ")
    assert "--verify --deep --strict --verbose=2" in commands[3]
    assert all("-d --entitlements -" in command for command in commands[4:7])
    assert "-a -vvv -t install" in commands[7]
    assert commands[8].startswith("xcrun stapler validate ")
    assert commands[9].startswith("hdiutil detach ")


def test_release_verifier_rejects_a_non_notarized_gatekeeper_source() -> None:
    completed, commands, _ = _run_verifier("accepted\nsource=Developer ID")

    assert completed.returncode != 0
    assert "Notarized Developer ID" in completed.stderr
    assert [command.split()[0] for command in commands] == [
        "hdiutil",
        "xcrun",
        "hdiutil",
        "codesign",
        "codesign",
        "codesign",
        "codesign",
        "spctl",
        "hdiutil",
    ]


def test_release_verifier_executes_bundled_chromium_javascript() -> None:
    completed, _, browser_smoke = _run_verifier(
        "accepted\nsource=Notarized Developer ID"
    )
    verifier = VERIFIER.read_text(encoding="utf-8")

    assert completed.returncode == 0, completed.stderr
    assert browser_smoke == "called\n"
    assert "spawnSync(executablePath" in verifier
    assert '"--dump-dom"' in verifier
    assert 'require(join(runtimeRoot, "vendor/playwright-core"))' not in verifier


def _run_apple_password_secret_setter(
    osascript_body: str, dialog_password: str = ""
) -> tuple[subprocess.CompletedProcess[str], str, str, str]:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        osascript_input = root / "osascript-input.txt"
        gh_arguments = root / "gh-arguments.txt"
        gh_stdin = root / "gh-stdin.txt"

        _write_command(bin_dir / "osascript", osascript_body)
        _write_command(
            bin_dir / "gh",
            'printf \'%s\n\' "$*" > "$GH_ARGUMENTS"\n'
            'cat > "$GH_STDIN"',
        )

        env = os.environ.copy()
        env.update(
            {
                "PATH": f"{bin_dir}{os.pathsep}{env['PATH']}",
                "OSASCRIPT_INPUT": str(osascript_input),
                "GH_ARGUMENTS": str(gh_arguments),
                "GH_STDIN": str(gh_stdin),
                "DIALOG_PASSWORD": dialog_password,
            }
        )
        completed = subprocess.run(
            ["bash", str(APPLE_PASSWORD_SECRET_SETTER)],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )
        return (
            completed,
            osascript_input.read_text(encoding="utf-8") if osascript_input.exists() else "",
            gh_arguments.read_text(encoding="utf-8") if gh_arguments.exists() else "",
            gh_stdin.read_text(encoding="utf-8") if gh_stdin.exists() else "",
        )


def test_apple_password_secret_setter_uses_a_hidden_dialog_and_stdin() -> None:
    transient_value = "transient-test-value"
    completed, prompt, gh_arguments, gh_stdin = _run_apple_password_secret_setter(
        'cat > "$OSASCRIPT_INPUT"\nprintf \'OK:%s\' "$DIALOG_PASSWORD"',
        transient_value,
    )

    assert completed.returncode == 0, completed.stderr
    assert "with hidden answer" in prompt
    assert (
        gh_arguments.strip()
        == "secret set APPLE_PASSWORD --repo semantic-craft/bibliosmith"
    )
    assert gh_stdin == transient_value
    assert transient_value not in completed.stdout
    assert transient_value not in completed.stderr


def test_apple_password_secret_setter_rejects_an_empty_dialog() -> None:
    completed, _, gh_arguments, gh_stdin = _run_apple_password_secret_setter(
        "printf 'OK:'"
    )

    assert completed.returncode != 0
    assert not gh_arguments
    assert not gh_stdin
    assert "empty" in completed.stderr.lower()


def test_apple_password_secret_setter_leaves_the_secret_unchanged_when_cancelled() -> None:
    completed, _, gh_arguments, gh_stdin = _run_apple_password_secret_setter(
        "printf 'CANCEL'"
    )

    assert completed.returncode == 0
    assert not gh_arguments
    assert not gh_stdin
    assert "cancelled" in completed.stderr.lower()
    assert "execution error" not in completed.stderr.lower()


def test_apple_password_secret_setter_does_not_confuse_password_text_with_cancel() -> None:
    marker_text = "__BIBLIOSMITH_DIALOG_CANCELLED__"
    completed, _, gh_arguments, gh_stdin = _run_apple_password_secret_setter(
        "printf 'OK:%s' '__BIBLIOSMITH_DIALOG_CANCELLED__'"
    )

    assert completed.returncode == 0, completed.stderr
    assert gh_arguments
    assert gh_stdin == marker_text


def test_apple_password_secret_setter_reports_non_cancel_dialog_failures() -> None:
    completed, _, gh_arguments, gh_stdin = _run_apple_password_secret_setter(
        'echo "execution error: Apple events unavailable. (-1743)" >&2\nexit 1'
    )

    assert completed.returncode != 0
    assert not gh_arguments
    assert not gh_stdin
    assert "could not open" in completed.stderr.lower()
    assert "execution error" not in completed.stderr.lower()


def test_release_workflow_verifies_the_app_before_publishing() -> None:
    workflow = WORKFLOW.read_text(encoding="utf-8")

    verifier = "./scripts/verify-macos-release.sh"
    notarytool = "xcrun notarytool submit"
    staple = "xcrun stapler staple"
    assert verifier in workflow
    assert "--wait" in workflow[workflow.index(notarytool) : workflow.index(staple)]
    assert workflow.index(BUILD_COMMAND) < workflow.index(notarytool)
    assert workflow.index(notarytool) < workflow.index(staple)
    assert workflow.index(staple) < workflow.index(verifier)
    assert workflow.index(verifier) < workflow.index('gh release create "$RELEASE_TAG"')
    assert f'{verifier} "${{dmgs[0]}}"' in workflow
    assert "apps=(src-tauri/target/release/bundle/macos/*.app)" not in workflow


def test_launcher_verifies_update_bundles_against_a_committed_public_key() -> None:
    """The in-app updater's trust root, which is separate from Apple's.

    Gatekeeper's answer is about a human opening the app. Nothing asks it when
    the running launcher overwrites itself, so the updater carries its own
    signature over the bundle, checked against this public key. Without a
    pubkey the plugin installs whatever the endpoint returns.
    """
    config = json.loads(TAURI_CONFIG.read_text(encoding="utf-8"))
    capabilities = json.loads(LAUNCHER_CAPABILITIES.read_text(encoding="utf-8"))

    updater = config["plugins"]["updater"]
    assert updater["pubkey"].strip(), "The updater must verify against a public key."
    assert updater["endpoints"] == [
        "https://github.com/semantic-craft/bibliosmith/releases/latest/download/latest.json"
    ]
    for endpoint in updater["endpoints"]:
        assert endpoint.startswith("https://")
    # Only set to allow plain HTTP, which would let anything on the path
    # substitute the manifest.
    assert "dangerousInsecureTransportProtocol" not in updater

    assert config["bundle"]["createUpdaterArtifacts"] is True
    assert "app" in config["bundle"]["targets"]

    permissions = capabilities["permissions"]
    assert "updater:default" in permissions
    # relaunch() in the frontend invokes plugin:process|restart.
    assert "process:allow-restart" in permissions


def test_release_publishes_a_signed_updater_bundle_and_a_live_manifest() -> None:
    """The four gates between a built app and an update a launcher can install.

    Each one covers a failure the DMG steps cannot see. The tarball is a
    different file from the DMG and carries its own copy of the app; the
    manifest is written rather than built if nothing derives it from the
    artifacts; and the endpoint is a redirect nothing else in the release ever
    requests -- a manifest that 404s reads to every installed launcher as "no
    update available", so the failure is silent and permanent.
    """
    workflow = WORKFLOW.read_text(encoding="utf-8")

    for name in ("TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"):
        assert f"${{{{ secrets.{name} }}}}" in workflow

    updater_verifier = "./scripts/verify-macos-updater-bundle.sh"
    manifest_builder = "./scripts/build-update-manifest.sh"
    endpoint_verifier = "./scripts/verify-update-endpoint.sh"
    publish = 'gh release create "$RELEASE_TAG"'
    for step in (updater_verifier, manifest_builder, endpoint_verifier):
        assert step in workflow

    assert workflow.index(BUILD_COMMAND) < workflow.index(updater_verifier)
    assert workflow.index(updater_verifier) < workflow.index(manifest_builder)
    assert workflow.index(manifest_builder) < workflow.index(publish)
    # Verifying the endpoint before publishing would only ever describe the
    # previous release.
    assert workflow.index(publish) < workflow.index(endpoint_verifier)

    # The manifest and the bundle it names have to be release assets; a
    # manifest left on the runner is one no launcher can reach.
    assert "tools/bibliosmith-launcher/source/update-manifest/*" in workflow

    for script in (UPDATER_VERIFIER, UPDATE_MANIFEST_BUILDER, UPDATE_ENDPOINT_VERIFIER):
        assert script.stat().st_mode & stat.S_IXUSR, f"{script.name} is not executable"


def test_updater_bundle_verifier_demands_a_notarized_stapled_app_and_a_signature() -> None:
    """What the DMG verifier proves says nothing about the tarball.

    An in-app update unpacks this bundle and never the DMG, so the same
    Gatekeeper questions have to be put to the app inside it. Stapling matters
    more here than for the DMG: an updated app that has to reach Apple to be
    admitted fails shut for an offline user.
    """
    verifier = UPDATER_VERIFIER.read_text(encoding="utf-8")

    assert "spctl -a -vvv -t install" in verifier
    assert "source=Notarized Developer ID" in verifier
    assert "xcrun stapler validate" in verifier
    assert "codesign --verify --deep --strict" in verifier
    # A tarball with no .sig is a release no launcher can install, not a
    # cosmetic gap.
    assert 'if [[ ! -s "$signature" ]]' in verifier
    # The sidecars and runtime are what the pipeline executes; an update that
    # dropped them opens and then cannot run a single job.
    assert "for sidecar in node uv" in verifier
    assert "bibliosmith-runtime" in verifier


def test_update_manifest_is_derived_from_the_artifacts_it_describes() -> None:
    builder = UPDATE_MANIFEST_BUILDER.read_text(encoding="utf-8")
    endpoint_verifier = UPDATE_ENDPOINT_VERIFIER.read_text(encoding="utf-8")

    # Version, signature and notes all come from files on the runner. A
    # hand-typed field is one that can describe a different build.
    assert "launcher-version.json" in builder
    assert "RELEASE_NOTES.md" in builder
    assert 'UPDATE_SIGNATURE_FILE="$out_dir/$asset_name.sig"' in builder
    assert 'if [[ "v$version" != "$release_tag" ]]' in builder

    # Apple Silicon only, as the DMG has always been. Any other platform is
    # told there is no update, which is true, rather than handed a bundle it
    # cannot run.
    assert '"darwin-aarch64"' in builder
    assert '"darwin-aarch64"' in endpoint_verifier

    # The product name has a space and GitHub rewrites spaces in asset names,
    # so the manifest would otherwise carry a URL that does not resolve.
    assert 'asset_name="BiblioSmith-Launcher_${version}_aarch64.app.tar.gz"' in builder
    assert "releases/latest/download/latest.json" in endpoint_verifier
    assert "--head" in endpoint_verifier


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
