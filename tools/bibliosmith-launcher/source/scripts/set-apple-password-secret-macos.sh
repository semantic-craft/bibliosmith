#!/usr/bin/env bash
# Prompt for the Apple app-specific password without putting it in shell history.
set +x
set -euo pipefail

if ! dialog_result=$(
  osascript 2>/dev/null <<'APPLESCRIPT'
try
  set dialog_result to display dialog "粘贴 Apple App 专用密码。密码只会写入 GitHub Secret，不会显示在终端中。" default answer "" with hidden answer buttons {"取消", "保存"} default button "保存" cancel button "取消" with title "BiblioSmith 公证凭据"
  return "OK:" & text returned of dialog_result
on error number -128
  return "CANCEL"
end try
APPLESCRIPT
); then
  echo "Could not open the password dialog; GitHub Secret was not changed." >&2
  exit 1
fi

case "$dialog_result" in
  CANCEL)
    unset dialog_result
    echo "Cancelled; GitHub Secret was not changed." >&2
    exit 0
    ;;
  OK:*)
    apple_password="${dialog_result#OK:}"
    unset dialog_result
    ;;
  *)
    unset dialog_result
    echo "The password dialog returned an unexpected result; GitHub Secret was not changed." >&2
    exit 1
    ;;
esac

if [[ -z "$apple_password" ]]; then
  unset apple_password
  echo "The Apple app-specific password was empty; GitHub Secret was not changed." >&2
  exit 1
fi

printf '%s' "$apple_password" \
  | gh secret set APPLE_PASSWORD --repo semantic-craft/bibliosmith
unset apple_password

echo "Updated GitHub Secret APPLE_PASSWORD for semantic-craft/bibliosmith."
