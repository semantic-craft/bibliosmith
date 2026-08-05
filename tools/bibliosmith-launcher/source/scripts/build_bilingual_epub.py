#!/usr/bin/env python3
"""Build a source-then-target bilingual EPUB from a local reading project.

This is the builder the launcher runner uses: ``prepare_bilingual_builder`` in
``src-tauri/src/book_pipeline.rs`` copies this file into the project's
``scripts/`` directory and runs it with ``--book-root``. It pairs paragraphs
positionally from ``metadata/source_map.json`` and ``chapters/final/``, falls
back to whole-chapter blocks when the counts differ, and writes
``output/reading/book_bilingual.epub`` — the path the runner then registers.

The launcher owns this builder alongside the single-language builder; see
``docs/bilingual-epub-builder.md`` for its input and output contract.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import re
import shutil
import subprocess
import uuid
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


HEADING = re.compile(r"^(#{1,6})\s+(.+?)\s*$", re.DOTALL)
# A fenced code block opener: up to three spaces of indent, three or more
# backticks or tildes, then an optional info string. A backtick fence's info
# string may not contain a backtick, which keeps an inline ``a `b` c`` span from
# being read as a fence.
FENCE_OPEN = re.compile(r"^([ \t]{0,3})(`{3,}|~{3,})[ \t]*(.*)$")
FENCE_CLOSE = re.compile(r"^[ \t]{0,3}(`{3,}|~{3,})[ \t]*$")
_SEMANTIC_TOKEN_NONCE = uuid.uuid4().hex


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8").replace("\r\n", "\n").replace("\r", "\n")


def fence_opener(line: str) -> tuple[int, str] | None:
    match = FENCE_OPEN.match(line)
    if not match:
        return None
    marker = match.group(2)
    if marker.startswith("`") and "`" in match.group(3):
        return None
    return len(match.group(1)), marker


def is_fence_closer(line: str, marker: str) -> bool:
    match = FENCE_CLOSE.match(line)
    return bool(match) and match.group(1)[0] == marker[0] and len(match.group(1)) >= len(marker)


def is_comment_only(block: str) -> bool:
    """Whether a block is nothing but HTML comments.

    The PaddleOCR assembler writes a ``<!-- page: N -->`` anchor between pages
    so a reviewer can map a translated passage back to a page of the original,
    and picked a comment precisely so the marker would stay out of the prose.
    Nothing here reads it, and `inline_text` escapes every block it is handed,
    so an anchor left in place reaches the reader as the literal text
    ``<!-- page: N -->``.

    Only a block that is *entirely* comments goes; a comment sitting in a real
    paragraph is that paragraph's content and stays with it.
    """
    cursor = 0
    found_comment = False
    while cursor < len(block):
        while cursor < len(block) and block[cursor].isspace():
            cursor += 1
        if cursor == len(block):
            break
        if not block.startswith("<!--", cursor):
            return False
        end = block.find("-->", cursor + 4)
        if end < 0 or "<!--" in block[cursor + 4 : end]:
            return False
        found_comment = True
        cursor = end + 3
    return found_comment


def fenced_line_flags(lines: list[str]) -> list[bool]:
    """Mark complete fence ranges so their comment samples stay literal."""
    flags = [False] * len(lines)
    index = 0
    while index < len(lines):
        fence = fence_opener(lines[index])
        if fence is None:
            index += 1
            continue
        _, marker = fence
        end = index + 1
        while end < len(lines) and not is_fence_closer(lines[end], marker):
            end += 1
        for line_number in range(index, min(end + 1, len(lines))):
            flags[line_number] = True
        index = end + 1
    return flags


def comment_closer_follows(
    lines: list[str], fenced_lines: list[bool], line_number: int, column: int
) -> bool:
    """Find a closer before another opener, ignoring fenced code samples."""
    for candidate_number in range(line_number, len(lines)):
        if fenced_lines[candidate_number]:
            continue
        candidate = lines[candidate_number]
        start = column if candidate_number == line_number else 0
        opener_at = candidate.find("<!--", start)
        closer_at = candidate.find("-->", start)
        if closer_at >= 0 and (opener_at < 0 or closer_at < opener_at):
            return True
        if opener_at >= 0:
            return False
    return False


def comment_is_open_after(
    line: str,
    initially_open: bool,
    *,
    line_is_fenced: bool,
    closer_follows: Callable[[int], bool],
) -> bool:
    """Track only complete HTML comments outside fenced code."""
    if line_is_fenced:
        return initially_open
    cursor = 0
    is_open = initially_open
    while cursor < len(line):
        if is_open:
            opener_at = line.find("<!--", cursor)
            closer_at = line.find("-->", cursor)
            if opener_at >= 0 and (closer_at < 0 or opener_at < closer_at):
                is_open = False
                cursor = opener_at
                continue
            if closer_at < 0:
                break
            is_open = False
            cursor = closer_at + 3
            continue
        opener_at = line.find("<!--", cursor)
        if opener_at < 0:
            break
        closer_at = line.find("-->", opener_at + 4)
        nested_opener_at = line.find("<!--", opener_at + 4)
        if closer_at >= 0 and (nested_opener_at < 0 or closer_at < nested_opener_at):
            is_open = True
            cursor = opener_at + 4
            continue
        if closer_follows(opener_at + 4):
            is_open = True
            break
        cursor = opener_at + 4
    return is_open


def split_paragraphs(text: str) -> list[str]:
    """Split a chapter into blocks on blank lines, keeping a fence whole.

    A fenced code block counts as one block however many blank lines it holds.
    That matters twice over: the block has to reach `render_block` intact to be
    rendered as code at all, and `render_chapter` pairs source and target
    *positionally*, so a fence split into pieces would inflate the block count.
    If the two sides then disagreed, the whole chapter would drop to
    chapter-level fallback and lose paragraph pairing everywhere, not just at
    the code.

    Comment-only blocks are dropped here, in the one splitter both sides of
    `render_chapter` go through, rather than in `render_block`: because the
    pairing is positional, a block dropped from the source alone would pair
    every later source paragraph with the wrong translation. Doing it here the
    two sides cannot disagree — and a page anchor the translator kept on one
    side but not the other now cancels out instead of costing the chapter its
    paragraph pairing.
    """
    lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    # Trim blank lines from both ends without touching the first content line's
    # own indentation: a chapter opening with a fence indented one to three
    # spaces would otherwise have that indent measured as zero, leaving it on
    # every line of the rendered code.
    while lines and not lines[0].strip():
        lines.pop(0)
    while lines and not lines[-1].strip():
        lines.pop()
    if not lines:
        return []

    fenced_lines = fenced_line_flags(lines)

    blocks: list[str] = []
    paragraph: list[str] = []
    comment_is_open = False

    def flush() -> None:
        block = "\n".join(paragraph).strip()
        paragraph.clear()
        # Fences are appended below without passing through here, so a code
        # sample that happens to be one comment is still a block of its own.
        if block and not is_comment_only(block):
            blocks.append(block)

    index = 0
    while index < len(lines):
        line = lines[index]
        fence = None if comment_is_open else fence_opener(line)
        if fence is not None:
            flush()
            _, marker = fence
            end = index + 1
            # An unclosed fence runs to the end of the chapter, as CommonMark says.
            while end < len(lines) and not is_fence_closer(lines[end], marker):
                end += 1
            blocks.append("\n".join(lines[index : min(end + 1, len(lines))]))
            index = end + 1
            continue
        if line.strip() or comment_is_open:
            paragraph.append(line)
        else:
            flush()
        comment_is_open = comment_is_open_after(
            line,
            comment_is_open,
            line_is_fenced=fenced_lines[index],
            closer_follows=lambda column: comment_closer_follows(
                lines, fenced_lines, index, column
            ),
        )
        index += 1
    flush()
    return blocks


def fenced_code(block: str) -> str | None:
    """The code inside a fenced block, or None if this block is not one.

    Only the delimiters and the opener's permitted indentation come off. Blank
    lines before the closing fence are part of the sample, so a listing that
    deliberately ends with vertical space keeps it.
    """
    lines = block.split("\n")
    fence = fence_opener(lines[0])
    if fence is None:
        return None
    indent, marker = fence
    body = lines[1:]
    if body and is_fence_closer(body[-1], marker):
        body = body[:-1]
    return "\n".join(strip_fence_indent(line, indent) for line in body)


def strip_fence_indent(line: str, indent: int) -> str:
    cut = 0
    while cut < indent and cut < len(line) and line[cut] in " \t":
        cut += 1
    return line[cut:]


def inline_text(block: str) -> str:
    escaped = html.escape(" ".join(line.strip() for line in block.splitlines()), quote=True)
    escaped = re.sub(
        r"\[([^\]]+)\]\((https?://[^)\s]+)\)",
        lambda match: f'<a href="{match.group(2)}">{match.group(1)}</a>',
        escaped,
    )
    escaped = re.sub(
        rf"@@BIBLIO_SOURCE_NOTEREF__{_SEMANTIC_TOKEN_NONCE}__([A-Za-z0-9_-]+)__([A-Za-z0-9_-]+)__(\d+)__([A-Za-z0-9_-]+)@@",
        lambda match: (
            f'<a class="bitext-source-noteref" id="{match.group(2)}-source" '
            f'href="{match.group(4)}.xhtml#{match.group(1)}-source">[{match.group(3)}]</a>'
        ),
        escaped,
    )
    return re.sub(
        rf"@@BIBLIO_NOTEREF__{_SEMANTIC_TOKEN_NONCE}__([A-Za-z0-9_-]+)__([A-Za-z0-9_-]+)__(\d+)__([A-Za-z0-9_-]+)@@",
        lambda match: (
            f'<a epub:type="noteref" id="{match.group(2)}" '
            f'href="{match.group(4)}.xhtml#{match.group(1)}">[{match.group(3)}]</a>'
        ),
        escaped,
    )


def markdown_table(block: str) -> list[list[str]] | None:
    lines = [line.strip() for line in block.splitlines()]
    if len(lines) < 2 or any(not line.startswith("|") or not line.endswith("|") for line in lines):
        return None
    rows = [[cell.strip() for cell in line[1:-1].split("|")] for line in lines]
    if not all(re.fullmatch(r":?-{3,}:?", cell) for cell in rows[1]):
        return None
    return [rows[0], *rows[2:]]


def table_html(rows: list[list[str]], css_class: str, language: str) -> str:
    header = rows[0]
    body = rows[1:]
    attributes = (
        f'class="{css_class}" lang="{html.escape(language, quote=True)}" '
        f'xml:lang="{html.escape(language, quote=True)}"'
    )
    head = "".join(f'<th scope="col">{inline_text(cell)}</th>' for cell in header)
    body_rows = []
    for row in body:
        padded = (row + [""] * len(header))[: len(header)]
        body_rows.append("<tr>" + "".join(f"<td>{inline_text(cell)}</td>" for cell in padded) + "</tr>")
    return (
        f'<div class="table-wrap" role="region" aria-label="结构化表格"><table {attributes}>'
        f'<caption>结构化表格</caption><thead><tr>{head}</tr></thead>'
        f'<tbody>{"".join(body_rows)}</tbody></table></div>'
    )


def render_block(block: str, css_class: str, language: str) -> str:
    source_note_prefix = f"@@BIBLIO_SOURCE_NOTE__{_SEMANTIC_TOKEN_NONCE}__"
    if block.startswith(source_note_prefix) and block.endswith("@@"):
        payload = json.loads(block[len(source_note_prefix) : -2])
        note_id = str(payload["noteId"])
        marker = int(payload["marker"])
        note_kind = str(payload["kind"])
        body = str(payload["body"])
        backlinks = " ".join(
            f'<a class="bitext-source-note-backlink" '
            f'href="{html.escape(str(reference["rootId"]), quote=True)}.xhtml#'
            f'{html.escape(str(reference["id"]), quote=True)}-source">↩'
            f'{index + 1 if len(payload["references"]) > 1 else ""}</a>'
            for index, reference in enumerate(payload["references"])
        )
        return (
            f'<div class="bitext-source-note note-{html.escape(note_kind, quote=True)} {css_class}" '
            f'data-presentation-for="{html.escape(note_id, quote=True)}" '
            f'id="{html.escape(note_id, quote=True)}-source" lang="{html.escape(language, quote=True)}" '
            f'xml:lang="{html.escape(language, quote=True)}"><p>'
            f'<span class="note-marker">[{marker}]</span> {inline_text(body)} {backlinks}'
            "</p></div>"
        )
    target_note_prefix = f"@@BIBLIO_TARGET_NOTE__{_SEMANTIC_TOKEN_NONCE}__"
    if block.startswith(target_note_prefix) and block.endswith("@@"):
        payload = json.loads(block[len(target_note_prefix) : -2])
        note_id = str(payload["noteId"])
        marker = int(payload["marker"])
        note_kind = str(payload["kind"])
        body = str(payload["body"])
        references = payload["references"]
        backlinks = " ".join(
            f'<a epub:type="backlink" href="{html.escape(str(reference["rootId"]), quote=True)}.xhtml#'
            f'{html.escape(str(reference["id"]), quote=True)}">↩'
            f'{index + 1 if len(references) > 1 else ""}</a>'
            for index, reference in enumerate(references)
        )
        epub_type = "endnote" if note_kind == "endnote" else "footnote"
        return (
            f'<aside epub:type="{epub_type}" class="publication-note '
            f'note-{html.escape(note_kind, quote=True)}" '
            f'data-note-kind="{html.escape(note_kind, quote=True)}" '
            f'id="{html.escape(note_id, quote=True)}"><p>'
            f'<span class="note-marker">[{marker}]</span> {inline_text(body)} {backlinks}'
            "</p></aside>"
        )
    attributes = (
        f'class="{css_class}" lang="{html.escape(language, quote=True)}" '
        f'xml:lang="{html.escape(language, quote=True)}"'
    )
    code = fenced_code(block)
    if code is not None:
        # Escaped only, never joined into a line: inside a code block the line
        # breaks and the backticks are the content.
        return f"<pre {attributes}><code>{html.escape(code)}</code></pre>"
    image = re.fullmatch(r"!\[([^\]]*)\]\((images/[^)]+)\)", block.strip())
    if image:
        return (
            f'<figure {attributes}><img src="{html.escape(image.group(2), quote=True)}" '
            f'alt="{html.escape(image.group(1), quote=True)}" />'
            f'<figcaption>{inline_text(image.group(1))}</figcaption></figure>'
        )
    table = markdown_table(block)
    if table is not None:
        return table_html(table, css_class, language)
    heading = HEADING.fullmatch(block)
    if heading:
        level = len(heading.group(1))
        return f"<h{level} {attributes}>{inline_text(heading.group(2))}</h{level}>"
    return f"<p {attributes}>{inline_text(block)}</p>"


def stage_markdown_assets(
    text: str,
    markdown_path: Path,
    project_root: Path,
    epub_root: Path,
    asset_media_types: dict[str, str],
) -> str:
    def replace(match: re.Match[str]) -> str:
        alt, raw_href = match.group(1), match.group(2).strip().strip("<>")
        if re.match(r"^[a-z]+:", raw_href, re.IGNORECASE):
            return match.group(0)
        source = (markdown_path.parent / raw_href).resolve()
        try:
            source.relative_to(project_root)
        except ValueError as error:
            raise ValueError(f"Bilingual image escaped the book project: {raw_href}") from error
        if not source.is_file():
            raise ValueError(f"Bilingual image is missing: {raw_href}")
        suffix = source.suffix.lower()
        media_type = {
            ".png": "image/png",
            ".jpg": "image/jpeg",
            ".jpeg": "image/jpeg",
            ".gif": "image/gif",
            ".svg": "image/svg+xml",
            ".webp": "image/webp",
        }.get(suffix)
        if media_type is None:
            raise ValueError(f"Unsupported bilingual image type: {suffix or 'none'}")
        digest = hashlib.sha256(source.read_bytes()).hexdigest()[:16]
        href = f"images/{digest}{suffix}"
        destination = epub_root / href
        destination.parent.mkdir(parents=True, exist_ok=True)
        if not destination.exists():
            shutil.copy2(source, destination)
        asset_media_types[href] = media_type
        return f"![{alt}]({href})"

    return re.sub(r"!\[([^\]]*)\]\(([^)]+)\)", replace, text)


def render_chapter(
    source_text: str,
    target_text: str,
    source_language: str,
    target_language: str,
) -> tuple[str, str, int, int]:
    source_paragraphs = split_paragraphs(source_text)
    target_paragraphs = split_paragraphs(target_text)
    if not source_paragraphs or not target_paragraphs:
        raise ValueError("Bilingual chapters must contain both source and target text.")

    if len(source_paragraphs) == len(target_paragraphs):
        units = []
        for source, target in zip(source_paragraphs, target_paragraphs, strict=True):
            units.append(
                "\n".join(
                    [
                        '<section class="bitext-unit">',
                        render_block(source, "bitext-source", source_language),
                        render_block(target, "bitext-target", target_language),
                        "</section>",
                    ]
                )
            )
        return "\n".join(units), "paragraph", len(source_paragraphs), len(target_paragraphs)

    source = "\n".join(
        render_block(block, "bitext-source-paragraph", source_language)
        for block in source_paragraphs
    )
    target = "\n".join(
        render_block(block, "bitext-target-paragraph", target_language)
        for block in target_paragraphs
    )
    body = "\n".join(
        [
            '<section class="bitext-unit bitext-fallback">',
            (
                f'<div class="bitext-source bitext-chapter-block" lang="{html.escape(source_language, quote=True)}" '
                f'xml:lang="{html.escape(source_language, quote=True)}">\n{source}\n</div>'
            ),
            (
                f'<div class="bitext-target bitext-chapter-block" lang="{html.escape(target_language, quote=True)}" '
                f'xml:lang="{html.escape(target_language, quote=True)}">\n{target}\n</div>'
            ),
            "</section>",
        ]
    )
    return body, "chapter-fallback", len(source_paragraphs), len(target_paragraphs)


def parse_simple_yaml(path: Path) -> dict[str, str]:
    if not path.is_file():
        return {}
    values: dict[str, str] = {}
    for line in read_text(path).splitlines():
        match = re.match(r"^([A-Za-z0-9_-]+):\s*(.*)$", line)
        if not match:
            continue
        values[match.group(1)] = match.group(2).strip().strip("\"'")
    return values


def normalized_language(value: Any, fallback: str) -> str:
    if not isinstance(value, str) or not value.strip():
        return fallback
    language = value.strip()
    if language.lower() in {"auto", "unknown"}:
        return fallback
    return language


def safe_project_path(project_root: Path, relative: str) -> Path:
    candidate = (project_root / relative).resolve()
    try:
        candidate.relative_to(project_root.resolve())
    except ValueError as error:
        raise ValueError(f"Source map path escapes the book project: {relative}") from error
    return candidate


def configured_cover(project_root: Path, metadata: dict[str, str]) -> tuple[Path, str] | None:
    configured = metadata.get("cover") or metadata.get("cover_path") or ""
    candidates = []
    if configured:
        candidates.append(Path(configured) if Path(configured).is_absolute() else project_root / configured)
    candidates.extend(project_root / "source" / f"cover{suffix}" for suffix in (".jpg", ".jpeg", ".png", ".webp", ".svg"))
    for candidate in candidates:
        resolved = candidate.resolve()
        try:
            resolved.relative_to(project_root)
        except ValueError as error:
            raise ValueError("Configured cover escapes the book project.") from error
        if not resolved.is_file():
            continue
        media_type = {
            ".jpg": "image/jpeg",
            ".jpeg": "image/jpeg",
            ".png": "image/png",
            ".webp": "image/webp",
            ".svg": "image/svg+xml",
        }.get(resolved.suffix.lower())
        if media_type is None:
            raise ValueError(f"Unsupported cover type: {resolved.suffix or 'none'}")
        return resolved, media_type
    return None


def chapter_title(target_text: str, source_text: str, chapter_id: str) -> str:
    for text in (target_text, source_text):
        for block in split_paragraphs(text):
            heading = HEADING.fullmatch(block)
            if heading:
                return " ".join(heading.group(2).splitlines()).strip()
    return chapter_id


def xhtml_document(title: str, body: str, target_language: str) -> str:
    escaped_language = html.escape(target_language, quote=True)
    return f'''<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{escaped_language}" xml:lang="{escaped_language}">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{html.escape(title)}</title>
  <link rel="stylesheet" type="text/css" href="styles/book.css" />
</head>
<body>
{body}
</body>
</html>
'''


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def markdown_headings(text: str) -> list[tuple[int, str]]:
    headings: list[tuple[int, str]] = []
    for block in split_paragraphs(text):
        match = HEADING.fullmatch(block)
        if match:
            headings.append((len(match.group(1)), " ".join(match.group(2).splitlines()).strip()))
    return headings


def prepare_semantic_notes(
    text: str, notes: list[dict[str, Any]], *, semantic: bool, current_root_id: str
) -> str:
    if not notes:
        return text
    scoped_notes = [
        note
        for note in notes
        if note.get("definitionRootId") == current_root_id
        or current_root_id in (note.get("referenceRootById") or {}).values()
    ]
    by_label = {}
    for index, note in enumerate(scoped_notes):
        reference_root_by_id = note.get("referenceRootById") or {}
        by_label[str(note["sourceLabel"])] = {
            **note,
            "marker": int(note.get("ordinal") or index + 1),
            "currentReferenceIds": [
                reference_id
                for reference_id in note.get("referenceIds", [])
                if reference_root_by_id.get(reference_id) == current_root_id
            ],
        }
    definitions: dict[str, str] = {}
    seen: dict[str, int] = {}
    kept: list[str] = []
    lines = text.splitlines()
    line_index = 0
    while line_index < len(lines):
        line = lines[line_index]
        definition = re.fullmatch(r"\[\^([^\]]+)\]:\s*(.*)", line.strip())
        if (
            definition
            and by_label.get(definition.group(1), {}).get("definitionRootId")
            == current_root_id
        ):
            body = [definition.group(2)]
            while line_index + 1 < len(lines):
                continuation = lines[line_index + 1]
                if continuation.startswith("    ") or continuation.startswith("\t"):
                    body.append(continuation[4:] if continuation.startswith("    ") else continuation[1:])
                    line_index += 1
                elif (
                    not continuation.strip()
                    and line_index + 2 < len(lines)
                    and (lines[line_index + 2].startswith("    ") or lines[line_index + 2].startswith("\t"))
                ):
                    body.append("")
                    line_index += 1
                else:
                    break
            definitions[definition.group(1)] = "\n".join(body)
            line_index += 1
            continue

        def replace_reference(match: re.Match[str]) -> str:
            label = match.group(1)
            note = by_label.get(label)
            if note is None:
                return match.group(0)
            occurrence = seen.get(label, 0)
            reference_ids = note.get("currentReferenceIds")
            if not isinstance(reference_ids, list) or occurrence >= len(reference_ids):
                raise ValueError(f"Note {note['id']} has more references than its contract.")
            seen[label] = occurrence + 1
            marker = int(note["marker"])
            if not semantic:
                return (
                    f"@@BIBLIO_SOURCE_NOTEREF__{_SEMANTIC_TOKEN_NONCE}__{note['id']}__"
                    f"{reference_ids[occurrence]}__{marker}__{note['definitionRootId']}@@"
                )
            return (
                f"@@BIBLIO_NOTEREF__{_SEMANTIC_TOKEN_NONCE}__{note['id']}__"
                f"{reference_ids[occurrence]}__{marker}__{note['definitionRootId']}@@"
            )

        kept.append(re.sub(r"\[\^([^\]]+)\]", replace_reference, line))
        line_index += 1

    for label, note in by_label.items():
        current_reference_ids = note.get("currentReferenceIds")
        if not isinstance(current_reference_ids, list) or seen.get(label, 0) != len(
            current_reference_ids
        ):
            raise ValueError(f"Semantic note reference count changed for {note['id']}")
        if semantic:
            if note.get("definitionRootId") != current_root_id:
                continue
            if label not in definitions:
                raise ValueError(f"Semantic note definition is missing: {note['id']}")
            reference_ids = note.get("referenceIds")
            reference_root_by_id = note.get("referenceRootById") or {}
            kept.append(
                f"@@BIBLIO_TARGET_NOTE__{_SEMANTIC_TOKEN_NONCE}__"
                + json.dumps(
                    {
                        "noteId": note["id"],
                        "kind": note.get("kind") or "footnote",
                        "marker": note["marker"],
                        "body": definitions[label],
                        "references": [
                            {"id": reference_id, "rootId": reference_root_by_id[reference_id]}
                            for reference_id in reference_ids
                        ],
                    },
                    ensure_ascii=False,
                    separators=(",", ":"),
                )
                + "@@"
            )
        elif note.get("definitionRootId") == current_root_id:
            if label not in definitions:
                raise ValueError(f"Source note definition is missing: {note['id']}")
            reference_ids = note.get("referenceIds")
            reference_root_by_id = note.get("referenceRootById") or {}
            kept.append(
                f"@@BIBLIO_SOURCE_NOTE__{_SEMANTIC_TOKEN_NONCE}__"
                + json.dumps(
                    {
                        "noteId": note["id"],
                        "kind": note.get("kind") or "footnote",
                        "marker": note["marker"],
                        "body": definitions[label],
                        "references": [
                            {"id": reference_id, "rootId": reference_root_by_id[reference_id]}
                            for reference_id in reference_ids
                        ],
                    },
                    ensure_ascii=False,
                    separators=(",", ":"),
                )
                + "@@"
            )
    return "\n".join(kept)


def anchor_target_headings(
    body: str, sections: list[dict[str, Any]], headings: list[tuple[int, str]]
) -> str:
    if len(sections) != len(headings):
        raise ValueError(
            f"Publication section/headings mismatch: sections={len(sections)}, "
            f"translatedHeadings={len(headings)}"
        )
    pattern = re.compile(
        r'<h([1-6]) class="(bitext-target(?:-paragraph)?)"([^>]*)>[\s\S]*?</h\1>'
    )
    index = 0

    def replace(match: re.Match[str]) -> str:
        nonlocal index
        if index >= len(sections):
            return match.group(0)
        section = sections[index]
        expected_level = int(section.get("headingLevel", 0))
        actual_level = int(match.group(1))
        translated_level = headings[index][0]
        if actual_level != expected_level or translated_level != expected_level:
            raise ValueError(
                f"Heading hierarchy changed for {section.get('id')}: "
                f"expected h{expected_level}, got h{actual_level}"
            )
        index += 1
        epub_type = {
            "title_page": "titlepage",
            "copyright": "copyright-page",
            "contents": "toc",
            "bibliography": "bibliography",
            "notes": "endnotes",
            "appendix": "appendix",
        }.get(str(section.get("kind") or ""))
        semantics = f' epub:type="{epub_type}"' if epub_type else ""
        return (
            f'<h{actual_level} id="{html.escape(str(section["id"]), quote=True)}" '
            f'class="{match.group(2)} publication-heading '
            f'publication-kind-{html.escape(str(section.get("kind") or "section"), quote=True)} '
            f'publication-role-{html.escape(str(section.get("role") or "bodymatter"), quote=True)}"'
            f'{semantics}{match.group(3)}>'
            f'{html.escape(str(section["title"]))}</h{actual_level}>'
        )

    anchored = pattern.sub(replace, body)
    if index != len(sections):
        raise ValueError("Not every publication section received a bilingual XHTML anchor.")
    return anchored


def nav_list(navigation: list[dict[str, Any]], parent_id: str | None) -> str:
    items: list[str] = []
    for entry in navigation:
        if entry.get("parentId") != parent_id:
            continue
        href = str(entry["href"])
        nested = nav_list(navigation, str(entry["id"]))
        nested_list = f"<ol>{nested}</ol>" if nested else ""
        items.append(
            f'<li><a href="{html.escape(href, quote=True)}">'
            f'{html.escape(str(entry["label"]))}</a>{nested_list}</li>'
        )
    return "\n".join(items)


def build_book(project_root: Path) -> Path:
    project_root = project_root.resolve()
    source_manifest_path = project_root / "metadata" / "source_manifest.json"
    compiler = Path(__file__).with_name("compile_publication_structure.cjs")
    compiled = subprocess.run(
        ["node", str(compiler), "--project-root", str(project_root)],
        capture_output=True,
        text=True,
        check=False,
    )
    if compiled.returncode != 0:
        raise ValueError(compiled.stderr.strip() or "Publication structure compiler failed.")
    structure = json.loads(compiled.stdout)
    source_manifest = json.loads(read_text(source_manifest_path))
    units = structure["translationUnits"]
    sections = structure["sections"]

    by_id = {str(section.get("id")): section for section in sections if isinstance(section, dict)}
    roots = structure["roots"]
    documents = structure["documents"]

    book_metadata = parse_simple_yaml(project_root / "metadata" / "book.yaml")
    source_language = normalized_language(source_manifest.get("source_language"), "und")
    target_language = normalized_language(
        book_metadata.get("language") or source_manifest.get("target_language"), "zh-Hans"
    )
    title = (
        book_metadata.get("title")
        or book_metadata.get("title_zh")
        or book_metadata.get("title_zh_hans")
        or str(roots[0]["title"])
    )
    creator = book_metadata.get("author") or book_metadata.get("creator") or ""
    contributor = book_metadata.get("contributor") or ""
    publisher = book_metadata.get("publisher") or ""
    publication_date = book_metadata.get("date") or ""
    source_metadata = (
        book_metadata.get("source_url")
        or book_metadata.get("source")
        or book_metadata.get("source_text_url")
        or ""
    )
    description = book_metadata.get("description") or book_metadata.get("subtitle") or ""
    rights = book_metadata.get("rights") or ""
    identifier = book_metadata.get("identifier") or f"urn:uuid:{uuid.uuid4()}"
    cover = configured_cover(project_root, book_metadata)

    final_dir = project_root / "chapters" / "final"
    final_files = {path.stem: path for path in sorted(final_dir.glob("*.md"))}
    if not final_files:
        raise ValueError("No promoted final chapters found under chapters/final.")

    work_root = project_root / "output" / "bilingual_epub_work"
    if work_root.exists():
        shutil.rmtree(work_root)
    epub_root = work_root / "EPUB"
    (work_root / "META-INF").mkdir(parents=True, exist_ok=True)
    (epub_root / "styles").mkdir(parents=True, exist_ok=True)

    write_text(work_root / "mimetype", "application/epub+zip")
    write_text(
        work_root / "META-INF" / "container.xml",
        '''<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml" />
  </rootfiles>
</container>
''',
    )
    write_text(
        epub_root / "styles" / "book.css",
        r'''html{margin:0;padding:0}
body{line-height:1.68;margin:0;padding:1em;overflow-wrap:anywhere;word-break:normal}
main{display:block;max-width:46em;margin:0 auto}
.publication-cover{max-width:none;padding:0;text-align:center}.publication-cover img{display:block;width:auto;max-width:100%;max-height:95vh;margin:0 auto;object-fit:contain}
.bitext-unit{margin:0 0 1.15em}
.bitext-source{font-size:.92em;line-height:1.5;color:#555;margin:0 0 .35em;text-indent:0}
.bitext-target{font-size:1em;line-height:1.72;color:inherit;margin:0;text-indent:2em}
.bitext-chapter-block{margin-bottom:.7em}
.bitext-source-paragraph{font-size:1em;line-height:1.5;margin:0 0 .7em;text-indent:0}
.bitext-target-paragraph{font-size:1em;line-height:1.72;margin:0 0 .7em;text-indent:2em}
h1,h2,h3,h4,h5,h6{line-height:1.3;text-indent:0;break-after:avoid-page;page-break-after:avoid}
h1{font-size:1.7em;margin:1.8em 0 1.2em;text-align:center}
h2{font-size:1.35em;margin:1.65em 0 .85em}
h3{font-size:1.16em;margin:1.4em 0 .65em}
h4{font-size:1.06em;margin:1.2em 0 .55em}
h5,h6{font-size:1em;margin:1.05em 0 .45em}
img{max-width:100%;height:auto}
figure{margin:1.2em 0;text-align:center;break-inside:avoid}
pre{margin:0 0 .35em;padding:.5em .6em;background:#f4f4f4;border:1px solid #e0e0e0;border-radius:3px;font-size:.82em;line-height:1.45;white-space:pre-wrap;overflow-wrap:anywhere;break-inside:avoid}
pre code{font-family:monospace;font-size:inherit}
.table-wrap{display:block;width:100%;max-width:100%;overflow-x:auto}
table{border-collapse:collapse;width:100%;max-width:100%;font-size:.8em;line-height:1.4}
th,td{border:1px solid currentColor;padding:.25em .35em;vertical-align:top;white-space:normal;overflow-wrap:anywhere;word-break:break-word}
aside,[epub\:type~="footnote"]{font-size:.88em;line-height:1.55;margin:.6em 0}
a[epub\:type~="noteref"]{font-size:.8em;vertical-align:super;text-decoration:none}
/* The bitext classes carry a first-line indent and a colour for prose; a class
   selector outranks the bare `pre` above, so the code cases are named here. */
pre.bitext-source,pre.bitext-target,pre.bitext-source-paragraph,pre.bitext-target-paragraph{text-indent:0;color:inherit}
@media print{body{padding:0}main{max-width:none}h1{break-before:page;page-break-before:always}.table-wrap{overflow:visible}}
@media (max-width:430px){body{padding:.7em}.bitext-source,.bitext-source-paragraph{font-size:.88em}h1{font-size:1.5em;margin-top:1.1em}}
''',
    )

    manifest_items = [
        '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav" />',
        '<item id="css" href="styles/book.css" media-type="text/css" />',
    ]
    asset_media_types: dict[str, str] = {}
    spine_items: list[str] = []
    title_by_section: dict[str, str] = {
        str(section["id"]): str(section["title"]) for section in sections
    }
    used_final_ids: set[str] = set()
    cover_landmark = ""
    cover_metadata = ""
    if cover is not None:
        cover_path, cover_media_type = cover
        cover_href = f"images/cover{cover_path.suffix.lower()}"
        (epub_root / "images").mkdir(parents=True, exist_ok=True)
        shutil.copy2(cover_path, epub_root / cover_href)
        write_text(
            epub_root / "cover.xhtml",
            xhtml_document(
                title,
                f'<main epub:type="cover" class="publication-cover"><img src="{html.escape(cover_href, quote=True)}" alt="{html.escape(title, quote=True)}" /></main>',
                target_language,
            ),
        )
        manifest_items.extend(
            [
                '<item id="cover-page" href="cover.xhtml" media-type="application/xhtml+xml" />',
                f'<item id="cover-image" href="{html.escape(cover_href, quote=True)}" media-type="{cover_media_type}" properties="cover-image" />',
            ]
        )
        spine_items.append('<itemref idref="cover-page" />')
        cover_landmark = '<li><a epub:type="cover" href="cover.xhtml">封面</a></li>'
        cover_metadata = '<meta name="cover" content="cover-image" />'
    for index, document in enumerate(documents, start=1):
        root_id = str(document["id"])
        root_section = by_id[root_id]
        subtree = [by_id[str(section_id)] for section_id in document["sectionIds"]]
        unit_ids = {str(unit_id) for unit_id in document["translationUnitIds"]}
        root_units = [unit for unit in units if str(unit.get("id")) in unit_ids]
        if not root_units:
            raise ValueError(f"Publication root has no translated content: {root_id}")
        source_texts: list[str] = []
        target_texts: list[str] = []
        for unit in root_units:
            unit_id = str(unit.get("id") or "").strip()
            source_relative = str(unit.get("sourceUnitPath") or "").strip()
            if not unit_id or not source_relative:
                raise ValueError("Translation unit is missing id or sourceUnitPath.")
            final_path = final_files.get(unit_id)
            if final_path is None:
                raise ValueError(f"Promoted final translation unit is missing: {unit_id}")
            source_path = safe_project_path(project_root, source_relative)
            if not source_path.is_file():
                raise ValueError(f"Source translation unit is missing: {source_relative}")
            source_texts.append(
                stage_markdown_assets(
                    read_text(source_path), source_path, project_root, epub_root, asset_media_types
                )
            )
            target_texts.append(
                stage_markdown_assets(
                    read_text(final_path), final_path, project_root, epub_root, asset_media_types
                )
            )
            used_final_ids.add(unit_id)
        source_text = prepare_semantic_notes(
            "\n".join(source_texts),
            structure["notes"],
            semantic=False,
            current_root_id=root_id,
        )
        target_text = prepare_semantic_notes(
            "\n".join(target_texts),
            structure["notes"],
            semantic=True,
            current_root_id=root_id,
        )
        body, alignment, source_count, target_count = render_chapter(
            source_text, target_text, source_language, target_language
        )
        headings = markdown_headings(target_text)
        if len(headings) != len(subtree):
            raise ValueError(
                f"Publication section/headings mismatch: sections={len(subtree)}, "
                f"translatedHeadings={len(headings)}"
            )
        for heading_index, (_, heading_title) in enumerate(headings):
            if re.fullmatch(
                r"(?:(?:chapter|unit|section)[-_ ]*\d+|continuation(?:\s+\d+)?)",
                heading_title.strip(),
                re.IGNORECASE,
            ):
                raise ValueError(
                    f"Translated publication title exposes an internal unit: {heading_title}"
                )
        body = anchor_target_headings(body, subtree, headings)
        title_text = title_by_section[root_id]
        href = str(document["href"])
        item_id = f"chapter-{index:03}"
        body_type = str(root_section.get("role") or "bodymatter")
        wrapped = (
            f'<main epub:type="{html.escape(body_type, quote=True)}" '
            f'class="publication-{html.escape(body_type, quote=True)} '
            f'publication-kind-{html.escape(str(root_section.get("kind") or "section"), quote=True)}">\n'
            f'{body}\n</main>'
        )
        write_text(epub_root / href, xhtml_document(title_text, wrapped, target_language))
        manifest_items.append(
            f'<item id="{item_id}" href="{href}" media-type="application/xhtml+xml" />'
        )
        spine_items.append(f'<itemref idref="{item_id}" />')
        print(
            f"{root_id}: alignment={alignment} "
            f"source_paragraphs={source_count} target_paragraphs={target_count}"
        )

    unmatched = sorted(set(final_files) - used_final_ids)
    if unmatched:
        raise ValueError(
            "Final translation units are absent from metadata/source_map.json: " + ", ".join(unmatched)
        )
    for index, (href, media_type) in enumerate(sorted(asset_media_types.items()), start=1):
        manifest_items.append(
            f'<item id="asset-{index:03}" href="{html.escape(href, quote=True)}" '
            f'media-type="{media_type}" />'
        )

    nav_items = nav_list(structure["navigation"], None)
    landmark_labels = {"frontmatter": "书前", "bodymatter": "正文", "backmatter": "书后"}
    landmarks = [
        f'<li><a epub:type="{landmark["role"]}" href="{html.escape(str(landmark["href"]), quote=True)}">{landmark_labels[landmark["role"]]}</a></li>'
        for landmark in structure["landmarks"]
    ]

    escaped_target_language = html.escape(target_language, quote=True)
    write_text(
        epub_root / "nav.xhtml",
        f'''<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{escaped_target_language}" xml:lang="{escaped_target_language}">
<head><meta charset="utf-8" /><title>Contents</title><link rel="stylesheet" type="text/css" href="styles/book.css" /></head>
<body><nav epub:type="toc" id="toc"><h1>Contents</h1><ol>{nav_items}</ol></nav>
<nav epub:type="landmarks" id="landmarks" hidden="hidden"><h2>导览</h2><ol>{cover_landmark}{''.join(landmarks)}</ol></nav></body>
</html>
''',
    )
    modified = datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    metadata_languages = "\n    ".join(
        [
            f"<dc:language>{html.escape(source_language)}</dc:language>",
            f"<dc:language>{html.escape(target_language)}</dc:language>",
        ]
    )
    creator_element = (
        f"<dc:creator>{html.escape(creator)}</dc:creator>" if creator else ""
    )
    optional_metadata = "\n    ".join(
        element
        for element in [
            f"<dc:contributor>{html.escape(contributor)}</dc:contributor>" if contributor else "",
            f"<dc:publisher>{html.escape(publisher)}</dc:publisher>" if publisher else "",
            f"<dc:date>{html.escape(publication_date)}</dc:date>" if publication_date else "",
            f"<dc:source>{html.escape(source_metadata)}</dc:source>" if source_metadata else "",
            f"<dc:description>{html.escape(description)}</dc:description>" if description else "",
            f"<dc:rights>{html.escape(rights)}</dc:rights>" if rights else "",
        ]
        if element
    )
    write_text(
        epub_root / "package.opf",
        f'''<?xml version="1.0" encoding="utf-8"?>
<package version="3.0" unique-identifier="bookid" xmlns="http://www.idpf.org/2007/opf">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/">
    <dc:identifier id="bookid">{html.escape(identifier)}</dc:identifier>
    <dc:title>{html.escape(title)}</dc:title>
    {creator_element}
    {optional_metadata}
    {metadata_languages}
    <meta property="dcterms:modified">{modified}</meta>
    {cover_metadata}
  </metadata>
  <manifest>
    {' '.join(manifest_items)}
  </manifest>
  <spine>
    {' '.join(spine_items)}
  </spine>
</package>
''',
    )

    output_path = project_root / "output" / "reading" / "book_bilingual.epub"
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.unlink(missing_ok=True)
    with zipfile.ZipFile(output_path, "w") as archive:
        archive.write(work_root / "mimetype", "mimetype", compress_type=zipfile.ZIP_STORED)
        for path in sorted(work_root.rglob("*")):
            if path.is_file() and path.name != "mimetype":
                archive.write(
                    path,
                    path.relative_to(work_root).as_posix(),
                    compress_type=zipfile.ZIP_DEFLATED,
                )
    print(f"wrote {output_path.relative_to(project_root).as_posix()}")
    return output_path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--book-root", type=Path, required=True)
    args = parser.parse_args()
    build_book(args.book_root)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
