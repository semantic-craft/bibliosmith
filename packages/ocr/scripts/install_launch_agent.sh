#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$PROJECT_DIR/../.." && pwd)"
LABEL="com.semantic-craft.books-translation.ocr.daily"
TARGET_PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
BOOTSTRAP_DOMAIN="gui/$(id -u)"
LOG_DIR="$HOME/Zotero/OCR_OUTPUT/.state/logs"

mkdir -p "$HOME/Library/LaunchAgents"
mkdir -p "$LOG_DIR"

cat > "$TARGET_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$PROJECT_DIR/scripts/run_daily.sh</string>
  </array>
  <key>WorkingDirectory</key>
  <string>$REPO_DIR</string>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Hour</key>
    <integer>9</integer>
    <key>Minute</key>
    <integer>0</integer>
  </dict>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  <string>$LOG_DIR/daily.out.log</string>
  <key>StandardErrorPath</key>
  <string>$LOG_DIR/daily.err.log</string>
</dict>
</plist>
PLIST

chmod 600 "$TARGET_PLIST"
plutil -lint "$TARGET_PLIST" >/dev/null

launchctl bootout "$BOOTSTRAP_DOMAIN" "$TARGET_PLIST" >/dev/null 2>&1 || true
launchctl bootstrap "$BOOTSTRAP_DOMAIN" "$TARGET_PLIST"
launchctl enable "$BOOTSTRAP_DOMAIN/$LABEL"
launchctl print "$BOOTSTRAP_DOMAIN/$LABEL" | sed -n '1,80p'
