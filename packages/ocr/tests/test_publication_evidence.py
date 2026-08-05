from __future__ import annotations

import sys
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PACKAGE_ROOT))

from publication_evidence import (
    SCHEMA,
    SourceDocument,
    build_markdown_evidence,
    persist_source_document,
    normalize_extracted_markdown_notes,
    source_documents_for_page_groups,
)


def test_pdf_heading_misclassification_is_normalized_back_to_a_note_definition() -> None:
    markdown = (
        "# Chapter\n\nClaim[^n1].\n\n## [^n1]: Note body.\n\n"
        "```markdown\n## [^example]: Literal example.\n```\n\n"
        "`multiline code\n## [^inline]: Literal inline example.\n`\n\n"
        "<!--\n## [^comment]: Literal comment.\n-->\n"
    )

    normalized = normalize_extracted_markdown_notes(markdown)

    assert normalized == (
        "# Chapter\n\nClaim[^n1].\n\n[^n1]: Note body.\n\n"
        "```markdown\n## [^example]: Literal example.\n```\n\n"
        "`multiline code\n## [^inline]: Literal inline example.\n`\n\n"
        "<!--\n## [^comment]: Literal comment.\n-->\n"
    )


def test_source_document_is_copied_into_the_markdown_sidecar_with_a_real_digest(
    tmp_path: Path,
) -> None:
    markdown_path = tmp_path / "book.md"
    markdown_path.write_text("# Chapter\n\nBody.\n", encoding="utf-8")
    raw = tmp_path / "scratch" / "part.jsonl"
    raw.parent.mkdir()
    raw.write_bytes(b'{"page":1}\n')

    document = persist_source_document(
        markdown_path,
        raw,
        "paddleocr/part.jsonl",
        start_line=1,
        end_line=3,
        pages=(1,),
        kind="paddleocr_jsonl",
    )

    persisted = tmp_path / document.path
    assert persisted.read_bytes() == raw.read_bytes()
    assert document.path == "book_assets/source_documents/paddleocr/part.jsonl"
    assert document.sha256 == "7670c4141d96d800edbf2f686da124de10d8757daa5fd3eeab68f0bf2eae08e2"


def test_page_group_without_markdown_text_is_retained_as_unmapped_evidence() -> None:
    documents = source_documents_for_page_groups(
        "<!-- page: 1 -->\n\nVisible.\n",
        [
            ("paddleocr/page-1.jsonl", (1,), "paddleocr_jsonl", "a" * 64),
            ("paddleocr/page-2.jsonl", (2,), "paddleocr_jsonl", "b" * 64),
        ],
    )

    assert len(documents) == 2
    assert documents[1].start_line == documents[1].end_line == 0
    assert documents[1].anomalies == ("source document produced no assembled Markdown lines",)


def test_one_publication_chapter_can_span_two_extractor_part_files() -> None:
    markdown = """# Contents

Chapter One 3

# Chapter One

<!-- page: 3 -->

Opening text.

<!-- page: 4 -->

Continued text.

# Appendix

Appendix text.
"""
    evidence = build_markdown_evidence(
        markdown,
        source_format="mineru",
        extraction_engine="MinerU Precision v4",
        source_documents=[
            SourceDocument("parts/0001/full.md", 1, 10, (1, 2, 3), sha256="a" * 64),
            SourceDocument("parts/0002/full.md", 11, 17, (4, 5), sha256="b" * 64),
        ],
    )

    chapter = next(
        section for section in evidence["sections"] if section["title"] == "Chapter One"
    )
    assert evidence["schema"] == SCHEMA
    assert chapter["sourcePages"] == [3, 4]
    assert chapter["sourceFiles"] == ["parts/0001/full.md", "parts/0002/full.md"]
    assert "publication section spans multiple extractor documents" in chapter["evidence"]
    assert "printed contents title/page match 3" in chapter["evidence"]


def test_layout_regions_become_node_level_evidence() -> None:
    markdown = """# Kapitel Eins

| Begriff | Wert |
| --- | --- |
| Geheimnis | 1 |

![Abbildung](assets/figure.png)

$$x = y$$

Text[^de-1].

[^de-1]: Eine Fußnote.
"""
    section = build_markdown_evidence(
        markdown,
        source_format="ocr",
        extraction_engine="PaddleOCR-VL-1.6",
    )["sections"][0]

    assert "table region in extractor Markdown" in section["evidence"]
    assert "figure region in extractor Markdown" in section["evidence"]
    assert "formula region in extractor Markdown" in section["evidence"]
    assert "note region in extractor Markdown" in section["evidence"]


def test_every_extractor_emits_the_same_source_bound_note_contract() -> None:
    markdown = """# Kapitel Eins

Behauptung[^legal-1], nochmals[^legal-1].

[^legal-1]: Eine Fußnote.
    Mit Fortsetzung.
"""
    contracts = []
    for source_format in ("epub", "pdf", "ocr", "mineru"):
        evidence = build_markdown_evidence(
            markdown,
            source_format=source_format,
            extraction_engine=f"{source_format}-fixture",
            source_documents=[
                SourceDocument(
                    f"{source_format}/source.txt",
                    1,
                    6,
                    (12,),
                    kind="fixture",
                    sha256="a" * 64,
                )
            ],
        )
        assert evidence["notes"] == [
            {
                "id": "note_001",
                "sourceLabel": "legal-1",
                "kind": "footnote",
                "publicationSectionId": "extracted_section_001",
                "sourceStartLine": 5,
                "sourceEndLine": 6,
                "sourcePages": [12],
                "sourceFiles": [f"{source_format}/source.txt"],
                "referenceSourceLines": [3, 3],
                "referenceIds": [
                    "noteref_note_001_001",
                    "noteref_note_001_002",
                ],
                "sourceAnchor": "markdown-footnote-note_001",
                "evidence": ["canonical Markdown note definition and reference"],
                "anomalies": [],
            }
        ]
        contracts.append(
            {
                key: evidence["notes"][0][key]
                for key in (
                    "id",
                    "sourceLabel",
                    "kind",
                    "sourceStartLine",
                    "sourceEndLine",
                    "referenceSourceLines",
                    "referenceIds",
                    "sourceAnchor",
                )
            }
        )
    assert contracts.count(contracts[0]) == 4


def test_note_contract_records_orphans_instead_of_silently_dropping_them() -> None:
    evidence = build_markdown_evidence(
        "# Chapter\n\nMissing[^lost].\n\n[^orphan]: Unreferenced.\n",
        source_format="ocr",
        extraction_engine="fixture",
        source_documents=[
            SourceDocument("ocr/page.jsonl", 1, 5, sha256="b" * 64)
        ],
    )

    assert evidence["notes"][0]["sourceLabel"] == "lost"
    assert evidence["notes"][0]["anomalies"] == ["note definition is missing"]
    assert evidence["notes"][1]["sourceLabel"] == "orphan"
    assert evidence["notes"][1]["anomalies"] == ["note has no references"]


def test_heading_free_extraction_requires_structure_correction() -> None:
    evidence = build_markdown_evidence(
        "Author Name\n\nUnstructured body.",
        source_format="pdf",
        extraction_engine="pymupdf",
        title="attachment-file-name",
    )

    section = evidence["sections"][0]
    assert section["confidence"] == 0.4
    assert section["anomalies"] == ["extractor produced no publication headings"]
