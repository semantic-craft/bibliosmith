"""Extractor-independent publication-structure evidence.

Every extractor writes the same sidecar beside its assembled Markdown.  The
sidecar records facts the extractor actually observed (headings, source pages,
part files, repeated furniture removal and layout regions); the launcher owns
the later policy decision that turns those facts into a Publication Map.
"""

from __future__ import annotations

import json
import hashlib
import re
import shutil
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Iterable, Mapping, Sequence


SCHEMA = "publication-extraction-evidence-v2"
PAGE_ANCHOR = re.compile(r"^\s*<!--\s*page:\s*(\d+)\s*-->\s*$")
HEADING = re.compile(r"^(#{1,6})\s+(.+?)\s*#*\s*$")
TOC_ENTRY = re.compile(r"^\s*(?:[-*+]\s+|\d+[.)]\s+)?(.+?)\s+(\d{1,4})\s*$")
IMAGE = re.compile(r"!\[[^\]]*\]\([^)]+\)")
FORMULA = re.compile(r"(?:\$\$|\\\[|\\begin\{(?:equation|align))")
FOOTNOTE_DEFINITION = re.compile(r"^\s*\[\^([^\]]+)\]:")
FOOTNOTE_REFERENCE = re.compile(r"\[\^([^\]]+)\](?!:)")
FENCE = re.compile(r"^\s*(`{3,}|~{3,})")
MISCLASSIFIED_NOTE_HEADING = re.compile(
    r"^\s*#{1,6}\s+(\[\^[^\]]+\]:.*)$"
)


@dataclass(frozen=True, slots=True)
class SourceDocument:
    path: str
    start_line: int
    end_line: int
    pages: tuple[int, ...] = ()
    kind: str = "markdown"
    sha256: str = ""
    anomalies: tuple[str, ...] = ()
    source_href: str = ""

    def as_json(self) -> dict[str, object]:
        return {
            "path": self.path,
            "startLine": self.start_line,
            "endLine": self.end_line,
            "pages": list(self.pages),
            "kind": self.kind,
            "sha256": self.sha256,
            "anomalies": list(self.anomalies),
            "sourceHref": self.source_href,
        }


def normalize_extracted_markdown_notes(markdown: str) -> str:
    """Undo a common PDF layout error without inventing note relationships.

    Some layout extractors classify a short footnote-definition row as a
    heading because it uses smaller, visually distinct type.  The note marker
    and definition delimiter are still explicit in the extracted text, so
    removing only that spurious heading prefix restores canonical Markdown.
    Ambiguous superscript numbers or plain footer text are deliberately left
    untouched and must be corrected by the structure gate instead of guessed.
    """

    trailing_newline = markdown.endswith("\n")
    normalized_lines: list[str] = []
    fence_marker = ""
    in_comment = False
    inline_width: int | None = None
    for line in markdown.splitlines():
        fence = FENCE.match(line)
        if fence_marker:
            normalized_lines.append(line)
            if (
                fence
                and fence.group(1)[0] == fence_marker[0]
                and len(fence.group(1)) >= len(fence_marker)
            ):
                fence_marker = ""
            continue
        if fence:
            fence_marker = fence.group(1)
            normalized_lines.append(line)
            continue
        visible, next_in_comment = _strip_html_comments(line, in_comment)
        semantic, inline_width = _strip_inline_code(visible, inline_width)
        match = (
            MISCLASSIFIED_NOTE_HEADING.match(line)
            if not in_comment and semantic == line
            else None
        )
        normalized_lines.append(match.group(1) if match else line)
        in_comment = next_in_comment
    normalized = "\n".join(normalized_lines)
    return normalized + ("\n" if trailing_newline else "")


def persist_source_document(
    markdown_path: Path,
    source_path: Path,
    relative_path: str,
    *,
    start_line: int,
    end_line: int,
    pages: Sequence[int] = (),
    kind: str = "markdown",
    resource_directory: Path | None = None,
    anomalies: Sequence[str] = (),
) -> SourceDocument:
    """Copy extractor evidence into the Markdown sidecar and bind its digest.

    Scratch directories are disposable.  A source document named in extraction
    evidence therefore has to live below the one sidecar directory the launcher
    carries into the local project.  The returned path is relative to the
    Markdown file, which keeps it valid both before and after handoff.
    """

    relative = PurePosixPath(relative_path)
    if relative.is_absolute() or not relative.parts or any(
        part in {"", ".", ".."} for part in relative.parts
    ):
        raise ValueError(f"Unsafe source-document path: {relative_path}")
    if not source_path.is_file():
        raise FileNotFoundError(source_path)
    resources = resource_directory or markdown_path.with_name(
        f"{markdown_path.stem}_assets"
    )
    destination = resources.joinpath("source_documents", *relative.parts)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source_path.resolve() != destination.resolve():
        shutil.copyfile(source_path, destination)
    digest = hashlib.sha256(destination.read_bytes()).hexdigest()
    path = destination.relative_to(markdown_path.parent).as_posix()
    return SourceDocument(
        path=path,
        start_line=start_line,
        end_line=end_line,
        pages=tuple(pages),
        kind=kind,
        sha256=digest,
        anomalies=tuple(anomalies),
    )


def source_documents_for_page_groups(
    markdown: str,
    groups: Sequence[tuple[str, Sequence[int], str, str]],
) -> list[SourceDocument]:
    """Map extractor part files to assembled Markdown line ranges by page anchors."""

    lines = markdown.splitlines()
    pages_by_line = _page_ranges(lines)
    documents: list[SourceDocument] = []
    for path, pages, kind, sha256 in groups:
        wanted = set(pages)
        matching = [
            index + 1 for index, page in enumerate(pages_by_line) if page in wanted
        ]
        documents.append(
            SourceDocument(
                path=path,
                start_line=min(matching) if matching else 0,
                end_line=max(matching) if matching else 0,
                pages=tuple(sorted(wanted)),
                kind=kind,
                sha256=sha256,
                anomalies=(
                    ()
                    if matching
                    else ("source document produced no assembled Markdown lines",)
                ),
            )
        )
    return documents


def _page_ranges(lines: Sequence[str]) -> list[int | None]:
    active: int | None = None
    result: list[int | None] = []
    for line in lines:
        match = PAGE_ANCHOR.match(line)
        if match:
            active = int(match.group(1))
        result.append(active)
    return result


def _documents_for_range(
    documents: Sequence[SourceDocument], start_line: int, end_line: int
) -> list[str]:
    return [
        document.path
        for document in documents
        if document.start_line <= end_line and start_line <= document.end_line
    ]


def _feature_evidence(lines: Sequence[str], start: int, end: int) -> list[str]:
    selected = lines[start - 1 : end]
    evidence: list[str] = []
    if any(IMAGE.search(line) for line in selected):
        evidence.append("figure region in extractor Markdown")
    if any(line.lstrip().startswith("|") for line in selected):
        evidence.append("table region in extractor Markdown")
    if any(FORMULA.search(line) for line in selected):
        evidence.append("formula region in extractor Markdown")
    if any(FOOTNOTE_DEFINITION.match(line) for line in selected):
        evidence.append("note region in extractor Markdown")
    return evidence


def _strip_html_comments(line: str, in_comment: bool) -> tuple[str, bool]:
    visible: list[str] = []
    remaining = line
    while remaining:
        if in_comment:
            end = remaining.find("-->")
            if end < 0:
                return "".join(visible), True
            remaining = remaining[end + 3 :]
            in_comment = False
            continue
        start = remaining.find("<!--")
        if start < 0:
            visible.append(remaining)
            break
        visible.append(remaining[:start])
        remaining = remaining[start + 4 :]
        in_comment = True
    return "".join(visible), in_comment


def _strip_inline_code(line: str, active_width: int | None) -> tuple[str, int | None]:
    visible: list[str] = []
    index = 0
    while index < len(line):
        if active_width is not None:
            closing = "`" * active_width
            end = line.find(closing, index)
            if end < 0:
                return "".join(visible), active_width
            index = end + active_width
            active_width = None
            continue
        if line[index] != "`":
            visible.append(line[index])
            index += 1
            continue
        width = len(line[index:]) - len(line[index:].lstrip("`"))
        closing = "`" * width
        end = line.find(closing, index + width)
        if end < 0:
            return "".join(visible), width
        index = end + width
    return "".join(visible), active_width


def _semantic_markdown_lines(markdown: str) -> list[str]:
    visible: list[str] = []
    in_comment = False
    fence_marker = ""
    inline_width: int | None = None
    for line in markdown.splitlines():
        fence = FENCE.match(line)
        if fence_marker:
            if fence and fence.group(1)[0] == fence_marker[0] and len(fence.group(1)) >= len(fence_marker):
                fence_marker = ""
            visible.append("")
            continue
        without_comments, in_comment = _strip_html_comments(line, in_comment)
        fence = FENCE.match(without_comments)
        if fence:
            fence_marker = fence.group(1)
            visible.append("")
            continue
        without_code, inline_width = _strip_inline_code(without_comments, inline_width)
        visible.append(without_code)
    return visible


def _range_pages(
    pages_by_line: Sequence[int | None],
    documents: Sequence[SourceDocument],
    start_line: int,
    end_line: int,
) -> list[int]:
    pages = {
        page
        for page in pages_by_line[start_line - 1 : end_line]
        if page is not None
    }
    if not pages:
        pages.update(
            page
            for document in documents
            if document.start_line <= end_line and start_line <= document.end_line
            for page in document.pages
        )
    return sorted(pages)


def build_note_evidence(
    markdown: str,
    pages_by_line: Sequence[int | None],
    documents: Sequence[SourceDocument],
    sections: Sequence[Mapping[str, object]],
) -> list[dict[str, object]]:
    lines = _semantic_markdown_lines(markdown)
    definitions: dict[str, tuple[int, int]] = {}
    references: dict[str, list[int]] = {}
    labels: list[str] = []
    for index, line in enumerate(lines):
        definition = FOOTNOTE_DEFINITION.match(line)
        if definition:
            label = definition.group(1)
            end_line = index + 1
            for continuation_index, continuation in enumerate(
                lines[index + 1 :], start=index + 1
            ):
                indented = continuation.startswith(("    ", "\t"))
                blank_before_indented = (
                    not continuation.strip()
                    and continuation_index + 1 < len(lines)
                    and lines[continuation_index + 1].startswith(("    ", "\t"))
                )
                if not (indented or blank_before_indented):
                    break
                end_line = continuation_index + 1
            definitions[label] = (index + 1, end_line)
        for reference in FOOTNOTE_REFERENCE.finditer(line):
            label = reference.group(1)
            if label not in references:
                labels.append(label)
            references.setdefault(label, []).append(index + 1)
    labels.extend(label for label in definitions if label not in references)

    notes: list[dict[str, object]] = []
    for ordinal, label in enumerate(labels, start=1):
        definition = definitions.get(label)
        reference_lines = references.get(label, [])
        anchor_line = definition[0] if definition else reference_lines[0]
        end_line = definition[1] if definition else anchor_line
        containing_sections = [
            section
            for section in sections
            if int(section.get("sourceStartLine", 0)) <= anchor_line
            and end_line <= int(section.get("sourceEndLine", 0))
        ]
        section = max(
            containing_sections,
            key=lambda item: (
                int(item.get("headingLevel", 0)),
                int(item.get("sourceStartLine", 0)),
            ),
            default=sections[0] if sections else {},
        )
        section_title = str(section.get("title", "")).casefold()
        kind = (
            "editorial"
            if label.casefold().startswith("editor")
            else "endnote"
            if label.casefold().startswith("end")
            or any(token in section_title for token in ("notes", "anmerkungen", "注释"))
            else "footnote"
        )
        note_id = f"note_{ordinal:03d}"
        anomalies: list[str] = []
        if definition is None:
            anomalies.append("note definition is missing")
        if not reference_lines:
            anomalies.append("note has no references")
        source_files = set(_documents_for_range(documents, anchor_line, end_line))
        for reference_line in reference_lines:
            source_files.update(
                _documents_for_range(documents, reference_line, reference_line)
            )
        notes.append(
            {
                "id": note_id,
                "sourceLabel": label,
                "kind": kind,
                "publicationSectionId": str(section.get("id", "")),
                "sourceStartLine": anchor_line,
                "sourceEndLine": end_line,
                "sourcePages": _range_pages(
                    pages_by_line, documents, anchor_line, end_line
                ),
                "sourceFiles": sorted(source_files),
                "referenceSourceLines": reference_lines,
                "referenceIds": [
                    f"noteref_{note_id}_{reference:03d}"
                    for reference in range(1, len(reference_lines) + 1)
                ],
                "sourceAnchor": f"markdown-footnote-{note_id}",
                "evidence": ["canonical Markdown note definition and reference"],
                "anomalies": anomalies,
            }
        )
    return notes


def _printed_toc_entries(lines: Sequence[str]) -> dict[str, int]:
    entries: dict[str, int] = {}
    in_contents = False
    contents_level = 0
    for line in lines:
        heading = HEADING.match(line)
        if heading:
            level = len(heading.group(1))
            title = heading.group(2).strip().casefold()
            if any(token in title for token in ("contents", "inhaltsverzeichnis", "目录")):
                in_contents = True
                contents_level = level
                continue
            if in_contents and level <= contents_level:
                in_contents = False
        if not in_contents:
            continue
        match = TOC_ENTRY.match(line)
        if match:
            entries[" ".join(match.group(1).split()).casefold()] = int(match.group(2))
    return entries


def build_markdown_evidence(
    markdown: str,
    *,
    source_format: str,
    extraction_engine: str,
    source_documents: Sequence[SourceDocument] = (),
    title: str = "",
    creator: str = "",
    publisher: str = "",
    publication_date: str = "",
    removed_furniture: Iterable[str] = (),
    extraction_facts: Mapping[str, object] | None = None,
) -> dict[str, object]:
    """Build evidence without inventing structure absent from the extractor."""

    lines = markdown.splitlines()
    pages_by_line = _page_ranges(lines)
    headings = [
        (index + 1, len(match.group(1)), match.group(2).strip())
        for index, line in enumerate(lines)
        if (match := HEADING.match(line))
    ]
    synthesized_body = not headings and any(line.strip() for line in lines)
    if synthesized_body:
        headings = [(1, 1, title.strip() or "Body")]
    toc_entries = _printed_toc_entries(lines)
    documents = list(source_documents) or [
        SourceDocument(
            path="assembled.md",
            start_line=1,
            end_line=max(1, len(lines)),
            pages=tuple(sorted({page for page in pages_by_line if page is not None})),
        )
    ]
    stack: list[tuple[int, str]] = []
    sections: list[dict[str, object]] = []
    for position, (start_line, level, section_title) in enumerate(headings):
        while stack and stack[-1][0] >= level:
            stack.pop()
        section_id = f"extracted_section_{position + 1:03d}"
        end_line = next(
            (
                candidate_line - 1
                for candidate_line, candidate_level, _ in headings[position + 1 :]
                if candidate_level <= level
            ),
            max(start_line, len(lines)),
        )
        source_pages = _range_pages(
            pages_by_line, documents, start_line, end_line
        )
        evidence = (
            ["no extractor heading; provisional whole-document body"]
            if synthesized_body
            else [f"normalized extractor heading at line {start_line}"]
        )
        if source_pages:
            evidence.append(
                "source page position " + ",".join(str(page) for page in source_pages)
            )
        normalized_title = " ".join(section_title.split()).casefold()
        if normalized_title in toc_entries:
            evidence.append(
                f"printed contents title/page match {toc_entries[normalized_title]}"
            )
        evidence.extend(_feature_evidence(lines, start_line, end_line))
        source_files = _documents_for_range(documents, start_line, end_line)
        if len(source_files) > 1:
            evidence.append("publication section spans multiple extractor documents")
        sections.append(
            {
                "id": section_id,
                "title": section_title,
                "parentId": stack[-1][1] if stack else None,
                "headingLevel": level,
                "sourceHref": (
                    f"{source_format}://page/{source_pages[0]}"
                    if source_pages
                    else f"{source_format}://line/{start_line}"
                ),
                "sourceStartLine": start_line,
                "sourceEndLine": end_line,
                "sourcePages": source_pages,
                "sourceFiles": source_files,
                "evidence": evidence,
                "confidence": (
                    0.4 if synthesized_body else (0.9 if len(evidence) > 1 else 0.8)
                ),
                "anomalies": (
                    ["extractor produced no publication headings"]
                    if synthesized_body
                    else []
                ),
            }
        )
        stack.append((level, section_id))

    notes = build_note_evidence(markdown, pages_by_line, documents, sections)
    return {
        "schema": SCHEMA,
        "sourceFormat": source_format,
        "extractionEngine": extraction_engine,
        "title": title,
        "creator": creator,
        "publisher": publisher,
        "date": publication_date,
        "sourceDocuments": [document.as_json() for document in documents],
        "removedFurniture": [value for value in removed_furniture if value],
        "facts": dict(extraction_facts or {}),
        "sections": sections,
        "notes": notes,
    }


def write_markdown_evidence(
    markdown_path: Path,
    *,
    source_format: str,
    extraction_engine: str,
    source_documents: Sequence[SourceDocument] = (),
    title: str = "",
    creator: str = "",
    publisher: str = "",
    publication_date: str = "",
    removed_furniture: Iterable[str] = (),
    extraction_facts: Mapping[str, object] | None = None,
) -> Path:
    if not source_documents:
        raise ValueError("Persisted source documents are required for extraction evidence")
    for document in source_documents:
        explicitly_unmapped = (
            document.start_line == 0
            and document.end_line == 0
            and bool(document.anomalies)
        )
        if (
            len(document.sha256) != 64
            or any(character not in "0123456789abcdefABCDEF" for character in document.sha256)
            or (
                not explicitly_unmapped
                and (
                    document.start_line <= 0
                    or document.end_line < document.start_line
                )
            )
        ):
            raise ValueError(f"Invalid persisted source document: {document.path}")
    evidence = build_markdown_evidence(
        markdown_path.read_text(encoding="utf-8"),
        source_format=source_format,
        extraction_engine=extraction_engine,
        source_documents=source_documents,
        title=title,
        creator=creator,
        publisher=publisher,
        publication_date=publication_date,
        removed_furniture=removed_furniture,
        extraction_facts=extraction_facts,
    )
    path = markdown_path.with_suffix(".publication.json")
    path.write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return path
