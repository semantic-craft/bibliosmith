"""One contract for committing and reconciling OCR conversion evidence.

Extractor adapters normalize their provider-owned output before calling this
module.  Recovery callers receive a typed outcome and never inspect Markdown,
filenames, modification times, or route-native sidecars.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Iterable, Mapping


EVIDENCE_SCHEMA = "ocr-conversion-evidence-v1"
SUPPORTED_ROUTES = frozenset({"pdf-text", "paddle-ocr", "mineru"})


def resolve_artifact_reference(artifact_root: Path, reference: str) -> Path:
    relative = Path(reference)
    if (
        relative.is_absolute()
        or not relative.parts
        or any(part in {"", ".", ".."} for part in relative.parts)
    ):
        raise ValueError("unsafe artifact reference")
    root = artifact_root.resolve()
    candidate = artifact_root.absolute()
    for part in relative.parts:
        candidate /= part
        if candidate.is_symlink():
            raise ValueError("unsafe artifact symlink")
    resolved = (root / relative).resolve()
    try:
        resolved.relative_to(root)
    except ValueError as exc:
        raise ValueError("artifact reference escaped its root") from exc
    return resolved


@dataclass(frozen=True)
class EvidenceArtifact:
    kind: str
    path: Path
    sha256: str
    reference: str

    def as_json(self) -> dict[str, str]:
        return {
            "kind": self.kind,
            "reference": self.reference,
            "sha256": self.sha256,
        }

    @classmethod
    def from_json(
        cls,
        value: Mapping[str, object],
        *,
        artifact_root: Path,
    ) -> "EvidenceArtifact":
        reference = str(value["reference"])
        return cls(
            kind=str(value["kind"]),
            path=resolve_artifact_reference(artifact_root, reference),
            sha256=str(value["sha256"]),
            reference=reference,
        )


@dataclass(frozen=True)
class ConversionEvidence:
    schema_version: str
    extraction_contract_version: str
    source_pdf_key: str
    source_md5: str
    source_sha256: str
    parent_item_key: str
    route: str
    page_count: int
    selected_pages: tuple[int, ...]
    artifacts: tuple[EvidenceArtifact, ...]
    markdown_attachment_key: str | None = None

    @property
    def markdown_artifact(self) -> EvidenceArtifact:
        return next(artifact for artifact in self.artifacts if artifact.kind == "markdown")

    @property
    def markdown_path(self) -> Path:
        return self.markdown_artifact.path

    def with_markdown_attachment(self, key: str) -> "ConversionEvidence":
        return replace(self, markdown_attachment_key=key)

    def as_json(self) -> dict[str, object]:
        return {
            "schemaVersion": self.schema_version,
            "extractionContractVersion": self.extraction_contract_version,
            "sourcePdfKey": self.source_pdf_key,
            "sourceMd5": self.source_md5,
            "sourceSha256": self.source_sha256,
            "parentItemKey": self.parent_item_key,
            "route": self.route,
            "pageCount": self.page_count,
            "selectedPages": list(self.selected_pages),
            "artifacts": [artifact.as_json() for artifact in self.artifacts],
            "markdownAttachmentKey": self.markdown_attachment_key,
        }

    def to_json(self) -> str:
        return json.dumps(self.as_json(), ensure_ascii=False, separators=(",", ":"))

    @classmethod
    def from_json(cls, raw: str, *, artifact_root: Path) -> "ConversionEvidence":
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise ValueError("conversion evidence must be an object")
        artifacts = value.get("artifacts")
        selected_pages = value.get("selectedPages")
        if not isinstance(artifacts, list) or not isinstance(selected_pages, list):
            raise ValueError("conversion evidence arrays are missing")
        return cls(
            schema_version=str(value.get("schemaVersion") or ""),
            extraction_contract_version=str(
                value.get("extractionContractVersion") or ""
            ),
            source_pdf_key=str(value.get("sourcePdfKey") or ""),
            source_md5=str(value.get("sourceMd5") or ""),
            source_sha256=str(value.get("sourceSha256") or ""),
            parent_item_key=str(value.get("parentItemKey") or ""),
            route=str(value.get("route") or ""),
            page_count=value.get("pageCount"),  # type: ignore[arg-type]
            selected_pages=tuple(selected_pages),  # type: ignore[arg-type]
            artifacts=tuple(
                EvidenceArtifact.from_json(artifact, artifact_root=artifact_root)
                for artifact in artifacts
                if isinstance(artifact, dict)
            ),
            markdown_attachment_key=(
                str(value["markdownAttachmentKey"])
                if value.get("markdownAttachmentKey")
                else None
            ),
        )


@dataclass(frozen=True)
class ReconciliationOutcome:
    accepted: bool
    status: str
    route: str | None = None
    selected_pages: tuple[int, ...] = ()
    evidence: ConversionEvidence | None = None
    error_code: str | None = None
    guidance: str | None = None


def digest_path(path: Path) -> str:
    if path.is_symlink():
        raise ValueError("unsafe artifact symlink")
    if path.is_file():
        return hashlib.sha256(path.read_bytes()).hexdigest()
    if not path.is_dir():
        raise FileNotFoundError(path)
    digest = hashlib.sha256()
    for child in sorted(path.rglob("*"), key=lambda item: item.relative_to(path).as_posix()):
        relative = child.relative_to(path).as_posix()
        if child.is_symlink():
            raise ValueError("unsafe artifact symlink")
        if child.is_dir():
            digest.update(f"directory\0{relative}\0".encode())
        elif child.is_file():
            digest.update(f"file\0{relative}\0".encode())
            digest.update(hashlib.sha256(child.read_bytes()).digest())
    return digest.hexdigest()


def coverage_status(selected_pages: Iterable[object], page_count: object, *, uploaded: bool) -> str:
    pages = tuple(selected_pages)
    if type(page_count) is not int or page_count < 1:
        raise ValueError("invalid_coverage")
    if not pages or any(type(page) is not int for page in pages):
        raise ValueError("invalid_coverage")
    if any(page < 1 or page > page_count for page in pages):
        raise ValueError("invalid_coverage")
    if any(current >= following for current, following in zip(pages, pages[1:])):
        raise ValueError("invalid_coverage")
    complete = pages == tuple(range(1, page_count + 1))
    if uploaded:
        return "completed" if complete else "uploaded_partial"
    return "local_complete" if complete else "local_partial"


def build_conversion_evidence(
    *,
    extraction_contract_version: str,
    source_pdf_key: str,
    source_md5: str,
    source_path: Path,
    parent_item_key: str | None,
    route: str,
    page_count: int,
    selected_pages: Iterable[object],
    artifacts: Iterable[tuple[str, Path]],
    artifact_root: Path,
) -> ConversionEvidence:
    pages = tuple(selected_pages)
    coverage_status(pages, page_count, uploaded=False)
    if not source_path.is_file():
        raise ValueError("missing_source")
    observed_md5 = hashlib.md5(source_path.read_bytes(), usedforsecurity=False).hexdigest()
    if source_md5 != observed_md5:
        raise ValueError("source_identity_mismatch")
    if route not in SUPPORTED_ROUTES:
        raise ValueError("unsupported_route")
    root = artifact_root.resolve()
    references: list[EvidenceArtifact] = []
    for kind, path in artifacts:
        if path.is_symlink():
            raise ValueError("unsafe_artifact_reference")
        resolved = path.resolve()
        try:
            reference = resolved.relative_to(root).as_posix()
        except ValueError as exc:
            raise ValueError("unsafe_artifact_reference") from exc
        resolve_artifact_reference(artifact_root, reference)
        references.append(
            EvidenceArtifact(
                kind=kind,
                path=resolved,
                sha256=digest_path(resolved),
                reference=reference,
            )
        )
    normalized_references = tuple(references)
    kinds = [reference.kind for reference in normalized_references]
    required = {"markdown", "route-sidecar", "publication-evidence"}
    if not required.issubset(kinds) or len(kinds) != len(set(kinds)):
        raise ValueError("missing_artifact")
    publication_path = next(
        reference.path
        for reference in normalized_references
        if reference.kind == "publication-evidence"
    )
    try:
        publication_evidence = json.loads(publication_path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ValueError("invalid_publication_evidence") from exc
    if (
        not isinstance(publication_evidence, dict)
        or publication_evidence.get("schema") != "publication-extraction-evidence-v2"
    ):
        raise ValueError("invalid_publication_evidence")
    return ConversionEvidence(
        schema_version=EVIDENCE_SCHEMA,
        extraction_contract_version=extraction_contract_version,
        source_pdf_key=source_pdf_key,
        source_md5=source_md5,
        source_sha256=digest_path(source_path),
        parent_item_key=parent_item_key or "",
        route=route,
        page_count=page_count,
        selected_pages=pages,  # type: ignore[arg-type]
        artifacts=normalized_references,
    )


def blocked(code: str, guidance: str) -> ReconciliationOutcome:
    return ReconciliationOutcome(
        accepted=False,
        status="blocked",
        error_code=code,
        guidance=guidance,
    )


def reconcile_conversion_evidence(
    *,
    raw_evidence: str | None,
    expected_contract_version: str,
    source_pdf_key: str,
    source_md5: str,
    source_path: Path,
    parent_item_key: str | None,
    page_count: int,
    artifact_root: Path,
) -> ReconciliationOutcome:
    if raw_evidence is None:
        return blocked("missing_evidence", "Rerun conversion to create current evidence.")
    try:
        evidence = ConversionEvidence.from_json(
            raw_evidence,
            artifact_root=artifact_root.resolve(),
        )
    except (KeyError, TypeError, ValueError, json.JSONDecodeError):
        return blocked("unsupported_evidence", "Rerun conversion to replace legacy evidence.")
    if (
        evidence.schema_version != EVIDENCE_SCHEMA
        or evidence.extraction_contract_version != expected_contract_version
    ):
        return blocked("unsupported_contract", "Rerun conversion with the current worker.")
    if evidence.route not in SUPPORTED_ROUTES:
        return blocked("unsupported_route", "Rerun conversion through a supported route.")
    if (
        evidence.source_pdf_key != source_pdf_key
        or evidence.parent_item_key != (parent_item_key or "")
    ):
        return blocked("source_identity_mismatch", "Rerun conversion for this Source PDF.")
    try:
        source_sha256 = digest_path(source_path) if source_path.is_file() else None
    except (OSError, ValueError):
        source_sha256 = None
    if evidence.source_md5 != source_md5 or source_sha256 != evidence.source_sha256:
        return blocked("source_drift", "Rerun conversion for the current Source PDF bytes.")
    if evidence.page_count != page_count:
        return blocked("page_count_drift", "Rerun conversion for the current Source PDF pages.")
    try:
        status = coverage_status(evidence.selected_pages, page_count, uploaded=False)
    except ValueError:
        return blocked("invalid_coverage", "Rerun conversion to rebuild page evidence.")
    kinds = [artifact.kind for artifact in evidence.artifacts]
    if (
        not {"markdown", "route-sidecar", "publication-evidence"}.issubset(kinds)
        or len(kinds) != len(set(kinds))
    ):
        return blocked("missing_artifact_reference", "Rerun conversion to rebuild evidence.")
    for artifact in evidence.artifacts:
        try:
            missing = not artifact.path.exists() or (
                artifact.kind in {"markdown", "route-sidecar", "publication-evidence"}
                and not artifact.path.is_file()
            )
            observed_sha256 = None if missing else digest_path(artifact.path)
        except (OSError, ValueError):
            return blocked("artifact_drift", "Rerun conversion for the current artifact bytes.")
        if missing:
            return blocked("missing_artifact", "Rerun conversion to restore referenced artifacts.")
        if observed_sha256 != artifact.sha256:
            return blocked("artifact_drift", "Rerun conversion for the current artifact bytes.")
    return ReconciliationOutcome(
        accepted=True,
        status=status,
        route=evidence.route,
        selected_pages=evidence.selected_pages,
        evidence=evidence,
    )
