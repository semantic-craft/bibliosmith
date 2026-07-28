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

  codesign --verify --deep --strict --verbose=2 "$candidate"
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
