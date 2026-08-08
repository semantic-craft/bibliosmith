#!/usr/bin/env bash
# Verify the macOS updater bundle before it is published.
#
# verify-macos-release.sh answers for the DMG. This answers for the tarball an
# installed launcher downloads instead, which carries its own copy of the .app
# and is the only thing an in-app update ever unpacks. The two are built from
# the same app bundle but they are separate files, and only one of them was
# ever checked.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle_dir="$root/src-tauri/target/release/bundle/macos"

shopt -s nullglob
tarballs=("$bundle_dir"/*.app.tar.gz)
if [[ "${#tarballs[@]}" -ne 1 ]]; then
  echo "Expected exactly one updater tarball in $bundle_dir, found ${#tarballs[@]}." >&2
  exit 1
fi
tarball="${tarballs[0]}"
signature="$tarball.sig"

# The plugin refuses a bundle whose signature does not verify against the
# public key in tauri.conf.json, so a missing .sig is not a cosmetic gap: it is
# a release no launcher can install.
if [[ ! -s "$signature" ]]; then
  echo "Updater tarball has no signature at $signature." >&2
  echo "The build ran without TAURI_SIGNING_PRIVATE_KEY, or with --no-sign." >&2
  exit 1
fi

work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT

tar -xzf "$tarball" -C "$work_dir"
extracted=("$work_dir"/*.app)
if [[ "${#extracted[@]}" -ne 1 ]]; then
  echo "Expected exactly one app in the updater tarball, found ${#extracted[@]}." >&2
  exit 1
fi
app="${extracted[0]}"

codesign --verify --deep --strict --verbose=2 "$app"

if ! gatekeeper_output=$(spctl -a -vvv -t install "$app" 2>&1); then
  echo "Gatekeeper rejected the app inside the updater tarball." >&2
  printf '%s\n' "$gatekeeper_output" >&2
  exit 1
fi
if ! grep -Eq '(^|: )accepted$' <<<"$gatekeeper_output" \
  || ! grep -Fqx 'source=Notarized Developer ID' <<<"$gatekeeper_output"; then
  echo "The app inside the updater tarball must be accepted with source=Notarized Developer ID." >&2
  printf '%s\n' "$gatekeeper_output" >&2
  exit 1
fi

# Without a stapled ticket the updated app has to reach Apple to be admitted,
# so the first launch after an update would fail for an offline user.
xcrun stapler validate "$app"

# The sidecars are what the pipeline actually executes. An updater tarball that
# lost them, or that carries them unsigned, produces a launcher that opens and
# then cannot run a single job.
for sidecar in node uv; do
  path="$app/Contents/MacOS/$sidecar"
  if [[ ! -x "$path" ]]; then
    echo "Updater tarball is missing the executable $sidecar sidecar." >&2
    exit 1
  fi
  codesign --verify --strict "$path"
done

if [[ ! -d "$app/Contents/Resources/bibliosmith-runtime" ]]; then
  echo "Updater tarball is missing the bundled runtime resources." >&2
  exit 1
fi

echo "Updater bundle ok: $(basename "$tarball") installs a notarized, stapled app"
