from dataclasses import dataclass, replace
from importlib.resources import files
import json as jsonlib
import os
from pathlib import Path
from queue import Empty, Queue
import re
from threading import Lock, Thread
import time
import tomllib
from typing import Callable, Mapping, MutableMapping, Protocol, Sequence, runtime_checkable
from urllib.parse import quote

import httpx

from .placeholders import PLACEHOLDER_PATTERN


@dataclass(frozen=True)
class TranslationRequest:
    text: str
    source_language: str
    target_language: str
    system_instruction: str


@dataclass(frozen=True)
class ProviderConfig:
    profile_id: str
    config_id: str
    provider_type: str
    base_url: str
    model: str
    timeout_seconds: float
    # How many translation units may be in flight against this service at once.
    # Read by the engine when it fans units out; see engine._unit_concurrency.
    concurrency_limit: int
    key_env: str
    base_url_env: str | None = None
    web_search_env: str | None = None
    web_search_enabled: bool = False


class ProviderError(RuntimeError):
    code = "provider_error"


class ProviderUnavailableError(ProviderError):
    code = "provider_unavailable"


class RateLimitError(ProviderUnavailableError):
    code = "provider_rate_limited"

    def __init__(
        self, message: str = "provider rate limited", *, retry_after_seconds: float = 0.0
    ) -> None:
        super().__init__(message)
        self.retry_after_seconds = max(0.0, retry_after_seconds)


class TransientError(ProviderUnavailableError):
    code = "provider_transient_error"


class ProviderTimeoutError(TransientError):
    code = "provider_timeout"


class ProviderServerError(TransientError):
    code = "provider_http_5xx"


class FatalError(ProviderError):
    code = "provider_fatal_error"


def normalize_api_keys(value: str) -> tuple[str, ...]:
    """Normalize a comma/newline-separated key value without changing order."""

    normalized: list[str] = []
    seen: set[str] = set()
    for candidate in re.split(r"[,\r\n]+", value):
        key = candidate.strip()
        if key and key not in seen:
            normalized.append(key)
            seen.add(key)
    return tuple(normalized)


class KeyPool:
    """Thread-safe round-robin credential pool with per-key throttling."""

    def __init__(
        self,
        keys: Sequence[str],
        *,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        normalized = normalize_api_keys("\n".join(keys))
        if not normalized:
            raise ValueError("credential pool requires at least one key")
        self._keys = normalized
        self._clock = clock
        self._cursor = 0
        self._throttled_until = {key: 0.0 for key in normalized}
        self._lock = Lock()

    @property
    def pool_size(self) -> int:
        return len(self._keys)

    def acquire(self) -> str:
        with self._lock:
            now = self._clock()
            for offset in range(self.pool_size):
                index = (self._cursor + offset) % self.pool_size
                key = self._keys[index]
                if self._throttled_until[key] <= now:
                    self._cursor = (index + 1) % self.pool_size
                    return key
            retry_after = min(self._throttled_until.values()) - now
        raise RateLimitError(
            "all provider credentials are throttled",
            retry_after_seconds=retry_after,
        )

    def report_rate_limit(self, key: str, *, retry_after_seconds: float) -> None:
        with self._lock:
            if key not in self._throttled_until:
                raise ValueError("credential does not belong to this pool")
            self._throttled_until[key] = max(
                self._throttled_until[key],
                self._clock() + max(0.0, retry_after_seconds),
            )


@runtime_checkable
class CredentialPool(Protocol):
    @property
    def pool_size(self) -> int: ...

    def acquire(self) -> str: ...

    def report_rate_limit(self, key: str, *, retry_after_seconds: float) -> None: ...


@runtime_checkable
class LLMProvider(Protocol):
    profile_id: str
    config_id: str
    # Mutable: a run may override the registry default before translating.
    model: str

    def translate(self, request: TranslationRequest) -> str: ...


class FakeProvider:
    """Deterministic offline provider used by fixtures and tests."""

    profile_id = "fake-provider-profile"
    model = "fake-model"

    def __init__(self, *, config_id: str) -> None:
        self.config_id = config_id

    def translate(self, request: TranslationRequest) -> str:
        if "translation editing" in request.system_instruction:
            payload = jsonlib.loads(request.text)
            draft = payload.get("draft")
            if isinstance(draft, str):
                return draft
        if "Output only the suggestions" in request.system_instruction:
            return "No changes required."
        return "".join(
            part if PLACEHOLDER_PATTERN.fullmatch(part) else part.upper()
            for part in re.split(f"({PLACEHOLDER_PATTERN.pattern})", request.text)
        )


class _RequestDeadlineGuard:
    """Prevent retries from overlapping a request that outlived its deadline."""

    def __init__(self) -> None:
        self._workers: list[Thread] = []
        self._lock = Lock()

    def before_request(self) -> None:
        with self._lock:
            self._workers = [worker for worker in self._workers if worker.is_alive()]
            if self._workers:
                raise httpx.ReadTimeout(
                    "previous timed-out provider request is still terminating"
                )

    def record_timeout(self, worker: Thread) -> None:
        with self._lock:
            self._workers.append(worker)


def _post_with_total_deadline(
    client: httpx.Client | None,
    url: str,
    *,
    deadline_guard: _RequestDeadlineGuard,
    headers: Mapping[str, str],
    json: object,
    timeout_seconds: float,
) -> httpx.Response:
    deadline_guard.before_request()
    active_client = client or httpx.Client(timeout=timeout_seconds)
    owns_client = client is None
    result: Queue[httpx.Response | BaseException] = Queue(maxsize=1)

    def post() -> None:
        try:
            outcome: httpx.Response | BaseException = active_client.post(
                url,
                headers=headers,
                json=json,
                timeout=timeout_seconds,
            )
        except BaseException as error:
            outcome = error
        result.put(outcome)

    worker = Thread(target=post, name="provider-request", daemon=True)
    worker.start()
    try:
        outcome = result.get(timeout=timeout_seconds)
    except Empty as error:
        deadline_guard.record_timeout(worker)
        if owns_client:
            active_client.close()
        raise httpx.ReadTimeout("provider request exceeded its total deadline") from error
    finally:
        if owns_client and not worker.is_alive():
            active_client.close()
    if isinstance(outcome, BaseException):
        raise outcome
    if owns_client:
        active_client.close()
    return outcome


class OpenAICompatibleProvider:
    def __init__(
        self,
        *,
        config: ProviderConfig,
        credential_pool: CredentialPool,
        http_client: httpx.Client | None = None,
        max_attempts: int = 3,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        if max_attempts < 1:
            raise ValueError("max_attempts must be positive")
        self.profile_id = config.profile_id
        self.config_id = config.config_id
        self.base_url = config.base_url
        self.model = config.model
        self.timeout_seconds = config.timeout_seconds
        self.concurrency_limit = config.concurrency_limit
        self.credential_pool = credential_pool
        self._http_client = http_client
        self._max_attempts = max_attempts
        self._sleep = sleep
        self._deadline_guard = _RequestDeadlineGuard()

    def translate(self, request: TranslationRequest) -> str:
        return _translate_with_retries(
            request=request,
            credential_pool=self.credential_pool,
            max_attempts=self._max_attempts,
            sleep=self._sleep,
            operation=self._translate_once,
        )

    def _translate_once(self, request: TranslationRequest, key: str) -> str:
        try:
            response = _post_with_total_deadline(
                self._http_client,
                f"{self.base_url}/chat/completions",
                deadline_guard=self._deadline_guard,
                headers={"Authorization": f"Bearer {key}"},
                json={
                    "model": self.model,
                    "messages": [
                        {"role": "system", "content": request.system_instruction},
                        {"role": "user", "content": request.text},
                    ],
                },
                timeout_seconds=self.timeout_seconds,
            )
        except httpx.TimeoutException as error:
            raise ProviderTimeoutError("provider request timed out") from error
        except httpx.TransportError as error:
            raise TransientError("provider request failed transiently") from error
        _raise_for_status(response)
        try:
            content = response.json()["choices"][0]["message"]["content"]
        except (KeyError, IndexError, TypeError, ValueError) as error:
            raise FatalError("provider returned an invalid response") from error
        if not isinstance(content, str) or not content.strip():
            raise FatalError("provider returned an invalid response")
        return _unwrap_translation(content)


class OpenAIResponsesProvider:
    """OpenAI-compatible Responses transport for stateless private translation."""

    def __init__(
        self,
        *,
        config: ProviderConfig,
        credential_pool: CredentialPool,
        http_client: httpx.Client | None = None,
        max_attempts: int = 3,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        if max_attempts < 1:
            raise ValueError("max_attempts must be positive")
        self.profile_id = config.profile_id
        self.config_id = config.config_id
        self.base_url = config.base_url
        self.model = config.model
        self.timeout_seconds = config.timeout_seconds
        self.concurrency_limit = config.concurrency_limit
        self.web_search_enabled = config.web_search_enabled
        self.credential_pool = credential_pool
        self._http_client = http_client or httpx.Client(timeout=self.timeout_seconds)
        self._max_attempts = max_attempts
        self._sleep = sleep

    def translate(self, request: TranslationRequest) -> str:
        return _translate_with_retries(
            request=request,
            credential_pool=self.credential_pool,
            max_attempts=self._max_attempts,
            sleep=self._sleep,
            operation=self._translate_once,
        )

    def _translate_once(self, request: TranslationRequest, key: str) -> str:
        body: dict[str, object] = {
            "model": self.model,
            "input": [
                {"role": "system", "content": request.system_instruction},
                {"role": "user", "content": request.text},
            ],
            # Local-reading source text is private and every translation unit
            # is self-contained, so server-side conversation state is neither
            # needed nor appropriate.
            "store": False,
        }
        if self.web_search_enabled:
            body["tools"] = [{"type": "web_search"}]
        try:
            response = self._http_client.post(
                f"{self.base_url}/responses",
                headers={"Authorization": f"Bearer {key}"},
                json=body,
                timeout=self.timeout_seconds,
            )
        except httpx.TransportError as error:
            raise TransientError("provider request failed transiently") from error
        _raise_for_status(response)
        try:
            output = response.json()["output"]
            content = "".join(
                part["text"]
                for item in output
                if isinstance(item, dict) and item.get("type") == "message"
                for part in item.get("content", [])
                if isinstance(part, dict)
                and part.get("type") == "output_text"
                and isinstance(part.get("text"), str)
            )
        except (KeyError, TypeError, ValueError) as error:
            raise FatalError("provider returned an invalid response") from error
        if not content.strip():
            raise FatalError("provider returned an invalid response")
        return _unwrap_translation(content)


class GeminiProvider:
    def __init__(
        self,
        *,
        config: ProviderConfig,
        credential_pool: CredentialPool,
        http_client: httpx.Client | None = None,
        max_attempts: int = 3,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        if max_attempts < 1:
            raise ValueError("max_attempts must be positive")
        self.profile_id = config.profile_id
        self.config_id = config.config_id
        self.base_url = config.base_url
        self.model = config.model
        self.timeout_seconds = config.timeout_seconds
        self.concurrency_limit = config.concurrency_limit
        self.credential_pool = credential_pool
        self._http_client = http_client
        self._max_attempts = max_attempts
        self._sleep = sleep
        self._deadline_guard = _RequestDeadlineGuard()

    def translate(self, request: TranslationRequest) -> str:
        return _translate_with_retries(
            request=request,
            credential_pool=self.credential_pool,
            max_attempts=self._max_attempts,
            sleep=self._sleep,
            operation=self._translate_once,
        )

    def _translate_once(self, request: TranslationRequest, key: str) -> str:
        try:
            response = _post_with_total_deadline(
                self._http_client,
                f"{self.base_url}/models/{quote(self.model, safe='')}:generateContent",
                deadline_guard=self._deadline_guard,
                headers={"x-goog-api-key": key},
                json={
                    "system_instruction": {
                        "parts": [{"text": request.system_instruction}]
                    },
                    "contents": [
                        {"role": "user", "parts": [{"text": request.text}]}
                    ],
                },
                timeout_seconds=self.timeout_seconds,
            )
        except httpx.TimeoutException as error:
            raise ProviderTimeoutError("provider request timed out") from error
        except httpx.TransportError as error:
            raise TransientError("provider request failed transiently") from error
        _raise_for_status(response)
        try:
            parts = response.json()["candidates"][0]["content"]["parts"]
            content = "".join(
                part["text"]
                for part in parts
                if isinstance(part, dict) and isinstance(part.get("text"), str)
            )
        except (KeyError, IndexError, TypeError, ValueError) as error:
            raise FatalError("provider returned an invalid response") from error
        if not content.strip():
            raise FatalError("provider returned an invalid response")
        return _unwrap_translation(content)


def create_provider(
    profile_id: str,
    *,
    config_id: str,
    credential_pool: CredentialPool | None = None,
    registry_path: Path | None = None,
    repo_root: Path | None = None,
    environ: Mapping[str, str] | None = None,
) -> LLMProvider:
    if profile_id == "fake-provider-profile":
        return FakeProvider(config_id=config_id)
    registry = load_provider_registry(registry_path)
    try:
        config = registry[(profile_id, config_id)]
    except KeyError as error:
        raise ValueError("unknown provider configuration") from error
    runtime_environ = os.environ if environ is None else environ
    if environ is None:
        load_root_dotenv(repo_root=repo_root)
    if config.base_url_env:
        base_url_override = runtime_environ.get(config.base_url_env, "").strip()
        if base_url_override:
            config = replace(config, base_url=base_url_override.rstrip("/"))
    if config.web_search_env:
        web_search_override = runtime_environ.get(config.web_search_env, "").strip().lower()
        if web_search_override:
            if web_search_override not in {"true", "false", "1", "0"}:
                raise ValueError(
                    f"provider {config.web_search_env} must be true or false"
                )
            config = replace(
                config,
                web_search_enabled=web_search_override in {"true", "1"},
            )
    if credential_pool is None:
        keys = normalize_api_keys(runtime_environ.get(config.key_env, ""))
        if not keys:
            raise ValueError(f"provider credentials missing from {config.key_env}")
        credential_pool = KeyPool(keys)
    # Dispatch on provider_type, not profile_id: the type names the wire
    # protocol, so any profile speaking it (deepseek included) uses the same
    # client. Keying on profile_id would have forced a near-duplicate client per
    # OpenAI-compatible vendor.
    if config.provider_type == "openai-compatible":
        return OpenAICompatibleProvider(
            config=config, credential_pool=credential_pool
        )
    if config.provider_type == "openai-responses":
        return OpenAIResponsesProvider(
            config=config, credential_pool=credential_pool
        )
    if config.provider_type == "gemini-native":
        return GeminiProvider(config=config, credential_pool=credential_pool)
    raise ValueError("unknown provider profile")


def load_root_dotenv(
    *,
    repo_root: Path | None = None,
    environ: MutableMapping[str, str] | None = None,
) -> None:
    target = os.environ if environ is None else environ
    root = repo_root or _discover_repo_root(Path(__file__).resolve())
    if root is None:
        return
    env_path = root / ".env"
    if not env_path.is_file():
        return
    for key, value in _parse_dotenv(
        env_path.read_text(encoding="utf-8")
    ).items():
        if key and value and key not in target:
            target[key] = value


def load_provider_registry(
    path: Path | None = None,
) -> dict[tuple[str, str], ProviderConfig]:
    if path is None:
        raw = files("translation_engine").joinpath("providers.toml").read_text(
            encoding="utf-8"
        )
    else:
        raw = path.read_text(encoding="utf-8")
    document = tomllib.loads(raw)
    if document.get("schema") != 1:
        raise ValueError("unsupported provider registry schema")
    entries = document.get("providers")
    if not isinstance(entries, list) or not entries:
        raise ValueError("provider registry is empty")

    registry: dict[tuple[str, str], ProviderConfig] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            raise ValueError("provider registry entry must be a table")
        config = _parse_provider_config(entry)
        key = (config.profile_id, config.config_id)
        if key in registry:
            raise ValueError("duplicate provider configuration")
        registry[key] = config
    return registry


def _parse_provider_config(entry: dict[str, object]) -> ProviderConfig:
    required = {
        "profile_id",
        "config_id",
        "provider_type",
        "base_url",
        "model",
        "timeout_seconds",
        "concurrency_limit",
        "key_env",
    }
    allowed = required | {"base_url_env", "web_search_env"}
    if not required.issubset(entry) or not set(entry).issubset(allowed):
        raise ValueError("provider registry entry has invalid fields")
    strings = {
        name: entry[name]
        for name in (
            "profile_id",
            "config_id",
            "provider_type",
            "base_url",
            "model",
            "key_env",
        )
    }
    if any(not isinstance(value, str) or not value.strip() for value in strings.values()):
        raise ValueError("provider registry string fields must not be empty")
    provider_type = str(strings["provider_type"])
    if provider_type not in {
        "openai-compatible",
        "openai-responses",
        "gemini-native",
    }:
        raise ValueError("unsupported provider type")
    timeout = entry["timeout_seconds"]
    if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or timeout <= 0:
        raise ValueError("provider timeout must be positive")
    concurrency = entry["concurrency_limit"]
    if (
        isinstance(concurrency, bool)
        or not isinstance(concurrency, int)
        or concurrency < 1
    ):
        raise ValueError("provider concurrency limit must be positive")
    key_env = str(strings["key_env"])
    if re.fullmatch(r"[A-Z_][A-Z0-9_]*", key_env) is None:
        raise ValueError("provider key_env must name an environment variable")
    base_url_env = entry.get("base_url_env")
    if base_url_env is not None and (
        not isinstance(base_url_env, str)
        or re.fullmatch(r"[A-Z_][A-Z0-9_]*", base_url_env) is None
    ):
        raise ValueError("provider base_url_env must name an environment variable")
    web_search_env = entry.get("web_search_env")
    if web_search_env is not None and (
        not isinstance(web_search_env, str)
        or re.fullmatch(r"[A-Z_][A-Z0-9_]*", web_search_env) is None
    ):
        raise ValueError("provider web_search_env must name an environment variable")
    return ProviderConfig(
        profile_id=str(strings["profile_id"]),
        config_id=str(strings["config_id"]),
        provider_type=provider_type,
        base_url=str(strings["base_url"]).rstrip("/"),
        model=str(strings["model"]),
        timeout_seconds=float(timeout),
        concurrency_limit=concurrency,
        key_env=key_env,
        base_url_env=base_url_env,
        web_search_env=web_search_env,
    )


def _discover_repo_root(start: Path) -> Path | None:
    for candidate in (start, *start.parents):
        if (candidate / "pyproject.toml").is_file() and (candidate / "packages").is_dir():
            return candidate
    return None


def _parse_dotenv(raw: str) -> dict[str, str]:
    values: dict[str, str] = {}
    lines = raw.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index].strip()
        index += 1
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.removeprefix("export ").strip()
        value = value.strip()
        if value.startswith(('"', "'")):
            quote_character = value[0]
            if len(value) >= 2 and value.endswith(quote_character):
                value = value[1:-1]
            else:
                parts = [value[1:]]
                closed = False
                while index < len(lines):
                    continuation = lines[index]
                    index += 1
                    if continuation.rstrip().endswith(quote_character):
                        parts.append(continuation.rstrip()[:-1])
                        closed = True
                        break
                    parts.append(continuation)
                if not closed:
                    continue
                value = "\n".join(parts)
        if key:
            values[key] = value
    return values


def _unwrap_translation(value: str) -> str:
    match = re.fullmatch(
        r"\s*<TRANSLATION>\s*(.*?)\s*</TRANSLATION>\s*",
        value,
        flags=re.DOTALL,
    )
    return match.group(1) if match else value


MAX_RATE_LIMIT_WAIT_SECONDS = 60.0


def _translate_with_retries(
    *,
    request: TranslationRequest,
    credential_pool: CredentialPool,
    max_attempts: int,
    sleep: Callable[[float], None],
    operation: Callable[[TranslationRequest, str], str],
) -> str:
    rate_limit_failures = 0
    transient_failures = 0
    rate_limit_budget = credential_pool.pool_size + max_attempts - 1
    while True:
        try:
            key = credential_pool.acquire()
        except RateLimitError as error:
            # Every credential is throttled: wait out a short throttle in
            # place; a long one propagates so the run pauses and resumes
            # from checkpoints instead of burning the retry budget.
            rate_limit_failures += 1
            if (
                rate_limit_failures >= rate_limit_budget
                or error.retry_after_seconds > MAX_RATE_LIMIT_WAIT_SECONDS
            ):
                raise
            sleep(error.retry_after_seconds)
            continue
        try:
            return operation(request, key)
        except RateLimitError as error:
            credential_pool.report_rate_limit(
                key, retry_after_seconds=error.retry_after_seconds
            )
            rate_limit_failures += 1
            if rate_limit_failures >= rate_limit_budget:
                raise
        except TransientError:
            transient_failures += 1
            if transient_failures >= max_attempts:
                raise
            sleep(0.5 * (2 ** (transient_failures - 1)))


def _raise_for_status(response: httpx.Response) -> None:
    if response.status_code == 429:
        raise RateLimitError(retry_after_seconds=_retry_after_seconds(response))
    if 500 <= response.status_code < 600:
        raise ProviderServerError("provider returned a server error")
    if response.status_code != 200:
        raise FatalError(f"provider returned HTTP {response.status_code}")


def _retry_after_seconds(response: httpx.Response) -> float:
    raw = response.headers.get("Retry-After", "")
    try:
        return max(0.0, float(raw))
    except ValueError:
        return 60.0
