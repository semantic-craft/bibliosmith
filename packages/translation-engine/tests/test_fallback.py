import unittest

from translation_engine.pipeline import translate_chunk_with_fallback
from translation_engine.providers import (
    ProviderUnavailableError,
    RateLimitError,
    TranslationRequest,
)


class RetryThenPreserveProvider:
    profile_id = "retry-test"
    config_id = "offline"

    def __init__(self) -> None:
        self.calls = 0

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        if self.calls == 1:
            return request.text.replace("⟦PH_000000⟧", "")
        return request.text.upper().replace("⟦PH_000000⟧", "⟦PH_000000⟧")


class AlignmentProvider:
    profile_id = "alignment-test"
    config_id = "offline"

    def translate(self, request: TranslationRequest) -> str:
        return request.text.replace("⟦PH_000000⟧", "").upper()


class FailingProvider:
    profile_id = "failure-test"
    config_id = "offline"

    def translate(self, request: TranslationRequest) -> str:
        raise ProviderUnavailableError("offline failure")


class RateLimitedProvider:
    profile_id = "rate-limit-test"
    config_id = "offline"

    def translate(self, request: TranslationRequest) -> str:
        raise RateLimitError(retry_after_seconds=120.0)


class UnavailableThenValidProvider:
    profile_id = "unavailable-then-valid-test"
    config_id = "offline"

    def __init__(self) -> None:
        self.calls = 0

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        if self.calls == 1:
            raise ProviderUnavailableError("offline failure")
        return request.text.upper()


class MidWordAlignmentProvider:
    profile_id = "mid-word-alignment-test"
    config_id = "offline"

    def translate(self, request: TranslationRequest) -> str:
        return "AB CDEFG"


class PromptPlaceholderLeakingProvider:
    profile_id = "prompt-placeholder-leak-test"
    config_id = "offline"

    def translate(self, request: TranslationRequest) -> str:
        if "⟦PH_000000⟧" in request.text:
            return f"⟦PH_000999⟧{request.text}"
        return "⟦PH_000999⟧TRANSLATED TEXT"


class StructureChangingThenValidProvider:
    profile_id = "structure-test"
    config_id = "offline"

    def __init__(self) -> None:
        self.calls = 0

    def translate(self, request: TranslationRequest) -> str:
        self.calls += 1
        if self.calls == 1:
            return request.text + "\n\nadded paragraph"
        return request.text.upper()


class PlaceholderFallbackTests(unittest.TestCase):
    def test_candidate_validator_retries_structure_changing_output(self) -> None:
        provider = StructureChangingThenValidProvider()
        request = TranslationRequest(
            text="before ⟦PH_000000⟧ after",
            source_language="auto",
            target_language="zh-Hans",
            system_instruction="translate",
        )

        result = translate_chunk_with_fallback(
            provider,
            request,
            placeholder_retries=1,
            candidate_validator=lambda candidate: "\n\n" not in candidate,
        )

        self.assertEqual(result.degradation, "none")
        self.assertEqual(result.provider_attempts, 2)
        self.assertEqual(result.text, request.text.upper())

    def test_retry_alignment_and_source_fallback_all_preserve_placeholder_structure(self) -> None:
        request = TranslationRequest(
            text="before ⟦PH_000000⟧ after",
            source_language="auto",
            target_language="zh-Hans",
            system_instruction="translate",
        )

        retried = translate_chunk_with_fallback(
            RetryThenPreserveProvider(), request, placeholder_retries=1
        )
        aligned = translate_chunk_with_fallback(
            AlignmentProvider(), request, placeholder_retries=1
        )
        self.assertEqual(retried.degradation, "none")
        self.assertEqual(retried.text, "BEFORE ⟦PH_000000⟧ AFTER")
        self.assertEqual(aligned.degradation, "aligned")
        self.assertEqual(aligned.text.count("⟦PH_000000⟧"), 1)

    def test_provider_unavailability_propagates_instead_of_degrading_to_source(self) -> None:
        with self.assertRaises(ProviderUnavailableError):
            translate_chunk_with_fallback(
                FailingProvider(),
                TranslationRequest(
                    text="before ⟦PH_000000⟧ after",
                    source_language="auto",
                    target_language="zh-Hans",
                    system_instruction="translate",
                ),
                placeholder_retries=1,
            )

    def test_provider_unavailability_stops_fallback_attempts_immediately(self) -> None:
        provider = UnavailableThenValidProvider()
        with self.assertRaises(ProviderUnavailableError):
            translate_chunk_with_fallback(
                provider,
                TranslationRequest(
                    text="source",
                    source_language="auto",
                    target_language="zh-Hans",
                    system_instruction="translate",
                ),
                placeholder_retries=1,
            )
        self.assertEqual(provider.calls, 1)

    def test_alignment_snaps_placeholders_to_the_nearest_word_boundary(self) -> None:
        result = translate_chunk_with_fallback(
            MidWordAlignmentProvider(),
            TranslationRequest(
                text="abcde⟦PH_000000⟧fghij",
                source_language="auto",
                target_language="zh-Hans",
                system_instruction="translate",
            ),
            placeholder_retries=0,
        )

        self.assertEqual(result.degradation, "aligned")
        self.assertEqual(result.text, "AB ⟦PH_000000⟧CDEFG")

    def test_plain_fallback_removes_placeholders_leaked_from_the_prompt(self) -> None:
        result = translate_chunk_with_fallback(
            PromptPlaceholderLeakingProvider(),
            TranslationRequest(
                text="before ⟦PH_000000⟧ after",
                source_language="auto",
                target_language="zh-Hans",
                system_instruction="translate",
            ),
            placeholder_retries=0,
        )

        self.assertEqual(result.degradation, "aligned")
        self.assertEqual(result.text.count("⟦PH_000000⟧"), 1)
        self.assertNotIn("⟦PH_000999⟧", result.text)

    def test_rate_limit_propagates_instead_of_degrading_to_source(self) -> None:
        with self.assertRaises(RateLimitError):
            translate_chunk_with_fallback(
                RateLimitedProvider(),
                TranslationRequest(
                    text="before ⟦PH_000000⟧ after",
                    source_language="auto",
                    target_language="zh-Hans",
                    system_instruction="translate",
                ),
                placeholder_retries=1,
            )


if __name__ == "__main__":
    unittest.main()
