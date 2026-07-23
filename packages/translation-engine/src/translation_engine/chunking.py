from __future__ import annotations

import re

from .placeholders import PLACEHOLDER_PATTERN


_SAFE_BREAK_PATTERN = re.compile(r"\n[ \t]*\n|\n|[.!?。！？][ \t]+|[ \t]+")


class Utf8ByteTokenCounter:
    """Offline upper bound: cl100k token count cannot exceed UTF-8 byte count."""

    def count(self, text: str) -> int:
        return len(text.encode("utf-8"))

    def spans(self, text: str) -> list[tuple[int, int]]:
        spans: list[tuple[int, int]] = []
        cursor = 0
        for placeholder in PLACEHOLDER_PATTERN.finditer(text):
            spans.extend(self._character_byte_spans(text, cursor, placeholder.start()))
            spans.extend(
                [placeholder.span()] * len(placeholder.group(0).encode("utf-8"))
            )
            cursor = placeholder.end()
        spans.extend(self._character_byte_spans(text, cursor, len(text)))
        return spans

    @staticmethod
    def _character_byte_spans(text: str, start: int, end: int) -> list[tuple[int, int]]:
        spans: list[tuple[int, int]] = []
        for index in range(start, end):
            spans.extend([(index, index + 1)] * len(text[index].encode("utf-8")))
        return spans


class TokenChunker:
    def __init__(self, *, max_tokens: int, counter: Utf8ByteTokenCounter) -> None:
        if max_tokens < 1:
            raise ValueError("max_tokens must be positive")
        self.max_tokens = max_tokens
        self.counter = counter

    def split(self, text: str) -> list[str]:
        if not text:
            return [""]
        chunks: list[str] = []
        cursor = 0
        while cursor < len(text):
            remaining = text[cursor:]
            spans = self.counter.spans(remaining)
            if len(spans) <= self.max_tokens:
                chunks.append(remaining)
                break

            hard_end = spans[self.max_tokens][0]
            if hard_end == 0:
                raise ValueError("max_tokens is too small for one protected text atom")
            safe_end = self._last_safe_break(remaining, hard_end)
            end = safe_end if safe_end is not None else hard_end
            if end <= 0:
                end = spans[self.max_tokens - 1][1]
            chunk = remaining[:end]
            if self.counter.count(chunk) > self.max_tokens:
                chunk = remaining[:hard_end]
                end = hard_end
            chunks.append(chunk)
            cursor += end
        return chunks

    def _last_safe_break(self, text: str, hard_end: int) -> int | None:
        candidates = [
            match.end()
            for match in _SAFE_BREAK_PATTERN.finditer(text, 0, hard_end)
            if self.counter.count(text[: match.end()]) > 0
        ]
        return candidates[-1] if candidates else None
