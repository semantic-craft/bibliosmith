#!/usr/bin/env bash
# Verify the public macOS release acceptance boundary before upload.
set -euo pipefail

if [[ "$#" -ne 1 || ! -f "$1" || "$1" != *.dmg ]]; then
  echo "usage: verify-macos-release.sh /path/to/dmg" >&2
  exit 2
fi

dmg_path="$1"

verify_app() {
  local candidate="$1"
  local gatekeeper_output
  local runtime_root="$candidate/Contents/Resources/bibliosmith-runtime"
  local node_sidecar="$candidate/Contents/MacOS/node"
  local uv_sidecar="$candidate/Contents/MacOS/uv"
  local node_version
  local node_child_uv_version
  local uv_version
  local forbidden
  local entitlement_output
  local entitlement_payload

  codesign --verify --deep --strict --verbose=2 "$candidate"
  [[ -x "$node_sidecar" ]] || { echo "Missing executable Node sidecar." >&2; exit 1; }
  [[ -x "$uv_sidecar" ]] || { echo "Missing executable uv sidecar." >&2; exit 1; }
  node_version=$("$node_sidecar" --version)
  uv_version=$("$uv_sidecar" --version)
  [[ "$node_version" == "v22.23.2" ]] || {
    echo "Expected Node v22.23.2, got $node_version." >&2
    exit 1
  }
  [[ "$uv_version" == "uv 0.11.8 "* ]] || {
    echo "Expected uv 0.11.8, got $uv_version." >&2
    exit 1
  }
  [[ $("$node_sidecar" --jitless -e 'process.stdout.write("node-js-ok")') == "node-js-ok" ]] || {
    echo "Bundled Node cannot execute JIT-less JavaScript under Hardened Runtime." >&2
    exit 1
  }
  node_child_uv_version=$("$node_sidecar" --jitless -e '
    const { spawnSync } = require("child_process");
    const result = spawnSync(process.argv[1], ["--version"], { encoding: "utf8" });
    if (result.error || result.status !== 0) process.exit(1);
    process.stdout.write(result.stdout.trim());
  ' "$uv_sidecar")
  [[ "$node_child_uv_version" == "uv 0.11.8 "* ]] || {
    echo "Bundled Node cannot launch the uv sidecar." >&2
    exit 1
  }
  for executable in "$candidate" "$node_sidecar" "$uv_sidecar"; do
    if ! entitlement_output=$(codesign -d --entitlements - "$executable" 2>&1); then
      echo "Could not inspect entitlements on $(basename "$executable")." >&2
      exit 1
    fi
    entitlement_payload=$(sed '/^Executable=/d' <<<"$entitlement_output")
    if [[ -n "${entitlement_payload//[[:space:]]/}" ]]; then
      echo "Unexpected entitlement on $(basename "$executable")." >&2
      exit 1
    fi
  done
  required_runtime_files=(
    "pyproject.toml"
    "uv.lock"
    "bundle-input.json"
    "sidecar-manifest.json"
    "tools/bibliosmith-launcher/source/scripts/build_bilingual_epub.py"
    "tools/bibliosmith-launcher/source/scripts/build_epub.cjs"
    "tools/bibliosmith-launcher/source/scripts/run_python.cjs"
    "packages/translation-engine/src/translation_engine/__main__.py"
    "packages/layout-pdf/src/layout_pdf/__main__.py"
    "packages/ocr/mineru.py"
    "packages/zotero-cli/src/zotero_cli/cli.py"
    "packages/digest/bibliosmith_digest/core.py"
    "licenses/node/LICENSE"
    "licenses/uv/LICENSE-MIT"
    "licenses/uv/LICENSE-APACHE"
  )
  for relative_path in "${required_runtime_files[@]}"; do
    [[ -f "$runtime_root/$relative_path" ]] || {
      echo "Missing App runtime resource: $relative_path" >&2
      exit 1
    }
  done
  epubcheck_jars=("$runtime_root"/vendor/epubchecker/vendors/*/epubcheck.jar)
  [[ "${#epubcheck_jars[@]}" -eq 1 && -f "${epubcheck_jars[0]}" ]] || {
    echo "Expected exactly one bundled EPUBCheck jar." >&2
    exit 1
  }
  forbidden=$(find "$runtime_root" \( \
    \( -type d \( -name books -o -name local -o -name tests -o -name __pycache__ \) \) -o \
    \( -type f \( -name '.env' -o -name '.env.*' -o -name 'credentials.json' -o \
      -name 'secrets.json' -o -name '*.db' -o -name '*.sqlite*' -o -name '*.log' -o \
      -name '*.pyc' -o -name '*.pyo' -o -name '*.pem' -o -name '*.key' \) \) \
    \) -print)
  if [[ -n "$forbidden" ]]; then
    echo "Private or mutable files entered App resources:" >&2
    printf '%s\n' "$forbidden" >&2
    exit 1
  fi
  if ! gatekeeper_output=$(spctl -a -vvv -t install "$candidate" 2>&1); then
    echo "Gatekeeper rejected the release app." >&2
    printf '%s\n' "$gatekeeper_output" >&2
    exit 1
  fi
  if ! grep -Eq '(^|: )accepted$' <<<"$gatekeeper_output" \
    || ! grep -Fqx 'source=Notarized Developer ID' <<<"$gatekeeper_output"; then
    echo "Gatekeeper must report accepted with source=Notarized Developer ID." >&2
    exit 1
  fi
  echo "Gatekeeper accepted: source=Notarized Developer ID"
  xcrun stapler validate "$candidate"
}

hdiutil verify "$dmg_path"
xcrun stapler validate "$dmg_path"

mount_point=$(mktemp -d)
mounted=false
cleanup() {
  if [[ "$mounted" == true ]]; then
    hdiutil detach "$mount_point" -quiet || true
  fi
  rmdir "$mount_point" 2>/dev/null || true
}
trap cleanup EXIT

hdiutil attach "$dmg_path" -nobrowse -readonly -mountpoint "$mount_point"
mounted=true
shopt -s nullglob
mounted_apps=("$mount_point"/*.app)
if [[ "${#mounted_apps[@]}" -ne 1 ]]; then
  echo "Expected exactly one app in the release DMG, found ${#mounted_apps[@]}." >&2
  exit 1
fi
verify_app "${mounted_apps[0]}"
