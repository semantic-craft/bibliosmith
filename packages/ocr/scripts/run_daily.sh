#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PYTHON_BIN="${PYTHON_BIN:-/opt/homebrew/bin/python3.11}"

cd "$PROJECT_DIR"
exec "$PYTHON_BIN" "$PROJECT_DIR/scripts/zotero_llm_worker.py" \
  --parent-item-type book \
  --min-size-mb 10 \
  --max-runtime-minutes 55
