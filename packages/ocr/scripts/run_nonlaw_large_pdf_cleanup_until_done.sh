#!/usr/bin/env bash
set -euo pipefail

DB="$HOME/Zotero/OCR_OUTPUT/.state/zotero_llm.sqlite3"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RUNNER="${RUNNER:-$PROJECT_DIR/scripts/nonlaw_large_pdf_cleanup.py}"
PYTHON_BIN="/opt/homebrew/bin/python3.11"
SLEEP_SECONDS="${SLEEP_SECONDS:-20}"
MAX_RUNTIME_MINUTES="${MAX_RUNTIME_MINUTES:-240}"

remaining_targets() {
  sqlite3 "$DB" <<'SQL'
WITH targets(pdf_key) AS (
  VALUES
    ('PSMYUFKY'),
    ('SS73B9E8'),
    ('UW2LAXRW'),
    ('IY6KVVQ4'),
    ('C2QS8KF3'),
    ('GMGAVTK3'),
    ('FVNGYXSH'),
    ('GTC9FGFF'),
    ('SNR3WW9I'),
    ('X3KBP8VB'),
    ('88RAIGNF'),
    ('PYW8X63W'),
    ('BQYDHP5V'),
    ('NX4NZ458'),
    ('YW9RJ3T7'),
    ('TQXG6DLW'),
    ('MJYKP8BD'),
    ('BURUE7PK')
)
SELECT COUNT(*)
FROM targets t
LEFT JOIN nonlaw_large_pdf_cleanup_jobs j
  ON j.pdf_key = t.pdf_key
WHERE t.pdf_key <> 'SS73B9E8'
  AND COALESCE(j.source_deleted, 0) = 0;
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
