from __future__ import annotations

import json
import re
from pathlib import Path


HEADING = re.compile(r"^(#{1,6})\s+(.+)$")


def write_publication_contract(book_root: Path, *, title: str = "Fixture Book") -> None:
    """Create the explicit publication/source contracts for builder fixtures."""
    final_files = sorted((book_root / "chapters/final").glob("*.md"))
    sections: list[dict[str, object]] = []
    units: list[dict[str, object]] = []
    next_source_line = 1
    for final_file in final_files:
        headings: list[tuple[int, int, str]] = []
        in_fence: tuple[str, int] | None = None
        text = final_file.read_text(encoding="utf-8")
        # Complete comments are not publication structure. An unterminated
        # opener is deliberately left in place, matching the real renderer.
        text = re.sub(r"<!--[\s\S]*?-->", "", text)
        lines = text.splitlines()
        for line_number, line in enumerate(lines, start=1):
            stripped = line.lstrip()
            fence = re.match(r"^(`{3,}|~{3,})", stripped)
            if in_fence:
                if fence and fence.group(1)[0] == in_fence[0] and len(fence.group(1)) >= in_fence[1]:
                    in_fence = None
                continue
            if fence:
                in_fence = (fence.group(1)[0], len(fence.group(1)))
                continue
            match = HEADING.match(line)
            if match:
                headings.append(
                    (line_number, len(match.group(1)), match.group(2).strip())
                )
        if not headings:
            raise AssertionError(f"Builder fixture has no publication heading: {final_file}")
        stack: list[tuple[int, str]] = []
        root_id = ""
        source_end_line = next_source_line + max(1, len(lines)) - 1
        for position, (line_number, level, heading_title) in enumerate(headings):
            while stack and stack[-1][0] >= level:
                stack.pop()
            section_id = f"section_{len(sections) + 1:03d}"
            parent_id = stack[-1][1] if stack else None
            if parent_id is None and not root_id:
                root_id = section_id
            sections.append(
                {
                    "id": section_id,
                    "ordinal": len(sections) + 1,
                    "title": heading_title,
                    "shortTitle": heading_title,
                    "readerTitle": heading_title,
                    "readerShortTitle": heading_title,
                    "headingLevel": level,
                    "parentId": parent_id,
                    "role": "bodymatter",
                    "kind": "chapter" if parent_id is None else "section",
                    "sourceStartLine": next_source_line + line_number - 1,
                    "sourceEndLine": next(
                        (
                            next_source_line + candidate_line - 2
                            for candidate_line, candidate_level, _ in headings[position + 1 :]
                            if candidate_level <= level
                        ),
                        source_end_line,
                    ),
                }
            )
            stack.append((level, section_id))
        units.append(
            {
                "id": final_file.stem,
                "publicationSectionId": root_id,
                "sourceStartLine": next_source_line,
                "sourceEndLine": source_end_line,
            }
        )
        next_source_line = source_end_line + 2

    metadata = book_root / "metadata"
    metadata.mkdir(parents=True, exist_ok=True)
    metadata.joinpath("publication_map.json").write_text(
        json.dumps(
            {
                "schema": "local-reading-publication-map-v1",
                "audit": {"status": "passed", "source": "fixture", "confidence": 1},
                "sections": sections,
                "notes": [],
            },
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )
    metadata.joinpath("source_map.json").write_text(
        json.dumps(
            {"schema": "local-reading-source-map-v2", "translationUnits": units},
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )
    metadata.joinpath("book.yaml").write_text(
        f"title: {title}\nlanguage: zh-Hans\n", encoding="utf-8"
    )
    source = book_root / "source"
    source.mkdir(parents=True, exist_ok=True)
    source_line_count = max(
        (int(section["sourceEndLine"]) for section in sections), default=1
    )
    source.joinpath("source.md").write_text(
        "\n".join("fixture" for _ in range(source_line_count)) + "\n",
        encoding="utf-8",
    )
