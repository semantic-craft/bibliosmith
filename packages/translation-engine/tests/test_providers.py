import json
from dataclasses import replace
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest import mock

import httpx

from translation_engine.providers import (
    CredentialPool,
    FatalError,
    GeminiProvider,
    KeyPool,
    LLMProvider,
    OpenAICompatibleProvider,
    OpenAIResponsesProvider,
    ProviderConfig,
    ProviderServerError,
    ProviderTimeoutError,
    RateLimitError,
    TransientError,
    TranslationRequest,
    create_provider,
    load_provider_registry,
    normalize_api_keys,
)


class ProviderFactoryTests(unittest.TestCase):
    def test_factory_keeps_the_deterministic_offline_provider(self) -> None:
        fake = create_provider(
            "fake-provider-profile", config_id="fake-config-no-secrets"
        )
        request = TranslationRequest(
            text="protected text",
            source_language="auto",
            target_language="zh-Hans",
            system_instruction="translate",
        )

        self.assertIsInstance(fake, LLMProvider)
        self.assertEqual(fake.translate(request), "PROTECTED TEXT")

    def test_default_registry_contains_only_non_secret_backend_configuration(
        self,
    ) -> None:
        registry = load_provider_registry()

        self.assertEqual(
            set(registry),
            {
                ("openai-compatible", "openai-default"),
                ("gemini-native", "gemini-default"),
                ("deepseek", "deepseek-default"),
                ("kimi", "kimi-default"),
                ("qwen", "payg"),
                ("doubao", "cn-beijing"),
                ("mimo", "payg"),
                ("mimo", "token-plan"),
            },
        )
        self.assertEqual(
            registry[("openai-compatible", "openai-default")].key_env,
            "OPENAI_COMPATIBLE_API_KEYS",
        )
        self.assertEqual(
            registry[("gemini-native", "gemini-default")].key_env,
            "GEMINI_API_KEYS",
        )
        # Providers that still expose only Chat Completions share the legacy
        # OpenAI-compatible client.
        for profile, config in [
            ("deepseek", "deepseek-default"),
            ("kimi", "kimi-default"),
            ("mimo", "token-plan"),
        ]:
            self.assertEqual(
                registry[(profile, config)].provider_type, "openai-compatible"
            )
        for profile, config in [("qwen", "payg"), ("doubao", "cn-beijing")]:
            self.assertEqual(
                registry[(profile, config)].provider_type, "openai-responses"
            )
        qwen_payg = registry[("qwen", "payg")]
        self.assertEqual(
            qwen_payg.base_url,
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
        )
        self.assertEqual(qwen_payg.base_url_env, "QWEN_API_BASE_URL")
        self.assertEqual(qwen_payg.web_search_env, "QWEN_WEB_SEARCH_ENABLED")
        self.assertEqual(qwen_payg.model, "qwen3.7-max")
        self.assertNotIn(("qwen", "token-plan"), registry)

        doubao = registry[("doubao", "cn-beijing")]
        self.assertEqual(
            doubao.base_url,
            "https://ark.cn-beijing.volces.com/api/v3",
        )
        self.assertEqual(doubao.model, "doubao-seed-2-1-pro-260628")
        self.assertEqual(doubao.key_env, "VOLCENGINE_ARK_API_KEYS")

    def test_registry_factory_loads_keys_only_from_the_repository_root_env(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            repo_root = Path(temporary_directory)
            registry_path = repo_root / "providers.toml"
            registry_path.write_text(
                """
schema = 1

[[providers]]
profile_id = "openai-compatible"
config_id = "openai-smoke"
provider_type = "openai-compatible"
base_url = "https://openai.example/v1"
model = "model-a"
timeout_seconds = 45
concurrency_limit = 3
key_env = "TEST_OPENAI_KEYS"

[[providers]]
profile_id = "gemini-native"
config_id = "gemini-smoke"
provider_type = "gemini-native"
base_url = "https://gemini.example/v1beta"
model = "model-b"
timeout_seconds = 60
concurrency_limit = 2
key_env = "TEST_GEMINI_KEYS"
""".strip()
                + "\n",
                encoding="utf-8",
            )
            (repo_root / ".env").write_text(
                'TEST_OPENAI_KEYS=" key-a,\nkey-b\nkey-a "\n'
                "TEST_GEMINI_KEYS=gem-key\n",
                encoding="utf-8",
            )
            package_root = repo_root / "packages" / "translation-engine"
            package_root.mkdir(parents=True)
            (package_root / ".env").write_text(
                "TEST_OPENAI_KEYS=wrong-package-key\n", encoding="utf-8"
            )

            with mock.patch.dict(
                os.environ, {"TEST_GEMINI_KEYS": "process-gem-key"}, clear=False
            ):
                os.environ.pop("TEST_OPENAI_KEYS", None)
                registry = load_provider_registry(registry_path)
                openai = create_provider(
                    "openai-compatible",
                    config_id="openai-smoke",
                    registry_path=registry_path,
                    repo_root=repo_root,
                )
                gemini = create_provider(
                    "gemini-native",
                    config_id="gemini-smoke",
                    registry_path=registry_path,
                    repo_root=repo_root,
                )

        self.assertEqual(set(registry), {("openai-compatible", "openai-smoke"), ("gemini-native", "gemini-smoke")})
        self.assertIsInstance(openai, OpenAICompatibleProvider)
        self.assertIsInstance(gemini, GeminiProvider)
        self.assertIsInstance(openai, LLMProvider)
        self.assertIsInstance(gemini, LLMProvider)
        self.assertEqual(openai.model, "model-a")
        self.assertEqual(openai.timeout_seconds, 45.0)
        self.assertEqual(openai.concurrency_limit, 3)
        self.assertEqual(openai.credential_pool.acquire(), "key-a")
        self.assertEqual(openai.credential_pool.acquire(), "key-b")
        self.assertEqual(gemini.credential_pool.acquire(), "process-gem-key")

    def test_a_profile_dispatches_on_provider_type_not_its_name(self) -> None:
        # DeepSeek speaks the OpenAI protocol under its own profile name, so the
        # factory has to pick the client from provider_type. Keying on profile_id
        # would have left this profile with no client.
        with tempfile.TemporaryDirectory() as temporary_directory:
            registry_path = Path(temporary_directory) / "providers.toml"
            registry_path.write_text(
                """
schema = 1

[[providers]]
profile_id = "deepseek"
config_id = "deepseek-default"
provider_type = "openai-compatible"
base_url = "https://api.deepseek.com"
model = "deepseek-chat"
timeout_seconds = 120
concurrency_limit = 4
key_env = "DEEPSEEK_TEST_KEYS"
""".strip()
                + "\n",
                encoding="utf-8",
            )

            provider = create_provider(
                "deepseek",
                config_id="deepseek-default",
                registry_path=registry_path,
                environ={"DEEPSEEK_TEST_KEYS": "ds-key"},
            )

        self.assertIsInstance(provider, OpenAICompatibleProvider)
        self.assertEqual(provider.model, "deepseek-chat")
        self.assertEqual(provider.base_url, "https://api.deepseek.com")

    def test_responses_profiles_dispatch_on_the_wire_protocol(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            registry_path = Path(temporary_directory) / "providers.toml"
            registry_path.write_text(
                """
schema = 1

[[providers]]
profile_id = "qwen"
config_id = "payg"
provider_type = "openai-responses"
base_url = "https://dashscope.example/v1"
model = "qwen3.7-max"
timeout_seconds = 120
concurrency_limit = 4
key_env = "QWEN_TEST_KEYS"
""".strip()
                + "\n",
                encoding="utf-8",
            )

            provider = create_provider(
                "qwen",
                config_id="payg",
                registry_path=registry_path,
                environ={"QWEN_TEST_KEYS": "qwen-key"},
            )

        self.assertIsInstance(provider, OpenAIResponsesProvider)
        self.assertIsInstance(provider, LLMProvider)
        self.assertEqual(provider.model, "qwen3.7-max")

    def test_qwen_workspace_base_url_can_be_supplied_at_runtime(self) -> None:
        provider = create_provider(
            "qwen",
            config_id="payg",
            environ={
                "QWEN_PAYG_API_KEYS": "qwen-key",
                "QWEN_API_BASE_URL": (
                    "https://ws-example.cn-beijing.maas.aliyuncs.com/"
                    "compatible-mode/v1"
                ),
                "QWEN_WEB_SEARCH_ENABLED": "true",
            },
        )

        self.assertEqual(
            provider.base_url,
            "https://ws-example.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
        )
        self.assertTrue(provider.web_search_enabled)


class HTTPProviderTests(unittest.TestCase):
    def test_openai_compatible_provider_sends_chat_completion_and_returns_text(
        self,
    ) -> None:
        def handle(request: httpx.Request) -> httpx.Response:
            self.assertEqual(
                str(request.url), "https://openai.example/v1/chat/completions"
            )
            self.assertEqual(request.headers["authorization"], "Bearer fake-key")
            self.assertEqual(
                json.loads(request.content),
                {
                    "model": "model-a",
                    "messages": [
                        {"role": "system", "content": "Translate faithfully."},
                        {"role": "user", "content": "Source chapter"},
                    ],
                },
            )
            return httpx.Response(
                200,
                json={
                    "choices": [
                        {
                            "message": {
                                "content": "<TRANSLATION>\n译文章节\n</TRANSLATION>"
                            }
                        }
                    ]
                },
            )

        client = httpx.Client(transport=httpx.MockTransport(handle))
        provider = OpenAICompatibleProvider(
            config=_provider_config(
                profile_id="openai-compatible",
                provider_type="openai-compatible",
                base_url="https://openai.example/v1",
                model="model-a",
            ),
            credential_pool=KeyPool(("fake-key",)),
            http_client=client,
        )

        translated = provider.translate(
            TranslationRequest(
                text="Source chapter",
                source_language="en",
                target_language="zh-Hans",
                system_instruction="Translate faithfully.",
            )
        )

        self.assertEqual(translated, "译文章节")

    def test_responses_provider_sends_private_input_and_returns_message_text(
        self,
    ) -> None:
        def handle(request: httpx.Request) -> httpx.Response:
            self.assertEqual(str(request.url), "https://responses.example/v1/responses")
            self.assertEqual(request.headers["authorization"], "Bearer fake-key")
            self.assertEqual(
                json.loads(request.content),
                {
                    "model": "model-r",
                    "input": [
                        {"role": "system", "content": "Translate faithfully."},
                        {"role": "user", "content": "Source chapter"},
                    ],
                    "store": False,
                },
            )
            return httpx.Response(
                200,
                json={
                    "object": "response",
                    "status": "completed",
                    "output": [
                        {
                            "type": "reasoning",
                            "status": "completed",
                            "summary": [
                                {"type": "summary_text", "text": "Translate."}
                            ],
                        },
                        {
                            "type": "message",
                            "role": "assistant",
                            "status": "completed",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "<TRANSLATION>\n译文章节\n</TRANSLATION>",
                                }
                            ],
                        },
                    ],
                },
            )

        provider = OpenAIResponsesProvider(
            config=_provider_config(
                profile_id="qwen",
                provider_type="openai-responses",
                base_url="https://responses.example/v1",
                model="model-r",
            ),
            credential_pool=KeyPool(("fake-key",)),
            http_client=httpx.Client(transport=httpx.MockTransport(handle)),
        )

        self.assertEqual(provider.translate(_translation_request()), "译文章节")

    def test_responses_provider_rejects_a_response_without_output_text(self) -> None:
        provider = OpenAIResponsesProvider(
            config=_provider_config(
                profile_id="doubao",
                provider_type="openai-responses",
                base_url="https://responses.example/v1",
                model="model-r",
            ),
            credential_pool=KeyPool(("fake-key",)),
            http_client=httpx.Client(
                transport=httpx.MockTransport(
                    lambda _request: httpx.Response(
                        200,
                        json={
                            "object": "response",
                            "status": "completed",
                            "output": [{"type": "reasoning", "summary": []}],
                        },
                    )
                )
            ),
        )

        with self.assertRaisesRegex(FatalError, "invalid response"):
            provider.translate(_translation_request())

    def test_responses_provider_enables_qwen_web_search_with_a_tool(self) -> None:
        def handle(request: httpx.Request) -> httpx.Response:
            body = json.loads(request.content)
            self.assertEqual(body["tools"], [{"type": "web_search"}])
            self.assertNotIn("enable_search", body)
            return httpx.Response(
                200,
                json={
                    "output": [
                        {"type": "web_search_call", "status": "completed"},
                        {
                            "type": "message",
                            "content": [
                                {"type": "output_text", "text": "联网核对后的译文"}
                            ],
                        },
                    ]
                },
            )

        provider = OpenAIResponsesProvider(
            config=replace(
                _provider_config(
                    profile_id="qwen",
                    provider_type="openai-responses",
                    base_url="https://responses.example/v1",
                    model="qwen3.7-max",
                ),
                web_search_enabled=True,
            ),
            credential_pool=KeyPool(("fake-key",)),
            http_client=httpx.Client(transport=httpx.MockTransport(handle)),
        )

        self.assertEqual(
            provider.translate(_translation_request()), "联网核对后的译文"
        )

    def test_429_rotates_keys_on_an_independent_pool_sized_budget(self) -> None:
        attempted_keys: list[str] = []

        def handle(request: httpx.Request) -> httpx.Response:
            attempted_keys.append(request.headers["authorization"].removeprefix("Bearer "))
            return httpx.Response(429, headers={"Retry-After": "0"})

        provider = OpenAICompatibleProvider(
            config=_provider_config(
                profile_id="openai-compatible",
                provider_type="openai-compatible",
                base_url="https://openai.example/v1",
                model="model-a",
            ),
            credential_pool=KeyPool(("key-a", "key-b")),
            http_client=httpx.Client(transport=httpx.MockTransport(handle)),
            max_attempts=3,
            sleep=lambda _: None,
        )

        with self.assertRaises(RateLimitError):
            provider.translate(_translation_request())

        self.assertEqual(attempted_keys, ["key-a", "key-b", "key-a", "key-b"])

    def test_429_does_not_consume_the_transient_retry_budget(self) -> None:
        statuses = iter((429, 503, 200))
        attempted_keys: list[str] = []

        def handle(request: httpx.Request) -> httpx.Response:
            attempted_keys.append(request.headers["authorization"].removeprefix("Bearer "))
            status = next(statuses)
            if status == 200:
                return httpx.Response(
                    200,
                    json={"choices": [{"message": {"content": "译文"}}]},
                )
            return httpx.Response(status, headers={"Retry-After": "0"})

        provider = OpenAICompatibleProvider(
            config=_provider_config(
                profile_id="openai-compatible",
                provider_type="openai-compatible",
                base_url="https://openai.example/v1",
                model="model-a",
            ),
            credential_pool=KeyPool(("key-a", "key-b")),
            http_client=httpx.Client(transport=httpx.MockTransport(handle)),
            max_attempts=2,
            sleep=lambda _: None,
        )

        translated = provider.translate(_translation_request())

        self.assertEqual(translated, "译文")
        self.assertEqual(attempted_keys, ["key-a", "key-b", "key-a"])

    def test_fully_throttled_pool_waits_out_a_short_rate_limit(self) -> None:
        now = [100.0]
        sleeps: list[float] = []
        statuses = iter((429, 429, 200))

        def handle(request: httpx.Request) -> httpx.Response:
            status = next(statuses)
            if status == 200:
                return httpx.Response(
                    200,
                    json={"choices": [{"message": {"content": "译文"}}]},
                )
            return httpx.Response(status, headers={"Retry-After": "3"})

        def sleep(duration: float) -> None:
            sleeps.append(duration)
            now[0] += duration

        provider = OpenAICompatibleProvider(
            config=_provider_config(
                profile_id="openai-compatible",
                provider_type="openai-compatible",
                base_url="https://openai.example/v1",
                model="model-a",
            ),
            credential_pool=KeyPool(("key-a", "key-b"), clock=lambda: now[0]),
            http_client=httpx.Client(transport=httpx.MockTransport(handle)),
            max_attempts=3,
            sleep=sleep,
        )

        self.assertEqual(provider.translate(_translation_request()), "译文")
        self.assertEqual(sleeps, [3.0])

    def test_fully_throttled_pool_propagates_a_long_rate_limit(self) -> None:
        now = [100.0]
        sleeps: list[float] = []

        def handle(request: httpx.Request) -> httpx.Response:
            return httpx.Response(429, headers={"Retry-After": "600"})

        provider = OpenAICompatibleProvider(
            config=_provider_config(
                profile_id="openai-compatible",
                provider_type="openai-compatible",
                base_url="https://openai.example/v1",
                model="model-a",
            ),
            credential_pool=KeyPool(("key-a", "key-b"), clock=lambda: now[0]),
            http_client=httpx.Client(transport=httpx.MockTransport(handle)),
            max_attempts=3,
            sleep=sleeps.append,
        )

        with self.assertRaises(RateLimitError):
            provider.translate(_translation_request())

        self.assertEqual(sleeps, [])

    def test_timeout_and_5xx_use_bounded_backoff_retries(self) -> None:
        expected_errors = {
            "timeout": ProviderTimeoutError,
            "server-error": ProviderServerError,
        }
        for failure_kind, expected_error in expected_errors.items():
            with self.subTest(failure_kind=failure_kind):
                attempts = 0
                sleeps: list[float] = []

                def handle(request: httpx.Request) -> httpx.Response:
                    nonlocal attempts
                    attempts += 1
                    if failure_kind == "timeout":
                        raise httpx.ReadTimeout("timeout", request=request)
                    return httpx.Response(503)

                provider = OpenAICompatibleProvider(
                    config=_provider_config(
                        profile_id="openai-compatible",
                        provider_type="openai-compatible",
                        base_url="https://openai.example/v1",
                        model="model-a",
                    ),
                    credential_pool=KeyPool(("fake-key",)),
                    http_client=httpx.Client(
                        transport=httpx.MockTransport(handle)
                    ),
                    max_attempts=2,
                    sleep=sleeps.append,
                )

                with self.assertRaises(expected_error) as raised:
                    provider.translate(_translation_request())

                self.assertEqual(attempts, 2)
                self.assertEqual(sleeps, [0.5])
                self.assertEqual(
                    raised.exception.code,
                    "provider_timeout"
                    if failure_kind == "timeout"
                    else "provider_http_5xx",
                )

    def test_provider_enforces_the_configured_total_request_deadline(self) -> None:
        def handle(request: httpx.Request) -> httpx.Response:
            time.sleep(0.2)
            return httpx.Response(
                200,
                json={"choices": [{"message": {"content": "迟到的译文"}}]},
            )

        provider = OpenAICompatibleProvider(
            config=_provider_config(
                profile_id="openai-compatible",
                provider_type="openai-compatible",
                base_url="https://openai.example/v1",
                model="model-a",
                timeout_seconds=0.02,
            ),
            credential_pool=KeyPool(("fake-key",)),
            http_client=httpx.Client(transport=httpx.MockTransport(handle)),
            max_attempts=1,
        )

        started = time.monotonic()
        with self.assertRaises(ProviderTimeoutError):
            provider.translate(_translation_request())

        self.assertLess(time.monotonic() - started, 0.15)

    def test_total_deadline_does_not_close_or_overlap_an_injected_client(self) -> None:
        attempts = 0
        first_finished = threading.Event()

        def handle(request: httpx.Request) -> httpx.Response:
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                time.sleep(0.05)
                first_finished.set()
            return httpx.Response(
                200,
                json={"choices": [{"message": {"content": "译文"}}]},
            )

        client = httpx.Client(transport=httpx.MockTransport(handle))
        provider = OpenAICompatibleProvider(
            config=_provider_config(
                profile_id="openai-compatible",
                provider_type="openai-compatible",
                base_url="https://openai.example/v1",
                model="model-a",
                timeout_seconds=0.01,
            ),
            credential_pool=KeyPool(("fake-key",)),
            http_client=client,
            max_attempts=2,
            sleep=lambda _: None,
        )

        with self.assertRaises(ProviderTimeoutError):
            provider.translate(_translation_request())

        self.assertEqual(attempts, 1)
        self.assertFalse(client.is_closed)
        self.assertTrue(first_finished.wait(0.2))
        self.assertEqual(provider.translate(_translation_request()), "译文")
        self.assertEqual(attempts, 2)

    def test_non_rate_limit_4xx_fails_fast_as_fatal(self) -> None:
        attempts = 0
        sleeps: list[float] = []

        def handle(request: httpx.Request) -> httpx.Response:
            nonlocal attempts
            attempts += 1
            return httpx.Response(400, json={"error": "fake bad request"})

        provider = OpenAICompatibleProvider(
            config=_provider_config(
                profile_id="openai-compatible",
                provider_type="openai-compatible",
                base_url="https://openai.example/v1",
                model="model-a",
            ),
            credential_pool=KeyPool(("fake-key",)),
            http_client=httpx.Client(transport=httpx.MockTransport(handle)),
            sleep=sleeps.append,
        )

        with self.assertRaises(FatalError):
            provider.translate(_translation_request())

        self.assertEqual(attempts, 1)
        self.assertEqual(sleeps, [])

    def test_gemini_provider_sends_native_generate_content_and_returns_text(
        self,
    ) -> None:
        def handle(request: httpx.Request) -> httpx.Response:
            self.assertEqual(
                str(request.url),
                "https://gemini.example/v1beta/models/model-b:generateContent",
            )
            self.assertEqual(request.headers["x-goog-api-key"], "fake-gemini-key")
            self.assertEqual(
                json.loads(request.content),
                {
                    "system_instruction": {
                        "parts": [{"text": "Translate faithfully."}]
                    },
                    "contents": [
                        {
                            "role": "user",
                            "parts": [{"text": "Source chapter"}],
                        }
                    ],
                },
            )
            return httpx.Response(
                200,
                json={
                    "candidates": [
                        {
                            "content": {
                                "parts": [{"text": "译文"}, {"text": "章节"}]
                            }
                        }
                    ]
                },
            )

        client = httpx.Client(transport=httpx.MockTransport(handle))
        provider = GeminiProvider(
            config=_provider_config(
                profile_id="gemini-native",
                provider_type="gemini-native",
                base_url="https://gemini.example/v1beta",
                model="model-b",
            ),
            credential_pool=KeyPool(("fake-gemini-key",)),
            http_client=client,
        )

        translated = provider.translate(
            TranslationRequest(
                text="Source chapter",
                source_language="en",
                target_language="zh-Hans",
                system_instruction="Translate faithfully.",
            )
        )

        self.assertEqual(translated, "译文章节")

    def test_gemini_429_uses_the_same_zero_wait_key_rotation_contract(self) -> None:
        statuses = iter((429, 200))
        attempted_keys: list[str] = []

        def handle(request: httpx.Request) -> httpx.Response:
            attempted_keys.append(request.headers["x-goog-api-key"])
            status = next(statuses)
            if status == 200:
                return httpx.Response(
                    200,
                    json={
                        "candidates": [
                            {"content": {"parts": [{"text": "译文"}]}}
                        ]
                    },
                )
            return httpx.Response(429, headers={"Retry-After": "0"})

        provider = GeminiProvider(
            config=_provider_config(
                profile_id="gemini-native",
                provider_type="gemini-native",
                base_url="https://gemini.example/v1beta",
                model="model-b",
            ),
            credential_pool=KeyPool(("gem-key-a", "gem-key-b")),
            http_client=httpx.Client(transport=httpx.MockTransport(handle)),
        )

        translated = provider.translate(_translation_request())

        self.assertEqual(translated, "译文")
        self.assertEqual(attempted_keys, ["gem-key-a", "gem-key-b"])


class KeyPoolTests(unittest.TestCase):
    def test_round_robin_skips_throttled_keys_without_waiting(self) -> None:
        now = [100.0]
        keys = normalize_api_keys(" key-a, key-b\nkey-a,, key-c ")
        pool = KeyPool(keys, clock=lambda: now[0])

        self.assertEqual(keys, ("key-a", "key-b", "key-c"))
        self.assertIsInstance(pool, CredentialPool)
        self.assertEqual(pool.acquire(), "key-a")
        pool.report_rate_limit("key-a", retry_after_seconds=10.0)
        self.assertEqual(pool.acquire(), "key-b")
        self.assertEqual(pool.acquire(), "key-c")
        self.assertEqual(pool.acquire(), "key-b")

        pool.report_rate_limit("key-b", retry_after_seconds=10.0)
        pool.report_rate_limit("key-c", retry_after_seconds=10.0)
        with self.assertRaises(RateLimitError):
            pool.acquire()

        now[0] = 110.0
        self.assertEqual(pool.acquire(), "key-c")


def _provider_config(
    *,
    profile_id: str,
    provider_type: str,
    base_url: str,
    model: str,
    timeout_seconds: float = 30.0,
) -> ProviderConfig:
    return ProviderConfig(
        profile_id=profile_id,
        config_id="test-config",
        provider_type=provider_type,
        base_url=base_url,
        model=model,
        timeout_seconds=timeout_seconds,
        concurrency_limit=2,
        key_env="TEST_KEYS",
    )


def _translation_request() -> TranslationRequest:
    return TranslationRequest(
        text="Source chapter",
        source_language="en",
        target_language="zh-Hans",
        system_instruction="Translate faithfully.",
    )


if __name__ == "__main__":
    unittest.main()
