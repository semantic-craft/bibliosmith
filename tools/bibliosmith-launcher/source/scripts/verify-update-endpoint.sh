#!/usr/bin/env bash
# Ask the published update endpoint the same question an installed launcher
# asks, and check the answer describes the release that was just published.
#
# The endpoint is a /releases/latest/download/ redirect, which nothing earlier
# in the release exercises. A manifest that 404s, or one whose bundle URL does
# not resolve, reads to every installed launcher as "no update available" — the
# failure is silent, and it stays silent until someone notices that nobody ever
# updated.
set -euo pipefail

: "${RELEASE_REPOSITORY:?RELEASE_REPOSITORY must be set to owner/repo}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version=$(node -p "require('$root/launcher-version.json').version")
endpoint="https://github.com/$RELEASE_REPOSITORY/releases/latest/download/latest.json"

manifest=""
for attempt in 1 2 3 4 5; do
  if manifest=$(curl --fail --silent --show-error --location "$endpoint"); then
    break
  fi
  echo "Update endpoint not answering yet (attempt $attempt)."
  manifest=""
  sleep 6
done

if [[ -z "$manifest" ]]; then
  echo "Update endpoint never served a manifest: $endpoint" >&2
  exit 1
fi

bundle_url=$(MANIFEST="$manifest" EXPECTED_VERSION="$version" node -e '
  const manifest = JSON.parse(process.env.MANIFEST);
  const expected = process.env.EXPECTED_VERSION;
  if (manifest.version !== expected) {
    console.error(`Update endpoint reports ${manifest.version}, expected ${expected}.`);
    process.exit(1);
  }
  const platform = manifest.platforms?.["darwin-aarch64"];
  if (!platform?.url || !platform?.signature) {
    console.error("Update manifest has no signed darwin-aarch64 bundle.");
    process.exit(1);
  }
  process.stdout.write(platform.url);
')

# --location because the download URL redirects to object storage, and -I so a
# gigabyte-scale bundle is not pulled down just to prove it is there.
if ! curl --fail --silent --show-error --location --head "$bundle_url" > /dev/null; then
  echo "Update manifest names a bundle that cannot be fetched: $bundle_url" >&2
  exit 1
fi

echo "Update endpoint ok: $version at $bundle_url"
