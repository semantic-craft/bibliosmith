#!/usr/bin/env bash
set -euo pipefail

DB="$HOME/Zotero/OCR_OUTPUT/.state/zotero_llm.sqlite3"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RUNNER="${RUNNER:-$PROJECT_DIR/scripts/over100_book_paddle_mixed_cleanup.py}"
PYTHON_BIN="/opt/homebrew/bin/python3.11"
SLEEP_SECONDS="${SLEEP_SECONDS:-20}"
MAX_RUNTIME_MINUTES="${MAX_RUNTIME_MINUTES:-240}"

remaining_targets() {
  sqlite3 "$DB" <<'SQL'
WITH targets(pdf_key, delete_policy) AS (
  VALUES
    ('T4FBENY6','keep'),
    ('SHFFDZGZ','keep'),
    ('EKZU6TCE','keep'),
    ('NKMLNKRY','delete'),
    ('H33B3PEY','keep'),
    ('WDT47IM6','delete'),
    ('RN4C8H4Z','keep'),
    ('ZBE34PFH','keep'),
    ('2XJ6LKXL','keep'),
    ('R27YDYDH','keep'),
    ('TTMW7SSZ','keep'),
    ('UWUBDP32','keep'),
    ('2LGXT37S','delete'),
    ('MGDC4Q77','delete'),
    ('N6N3U82I','delete'),
    ('WQYMW6FX','keep'),
    ('FZZ76QFC','delete'),
    ('V9SV8BL3','keep'),
    ('G3EFQHK5','keep'),
    ('UXWGI8F3','delete'),
    ('EQM6AL5A','keep'),
    ('7G9VXXGF','keep'),
    ('YY87AZVF','delete'),
    ('MPVPGYRZ','delete'),
    ('TXW9R9E4','delete'),
    ('QLADNT7K','keep'),
    ('C8VDJSDV','keep'),
    ('6F9LXQNR','keep'),
    ('5ZLFNRNI','delete'),
    ('ZWM5RY6V','delete'),
    ('4ENGJK5E','keep'),
    ('SAPQ5I33','keep'),
    ('FPWQUYQR','delete'),
    ('VTCLMFXC','keep'),
    ('4W6H3C8F','keep'),
    ('6HB86MHF','keep'),
    ('DY5M2EXA','delete'),
    ('ZW4D7X7M','delete'),
    ('GVU6ZPTP','keep'),
    ('5TUC2QXH','keep'),
    ('R7XKKGEX','keep'),
    ('MISPVHJC','keep'),
    ('HN9GA3CE','keep')
)
SELECT COUNT(*)
FROM targets t
LEFT JOIN over100_book_pdf_cleanup_jobs j
  ON j.pdf_key = t.pdf_key
WHERE NOT (
  (t.delete_policy = 'delete' AND COALESCE(j.source_deleted, 0) = 1)
  OR
  (t.delete_policy = 'keep' AND j.md_attachment_key IS NOT NULL AND COALESCE(j.delete_status, '') = 'kept_source_pdf')
);
SQL
}

while true; do
  "$PYTHON_BIN" "$RUNNER" --max-runtime-minutes "$MAX_RUNTIME_MINUTES"
  remaining="$(remaining_targets)"
  if [[ "$remaining" == "0" ]]; then
    exit 0
  fi
  sleep "$SLEEP_SECONDS"
done
