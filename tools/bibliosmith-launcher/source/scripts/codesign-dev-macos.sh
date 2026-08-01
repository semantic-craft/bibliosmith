#!/usr/bin/env bash
# Re-sign local Debug binaries with a stable designated requirement so macOS
# Keychain ACL stops treating every rebuild as a different app.
#
# Why: `cargo test` and `tauri dev` run binaries that rustc produced and signed
# ad-hoc. tauri.conf.json's signingIdentity does not reach them — it applies to
# the bundle `tauri build` packages, not to a test harness. An ad-hoc signature
# yields
#   designated => cdhash H"<content-hash>"
# which changes on every compile, so a Keychain "Always Allow" only ever sticks
# to the build that prompted for it and the next `cargo test` asks again.
# Signing with Apple Development plus a fixed bundle identifier yields
#   designated => identifier "com.bibliosmith.launcher" and certificate ...
# which is stable across rebuilds, so the grant holds.
#
# Usage (from tools/bibliosmith-launcher/source):
#   ./scripts/codesign-dev-macos.sh
#   ./scripts/codesign-dev-macos.sh src-tauri/target/debug/deps/bibliosmith_launcher_lib-*
#
# Optional:
#   BIBLIOSMITH_CODESIGN_IDENTITY="Apple Development: you@example.com (TEAMID)"
#   BIBLIOSMITH_CODESIGN_IDENTIFIER="com.bibliosmith.launcher"   # default
set -euo pipefail

IDENTIFIER="${BIBLIOSMITH_CODESIGN_IDENTIFIER:-com.bibliosmith.launcher}"

resolve_identity() {
  if [[ -n "${BIBLIOSMITH_CODESIGN_IDENTITY:-}" ]]; then
    printf '%s\n' "$BIBLIOSMITH_CODESIGN_IDENTITY"
    return
  fi
  # Prefer Apple Development (local debug). Fall back to first codesigning identity.
  local line
  line="$(security find-identity -v -p codesigning 2>/dev/null \
    | grep -F 'Apple Development:' | head -1 || true)"
  if [[ -z "$line" ]]; then
    line="$(security find-identity -v -p codesigning 2>/dev/null \
      | grep -E '^\s+[0-9]+\)' | head -1 || true)"
  fi
  if [[ -z "$line" ]]; then
    echo "codesign-dev-macos: no codesigning identity found in the login keychain." >&2
    echo "Install an Apple Development certificate in Xcode, or set BIBLIOSMITH_CODESIGN_IDENTITY." >&2
    exit 1
  fi
  # line looks like:  1) ABCD... "Apple Development: you@… (TEAM)"
  if [[ "$line" =~ \"([^\"]+)\" ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}"
  else
    echo "codesign-dev-macos: could not parse identity from: $line" >&2
    exit 1
  fi
}

is_macho() {
  local f="$1"
  [[ -f "$f" && -x "$f" ]] || return 1
  file -b "$f" 2>/dev/null | grep -q 'Mach-O'
}

sign_one() {
  local f="$1"
  local identity="$2"
  codesign --force --sign "$identity" --identifier "$IDENTIFIER" --timestamp=none "$f"
  echo "signed: $f"
  codesign -d -r- "$f" 2>&1 | sed -n 's/^designated => /  DR: /p'
}

main() {
  local identity
  identity="$(resolve_identity)"
  echo "identity: $identity"
  echo "identifier: $IDENTIFIER"

  local -a targets=()
  if [[ "$#" -gt 0 ]]; then
    targets=("$@")
  else
    local root
    root="$(cd "$(dirname "$0")/.." && pwd)"
    local debug="$root/src-tauri/target/debug"
    [[ -d "$debug" ]] || {
      echo "codesign-dev-macos: no $debug — build first (cargo test / tauri dev)." >&2
      exit 1
    }
    # App/binary + unit-test harnesses that can touch Keychain.
    while IFS= read -r -d '' f; do
      targets+=("$f")
    done < <(
      find "$debug" -maxdepth 1 -type f -perm -111 \( \
        -name 'bibliosmith-launcher' -o \
        -name 'bibliosmith_launcher*' \
      \) -print0 2>/dev/null
      find "$debug/deps" -maxdepth 1 -type f -perm -111 \( \
        -name 'bibliosmith_launcher-*' -o \
        -name 'bibliosmith_launcher_lib-*' \
      \) ! -name '*.d' ! -name '*.rlib' ! -name '*.rmeta' -print0 2>/dev/null
    )
  fi

  local n=0
  local f
  for f in "${targets[@]+"${targets[@]}"}"; do
    if is_macho "$f"; then
      sign_one "$f" "$identity"
      n=$((n + 1))
    fi
  done

  if [[ "$n" -eq 0 ]]; then
    echo "codesign-dev-macos: nothing to sign." >&2
    exit 1
  fi
  echo "signed $n binary(ies)."
  echo "Next: open Keychain Access → login → Passwords → com.bibliosmith.launcher.models"
  echo "for each entry, Access Control → Always Allow (once). Rebuilds keep the same DR."
}

main "$@"
