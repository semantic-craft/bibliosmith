#!/usr/bin/env python3
"""Normalize Zotero PDF attachment display names and storage filenames.

Default mode is conservative: it only fixes obviously dirty PDF attachment names
such as "PDF", pure numeric filenames, SS-number suffixes, and download-site
tail filenames. Use --all to enforce the repository canonical name for every
imported PDF attachment that has a parent item.
"""

from __future__ import annotations

import argparse
import logging
import re
from pathlib import Path

from zotero_llm_worker import (
    Attachment,
    Config,
    ZoteroLocalClient,
    ZoteroWebClient,
    get_config,
    normalize_source_pdf_attachment_name,
    source_pdf_filename,
)


GENERIC_TITLES = {"PDF", "Full Text PDF", "Full Text", "Submitted Version", "Accepted Version"}
DOWNLOAD_SITE_MARKERS = ("z-library", "1lib.sk", "z-lib.sk", "annasarchive", "anna's archive", "anna’s archive")


def is_dirty_pdf_name(attachment: Attachment) -> bool:
    values = [attachment.title or "", attachment.path.name]
    for value in values:
        lower = value.lower()
        if value in GENERIC_TITLES:
            return True
        if re.fullmatch(r"\d{6,}(_yz)?\.pdf", value):
            return True
        if re.search(r"_[0-9]{7,}\.pdf$", value):
            return True
        if any(marker in lower for marker in DOWNLOAD_SITE_MARKERS):
            return True
    return False


def should_normalize(attachment: Attachment, *, include_all: bool) -> bool:
    if not attachment.parent_key or not attachment.parent_title:
        return False
    target = source_pdf_filename(attachment)
    if include_all:
        return attachment.title != target or attachment.path.name != target
    return is_dirty_pdf_name(attachment)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Normalize Zotero PDF attachment names.")
    parser.add_argument("--attachment-key", help="Only inspect one Zotero PDF attachment key.")
    parser.add_argument("--all", action="store_true", help="Rename every noncanonical parented PDF, not only dirty names.")
    parser.add_argument("--dry-run", action="store_true", help="Print planned changes without writing to Zotero/storage.")
    parser.add_argument("--limit", type=int, help="Maximum number of PDF attachments to inspect.")
    return parser


def iter_attachments(config: Config, args: argparse.Namespace) -> list[Attachment]:
    local = ZoteroLocalClient(config.zotero_local_api, config.zotero_storage, config.request_timeout)
    local.ping()
    if args.attachment_key:
        return [local.get_pdf_attachment(args.attachment_key)]
    return list(local.iter_pdf_attachments(limit=args.limit))


def main() -> int:
    args = build_parser().parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    config = get_config()
    candidates = [
        attachment
        for attachment in iter_attachments(config, args)
        if should_normalize(attachment, include_all=args.all)
    ]
    for attachment in candidates:
        target = source_pdf_filename(attachment)
        print(f"{attachment.key}\t{attachment.title}\t{attachment.path.name}\t->\t{target}")
    if args.dry_run:
        print(f"DRY_RUN candidates={len(candidates)}")
        return 0
    zotero = ZoteroWebClient(config)
    for attachment in candidates:
        normalize_source_pdf_attachment_name(attachment=attachment, config=config, zotero=zotero)
    print(f"UPDATED candidates={len(candidates)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
