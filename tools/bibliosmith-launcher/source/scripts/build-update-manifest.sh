#!/usr/bin/env bash
# Build the static update manifest the launcher's updater endpoint serves.
#
# Everything in it comes from files on this machine: the version from
# launcher-version.json, the notes from RELEASE_NOTES.md, the signature from
# the .sig the bundler wrote. Nothing is retyped, so the manifest cannot end up
# describing a build other than the one that was just verified.
#
# Output goes to update-manifest/, which the release step uploads whole.
set -euo pipefail

if [[ "$#" -ne 1 ]]; then
  echo "usage: build-update-manifest.sh <release-tag>" >&2
  exit 2
fi
release_tag="$1"
: "${RELEASE_REPOSITORY:?RELEASE_REPOSITORY must be set to owner/repo}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle_dir="$root/src-tauri/target/release/bundle/macos"
out_dir="$root/update-manifest"

version=$(node -p "require('$root/launcher-version.json').version")
if [[ "v$version" != "$release_tag" ]]; then
  echo "launcher-version.json is v$version but the release tag is $release_tag." >&2
  exit 1
fi

shopt -s nullglob
tarballs=("$bundle_dir"/*.app.tar.gz)
if [[ "${#tarballs[@]}" -ne 1 ]]; then
  echo "Expected exactly one updater tarball in $bundle_dir, found ${#tarballs[@]}." >&2
  exit 1
fi
tarball="${tarballs[0]}"
signature_file="$tarball.sig"
if [[ ! -s "$signature_file" ]]; then
  echo "Updater tarball has no signature at $signature_file." >&2
  exit 1
fi

# GitHub rewrites spaces in uploaded asset names, and "BiblioSmith Launcher.app"
# has one. Renaming here means the URL written into the manifest is the URL the
# release actually serves, rather than a guess at what GitHub did to the name.
asset_name="BiblioSmith-Launcher_${version}_aarch64.app.tar.gz"

rm -rf "$out_dir"
mkdir -p "$out_dir"
cp "$tarball" "$out_dir/$asset_name"
cp "$signature_file" "$out_dir/$asset_name.sig"

# darwin-aarch64 only: this release is Apple Silicon, as the DMG has always
# been. An Intel or Windows launcher finds no entry for its platform and is
# told there is no update, which is true, instead of being handed a bundle it
# cannot run.
UPDATE_ASSET_NAME="$asset_name" \
UPDATE_RELEASE_TAG="$release_tag" \
UPDATE_SIGNATURE_FILE="$out_dir/$asset_name.sig" \
UPDATE_NOTES_FILE="$root/RELEASE_NOTES.md" \
UPDATE_VERSION="$version" \
node -e '
  const fs = require("node:fs");
  const repository = process.env.RELEASE_REPOSITORY;
  const tag = process.env.UPDATE_RELEASE_TAG;
  const asset = process.env.UPDATE_ASSET_NAME;
  const manifest = {
    version: process.env.UPDATE_VERSION,
    // The whole file. The launcher picks the section for its interface
    // language, so trimming to one language here would decide that for it.
    notes: fs.readFileSync(process.env.UPDATE_NOTES_FILE, "utf8").trim(),
    pub_date: new Date().toISOString(),
    platforms: {
      "darwin-aarch64": {
        signature: fs.readFileSync(process.env.UPDATE_SIGNATURE_FILE, "utf8").trim(),
        url: `https://github.com/${repository}/releases/download/${tag}/${asset}`,
      },
    },
  };
  process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
' > "$out_dir/latest.json"

echo "Update manifest ok: $version -> $asset_name"
