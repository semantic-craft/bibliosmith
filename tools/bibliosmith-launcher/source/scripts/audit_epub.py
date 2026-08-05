#!/usr/bin/env python3
"""Audit an EPUB's publication structure independently of EPUBCheck.

EPUBCheck proves package validity.  This audit proves that the package still
looks like the book described by BiblioSmith's publication map: nested
navigation, meaningful body documents, heading anchors, semantic notes and
reader-safe CSS.  Its checks are deterministic; the report also records an
evidence timestamp and contains no book text.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import posixpath
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from datetime import datetime, timezone
from urllib.parse import unquote, urlsplit
from xml.etree import ElementTree as ET
from zipfile import ZipFile


CONTAINER_NS = "urn:oasis:names:tc:opendocument:xmlns:container"
OPF_NS = "http://www.idpf.org/2007/opf"
DC_NS = "http://purl.org/dc/elements/1.1/"
XHTML_NS = "http://www.w3.org/1999/xhtml"
EPUB_NS = "http://www.idpf.org/2007/ops"
GENERIC_LABEL = re.compile(
    r"^(?:chapter|unit|section)[-_ ]*\d+$|^continuation(?:\s+\d+)?$", re.I
)
CSS_URL = re.compile(r"url\(\s*(['\"]?)(.*?)\1\s*\)", re.I)
CSS_IMPORT = re.compile(r"@import\s+['\"]([^'\"]+)['\"]", re.I)
INLINE_RESOURCE_SCHEMES = {"data", "blob"}
SAFE_EXTERNAL_LINK_SCHEMES = {"http", "https", "mailto", "tel"}


def q(namespace: str, name: str) -> str:
    return f"{{{namespace}}}{name}"


def text_of(element: ET.Element | None) -> str:
    return " ".join("".join(element.itertext()).split()) if element is not None else ""


def resolve(base: str, href: str) -> tuple[str, str]:
    parsed = urlsplit(href)
    path = unquote(parsed.path)
    fragment = unquote(parsed.fragment)
    target = posixpath.normpath(posixpath.join(posixpath.dirname(base), path)) if path else base
    return target, fragment


def nested_nav_items(ol: ET.Element, depth: int = 1) -> list[dict[str, object]]:
    items: list[dict[str, object]] = []
    for li in ol.findall(q(XHTML_NS, "li")):
        anchor = li.find(q(XHTML_NS, "a"))
        if anchor is not None:
            items.append(
                {"href": anchor.get("href", ""), "label": text_of(anchor), "depth": depth}
            )
        child = li.find(q(XHTML_NS, "ol"))
        if child is not None:
            items.extend(nested_nav_items(child, depth + 1))
    return items


def add_finding(findings: list[dict[str, str]], code: str, message: str) -> None:
    findings.append({"severity": "error", "code": code, "message": message})


def resource_url_is_external(value: str) -> bool:
    parsed = urlsplit(value.strip())
    return bool(
        parsed.netloc
        or (parsed.scheme and parsed.scheme.lower() not in INLINE_RESOURCE_SCHEMES)
    )


def resource_url_escapes_root(base: str, value: str) -> bool:
    raw = unquote(value.strip())
    parsed = urlsplit(raw)
    if parsed.scheme.lower() in INLINE_RESOURCE_SCHEMES:
        return False
    resource_path = unquote(parsed.path)
    if not resource_path:
        return False
    if "\\" in resource_path or resource_path.startswith("/"):
        return True
    target = posixpath.normpath(
        posixpath.join(posixpath.dirname(base), resource_path)
    )
    return target == ".." or target.startswith("../") or target.startswith("/")


def resource_url_is_unsafe(base: str, value: str) -> bool:
    return resource_url_is_external(value) or resource_url_escapes_root(base, value)


def css_resource_urls(value: str) -> list[str]:
    urls = [match.group(2) for match in CSS_URL.finditer(value)]
    urls.extend(match.group(1) for match in CSS_IMPORT.finditer(value))
    return urls


def attribute_resource_urls(name: str, value: str) -> list[str]:
    if name == "style":
        return css_resource_urls(value)
    if name == "srcset":
        urls: list[str] = []
        position = 0
        while position < len(value):
            while position < len(value) and (
                value[position].isspace() or value[position] == ","
            ):
                position += 1
            start = position
            data_url = value[start : start + 5].lower() == "data:"
            while position < len(value) and not value[position].isspace():
                if value[position] == "," and not data_url:
                    break
                position += 1
            if position > start:
                urls.append(value[start:position])
            while position < len(value) and value[position] != ",":
                position += 1
            if position < len(value):
                position += 1
        return urls
    return [value]


def element_resource_attributes(element: ET.Element) -> list[tuple[str, str]]:
    local_name = element.tag.rsplit("}", 1)[-1]
    resources: list[tuple[str, str]] = []
    for raw_name, value in element.attrib.items():
        attribute_name = raw_name.rsplit("}", 1)[-1]
        if attribute_name in {"src", "poster", "data", "srcset", "style"}:
            resources.append((attribute_name, value))
        elif attribute_name == "href" and local_name not in {"a", "area"}:
            resources.append((attribute_name, value))
    return resources


def validate_local_target(
    findings: list[dict[str, str]],
    names: set[str],
    fragment_ids_by_document: dict[str, set[str]],
    base_path: str,
    resource_url: str,
    source_kind: str,
) -> None:
    target_path, fragment = resolve(base_path, resource_url)
    if target_path not in names:
        add_finding(
            findings,
            "resource.href",
            f"{source_kind} resource is missing: {target_path}",
        )
    elif (
        fragment
        and target_path in fragment_ids_by_document
        and fragment not in fragment_ids_by_document[target_path]
    ):
        add_finding(
            findings,
            "resource.fragment",
            f"{source_kind} fragment target is missing: {target_path}#{fragment}",
        )


def chrome_executable() -> str:
    configured = os.environ.get("BIBLIOSMITH_CHROME", "").strip()
    repository_or_runtime_root = Path(__file__).resolve().parents[4]
    managed_manifest = (
        repository_or_runtime_root
        / "vendor"
        / "playwright-core"
        / "browser-manifest.json"
    )
    managed_candidates: list[str] = []
    if managed_manifest.is_file():
        manifest = json.loads(managed_manifest.read_text(encoding="utf-8"))
        if manifest.get("schema") != "bibliosmith-browser-runtime-v1":
            raise RuntimeError("Bundled Chromium manifest has an unsupported schema.")
        managed = repository_or_runtime_root / str(manifest.get("relativePath") or "")
        if not managed.is_file():
            raise RuntimeError("Bundled Chromium executable is missing.")
        actual_sha256 = hashlib.sha256(managed.read_bytes()).hexdigest()
        if actual_sha256 != manifest.get("sha256"):
            raise RuntimeError("Bundled Chromium executable failed its SHA-256 check.")
        managed_candidates.append(str(managed))
    for root in (
        repository_or_runtime_root / "books" / "node_modules" / "playwright-core",
        repository_or_runtime_root / "vendor" / "playwright-core",
    ):
        managed_candidates.extend(
            str(path)
            for pattern in ("chrome-headless-shell", "chrome-headless-shell.exe", "headless_shell", "headless_shell.exe")
            for path in root.glob(f".local-browsers/**/{pattern}")
        )
    candidates = [
        configured,
        *managed_candidates,
        shutil.which("google-chrome") or "",
        shutil.which("chromium") or "",
        shutil.which("chromium-browser") or "",
        shutil.which("msedge") or "",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ]
    for candidate in candidates:
        if candidate and Path(candidate).is_file() and os.access(candidate, os.X_OK):
            return candidate
    raise RuntimeError(
        "The managed Chromium renderer is missing; rebuild the App runtime resources."
    )


def rendered_viewport_smoke(
    archive: ZipFile, spine_paths: list[str]
) -> list[dict[str, object]]:
    unsafe_names = [
        name
        for name in archive.namelist()
        if name.startswith("/") or ".." in Path(name).parts
    ]
    if unsafe_names:
        raise ValueError("EPUB contains an unsafe archive path")
    chrome = chrome_executable()
    widths = (390, 430)
    with tempfile.TemporaryDirectory(prefix="bibliosmith-render-") as temporary:
        root = Path(temporary)
        archive.extractall(root)
        node = os.environ.get("BIBLIOSMITH_NODE", "").strip() or shutil.which("node")
        if not node:
            raise RuntimeError("Node.js is required for the Playwright render smoke.")
        renderer = Path(__file__).with_name("render_epub_smoke.cjs")
        if not renderer.is_file():
            raise RuntimeError("Bundled Playwright render-smoke script is missing.")
        completed = subprocess.run(
            [
                node,
                str(renderer),
                "--root",
                str(root),
                "--browser",
                chrome,
                "--spine-json",
                json.dumps(spine_paths),
            ],
            capture_output=True,
            text=True,
            timeout=45,
            check=False,
        )
        if completed.returncode != 0:
            raise RuntimeError(
                "Playwright Chromium render smoke failed: "
                + (completed.stderr.strip().splitlines()[-1] if completed.stderr.strip() else "unknown error")
            )
        render_result = json.loads(completed.stdout)
        measurements = render_result["measurements"]
        browser_version = str(render_result["browserVersion"])
    results: list[dict[str, object]] = []
    for width in widths:
        samples = [item for item in measurements if item.get("width") == width]
        risks: list[str] = []
        for sample in samples:
            path = sample.get("path", "unknown")
            if sample.get("error"):
                risks.append(f"{path}: {sample['error']}")
            if sample.get("overflow"):
                risks.append(
                    f"{path}: rendered width {sample.get('scrollWidth')} exceeds viewport {sample.get('viewportWidth')}"
                )
            if sample.get("clippedHeadings"):
                risks.append(f"{path}: {sample['clippedHeadings']} heading(s) are clipped")
            if sample.get("blankFirstScreen"):
                risks.append(f"{path}: first rendered screen is blank")
        results.append(
            {
                "width": width,
                "renderer": Path(chrome).name,
                "rendererVersion": browser_version,
                "documents": len(samples),
                "status": "passed" if samples and not risks else "failed",
                "risks": risks or ([] if samples else ["no rendered spine documents"]),
            }
        )
    return results


def audit(epub_path: Path, publication_map_path: Path) -> dict[str, object]:
    publication = json.loads(publication_map_path.read_text(encoding="utf-8"))
    if publication.get("schema") != "local-reading-publication-map-v1":
        raise ValueError("unsupported publication map schema")
    sections = publication.get("sections")
    if not isinstance(sections, list) or not sections:
        raise ValueError("publication map has no sections")
    notes = publication.get("notes", [])
    findings: list[dict[str, str]] = []
    for kind in ("title_page", "copyright", "contents"):
        if sum(1 for section in sections if section.get("kind") == kind) > 1:
            add_finding(
                findings,
                "publication.duplicate_frontmatter",
                f"Publication map contains repeated {kind} blocks.",
            )

    with ZipFile(epub_path) as archive:
        names = set(archive.namelist())
        container = ET.fromstring(archive.read("META-INF/container.xml"))
        rootfile = container.find(f".//{q(CONTAINER_NS, 'rootfile')}")
        if rootfile is None or not rootfile.get("full-path"):
            raise ValueError("container has no package document")
        package_path = rootfile.get("full-path", "")
        package = ET.fromstring(archive.read(package_path))

        metadata = package.find(q(OPF_NS, "metadata"))
        title = text_of(metadata.find(q(DC_NS, "title")) if metadata is not None else None)
        language = text_of(metadata.find(q(DC_NS, "language")) if metadata is not None else None)
        identifier = text_of(metadata.find(q(DC_NS, "identifier")) if metadata is not None else None)
        creator = text_of(metadata.find(q(DC_NS, "creator")) if metadata is not None else None)
        if not title or GENERIC_LABEL.match(title):
            add_finding(findings, "metadata.title", "Package title is empty or generic.")
        if not language:
            add_finding(findings, "metadata.language", "Package language is missing.")
        if not identifier:
            add_finding(findings, "metadata.identifier", "Package identifier is missing.")
        if not creator:
            add_finding(findings, "metadata.creator", "Package creator is missing.")

        manifest: dict[str, tuple[str, str]] = {}
        manifest_resources: list[tuple[str, str]] = []
        manifest_ids: set[str] = set()
        manifest_hrefs: set[str] = set()
        nav_path = ""
        css_paths: list[str] = []
        cover_image_paths: list[str] = []
        for item in package.findall(f".//{q(OPF_NS, 'manifest')}/{q(OPF_NS, 'item')}"):
            item_id = item.get("id", "")
            href = item.get("href", "")
            full_path, _ = resolve(package_path, href)
            if not item_id:
                add_finding(
                    findings,
                    "manifest.id",
                    "Package manifest contains an item without an ID.",
                )
            elif item_id in manifest_ids:
                add_finding(
                    findings,
                    "manifest.duplicate_id",
                    f"Package manifest contains a duplicate ID: {item_id}",
                )
            else:
                manifest_ids.add(item_id)
            if not href:
                add_finding(
                    findings,
                    "manifest.href",
                    "Package manifest contains an item without an href.",
                )
            elif full_path in manifest_hrefs:
                add_finding(
                    findings,
                    "manifest.duplicate_href",
                    f"Package manifest contains a duplicate href: {href}",
                )
            else:
                manifest_hrefs.add(full_path)
            if href and resource_url_is_unsafe(package_path, href):
                add_finding(
                    findings,
                    "resource.external",
                    "The package manifest references an external or absolute resource.",
                )
            if href and full_path not in names:
                add_finding(
                    findings,
                    "manifest.href",
                    f"Package manifest target is missing: {full_path}",
                )
            if item_id and item_id not in manifest:
                manifest[item_id] = (full_path, item.get("media-type", ""))
            if href:
                manifest_resources.append((full_path, item.get("media-type", "")))
            if href and "nav" in item.get("properties", "").split():
                nav_path = full_path
            if href and "cover-image" in item.get("properties", "").split():
                cover_image_paths.append(full_path)
            if href and item.get("media-type") == "text/css":
                css_paths.append(full_path)
        xhtml_documents: dict[str, ET.Element] = {}
        fragment_ids_by_document: dict[str, set[str]] = {}
        all_ids: dict[str, str] = {}
        duplicate_xhtml_ids = 0
        local_url_targets: list[tuple[str, str]] = []
        for document_path, media_type in set(manifest_resources):
            if document_path not in names or not (
                media_type == "application/xhtml+xml"
                or media_type == "image/svg+xml"
                or media_type.endswith("+xml")
            ):
                continue
            document = ET.fromstring(archive.read(document_path))
            document_ids = {
                element_id
                for element in document.iter()
                if (element_id := element.get("id"))
            }
            fragment_ids_by_document[document_path] = document_ids
            if media_type != "application/xhtml+xml":
                continue
            xhtml_documents[document_path] = document
            for element in document.iter():
                element_id = element.get("id")
                if element_id:
                    if element_id in all_ids:
                        duplicate_xhtml_ids += 1
                        add_finding(
                            findings,
                            "xhtml.duplicate_id",
                            f"Duplicate XHTML ID: {element_id}",
                        )
                    all_ids[element_id] = document_path
                resources = element_resource_attributes(element)
                for name, value in resources:
                    for resource_url in attribute_resource_urls(name, value):
                        if resource_url_is_unsafe(document_path, resource_url):
                            add_finding(
                                findings,
                                "resource.external",
                                f"XHTML references an external or absolute resource: {document_path}",
                            )
                        elif urlsplit(resource_url.strip()).scheme.lower() not in INLINE_RESOURCE_SCHEMES:
                            local_url_targets.append((document_path, resource_url))
                local_name = element.tag.rsplit("}", 1)[-1]
                if local_name in {"a", "area"} and (href := element.get("href")):
                    parsed_href = urlsplit(href.strip())
                    if (
                        parsed_href.scheme
                        and parsed_href.scheme.lower() not in SAFE_EXTERNAL_LINK_SCHEMES
                    ) or (parsed_href.netloc and not parsed_href.scheme):
                        add_finding(
                            findings,
                            "resource.external",
                            f"XHTML references an unsafe hyperlink scheme: {document_path}",
                        )
                    elif not parsed_href.scheme and not parsed_href.netloc:
                        if resource_url_escapes_root(document_path, href):
                            add_finding(
                                findings,
                                "resource.external",
                                f"XHTML references an absolute or root-escaping local link: {document_path}",
                            )
                        else:
                            local_url_targets.append((document_path, href))
        for document_path, resource_url in local_url_targets:
            validate_local_target(
                findings,
                names,
                fragment_ids_by_document,
                document_path,
                resource_url,
                "XHTML",
            )
        if not nav_path or nav_path not in names:
            add_finding(findings, "navigation.missing", "EPUB navigation document is missing.")
            nav_items: list[dict[str, object]] = []
            landmark_hrefs: list[str] = []
        else:
            nav = ET.fromstring(archive.read(nav_path))
            toc = next(
                (
                    node
                    for node in nav.findall(f".//{q(XHTML_NS, 'nav')}")
                    if node.get(q(EPUB_NS, "type")) == "toc"
                ),
                None,
            )
            toc_ol = toc.find(q(XHTML_NS, "ol")) if toc is not None else None
            nav_items = nested_nav_items(toc_ol) if toc_ol is not None else []
            landmarks = next(
                (
                    node
                    for node in nav.findall(f".//{q(XHTML_NS, 'nav')}")
                    if node.get(q(EPUB_NS, "type")) == "landmarks"
                ),
                None,
            )
            landmark_by_role = {
                anchor.get(q(EPUB_NS, "type"), ""): anchor.get("href", "")
                for anchor in (landmarks.findall(f".//{q(XHTML_NS, 'a')}") if landmarks is not None else [])
            }
            landmark_hrefs = [landmark_by_role["bodymatter"]] if "bodymatter" in landmark_by_role else []
            if cover_image_paths and "cover" not in landmark_by_role:
                add_finding(findings, "landmarks.cover", "A packaged cover image has no cover landmark.")
            expected_roles = {
                str(section.get("role"))
                for section in sections
                if not section.get("parentId")
                and section.get("role") in {"frontmatter", "bodymatter", "backmatter"}
            }
            if not expected_roles.issubset(landmark_by_role):
                add_finding(findings, "landmarks.roles", "Landmark roles do not cover publication root roles.")

        expected_ids = [str(section.get("id", "")) for section in sections]
        actual_ids = [str(item["href"]).partition("#")[2] for item in nav_items]
        if actual_ids != expected_ids:
            add_finding(findings, "navigation.order", "Navigation targets do not match publication-map order.")
        section_by_id = {str(section.get("id", "")): section for section in sections}
        expected_depths: list[int] = []
        for section in sections:
            depth = 1
            parent_id = section.get("parentId")
            visited: set[str] = set()
            while isinstance(parent_id, str) and parent_id:
                if parent_id in visited or parent_id not in section_by_id:
                    depth = -1
                    break
                visited.add(parent_id)
                depth += 1
                parent_id = section_by_id[parent_id].get("parentId")
            expected_depths.append(depth)
        if [int(item["depth"]) for item in nav_items] != expected_depths:
            add_finding(findings, "navigation.depth", "Navigation nesting does not match the publication map.")
        if len(set(str(item["href"]) for item in nav_items)) != len(nav_items):
            add_finding(findings, "navigation.duplicate", "Navigation contains duplicate targets.")
        for item in nav_items:
            if not str(item["label"]).strip() or GENERIC_LABEL.match(str(item["label"]).strip()):
                add_finding(findings, "navigation.label", "Navigation contains an empty or internal-unit label.")
                break
            target, fragment = resolve(nav_path, str(item["href"]))
            if target not in names:
                add_finding(findings, "navigation.href", f"Navigation target is missing: {target}")
            elif fragment and fragment not in fragment_ids_by_document.get(target, set()):
                add_finding(findings, "navigation.anchor", f"Navigation anchor is missing: {fragment}")
        if not landmark_hrefs:
            add_finding(findings, "landmarks.bodymatter", "Landmarks do not identify bodymatter.")
        if len(cover_image_paths) > 1:
            add_finding(findings, "cover.duplicate", "Package declares more than one cover image.")
        for cover_image_path in cover_image_paths:
            if cover_image_path not in names:
                add_finding(findings, "cover.image", "Declared cover image is missing.")

        spine_paths: list[str] = []
        for itemref in package.findall(f".//{q(OPF_NS, 'spine')}/{q(OPF_NS, 'itemref')}"):
            item = manifest.get(itemref.get("idref", ""))
            if item:
                spine_paths.append(item[0])
        heading_ids: list[str] = []
        heading_levels: dict[str, int] = {}
        noteref_ids: set[str] = set()
        noteref_targets: dict[str, str] = {}
        footnote_ids: set[str] = set()
        backlink_targets: set[str] = set()
        backlinks_by_note: dict[str, set[str]] = {}
        semantic_link_targets: list[tuple[str, str, str]] = []
        meaningful_spine = 0
        fixed_inline_widths: list[int] = []
        table_count = 0
        responsive_table_wrappers = 0
        for spine_path in spine_paths:
            if spine_path not in names:
                add_finding(findings, "spine.href", f"Spine document is missing: {spine_path}")
                continue
            markup = archive.read(spine_path).decode("utf-8", errors="replace")
            document = xhtml_documents.get(spine_path)
            if document is None:
                document = ET.fromstring(markup)
            fixed_inline_widths.extend(
                int(match)
                for match in re.findall(
                    r"(?:min-)?width\s*:\s*(\d+)px", markup, flags=re.IGNORECASE
                )
            )
            table_count += len(document.findall(f".//{q(XHTML_NS, 'table')}"))
            responsive_table_wrappers += sum(
                1
                for element in document.iter()
                if "table-wrap" in element.get("class", "").split()
            )
            body = document.find(q(XHTML_NS, "body"))
            body_text = text_of(body)
            cover_main = document.find(
                f".//{q(XHTML_NS, 'main')}[@{q(EPUB_NS, 'type')}='cover']"
            )
            if cover_main is not None:
                images = cover_main.findall(f".//{q(XHTML_NS, 'img')}")
                if len(images) != 1:
                    add_finding(findings, "cover.page", "Cover page must contain exactly one image.")
                elif resolve(spine_path, images[0].get("src", ""))[0] not in cover_image_paths:
                    add_finding(findings, "cover.target", "Cover page does not reference the declared cover image.")
                meaningful_spine += 1
            elif len(body_text) < 20:
                add_finding(findings, "spine.empty", f"Spine document has no meaningful body: {spine_path}")
            else:
                meaningful_spine += 1
            for element in document.iter():
                element_id = element.get("id")
                local = element.tag.rsplit("}", 1)[-1]
                if local in {"h1", "h2", "h3", "h4", "h5", "h6"}:
                    if not text_of(element):
                        add_finding(findings, "heading.empty", "A publication heading is empty.")
                    if element_id in expected_ids:
                        heading_ids.append(element_id)
                        heading_levels[element_id] = int(local[1])
                        section = section_by_id[element_id]
                        classes = set(element.get("class", "").split())
                        kind = str(section.get("kind") or "section")
                        role = str(section.get("role") or "bodymatter")
                        if f"publication-kind-{kind}" not in classes or f"publication-role-{role}" not in classes:
                            add_finding(findings, "heading.publication_hook", f"Publication kind/role hook is missing for {element_id}.")
                        expected_epub_type = {
                            "title_page": "titlepage",
                            "copyright": "copyright-page",
                            "contents": "toc",
                            "bibliography": "bibliography",
                            "notes": "endnotes",
                            "appendix": "appendix",
                        }.get(kind, "")
                        if expected_epub_type and expected_epub_type not in element.get(q(EPUB_NS, "type"), "").split():
                            add_finding(findings, "heading.epub_type", f"Publication epub:type is missing for {element_id}.")
                if element.get(q(EPUB_NS, "type")) == "noteref":
                    if element_id:
                        noteref_ids.add(element_id)
                        href = element.get("href", "")
                        noteref_targets[element_id] = href.partition("#")[2]
                        semantic_link_targets.append((spine_path, href, "noteref"))
                if element.get(q(EPUB_NS, "type")) in {"footnote", "endnote"}:
                    if element_id:
                        footnote_ids.add(element_id)
                        backlinks_by_note[element_id] = {
                            descendant.get("href", "").partition("#")[2]
                            for descendant in element.iter()
                            if descendant.get(q(EPUB_NS, "type")) == "backlink"
                            or "footnote-backlink" in descendant.get("class", "").split()
                        }
                if (
                    element.get(q(EPUB_NS, "type")) == "backlink"
                    or "footnote-backlink" in element.get("class", "").split()
                ):
                    href = element.get("href", "")
                    backlink_targets.add(href.partition("#")[2])
                    semantic_link_targets.append((spine_path, href, "backlink"))
        if heading_ids != expected_ids:
            add_finding(findings, "heading.order", "Heading anchors do not match publication-map order.")
        for section in sections:
            section_id = str(section.get("id", ""))
            expected_level = int(section.get("headingLevel", 0) or 0)
            if heading_levels.get(section_id) != expected_level:
                add_finding(findings, "heading.level", f"Heading level changed for {section_id}.")
        if meaningful_spine == 0:
            add_finding(findings, "spine.body", "No spine document contains meaningful body text.")
        if landmark_hrefs:
            first_body_path, _ = resolve(nav_path, landmark_hrefs[0])
            if first_body_path not in names:
                add_finding(findings, "landmarks.body_target", "The first bodymatter landmark is missing.")
            else:
                first_body = ET.fromstring(archive.read(first_body_path)).find(q(XHTML_NS, "body"))
                prose_lines = [
                    text_of(element)
                    for element in first_body.iter()
                    if element.tag.rsplit("}", 1)[-1] in {"p", "li", "blockquote"}
                    and text_of(element)
                ] if first_body is not None else []
                prose_text = " ".join(prose_lines)
                prose_characters = sum(character.isalnum() for character in prose_text)
                author_only_shape = (
                    prose_characters < 12
                    or (
                        len(prose_lines) <= 1
                        and (
                            prose_characters < 40
                            or not re.search(r"[.!?。！？;；:]", prose_text)
                        )
                    )
                    or prose_text.strip().isdigit()
                )
                if author_only_shape:
                    add_finding(findings, "landmarks.body_content", "The first bodymatter is empty or looks like isolated attribution/page text.")

        for source_path, href, link_kind in semantic_link_targets:
            target_path, fragment = resolve(source_path, href)
            if target_path not in names or not fragment or all_ids.get(fragment) != target_path:
                add_finding(
                    findings,
                    f"notes.{link_kind}_target",
                    f"Semantic {link_kind} target is missing or resolves to the wrong document.",
                )

        expected_note_ids = {str(note.get("id", "")) for note in notes}
        translated_note_ids = {
            str(note.get("id", ""))
            for note in notes
            if note.get("targetContentStatus") == "translated"
        }
        if translated_note_ids != expected_note_ids:
            add_finding(findings, "notes.translation_status", "Not every publication note has translated target content.")
        expected_reference_ids = {
            str(reference_id)
            for note in notes
            for reference_id in note.get("referenceIds", [])
        }
        if noteref_ids != expected_reference_ids:
            add_finding(findings, "notes.references", "Semantic note references do not match the publication map.")
        if footnote_ids != expected_note_ids:
            add_finding(findings, "notes.bodies", "Semantic note bodies do not match the publication map.")
        if backlink_targets != expected_reference_ids:
            add_finding(findings, "notes.backlinks", "Note backlinks do not resolve to every reference.")
        for note in notes:
            note_id = str(note.get("id", ""))
            note_reference_ids = {
                str(reference_id) for reference_id in note.get("referenceIds", [])
            }
            if any(noteref_targets.get(reference_id) != note_id for reference_id in note_reference_ids):
                add_finding(
                    findings,
                    "notes.reference_target",
                    f"A semantic reference does not target its contracted note: {note_id}",
                )
            if backlinks_by_note.get(note_id, set()) != note_reference_ids:
                add_finding(
                    findings,
                    "notes.note_backlinks",
                    f"A note does not backlink to exactly its contracted references: {note_id}",
                )
        for reference_id in noteref_ids:
            if reference_id not in all_ids:
                add_finding(findings, "notes.reference_anchor", f"Missing note reference anchor: {reference_id}")

        css_documents = [
            (path, archive.read(path).decode("utf-8", errors="replace"))
            for path in css_paths
            if path in names
        ]
        css = "\n".join(content for _, content in css_documents)
        for path, content in css_documents:
            for resource_url in css_resource_urls(content):
                if resource_url_is_unsafe(path, resource_url):
                    add_finding(
                        findings,
                        "resource.external",
                        "A packaged stylesheet references an external network resource.",
                    )
                elif urlsplit(resource_url.strip()).scheme.lower() not in INLINE_RESOURCE_SCHEMES:
                    validate_local_target(
                        findings,
                        names,
                        fragment_ids_by_document,
                        path,
                        resource_url,
                        "Stylesheet",
                    )
        compact_css = re.sub(r"\s+", " ", css.lower())
        unsafe_patterns = [
            (r"body\s*\{[^}]*\b(?:width|height)\s*:\s*\d+px", "css.fixed_body"),
            (r"body\s*\{[^}]*position\s*:\s*absolute", "css.absolute_body"),
            (r"body\s*\{[^}]*margin(?:-left|-right)?\s*:\s*(?:[5-9]|\d{2,})(?:em|rem)", "css.margin"),
        ]
        for pattern, code in unsafe_patterns:
            if re.search(pattern, compact_css):
                add_finding(findings, code, "Stylesheet contains a fixed-page rule unsafe for reflowable readers.")
        for required, code, message in [
            ("@media print", "css.print", "Stylesheet has no print policy."),
            ("max-width", "css.media", "Stylesheet does not constrain wide media."),
            ("overflow-wrap", "css.wrap", "Stylesheet has no long-token wrapping safeguard."),
            ("overflow-x", "css.table", "Stylesheet has no horizontal-overflow safeguard."),
        ]:
            if required not in compact_css:
                add_finding(findings, code, message)

        if any(fixed_width > 430 for fixed_width in fixed_inline_widths):
            add_finding(findings, "css.inline_width", "XHTML contains fixed inline content wider than 430px.")
        if table_count > responsive_table_wrappers:
            add_finding(findings, "css.table_wrapper", "A table has no responsive overflow wrapper.")
        try:
            viewport_smoke = rendered_viewport_smoke(archive, spine_paths)
        except Exception as error:
            viewport_smoke = [
                {
                    "width": width,
                    "renderer": "unavailable",
                    "rendererVersion": "unavailable",
                    "documents": 0,
                    "status": "failed",
                    "risks": [str(error)],
                }
                for width in (390, 430)
            ]
        for viewport in viewport_smoke:
            for risk in viewport["risks"]:
                add_finding(
                    findings,
                    f"viewport.{viewport['width']}",
                    f"{viewport['width']}px Chromium geometry smoke: {risk}.",
                )

    checks = {
        "packageManifest": not any(item["code"].startswith("manifest.") for item in findings),
        "xhtmlIntegrity": not any(item["code"].startswith("xhtml.") for item in findings),
        "resourceResolution": not any(item["code"].startswith("resource.") for item in findings),
        "metadata": not any(item["code"].startswith("metadata.") for item in findings),
        "navigation": not any(item["code"].startswith(("navigation.", "landmarks.")) for item in findings),
        "spineAndHeadings": not any(item["code"].startswith(("spine.", "heading.")) for item in findings),
        "semanticNotes": not any(item["code"].startswith("notes.") for item in findings),
        "reflowAndPrintCss": not any(item["code"].startswith("css.") for item in findings),
        "narrowViewportSmoke": not any(item["code"].startswith("viewport.") for item in findings),
    }
    return {
        "schema": "structural-readability-report-v1",
        "generatedAt": datetime.now(timezone.utc).replace(microsecond=0).isoformat(),
        "auditor": {"name": "BiblioSmith structural readability auditor", "version": "1"},
        "status": "passed" if not findings else "failed",
        "epub": epub_path.name,
        "publicationMapSchema": publication.get("schema"),
        "checks": checks,
        "metrics": {
            "publicationSections": len(sections),
            "navigationEntries": len(nav_items),
            "spineDocuments": len(spine_paths),
            "semanticNotes": len(notes),
            "sourceNoteReferences": len(expected_reference_ids),
            "builtNotes": len(footnote_ids),
            "translatedNotes": len(translated_note_ids),
            "builtNoteReferences": len(noteref_ids),
            "builtNoteBacklinks": len(backlink_targets),
            "orphanNoteBodies": len(footnote_ids - expected_note_ids),
            "orphanNoteReferences": len(noteref_ids - expected_reference_ids),
            "duplicateXhtmlIds": duplicate_xhtml_ids,
            "viewportSmoke": viewport_smoke,
            "findings": len(findings),
        },
        "findings": findings,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--epub", type=Path, required=True)
    parser.add_argument("--publication-map", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        report = audit(args.epub, args.publication_map)
    except Exception as error:  # operational failures are still durable evidence
        report = {
            "schema": "structural-readability-report-v1",
            "generatedAt": datetime.now(timezone.utc).replace(microsecond=0).isoformat(),
            "auditor": {"name": "BiblioSmith structural readability auditor", "version": "1"},
            "status": "failed",
            "epub": args.epub.name,
            "checks": {},
            "metrics": {"findings": 1},
            "findings": [
                {"severity": "error", "code": "audit.operational", "message": str(error)}
            ],
        }
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"structural-readability: {report['status']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
