#!/usr/bin/env bash
set -euo pipefail

DB="$HOME/Zotero/OCR_OUTPUT/.state/zotero_llm.sqlite3"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RUNNER="${RUNNER:-$PROJECT_DIR/scripts/ten_to_50_pdf_paddle_mixed_cleanup.py}"
PYTHON_BIN="/opt/homebrew/bin/python3.11"
SLEEP_SECONDS="${SLEEP_SECONDS:-20}"
MAX_RUNTIME_MINUTES="${MAX_RUNTIME_MINUTES:-240}"

remaining_targets() {
  sqlite3 "$DB" <<'SQL'
SELECT COUNT(*)
FROM ten_to_50_pdf_cleanup_jobs
WHERE NOT (
  (delete_policy = 'delete' AND COALESCE(source_deleted, 0) = 1)
  OR
  (delete_policy = 'keep' AND md_attachment_key IS NOT NULL AND COALESCE(delete_status, '') = 'kept_source_pdf')
  OR
  (md_status = 'source_missing_before_completed_md' AND error IS NOT NULL)
  OR
  (md_status = 'failed' AND error IS NOT NULL)
);
SQL
}

daily_quota_reached() {
  sqlite3 "$DB" <<'SQL'
SELECT CASE WHEN EXISTS (
  SELECT 1
  FROM ten_to_50_pdf_cleanup_jobs
  WHERE md_status = 'stopped'
    AND (
      error LIKE '%已达每日页数上限%'
      OR error LIKE '%"code":12001%'
      OR error LIKE '%code=12001%'
    )
) THEN 1 ELSE 0 END;
SQL
}

while true; do
  "$PYTHON_BIN" "$RUNNER" --max-runtime-minutes "$MAX_RUNTIME_MINUTES"
  if [[ "$(daily_quota_reached)" == "1" ]]; then
    exit 75
  fi
  remaining="$(remaining_targets)"
  if [[ "$remaining" == "0" ]]; then
    exit 0
  fi
  sleep "$SLEEP_SECONDS"
done
