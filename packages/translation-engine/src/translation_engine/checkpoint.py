from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import re
from typing import Any

from .files import atomic_write_text


_UNIT_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*")


@dataclass(frozen=True)
class UnitIdempotencyKey:
    task_manifest_sha256: str
    provider_profile_id: str
    provider_config_id: str
    translation_policy_version: str
    pass_id: str = "translation-v1"

    def to_dict(self) -> dict[str, str]:
        return {
            "taskManifestSha256": self.task_manifest_sha256,
            "providerProfileId": self.provider_profile_id,
            "providerConfigId": self.provider_config_id,
            "translationPolicyVersion": self.translation_policy_version,
            "passId": self.pass_id,
        }


@dataclass(frozen=True)
class UnitCheckpoint:
    next_chunk_index: int
    translated_chunks: tuple[str, ...]
    reflection_chunks: tuple[str, ...] = ()


class CheckpointStore:
    def __init__(self, partial_directory: Path) -> None:
        self.partial_directory = partial_directory

    def path_for(self, unit_id: str) -> Path:
        if _UNIT_ID.fullmatch(unit_id) is None:
            raise ValueError("invalid unit id")
        return self.partial_directory / f"{unit_id}.json"

    def load(
        self, unit_id: str, key: UnitIdempotencyKey
    ) -> UnitCheckpoint | None:
        path = self.path_for(unit_id)
        if not path.is_file():
            return None
        try:
            document = json.loads(path.read_text(encoding="utf-8"))
            if (
                not isinstance(document, dict)
                or document.get("schema") != "translation-engine-checkpoint-v1"
                or document.get("unitId") != unit_id
                or document.get("idempotencyKey") != key.to_dict()
            ):
                self.delete(unit_id)
                return None
            checkpoint = self._parse_checkpoint(document)
        except (OSError, json.JSONDecodeError, TypeError, ValueError):
            self.delete(unit_id)
            return None
        return checkpoint

    def save(
        self,
        unit_id: str,
        key: UnitIdempotencyKey,
        checkpoint: UnitCheckpoint,
    ) -> None:
        if checkpoint.next_chunk_index != len(checkpoint.translated_chunks):
            raise ValueError("checkpoint must contain a contiguous translated prefix")
        if checkpoint.reflection_chunks and checkpoint.next_chunk_index != len(
            checkpoint.reflection_chunks
        ):
            raise ValueError("checkpoint must contain a contiguous reflection prefix")
        document = {
            "schema": "translation-engine-checkpoint-v1",
            "unitId": unit_id,
            "idempotencyKey": key.to_dict(),
            "nextChunkIndex": checkpoint.next_chunk_index,
            "translatedChunks": list(checkpoint.translated_chunks),
        }
        if checkpoint.reflection_chunks:
            document["reflectionChunks"] = list(checkpoint.reflection_chunks)
        path = self.path_for(unit_id)
        atomic_write_text(path, json.dumps(document, separators=(",", ":")) + "\n")

    def delete(self, unit_id: str) -> None:
        self.path_for(unit_id).unlink(missing_ok=True)

    @staticmethod
    def _parse_checkpoint(document: dict[str, Any]) -> UnitCheckpoint:
        next_chunk_index = document.get("nextChunkIndex")
        translated_chunks = document.get("translatedChunks")
        reflection_chunks = document.get("reflectionChunks", [])
        if (
            not isinstance(next_chunk_index, int)
            or isinstance(next_chunk_index, bool)
            or next_chunk_index < 0
            or not isinstance(translated_chunks, list)
            or not all(isinstance(chunk, str) for chunk in translated_chunks)
            or next_chunk_index != len(translated_chunks)
            or not isinstance(reflection_chunks, list)
            or not all(isinstance(chunk, str) for chunk in reflection_chunks)
            or (reflection_chunks and next_chunk_index != len(reflection_chunks))
        ):
            raise ValueError("invalid checkpoint")
        return UnitCheckpoint(
            next_chunk_index,
            tuple(translated_chunks),
            tuple(reflection_chunks),
        )
