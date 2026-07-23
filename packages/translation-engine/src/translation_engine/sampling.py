from __future__ import annotations

import re
from typing import TypeVar


Block = TypeVar("Block")

_SENTENCE_BOUNDARY = re.compile(
    r"[.!?。！？]+(?:[\"'”’）】》」』]+)?(?=\s|\Z)"
)


def select_internal_blocks(blocks: list[Block], count: int) -> list[Block]:
    """Select uniformly distributed blocks while excluding both endpoints."""
    if count < 1:
        raise ValueError("sample count must be positive")
    total = len(blocks)
    internal_count = max(total - 2, 0)
    if internal_count <= count:
        return list(blocks[1:-1])

    selected_indices: list[int] = []
    for index in range(count):
        candidate = round((index + 1) * (total - 1) / (count + 1))
        candidate = min(max(candidate, 1), total - 2)
        if candidate not in selected_indices:
            selected_indices.append(candidate)
    return [blocks[index] for index in selected_indices]


def truncate_at_sentence_boundary(text: str, character_budget: int) -> str:
    """End at the first complete sentence at or beyond the character budget."""
    if character_budget < 1:
        raise ValueError("character budget must be positive")
    if len(text) <= character_budget:
        return text
    for boundary in _SENTENCE_BOUNDARY.finditer(text):
        if boundary.end() >= character_budget:
            return text[: boundary.end()]
    return text
