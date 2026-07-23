#!/usr/bin/env python3
"""Create a local reading/translation project from a user-owned EPUB/PDF/etc."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import unicodedata
from datetime import datetime, timezone
from pathlib import Path


BOOK_DIR_PATTERN = re.compile(r"^(\d+)_")
WINDOWS_FORBIDDEN_CHARS = set('<>:"/\\|?*')
MAX_SLUG_LENGTH = 80


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Create books/local/{target}/{number}_{slug} from a local EPUB/PDF/source file."
    )
    parser.add_argument("book_slug", help="Readable title_author slug, for example 书名_作者.")
    parser.add_argument("--source-file", required=True, help="Existing local EPUB/PDF/HTML/TXT/MD file.")
    parser.add_argument("--source-language", default="auto", help="Source language tag, for example en, de, ja, zh.")
    parser.add_argument("--target-language", default="zh-Hans", help="Target language directory, default zh-Hans.")
    parser.add_argument("--project-type", choices=["book", "paper"], default="book")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def clean_slug(raw: str) -> str:
    slug = unicodedata.normalize("NFKC", BOOK_DIR_PATTERN.sub("", raw.strip()))
    chars: list[str] = []
    previous_separator = False
    for char in slug:
        if char in WINDOWS_FORBIDDEN_CHARS or ord(char) < 32:
            replacement = "_"
        elif char.isalnum() or char in {".", "_", "-"}:
            replacement = char
        else:
            replacement = "_"
        if replacement == "_":
            if previous_separator:
                continue
            previous_separator = True
        else:
            previous_separator = False
        chars.append(replacement)
    slug = "".join(chars).strip(" ._-")
    if len(slug) > MAX_SLUG_LENGTH:
        slug = slug[:MAX_SLUG_LENGTH].rstrip(" ._-")
    if not slug:
        raise SystemExit("Book slug is empty after normalization.")
    return slug


def next_number(target_dir: Path) -> int:
    highest = 0
    if target_dir.exists():
        for entry in target_dir.iterdir():
            if entry.is_dir():
                match = BOOK_DIR_PATTERN.match(entry.name)
                if match:
                    highest = max(highest, int(match.group(1)))
    return highest + 1


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def create_project(project_root: Path, source_path: Path, args: argparse.Namespace) -> None:
    for relative in [
        "source",
        "chapters/src",
        "chapters/translated",
        "chapters/final",
        "glossary",
        "metadata",
        "qa/chapter_controls",
        "notes",
        "output/reading",
    ]:
        (project_root / relative).mkdir(parents=True, exist_ok=True)

    suffix = source_path.suffix or ".source"
    copied_source = project_root / "source" / f"original{suffix}"
    shutil.copy2(source_path, copied_source)

    manifest = {
        "schema": "local-reading-source-manifest-v1",
        "project_type": args.project_type,
        "source_file_name": source_path.name,
        "stored_source_path": copied_source.relative_to(project_root).as_posix(),
        "source_sha256": sha256_file(source_path),
        "source_format": suffix.lstrip(".").lower() or "unknown",
        "source_language": args.source_language,
        "target_language": args.target_language,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "extraction_status": "pending",
        "notes": "",
    }
    write_text(
        project_root / "metadata" / "source_manifest.json",
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
    )
    write_text(project_root / "source" / "source.md", "# Source Text\n\nTODO: Extract source/original.* here.\n")
    write_text(
        project_root / "glossary" / "terms.csv",
        "source,translation,category,note\n",
    )
    write_text(
        project_root / "metadata" / "style_profile.md",
        "# Style Profile\n\n- Source language: {source}\n- Target language: {target}\n- Project type: {kind}\n".format(
            source=args.source_language,
            target=args.target_language,
            kind=args.project_type,
        ),
    )
    write_text(
        project_root / "qa" / "status.md",
        "# QA Status\n\n- extraction: pending\n- split: pending\n- translation: pending\n- reading output: pending\n",
    )
    write_text(
        project_root / "AGENTS.md",
        "# Book Project Instructions\n\n"
        "- Use `skills/local-book-reading-pipeline/SKILL.md` from the repository root.\n"
        "- Do not run public-domain rights checks or release/private-use artifact steps.\n"
        "- Write extracted text to `source/source.md`.\n"
        "- Put source chapters in `chapters/src/`, drafts in `chapters/translated/`, final text in `chapters/final/`.\n"
        "- Put readable outputs in `output/reading/`.\n",
    )
    write_text(
        project_root / "README.md",
        "# {name}\n\n"
        "Local reading project created from `source/original{suffix}`.\n\n"
        "Next step: extract `source/original{suffix}` to `source/source.md`, then split chapters.\n".format(
            name=project_root.name,
            suffix=suffix,
        ),
    )


def main() -> None:
    args = parse_args()
    root = repo_root()
    source_path = Path(args.source_file).expanduser().resolve()
    if not source_path.is_file():
        raise SystemExit(f"--source-file must point to an existing file: {source_path}")

    target_dir = root / "books" / "local" / args.target_language
    project_name = f"{next_number(target_dir):03d}_{clean_slug(args.book_slug)}"
    project_root = target_dir / project_name
    print(project_root.relative_to(root).as_posix())
    if args.dry_run:
        return
    if project_root.exists():
        raise SystemExit(f"Refusing to overwrite existing project: {project_root}")

    create_project(project_root, source_path, args)


if __name__ == "__main__":
    main()
