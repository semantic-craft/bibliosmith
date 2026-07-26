"""Shared machine-facing output contract for zsearch and zfulltext."""

from __future__ import annotations

from contextlib import redirect_stdout
import io
import json
import os
import sys
from typing import Any

import click
import httpx

from . import __version__

SCHEMA_VERSION = "zotero-cli-agent-v1"
_FORMATS = {"json", "table"}
_EXEMPT_COMMANDS = {
    ("zsearch", ("collection-snapshot",)),
    ("zfulltext", ("profile",)),
    ("zfulltext", ("index",)),
    ("zsearch", ("serve",)),
}
_WRITE_COMMANDS = {
    ("zsearch", ("add", "doi")),
    ("zsearch", ("add", "file")),
    ("zsearch", ("coll", "create")),
    ("zsearch", ("coll", "rm")),
    ("zsearch", ("edit",)),
    ("zsearch", ("enrich",)),
    ("zsearch", ("ingest", "arxiv")),
    ("zsearch", ("ingest", "cnki")),
    ("zsearch", ("ingest", "ssrn")),
    ("zsearch", ("note", "add")),
    ("zsearch", ("note", "rm")),
    ("zsearch", ("parse",)),
    ("zsearch", ("sync",)),
    ("zsearch", ("tag", "add")),
    ("zsearch", ("tag", "rm")),
    ("zfulltext", ("index",)),
    ("zfulltext", ("sync",)),
}


class AgentError(RuntimeError):
    """A stable error category that agents can branch on."""

    def __init__(
        self,
        code: str,
        message: str,
        *,
        exit_code: int,
        retryable: bool = False,
        hint: str | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.exit_code = exit_code
        self.retryable = retryable
        self.hint = hint


def validation_error(message: str, *, hint: str | None = None) -> AgentError:
    return AgentError("validation_error", message, exit_code=3, hint=hint)


def not_found_error(message: str, *, hint: str | None = None) -> AgentError:
    return AgentError("not_found", message, exit_code=4, hint=hint)


def auth_missing_error(message: str) -> AgentError:
    return AgentError(
        "auth_missing",
        message,
        exit_code=2,
        hint="Set the required Zotero Web API environment variables and retry.",
    )


def _runtime_error(message: str) -> AgentError:
    return AgentError("runtime_error", message, exit_code=1)


def _network_error(message: str) -> AgentError:
    return AgentError(
        "network_error",
        message,
        exit_code=5,
        retryable=True,
        hint="Retry after checking network reachability and the upstream service.",
    )


def _http_error(exc: httpx.HTTPStatusError) -> AgentError:
    status = exc.response.status_code
    if status in {401, 403}:
        return AgentError(
            "auth_invalid",
            str(exc),
            exit_code=2,
            hint="Check the Zotero Web API key and library permissions.",
        )
    if status == 404:
        return not_found_error(str(exc))
    if status == 412:
        return AgentError(
            "conflict",
            str(exc),
            exit_code=6,
            hint="Refresh the Zotero item version before retrying the write.",
        )
    if status in {408, 429} or status >= 500:
        return _network_error(str(exc))
    return _runtime_error(str(exc))


def _command_identity(ctx: click.Context) -> tuple[str, tuple[str, ...]]:
    parts: list[str] = []
    current: click.Context | None = ctx
    while current is not None and current.parent is not None:
        if current.info_name:
            parts.append(current.info_name)
        current = current.parent
    root = ctx.find_root().command
    entry_point = getattr(root, "agent_entry_point", root.name or "")
    return entry_point, tuple(reversed(parts))


def _json_requested(ctx: click.Context) -> bool:
    if _command_identity(ctx) == ("zsearch", ("schema",)):
        return True
    if bool(ctx.params.get("as_json")):
        return True
    configured = os.environ.get("ZSEARCH_FORMAT", "").strip().lower()
    if configured in _FORMATS:
        return configured == "json"
    return not sys.stdout.isatty()


def machine_output_requested() -> bool:
    """Return whether the active leaf command must keep stdout machine-clean."""
    ctx = click.get_current_context(silent=True)
    return _json_requested(ctx) if ctx is not None else not sys.stdout.isatty()


def _decode_captured_output(output: str) -> Any:
    output = output.strip()
    if not output:
        return None
    try:
        return json.loads(output)
    except json.JSONDecodeError:
        return {"output": output}


def _success_envelope(data: Any) -> dict[str, Any]:
    return {
        "ok": True,
        "data": data,
        "meta": {
            "schema_version": SCHEMA_VERSION,
            "cli_version": __version__,
        },
    }


def _json_safe_default(value: Any) -> Any:
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, (list, tuple)):
        return [_json_safe_default(item) for item in value]
    return str(value)


def _parameter_schema(parameter: click.Parameter) -> dict[str, Any]:
    schema: dict[str, Any] = {
        "name": parameter.name,
        "kind": "argument" if isinstance(parameter, click.Argument) else "option",
        "type": parameter.type.name or parameter.type.__class__.__name__,
        "required": parameter.required,
        "multiple": parameter.multiple,
        "nargs": parameter.nargs,
        "default": _json_safe_default(parameter.default),
    }
    if isinstance(parameter, click.Option):
        schema["flags"] = [*parameter.opts, *parameter.secondary_opts]
        schema["help"] = parameter.help
    return schema


def _output_contract(entry_point: str, path: tuple[str, ...]) -> str:
    identity = (entry_point, path)
    if identity == ("zsearch", ("serve",)):
        return "stdio-mcp"
    if identity in _EXEMPT_COMMANDS:
        return "bare-json"
    return SCHEMA_VERSION


def command_surface_schema(entry_points: dict[str, click.Group]) -> dict[str, Any]:
    """Return a deterministic flat schema for both console entry points."""
    commands: list[dict[str, Any]] = []

    def visit(entry_point: str, group: click.Group, prefix: tuple[str, ...]) -> None:
        for name, command in sorted(group.commands.items()):
            path = (*prefix, name)
            if isinstance(command, click.Group):
                visit(entry_point, command, path)
                continue
            commands.append(
                {
                    "entry_point": entry_point,
                    "path": ".".join(path),
                    "help": command.help or "",
                    "safety": ("write" if (entry_point, path) in _WRITE_COMMANDS else "read"),
                    "output_contract": _output_contract(entry_point, path),
                    "parameters": [_parameter_schema(parameter) for parameter in command.params],
                }
            )

    for entry_point, root in sorted(entry_points.items()):
        visit(entry_point, root, ())
    return {"commands": commands}


def _error_envelope(error: AgentError) -> dict[str, Any]:
    return {
        "ok": False,
        "error": {
            "code": error.code,
            "message": error.message,
            "retryable": error.retryable,
            "hint": error.hint,
        },
        "meta": {
            "schema_version": SCHEMA_VERSION,
            "cli_version": __version__,
        },
    }


class _AgentClickException(click.ClickException):
    def __init__(
        self,
        error: AgentError,
        *,
        json_output: bool,
        original: click.ClickException | None = None,
    ) -> None:
        super().__init__(error.message)
        self.error = error
        self.exit_code = error.exit_code
        self.json_output = json_output
        self.original = original

    def show(self, file: Any | None = None) -> None:
        if self.json_output:
            click.echo(
                json.dumps(
                    _error_envelope(self.error),
                    ensure_ascii=False,
                    separators=(",", ":"),
                ),
                file=sys.stdout,
            )
        elif self.original is not None:
            self.original.show(file)
        else:
            super().show(file)


def _click_error(
    error: AgentError,
    ctx: click.Context,
    *,
    original: click.ClickException | None = None,
) -> _AgentClickException:
    return _AgentClickException(
        error,
        json_output=_json_requested(ctx),
        original=original,
    )


class _AgentParseMixin:
    def parse_args(self, ctx: click.Context, args: list[str]) -> list[str]:
        if _command_identity(ctx) in _EXEMPT_COMMANDS:
            return super().parse_args(ctx, args)  # type: ignore[misc]
        try:
            return super().parse_args(ctx, args)  # type: ignore[misc]
        except click.UsageError as exc:
            raise _click_error(
                validation_error(exc.format_message()),
                ctx,
                original=exc,
            ) from exc


class AgentCommand(_AgentParseMixin, click.Command):
    """Click command that envelopes stdout only for the machine-facing mode."""

    def invoke(self, ctx: click.Context) -> Any:
        identity = _command_identity(ctx)
        if identity in _EXEMPT_COMMANDS:
            return super().invoke(ctx)

        json_output = _json_requested(ctx)
        captured = io.StringIO()
        try:
            if json_output:
                with redirect_stdout(captured):
                    result = super().invoke(ctx)
            else:
                result = super().invoke(ctx)
        except _AgentClickException:
            raise
        except AgentError as exc:
            raise _click_error(exc, ctx) from exc
        except httpx.HTTPStatusError as exc:
            raise _click_error(_http_error(exc), ctx) from exc
        except httpx.RequestError as exc:
            raise _click_error(_network_error(str(exc)), ctx) from exc
        except click.ClickException as exc:
            raise _click_error(
                _runtime_error(exc.format_message()),
                ctx,
                original=exc,
            ) from exc
        except Exception as exc:
            if not json_output:
                raise
            raise _click_error(_runtime_error(str(exc)), ctx) from exc

        if not json_output:
            return result

        data = result if result is not None else _decode_captured_output(captured.getvalue())
        click.echo(
            json.dumps(
                _success_envelope(data),
                ensure_ascii=False,
                separators=(",", ":"),
            )
        )
        return result


class AgentGroup(_AgentParseMixin, click.Group):
    """Group that gives every leaf command the shared agent adapter."""

    command_class = AgentCommand
    group_class = type
