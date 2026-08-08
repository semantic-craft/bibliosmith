#!/usr/bin/env bash
# Upload the updater's minisign private key to GitHub Secrets from a key file,
# without printing it or putting it in shell history.
#
# The key pairs with the pubkey in tauri.conf.json. Losing it means no release
# can ever be signed for the public key installed launchers already trust, and
# the only way back is a new key in a new release that every user installs by
# hand — so keep the file, back it up, and do not regenerate it casually.
#
# Generate a key first, if there is not one yet:
#
#   npx tauri signer generate -w ~/.tauri/bibliosmith-launcher.key
#
# then put the printed public key in src-tauri/tauri.conf.json under
# plugins.updater.pubkey, and run this to publish the private half.
set +x
set -euo pipefail

repo="semantic-craft/bibliosmith"
key_path="${1:-$HOME/.tauri/bibliosmith-launcher.key}"

if [[ ! -f "$key_path" ]]; then
  echo "No private key at $key_path." >&2
  echo "Pass the path as the first argument, or generate one with:" >&2
  echo "  npx tauri signer generate -w $key_path" >&2
  exit 1
fi

# The bundler reads the key from the environment as a string, so the secret
# holds the file's contents rather than a path.
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo "$repo" < "$key_path"
echo "Updated GitHub Secret TAURI_SIGNING_PRIVATE_KEY for $repo."

# The bundler always reads the password variable, so an unset secret fails the
# release even when the key has no password. An empty value is the correct
# answer for a key generated without one, and is what this writes unless a
# password is typed.
if ! dialog_result=$(
  osascript 2>/dev/null <<'APPLESCRIPT'
try
  set dialog_result to display dialog "输入该私钥的密码；生成时没设密码就留空。内容只会写入 GitHub Secret，不会显示在终端中。" default answer "" with hidden answer buttons {"取消", "保存"} default button "保存" cancel button "取消" with title "BiblioSmith 更新签名密钥"
  return "OK:" & text returned of dialog_result
on error number -128
  return "CANCEL"
end try
APPLESCRIPT
); then
  echo "Could not open the password dialog; TAURI_SIGNING_PRIVATE_KEY_PASSWORD was not changed." >&2
  exit 1
fi

case "$dialog_result" in
  CANCEL)
    unset dialog_result
    echo "Cancelled; TAURI_SIGNING_PRIVATE_KEY_PASSWORD was not changed." >&2
    exit 0
    ;;
  OK:*)
    key_password="${dialog_result#OK:}"
    unset dialog_result
    ;;
  *)
    unset dialog_result
    echo "The password dialog returned an unexpected result; TAURI_SIGNING_PRIVATE_KEY_PASSWORD was not changed." >&2
    exit 1
    ;;
esac

printf '%s' "$key_password" \
  | gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo "$repo"
unset key_password

echo "Updated GitHub Secret TAURI_SIGNING_PRIVATE_KEY_PASSWORD for $repo."
