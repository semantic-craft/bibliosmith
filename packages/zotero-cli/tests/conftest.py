from __future__ import annotations

import pytest


@pytest.fixture(autouse=True)
def preserve_human_cli_output(monkeypatch: pytest.MonkeyPatch) -> None:
    """Keep legacy CLI assertions in table mode unless a test opts into agent mode."""
    monkeypatch.setenv("ZSEARCH_FORMAT", "table")
