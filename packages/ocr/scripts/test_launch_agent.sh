#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LABEL="com.example.zotero-llm.test"
BOOTSTRAP_DOMAIN="gui/$(id -u)"
PLIST="/tmp/$LABEL.plist"
LOG_DIR="$HOME/Zotero/OCR_OUTPUT/.state/logs"

mkdir -p "$LOG_DIR"

cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>/opt/homebrew/bin/python3.11</string>
    <string>$PROJECT_DIR/scripts/zotero_llm_worker.py</string>
    <string>--dry-run</string>
    <string>--limit</string>
    <string>5</string>
    <string>--max-runtime-minutes</string>
    <string>5</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>WorkingDirectory</key>
  <string>$PROJECT_DIR</string>
  <key>StandardOutPath</key>
  <string>$LOG_DIR/test-launchd.out.log</string>
  <key>StandardErrorPath</key>
  <string>$LOG_DIR/test-launchd.err.log</string>
</dict>
</plist>
PLIST

plutil -lint "$PLIST" >/dev/null
launchctl bootout "$BOOTSTRAP_DOMAIN" "$PLIST" >/dev/null 2>&1 || true
launchctl bootstrap "$BOOTSTRAP_DOMAIN" "$PLIST"
sleep 8
launchctl bootout "$BOOTSTRAP_DOMAIN" "$PLIST" >/dev/null 2>&1 || true
rm -f "$PLIST"

echo "stdout:"
tail -40 "$LOG_DIR/test-launchd.out.log" 2>/dev/null || true
echo "stderr:"
tail -40 "$LOG_DIR/test-launchd.err.log" 2>/dev/null || true
