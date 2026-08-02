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
import html
import json
import re
import shutil
import uuid
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


HEADING = re.compile(r"^(#{1,6})\s+(.+?)\s*$", re.DOTALL)
# A fenced code block opener: up to three spaces of indent, three or more
# backticks or tildes, then an optional info string. A backtick fence's info
# string may not contain a backtick, which keeps an inline ``a `b` c`` span from
# being read as a fence.
FENCE_OPEN = re.compile(r"^([ \t]{0,3})(`{3,}|~{3,})[ \t]*(.*)$")
FENCE_CLOSE = re.compile(r"^[ \t]{0,3}(`{3,}|~{3,})[ \t]*$")


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


def split_paragraphs(text: str) -> list[str]:
    """Split a chapter into blocks on blank lines, keeping a fence whole.

    A fenced code block counts as one block however many blank lines it holds.
    That matters twice over: the block has to reach `render_block` intact to be
    rendered as code at all, and `render_chapter` pairs source and target
    *positionally*, so a fence split into pieces would inflate the block count.
    If the two sides then disagreed, the whole chapter would drop to
    chapter-level fallback and lose paragraph pairing everywhere, not just at
    the code.
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

    blocks: list[str] = []
    paragraph: list[str] = []

    def flush() -> None:
        block = "\n".join(paragraph).strip()
        paragraph.clear()
        if block:
            blocks.append(block)

    index = 0
    while index < len(lines):
        fence = fence_opener(lines[index])
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
        if lines[index].strip():
            paragraph.append(lines[index])
        else:
            flush()
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
    return html.escape(" ".join(line.strip() for line in block.splitlines()), quote=True)


def render_block(block: str, css_class: str, language: str) -> str:
    attributes = (
        f'class="{css_class}" lang="{html.escape(language, quote=True)}" '
        f'xml:lang="{html.escape(language, quote=True)}"'
    )
    code = fenced_code(block)
    if code is not None:
        # Escaped only, never joined into a line: inside a code block the line
        # breaks and the backticks are the content.
        return f"<pre {attributes}><code>{html.escape(code)}</code></pre>"
    heading = HEADING.fullmatch(block)
    if heading:
        level = min(len(heading.group(1)), 3)
        return f"<h{level} {attributes}>{inline_text(heading.group(2))}</h{level}>"
    return f"<p {attributes}>{inline_text(block)}</p>"


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


def build_book(project_root: Path) -> Path:
    project_root = project_root.resolve()
    source_map_path = project_root / "metadata" / "source_map.json"
    source_manifest_path = project_root / "metadata" / "source_manifest.json"
    source_map = json.loads(read_text(source_map_path))
    source_manifest = json.loads(read_text(source_manifest_path))
    chapters = source_map.get("chapters")
    if not isinstance(chapters, list) or not chapters:
        raise ValueError("metadata/source_map.json has no chapters.")

    book_metadata = parse_simple_yaml(project_root / "metadata" / "book.yaml")
    source_language = normalized_language(source_manifest.get("source_language"), "und")
    target_language = normalized_language(
        book_metadata.get("language") or source_manifest.get("target_language"), "zh-Hans"
    )
    source_file_name = source_manifest.get("source_file_name")
    source_title = (
        Path(source_file_name).stem
        if isinstance(source_file_name, str) and source_file_name.strip()
        else re.sub(r"^\d+_", "", project_root.name).replace("_", " ").strip()
    )
    title = (
        book_metadata.get("title")
        or book_metadata.get("title_zh")
        or book_metadata.get("title_zh_hans")
        or source_title
        or project_root.name
    )
    creator = book_metadata.get("author") or book_metadata.get("creator") or ""
    identifier = book_metadata.get("identifier") or f"urn:uuid:{uuid.uuid4()}"

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
        '''body{line-height:1.72;margin:0;padding:1.2em;overflow-wrap:break-word}
.bitext-unit{margin:0 0 1.15em}
.bitext-source{font-size:.92em;line-height:1.5;color:#555;margin:0 0 .35em;text-indent:0}
.bitext-target{font-size:1em;line-height:1.72;color:inherit;margin:0;text-indent:2em}
.bitext-chapter-block{margin-bottom:.7em}
.bitext-source-paragraph{font-size:1em;line-height:1.5;margin:0 0 .7em;text-indent:0}
.bitext-target-paragraph{font-size:1em;line-height:1.72;margin:0 0 .7em;text-indent:2em}
h1,h2,h3{line-height:1.3;text-indent:0}
pre{margin:0 0 .35em;padding:.5em .6em;background:#f4f4f4;border:1px solid #e0e0e0;border-radius:3px;font-size:.82em;line-height:1.45;white-space:pre-wrap;overflow-wrap:anywhere;break-inside:avoid}
pre code{font-family:monospace;font-size:inherit}
/* The bitext classes carry a first-line indent and a colour for prose; a class
   selector outranks the bare `pre` above, so the code cases are named here. */
pre.bitext-source,pre.bitext-target,pre.bitext-source-paragraph,pre.bitext-target-paragraph{text-indent:0;color:inherit}
''',
    )

    manifest_items = [
        '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav" />',
        '<item id="css" href="styles/book.css" media-type="text/css" />',
    ]
    spine_items: list[str] = []
    nav_items: list[str] = []
    matched_final_ids: set[str] = set()

    for index, chapter in enumerate(chapters, start=1):
        if not isinstance(chapter, dict):
            raise ValueError("Source map chapter entries must be objects.")
        chapter_id = str(chapter.get("id") or "").strip()
        source_relative = str(
            chapter.get("chapterSourcePath") or chapter.get("chapter_source_path") or ""
        ).strip()
        if not chapter_id or not source_relative:
            raise ValueError("Source map chapter is missing id or chapter_source_path.")
        final_path = final_files.get(chapter_id)
        if final_path is None:
            raise ValueError(f"Promoted final chapter is missing for source-map chapter {chapter_id}.")
        source_path = safe_project_path(project_root, source_relative)
        if not source_path.is_file():
            raise ValueError(f"Source chapter is missing for {chapter_id}: {source_relative}")

        source_text = read_text(source_path)
        target_text = read_text(final_path)
        body, alignment, source_count, target_count = render_chapter(
            source_text, target_text, source_language, target_language
        )
        title_text = chapter_title(target_text, source_text, chapter_id)
        href = f"chapter_{index:03}.xhtml"
        item_id = f"chapter-{index:03}"
        write_text(epub_root / href, xhtml_document(title_text, body, target_language))
        manifest_items.append(
            f'<item id="{item_id}" href="{href}" media-type="application/xhtml+xml" />'
        )
        spine_items.append(f'<itemref idref="{item_id}" />')
        nav_items.append(f'<li><a href="{href}">{html.escape(title_text)}</a></li>')
        matched_final_ids.add(chapter_id)
        print(
            f"{chapter_id}: alignment={alignment} "
            f"source_paragraphs={source_count} target_paragraphs={target_count}"
        )

    unmatched = sorted(set(final_files) - matched_final_ids)
    if unmatched:
        raise ValueError(
            "Final chapters are absent from metadata/source_map.json: " + ", ".join(unmatched)
        )

    escaped_target_language = html.escape(target_language, quote=True)
    write_text(
        epub_root / "nav.xhtml",
        f'''<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{escaped_target_language}" xml:lang="{escaped_target_language}">
<head><meta charset="utf-8" /><title>Contents</title><link rel="stylesheet" type="text/css" href="styles/book.css" /></head>
<body><nav epub:type="toc" id="toc"><h1>Contents</h1><ol>{''.join(nav_items)}</ol></nav></body>
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
    write_text(
        epub_root / "package.opf",
        f'''<?xml version="1.0" encoding="utf-8"?>
<package version="3.0" unique-identifier="bookid" xmlns="http://www.idpf.org/2007/opf">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/">
    <dc:identifier id="bookid">{html.escape(identifier)}</dc:identifier>
    <dc:title>{html.escape(title)}</dc:title>
    {creator_element}
    {metadata_languages}
    <meta property="dcterms:modified">{modified}</meta>
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
