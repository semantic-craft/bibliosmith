"""Load the monorepo root .env without affecting standalone installs."""

from __future__ import annotations

import os
from pathlib import Path


def _load_dotenv(path: Path) -> None:
    if not path.exists():
        return
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        if key and value and key not in os.environ:
            os.environ[key] = value


def load_root_dotenv(start: Path | None = None) -> None:
    if os.environ.get("BIBLIOSMITH_DISABLE_DOTENV") == "1":
        return
    current = (start or Path(__file__).resolve().parent).resolve()
    for candidate in (current, *current.parents):
        if (candidate / "pyproject.toml").is_file() and (candidate / "packages").is_dir():
            _load_dotenv(candidate / ".env")
            return
