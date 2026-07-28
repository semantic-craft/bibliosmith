from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

from click.testing import CliRunner
import httpx

from zotero_cli import cli
from zotero_cli import zfulltext_cli


def test_zsearch_automatically_envelopes_non_tty_output(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("ZSEARCH_FORMAT")
    monkeypatch.setattr(cli.zotero_db, "list_tags", lambda limit: [("agent-contract", 2)])

    result = CliRunner().invoke(cli.main, ["tags"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["ok"] is True
    assert "agent-contract" in payload["data"]["output"]
    assert payload["meta"] == {
        "schema_version": "zotero-cli-agent-v1",
        "cli_version": "0.1.0",
    }


def test_shared_format_switch_forces_zfulltext_json(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    class FakeStore:
        def __init__(self, _cfg) -> None:
            return None

        def __enter__(self) -> "FakeStore":
            return self

        def __exit__(self, *_exc: object) -> None:
            return None

        def count(self) -> int:
            return 3

        def existing_keys(self) -> dict[str, str]:
            return {"ITEM": "date", "ITEM#c0": "hash", "ITEM#c1": "hash"}

    monkeypatch.setenv("ZSEARCH_FORMAT", "json")
    monkeypatch.setattr(zfulltext_cli, "SQLiteVecStore", FakeStore)

    result = CliRunner().invoke(zfulltext_cli.main, ["info"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["ok"] is True
    assert "fulltext chunks:" in payload["data"]["output"]
    assert payload["meta"]["schema_version"] == "zotero-cli-agent-v1"


def test_table_override_preserves_human_output(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.setenv("ZSEARCH_FORMAT", "table")
    monkeypatch.setattr(cli.zotero_db, "list_tags", lambda limit: [("human-table", 1)])

    result = CliRunner().invoke(cli.main, ["tags"])

    assert result.exit_code == 0
    assert "human-table" in result.output
    assert not result.output.lstrip().startswith("{")


def test_validation_errors_have_a_typed_exit_and_envelope(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("ZSEARCH_FORMAT")

    result = CliRunner().invoke(cli.main, ["get"])

    assert result.exit_code == 3
    payload = json.loads(result.stdout)
    assert payload["ok"] is False
    assert payload["error"]["code"] == "validation_error"
    assert payload["error"]["retryable"] is False


def test_missing_items_have_a_typed_exit_and_envelope(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("ZSEARCH_FORMAT")
    monkeypatch.setattr(cli.zotero_db, "get_item", lambda _key: None)

    result = CliRunner().invoke(cli.main, ["get", "MISSING"])

    assert result.exit_code == 4
    payload = json.loads(result.stdout)
    assert payload["error"] == {
        "code": "not_found",
        "message": "item not found: MISSING",
        "retryable": False,
        "hint": "Run a search or listing command to discover a valid Zotero key.",
    }


def test_missing_web_api_credentials_have_a_typed_exit_and_envelope(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("ZSEARCH_FORMAT")
    monkeypatch.delenv("ZOTERO_API_KEY", raising=False)
    monkeypatch.delenv("ZOTERO_LIBRARY_ID", raising=False)

    result = CliRunner().invoke(cli.main, ["edit", "ITEM", "-f", "title=Changed"])

    assert result.exit_code == 2
    payload = json.loads(result.stdout)
    assert payload["error"]["code"] == "auth_missing"
    assert payload["error"]["retryable"] is False


def test_network_failures_are_retryable_and_use_exit_five(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("ZSEARCH_FORMAT")
    request = httpx.Request("PATCH", "https://api.zotero.test/items/ITEM")
    monkeypatch.setattr(
        cli.zotero_api,
        "update_item",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(
            httpx.ConnectError("connection reset", request=request)
        ),
    )

    result = CliRunner().invoke(cli.main, ["edit", "ITEM", "-f", "title=Changed"])

    assert result.exit_code == 5
    payload = json.loads(result.stdout)
    assert payload["error"]["code"] == "network_error"
    assert payload["error"]["retryable"] is True


def test_version_conflicts_use_exit_six(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("ZSEARCH_FORMAT")
    request = httpx.Request("PATCH", "https://api.zotero.test/items/ITEM")
    response = httpx.Response(412, request=request)
    monkeypatch.setattr(
        cli.zotero_api,
        "update_item",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(
            httpx.HTTPStatusError(
                "version conflict",
                request=request,
                response=response,
            )
        ),
    )

    result = CliRunner().invoke(cli.main, ["edit", "ITEM", "-f", "title=Changed"])

    assert result.exit_code == 6
    payload = json.loads(result.stdout)
    assert payload["error"]["code"] == "conflict"
    assert payload["error"]["retryable"] is False


def test_schema_covers_every_leaf_command_with_parameters_and_safety(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.setenv("ZSEARCH_FORMAT", "json")

    result = CliRunner().invoke(cli.main, ["schema"])

    assert result.exit_code == 0
    payload = json.loads(result.stdout)
    commands = payload["data"]["commands"]
    paths = {f"{command['entry_point']}.{command['path']}" for command in commands}
    assert paths == {
        "zsearch.add.doi",
        "zsearch.add.file",
        "zsearch.coll.create",
        "zsearch.coll.rm",
        "zsearch.collection-snapshot",
        "zsearch.dedupe",
        "zsearch.edit",
        "zsearch.enrich",
        "zsearch.get",
        "zsearch.grep",
        "zsearch.info",
        "zsearch.ingest.arxiv",
        "zsearch.ingest.cnki",
        "zsearch.ingest.ssrn",
        "zsearch.ingest.westlaw",
        "zsearch.ls",
        "zsearch.note.add",
        "zsearch.note.rm",
        "zsearch.notes",
        "zsearch.open",
        "zsearch.parse",
        "zsearch.phrases",
        "zsearch.query",
        "zsearch.recent",
        "zsearch.schema",
        "zsearch.serve",
        "zsearch.sync",
        "zsearch.tag.add",
        "zsearch.tag.rm",
        "zsearch.tags",
        "zfulltext.context",
        "zfulltext.excerpt",
        "zfulltext.index",
        "zfulltext.info",
        "zfulltext.profile",
        "zfulltext.query",
        "zfulltext.sync",
    }
    assert all(command["safety"] in {"read", "write"} for command in commands)
    by_path = {f"{command['entry_point']}.{command['path']}": command for command in commands}
    assert by_path["zsearch.collection-snapshot"]["output_contract"] == "bare-json"
    assert by_path["zfulltext.profile"]["output_contract"] == "bare-json"
    assert by_path["zfulltext.index"]["output_contract"] == "bare-json"
    assert by_path["zsearch.edit"]["safety"] == "write"
    assert by_path["zfulltext.query"]["safety"] == "read"
    assert {parameter["name"] for parameter in by_path["zsearch.query"]["parameters"]} >= {
        "text",
        "top_k",
        "item_type",
        "year",
        "tag",
        "rerank",
        "as_json",
    }


def test_schema_stays_machine_readable_when_table_mode_is_forced(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.setenv("ZSEARCH_FORMAT", "table")

    result = CliRunner().invoke(cli.main, ["schema"])

    assert result.exit_code == 0
    assert json.loads(result.stdout)["ok"] is True


def test_zfulltext_missing_content_uses_not_found_exit(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("ZSEARCH_FORMAT")
    monkeypatch.setattr(zfulltext_cli, "get_item_chunks", lambda *_args: [])

    result = CliRunner().invoke(zfulltext_cli.main, ["excerpt", "MISSING"])

    assert result.exit_code == 4
    assert json.loads(result.stdout)["error"]["code"] == "not_found"


def test_invalid_year_filter_uses_validation_exit(
    monkeypatch,
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("ZSEARCH_FORMAT")

    result = CliRunner().invoke(cli.main, ["query", "law", "--year", "not-a-year"])

    assert result.exit_code == 3
    assert json.loads(result.stdout)["error"]["code"] == "validation_error"


def test_parse_captures_child_output_in_agent_mode(tmp_path: Path, monkeypatch) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("ZSEARCH_FORMAT")
    pdf = tmp_path / "fixture.pdf"
    pdf.write_bytes(b"%PDF fixture")
    calls: list[dict] = []

    def fake_run(_args, **kwargs):  # type: ignore[no-untyped-def]
        calls.append(kwargs)
        return SimpleNamespace(returncode=0, stdout="mineru chatter", stderr="")

    monkeypatch.setattr(cli.subprocess, "run", fake_run)

    result = CliRunner().invoke(cli.main, ["parse", str(pdf)])

    assert result.exit_code == 0
    assert calls == [{"check": False, "capture_output": True, "text": True}]
    assert json.loads(result.stdout)["ok"] is True
    assert "mineru chatter" not in result.stdout
