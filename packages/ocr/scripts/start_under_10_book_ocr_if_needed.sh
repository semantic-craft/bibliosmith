#!/usr/bin/env bash
set -euo pipefail

DB="$HOME/Zotero/OCR_OUTPUT/.state/zotero_llm.sqlite3"
TARGET_LABEL="com.example.zotero-under-10-book-ocr"
TARGET_PLIST="$HOME/Library/LaunchAgents/${TARGET_LABEL}.plist"
UID_VALUE="$(id -u)"

table_exists() {
  sqlite3 "$DB" <<'SQL'
SELECT CASE WHEN EXISTS (
  SELECT 1 FROM sqlite_master
  WHERE type='table' AND name='under_10_book_pdf_cleanup_jobs'
) THEN 1 ELSE 0 END;
SQL
}

remaining_targets() {
  sqlite3 "$DB" <<'SQL'
SELECT COUNT(*)
FROM under_10_book_pdf_cleanup_jobs
WHERE NOT (
  (delete_policy = 'delete' AND COALESCE(source_deleted, 0) = 1)
  OR
  (delete_policy = 'keep' AND md_attachment_key IS NOT NULL AND COALESCE(delete_status, '') = 'kept_source_pdf')
  OR
  (md_status = 'failed' AND error IS NOT NULL)
);
SQL
}

active_count() {
  launchctl print "gui/${UID_VALUE}/${TARGET_LABEL}" 2>/dev/null \
    | awk -F'= ' '/active count =/ {print $2; exit}'
}

if [[ "$(table_exists)" == "1" && "$(remaining_targets)" == "0" ]]; then
  exit 0
fi

active="$(active_count || true)"
if [[ "${active:-0}" != "0" ]]; then
  exit 0
fi

open -ga Zotero >/dev/null 2>&1 || true
for _ in {1..30}; do
  if curl -fsS --max-time 2 http://127.0.0.1:23119/connector/ping >/dev/null 2>&1; then
    break
  fi
  sleep 2
done

if ! launchctl print "gui/${UID_VALUE}/${TARGET_LABEL}" >/dev/null 2>&1; then
  launchctl bootstrap "gui/${UID_VALUE}" "$TARGET_PLIST"
fi

launchctl kickstart -k "gui/${UID_VALUE}/${TARGET_LABEL}"
