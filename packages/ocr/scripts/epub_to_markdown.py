#!/usr/bin/env python3
"""EPUB -> Markdown chapter extraction for the Book Pipeline's `epub_source` route.

An EPUB already carries the structure OCR has to guess at, so this route skips
OCR entirely: the spine *is* the chapter list and the XHTML *is* the text. The
job here is to turn that into the one shape the rest of the pipeline eats -- a
single merged Markdown file plus a sidecar directory of images.

Contract with the launcher (`book_pipeline.rs`):

- `--output-dir` receives `<book stem>.md` and, when the book has images,
  `<book stem>_assets/`. The extract stage scans that directory and registers
  every `.md` it finds as a `kind="markdown"` artifact, which is the only kind
  `selected_markdown_artifact()` will hand off. One merged file per book keeps
  that selection unambiguous.
- `<stem>_assets` is the name the PaddleOCR wrapper already uses
  (`pdf_to_html_paddleocr.py`), and this route deliberately reuses it rather
  than inventing a second spelling: the handoff stage copies a sidecar sitting
  next to the chosen Markdown into the translation project's `source/`, and the
  split stage rewrites `](<stem>_assets/...)` so it still resolves from
  `chapters/src/`. One convention means both of those need one rule.
- The stem is the book's file name, not a fixed `full.md`: the handoff falls back
  to the Markdown file stem when naming the translation project, so a generic
  name would produce a generic project directory.

Chapter boundaries: every spine document contributes exactly one level-1 heading,
and nothing else in the merged file is level 1 -- headings found inside a
document are demoted, and body text that would read as a heading is escaped.
`split-policy-v3` splits at the shallowest heading level it finds, so those
level-1 headings *are* the split boundaries. The spine therefore survives into
`chapters/src/` without the split stage needing a passthrough mode or a policy
version bump.

Markdown is emitted in the forms the translation engine already protects
(`packages/translation-engine/src/translation_engine/placeholders.py`): fenced
blocks for `<pre>`, `$`/`$$` for maths, `[^id]` for footnotes, `](url)` for links
and images. Everything outside those placeholders is translatable prose, which is
exactly what should reach the model.

Usage:
    uv run --package ocr python scripts/epub_to_markdown.py \
        --input path/to/book.epub \
        --output-dir output/epub_books \
        [--book substring]

`--input` accepts either a single `.epub` file or a directory to scan for them.
No network access and no credentials: extraction is fully offline.
"""

from __future__ import annotations

import argparse
import logging
import posixpath
import re
import sys
from dataclasses import dataclass, field
from html.parser import HTMLParser
from pathlib import Path
from xml.etree import ElementTree
from zipfile import BadZipFile, ZipFile

from progress import OperationProgress


logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(message)s",
    datefmt="%H:%M:%S",
)

SIDECAR_SUFFIX = "_assets"
CONTAINER_PATH = "META-INF/container.xml"

CONTAINER_NAMESPACE = "urn:oasis:names:tc:opendocument:xmlns:container"
DUBLIN_CORE_NAMESPACE = "http://purl.org/dc/elements/1.1/"
NCX_NAMESPACE = "http://www.daisy.org/z3986/2005/ncx/"
OPF_NAMESPACE = "http://www.idpf.org/2007/opf"

NBSP = " "


class EpubExtractError(Exception):
    """A book could not be read; the caller reports it and moves to the next."""


# ---------------------------------------------------------------------------
# Minimal XHTML tree
# ---------------------------------------------------------------------------
# EPUB content documents are nominally XHTML, but books in the wild ship
# unclosed tags often enough that ElementTree refuses whole chapters. HTMLParser
# is tolerant, so the tree is built from it, and the XML parser is reserved for
# the package files (container.xml / OPF / NCX), which are machine-generated.

VOID_ELEMENTS = frozenset(
    {
        "area", "base", "br", "col", "embed", "hr", "img", "input",
        "link", "meta", "param", "source", "track", "wbr",
    }
)

# Tags an unclosed sibling implicitly closes, so a missing `</p>` does not
# swallow the rest of the chapter into a single paragraph.
IMPLICIT_CLOSERS = {
    "p": {"p"},
    "li": {"li"},
    "dt": {"dt", "dd"},
    "dd": {"dt", "dd"},
    "td": {"td", "th"},
    "th": {"td", "th"},
    "tr": {"tr"},
}

HEADING_TAGS = {"h1": 1, "h2": 2, "h3": 3, "h4": 4, "h5": 5, "h6": 6}

# Never rendered: presentation and metadata that carry no book text.
SKIPPED_TAGS = frozenset({"head", "link", "meta", "script", "style", "title"})

BLOCK_TAGS = frozenset(
    {
        "address", "article", "aside", "blockquote", "body", "caption", "dd",
        "div", "dl", "dt", "figcaption", "figure", "footer", "h1", "h2", "h3",
        "h4", "h5", "h6", "header", "hgroup", "hr", "html", "li", "main", "nav",
        "ol", "p", "pre", "section", "table", "tbody", "td", "tfoot", "th",
        "thead", "tr", "ul",
    }
)

# Elements plausible as a footnote body. Without this, a `<div id="chapter3">`
# wrapping a whole chapter could be pulled inline as somebody's footnote.
FOOTNOTE_BODY_TAGS = frozenset({"aside", "dd", "div", "li", "note", "p", "span", "td"})
FOOTNOTE_BODY_MAX_CHARACTERS = 2000


@dataclass
class Element:
    tag: str
    attrs: dict[str, str] = field(default_factory=dict)
    children: list["Element | str"] = field(default_factory=list)
    # Set on an element that serves as some note's body, so the document it
    # lives in can drop it once the chapter citing it has printed it inline.
    footnote_key: str | None = None


class _TreeBuilder(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.root = Element("#document")
        self._stack: list[Element] = [self.root]

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        tag = _local_name(tag)
        if self._stack[-1].tag in IMPLICIT_CLOSERS.get(tag, frozenset()):
            self._stack.pop()
        element = Element(tag, {_local_name(key): (value or "") for key, value in attrs})
        self._stack[-1].children.append(element)
        if tag not in VOID_ELEMENTS:
            self._stack.append(element)

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        tag = _local_name(tag)
        element = Element(tag, {_local_name(key): (value or "") for key, value in attrs})
        self._stack[-1].children.append(element)

    def handle_endtag(self, tag: str) -> None:
        tag = _local_name(tag)
        # Pop to the matching open tag when there is one; a stray `</div>` with
        # nothing open must not unwind the document root.
        for depth in range(len(self._stack) - 1, 0, -1):
            if self._stack[depth].tag == tag:
                del self._stack[depth:]
                return

    def handle_data(self, data: str) -> None:
        self._stack[-1].children.append(data)


def _local_name(name: str) -> str:
    """Drop an XML prefix: `epub:type` and `xlink:href` are the ones that matter."""
    return name.split(":")[-1].lower()


def parse_xhtml(markup: str) -> Element:
    builder = _TreeBuilder()
    builder.feed(markup)
    builder.close()
    return builder.root


def iter_elements(node: Element):
    """Depth-first, document order -- the order chapter titles are picked in."""
    for child in node.children:
        if isinstance(child, Element):
            yield child
            yield from iter_elements(child)


def element_text(node: Element) -> str:
    parts: list[str] = []
    for child in node.children:
        if isinstance(child, str):
            parts.append(child)
        elif child.tag not in SKIPPED_TAGS:
            parts.append(element_text(child))
    return collapse_whitespace("".join(parts))


def collapse_whitespace(text: str) -> str:
    return re.sub(r"\s+", " ", text.replace(NBSP, " ")).strip()


# ---------------------------------------------------------------------------
# Package files
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ManifestItem:
    item_id: str
    href: str
    media_type: str
    properties: str


@dataclass(frozen=True)
class Package:
    opf_path: str
    title: str
    manifest: dict[str, ManifestItem]
    spine: list[str]
    toc_id: str


def _qualified(namespace: str, tag: str) -> str:
    return f"{{{namespace}}}{tag}"


def read_package(archive: ZipFile) -> Package:
    try:
        container = ElementTree.fromstring(archive.read(CONTAINER_PATH))
    except KeyError as error:
        raise EpubExtractError(f"{CONTAINER_PATH} is missing") from error
    except ElementTree.ParseError as error:
        raise EpubExtractError(f"{CONTAINER_PATH} is not valid XML: {error}") from error

    rootfile = container.find(
        f".//{_qualified(CONTAINER_NAMESPACE, 'rootfiles')}"
        f"/{_qualified(CONTAINER_NAMESPACE, 'rootfile')}"
    )
    opf_path = rootfile.get("full-path", "") if rootfile is not None else ""
    if not opf_path:
        raise EpubExtractError("container.xml names no package document")

    try:
        package = ElementTree.fromstring(archive.read(opf_path))
    except KeyError as error:
        raise EpubExtractError(f"package document {opf_path} is missing") from error
    except ElementTree.ParseError as error:
        raise EpubExtractError(
            f"package document {opf_path} is not valid XML: {error}"
        ) from error

    manifest: dict[str, ManifestItem] = {}
    for item in package.iter(_qualified(OPF_NAMESPACE, "item")):
        item_id = item.get("id", "")
        href = item.get("href", "")
        if item_id and href:
            manifest[item_id] = ManifestItem(
                item_id=item_id,
                href=href,
                media_type=item.get("media-type", ""),
                properties=item.get("properties", ""),
            )

    spine: list[str] = []
    toc_id = ""
    spine_element = package.find(_qualified(OPF_NAMESPACE, "spine"))
    if spine_element is not None:
        toc_id = spine_element.get("toc", "")
        for itemref in spine_element.iter(_qualified(OPF_NAMESPACE, "itemref")):
            idref = itemref.get("idref", "")
            # `linear="no"` marks auxiliary documents -- endnote pages above all.
            # They are kept: dropping them would lose text the chapters cite.
            if idref in manifest:
                spine.append(idref)
    if not spine:
        raise EpubExtractError("package document has an empty spine")

    title_element = package.find(f".//{_qualified(DUBLIN_CORE_NAMESPACE, 'title')}")
    title = collapse_whitespace(title_element.text or "") if title_element is not None else ""
    return Package(
        opf_path=opf_path,
        title=title,
        manifest=manifest,
        spine=spine,
        toc_id=toc_id,
    )


def resolve_href(base_path: str, href: str) -> str:
    """Resolve an href against the document containing it, zip-path style."""
    href = href.split("#", 1)[0]
    if not href:
        return base_path
    base_directory = posixpath.dirname(base_path)
    if not base_directory:
        return posixpath.normpath(href)
    return posixpath.normpath(posixpath.join(base_directory, href))


def read_text_entry(archive: ZipFile, path: str) -> str | None:
    try:
        payload = archive.read(path)
    except KeyError:
        return None
    for encoding in ("utf-8", "utf-16", "latin-1"):
        try:
            return payload.decode(encoding)
        except UnicodeDecodeError:
            continue
    return None


def read_toc_titles(archive: ZipFile, package: Package) -> dict[str, str]:
    """Map spine document path -> table-of-contents label.

    Only consulted when a document carries no heading of its own; a chapter whose
    title lives solely in the TOC would otherwise be split under a generated name.
    """
    titles: dict[str, str] = {}

    nav_item = next(
        (item for item in package.manifest.values() if "nav" in item.properties.split()),
        None,
    )
    if nav_item is not None:
        nav_path = resolve_href(package.opf_path, nav_item.href)
        markup = read_text_entry(archive, nav_path)
        if markup is not None:
            for anchor in iter_elements(parse_xhtml(markup)):
                href = anchor.attrs.get("href", "") if anchor.tag == "a" else ""
                label = element_text(anchor) if href else ""
                if label:
                    titles.setdefault(resolve_href(nav_path, href), label)

    ncx_item = package.manifest.get(package.toc_id)
    if ncx_item is not None:
        ncx_path = resolve_href(package.opf_path, ncx_item.href)
        try:
            ncx = ElementTree.fromstring(archive.read(ncx_path))
        except (KeyError, ElementTree.ParseError):
            ncx = None
        if ncx is not None:
            for point in ncx.iter(_qualified(NCX_NAMESPACE, "navPoint")):
                content = point.find(_qualified(NCX_NAMESPACE, "content"))
                label = point.find(
                    f"{_qualified(NCX_NAMESPACE, 'navLabel')}"
                    f"/{_qualified(NCX_NAMESPACE, 'text')}"
                )
                if content is None or label is None or not content.get("src"):
                    continue
                text = collapse_whitespace(label.text or "")
                if text:
                    titles.setdefault(resolve_href(ncx_path, content.get("src", "")), text)

    return titles


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------


@dataclass
class Assets:
    """Images pulled out of the archive, keyed by their path inside the zip."""

    directory: Path
    reference_prefix: str
    archive: ZipFile
    names: dict[str, str] = field(default_factory=dict)

    def reference(self, zip_path: str) -> str | None:
        if zip_path not in self.names:
            try:
                payload = self.archive.read(zip_path)
            except KeyError:
                return None
            self.names[zip_path] = self._write(posixpath.basename(zip_path) or "image", payload)
        return f"{self.reference_prefix}/{self.names[zip_path]}"

    def _write(self, preferred_name: str, payload: bytes) -> str:
        stem, dot, suffix = preferred_name.rpartition(".")
        if not dot:
            stem, suffix = preferred_name, ""
        taken = set(self.names.values())
        name = preferred_name
        counter = 2
        while name in taken:
            name = f"{stem}-{counter}{dot}{suffix}"
            counter += 1
        self.directory.mkdir(parents=True, exist_ok=True)
        (self.directory / name).write_bytes(payload)
        return name


@dataclass
class ChapterContext:
    document_path: str
    assets: Assets
    ordinal: int
    footnote_bodies: dict[str, str]
    footnotes: list[tuple[str, str]] = field(default_factory=list)
    footnote_keys: dict[str, str] = field(default_factory=dict)
    # Book-level, not chapter-level: a note pulled into the chapter that cites it
    # must also stop the endnotes document from printing it a second time, and
    # that document is a different chapter with its own context.
    consumed_keys: set[str] = field(default_factory=set)
    heading_offset: int = 1
    skip_element: Element | None = None
    # Depth of enclosing <sup>. A superscripted link into another document is
    # the EPUB 2 house style for a note reference -- Calibre emits exactly that,
    # with no epub:type and no role to identify it by.
    superscript_depth: int = 0

    def footnote(self, target: str) -> str | None:
        """Return the `[^key]` reference for a note, pulling its body inline.

        The body is emitted at the end of *this* chapter even when it lives in a
        separate endnotes document, because after the split stage each chapter is
        translated on its own and a reference whose definition sits twenty
        chapters away would be dangling.
        """
        body = self.footnote_bodies.get(target)
        if body is None:
            return None
        if target not in self.footnote_keys:
            self.footnote_keys[target] = f"fn-{self.ordinal}-{len(self.footnote_keys) + 1}"
            self.footnotes.append((self.footnote_keys[target], body))
            self.consumed_keys.add(target)
        return self.footnote_keys[target]


# Body text that would be read as structure at the start of a Markdown line.
# The leading `#` matters most: an un-escaped one would become a split boundary
# and tear a chapter in half.
STRUCTURAL_LINE_START = re.compile(r"^(#{1,6}\s|>|[-+*]\s|\d+[.)]\s|-{3,}\s*$|=+\s*$)")


def escape_block_text(text: str) -> str:
    match = STRUCTURAL_LINE_START.match(text)
    return f"\\{text}" if match else text


def is_footnote_reference(node: Element) -> bool:
    epub_type = node.attrs.get("type", "")
    role = node.attrs.get("role", "")
    return "noteref" in epub_type or "noteref" in role or "biblioref" in role


def render_math(node: Element) -> str:
    """MathML -> TeX when the book supplies one, otherwise verbatim symbols.

    `alttext` and a TeX `<annotation>` are the two places EPUB producers keep a
    TeX source. When neither exists the symbols are wrapped in inline code rather
    than guessed at -- both forms are protected placeholders, so either way the
    translator cannot rewrite the maths.
    """
    tex = node.attrs.get("alttext", "").strip()
    if not tex:
        annotation = next(
            (
                child
                for child in iter_elements(node)
                if child.tag == "annotation"
                and "tex" in child.attrs.get("encoding", "").lower()
            ),
            None,
        )
        if annotation is not None:
            tex = element_text(annotation)
    if tex:
        return f"$${tex}$$" if node.attrs.get("display", "") == "block" else f"${tex}$"
    symbols = element_text(node)
    return f"`{symbols}`" if symbols else ""


def render_inline(node: Element, context: ChapterContext) -> str:
    parts: list[str] = []
    for child in node.children:
        if isinstance(child, str):
            parts.append(child.replace(NBSP, " "))
        else:
            parts.append(render_inline_element(child, context))
    return "".join(parts)


def render_inline_element(node: Element, context: ChapterContext) -> str:
    if node.tag in SKIPPED_TAGS or node is context.skip_element:
        return ""
    if node.tag == "br":
        # Soft line breaks are collapsed into the paragraph: a lone newline is
        # not a protected placeholder, so the model would be free to move it.
        return " "
    if node.tag == "math":
        return render_math(node)
    if node.tag == "img":
        return render_image(node.attrs.get("src", ""), node.attrs.get("alt", ""), context)
    if node.tag == "image":
        # SVG-wrapped covers reach the bitmap through xlink:href.
        return render_image(node.attrs.get("href", ""), "", context)
    if node.tag == "code":
        text = collapse_whitespace(element_text(node))
        return f"`{text}`" if text else ""
    if node.tag == "a":
        return render_anchor(node, context)
    if node.tag == "sup":
        context.superscript_depth += 1
        try:
            return render_inline(node, context)
        finally:
            context.superscript_depth -= 1
    inner = render_inline(node, context)
    if not inner.strip():
        return inner
    if node.tag in {"em", "i", "cite", "dfn", "var"}:
        return f"*{inner}*"
    if node.tag in {"strong", "b"}:
        return f"**{inner}**"
    return inner


def render_image(source: str, alt: str, context: ChapterContext) -> str:
    if not source:
        return ""
    reference = context.assets.reference(resolve_href(context.document_path, source))
    return f"![{collapse_whitespace(alt)}]({reference})" if reference else ""


def render_anchor(node: Element, context: ChapterContext) -> str:
    href = node.attrs.get("href", "")
    label = render_inline(node, context).strip()
    if href.startswith("#") or is_footnote_reference(node) or context.superscript_depth > 0:
        target = footnote_target(node, context.document_path)
        # An unresolvable target simply falls through to the label below, so a
        # superscripted ordinary link is not mistaken for a note.
        key = context.footnote(target) if target else None
        if key:
            return f"[^{key}]"
    if not href or href.startswith("#"):
        return label
    if re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", href) is None:
        # An internal cross-reference. Every target document is merged into this
        # same file, so the href would point at a file that no longer exists:
        # keep the words, drop the broken link.
        return label
    return f"[{label}]({href})" if label else ""


def footnote_target(node: Element, document_path: str) -> str | None:
    document, separator, fragment = node.attrs.get("href", "").partition("#")
    if not separator or not fragment:
        return None
    resolved = resolve_href(document_path, document) if document else document_path
    return f"{resolved}#{fragment}"


def collect_footnote_bodies(documents: dict[str, Element], assets: Assets) -> dict[str, str]:
    """Index every element that could be a footnote body, keyed `path#id`.

    Indexing by id rather than by `epub:type="footnote"` is deliberate: EPUB 2
    books mark nothing, and their notes are plain `<p id="fn1">` rows in an
    endnotes document -- often with the id on the back-link anchor *inside* the
    paragraph rather than on the paragraph, which is why an id on an inline
    element resolves to its nearest note-shaped ancestor.
    """
    bodies: dict[str, str] = {}
    for path, root in documents.items():
        index_footnote_bodies(root, path, assets, bodies, None)
    return bodies


def index_footnote_bodies(
    node: Element,
    path: str,
    assets: Assets,
    bodies: dict[str, str],
    enclosing_body: Element | None,
) -> None:
    for child in node.children:
        if not isinstance(child, Element) or child.tag in SKIPPED_TAGS:
            continue
        identifier = child.attrs.get("id", "")
        if identifier:
            if child.tag in FOOTNOTE_BODY_TAGS:
                body, marker = child, None
            else:
                # The id sits on the note's back-link; the note is the paragraph
                # around it, minus that link -- keeping it would prefix every
                # inlined note with a stray reference number.
                body, marker = enclosing_body, child
            if body is not None:
                key = f"{path}#{identifier}"
                context = ChapterContext(
                    document_path=path,
                    assets=assets,
                    ordinal=0,
                    footnote_bodies={},
                    skip_element=marker,
                )
                text = collapse_whitespace(render_inline(body, context))
                if text and len(text) <= FOOTNOTE_BODY_MAX_CHARACTERS:
                    bodies[key] = text
                    body.footnote_key = key
        next_body = child if child.tag in FOOTNOTE_BODY_TAGS else enclosing_body
        index_footnote_bodies(child, path, assets, bodies, next_body)


def render_blocks(node: Element, context: ChapterContext, depth: int = 0) -> list[str]:
    """Render a container's children into Markdown blocks.

    Inline runs between block children are gathered into paragraphs, which is what
    makes mixed content (`<div>text<p>more</p></div>`) come out as prose rather
    than one glued line.
    """
    blocks: list[str] = []
    pending: list[str] = []

    def flush() -> None:
        text = collapse_whitespace("".join(pending))
        pending.clear()
        if text:
            blocks.append(escape_block_text(text))

    for child in node.children:
        if isinstance(child, str):
            pending.append(child.replace(NBSP, " "))
            continue
        if child.tag in SKIPPED_TAGS:
            continue
        if child.tag in BLOCK_TAGS:
            flush()
            blocks.extend(render_block_element(child, context, depth))
            continue
        pending.append(render_inline_element(child, context))
    flush()
    return blocks


def render_block_element(node: Element, context: ChapterContext, depth: int) -> list[str]:
    if node is context.skip_element:
        return []
    if node.footnote_key is not None and node.footnote_key in context.consumed_keys:
        # Already emitted as a footnote definition by the chapter that cited it.
        return []

    if node.tag in HEADING_TAGS:
        text = collapse_whitespace(render_inline(node, context))
        level = min(6, HEADING_TAGS[node.tag] + context.heading_offset)
        return [f"{'#' * level} {text}"] if text else []

    if node.tag == "pre":
        return render_code_fence(node)

    if node.tag == "hr":
        return ["---"]

    if node.tag in {"ul", "ol"}:
        return render_list(node, context, depth)

    if node.tag == "table":
        return render_table(node, context)

    if node.tag == "blockquote":
        inner = render_blocks(node, context, depth)
        if not inner:
            return []
        quoted = "\n\n".join(inner).split("\n")
        return ["\n".join(f"> {line}".rstrip() for line in quoted)]

    return render_blocks(node, context, depth)


def render_code_fence(node: Element) -> list[str]:
    text = element_code_text(node).strip("\n")
    if not text.strip():
        return []
    fence = "`" * max(3, longest_backtick_run(text) + 1)
    return [f"{fence}\n{text}\n{fence}"]


def element_code_text(node: Element) -> str:
    parts: list[str] = []
    for child in node.children:
        if isinstance(child, str):
            parts.append(child)
        elif child.tag == "br":
            parts.append("\n")
        elif child.tag not in SKIPPED_TAGS:
            parts.append(element_code_text(child))
    return "".join(parts).replace(NBSP, " ")


def longest_backtick_run(text: str) -> int:
    return max((len(run) for run in re.findall(r"`+", text)), default=0)


def render_list(node: Element, context: ChapterContext, depth: int) -> list[str]:
    ordered = node.tag == "ol"
    lines: list[str] = []
    number = 0
    indent = "  " * depth
    for child in node.children:
        if not isinstance(child, Element) or child.tag != "li":
            continue
        number += 1
        marker = f"{number}. " if ordered else "- "
        item = render_blocks(child, context, depth + 1)
        if not item:
            continue
        body = "\n\n".join(item).split("\n")
        lines.append(f"{indent}{marker}{body[0]}")
        continuation = indent + " " * len(marker)
        lines.extend(f"{continuation}{line}" if line else "" for line in body[1:])
    return ["\n".join(lines)] if lines else []


def render_table(node: Element, context: ChapterContext) -> list[str]:
    rows: list[list[str]] = []
    for row in iter_elements(node):
        if row.tag != "tr":
            continue
        cells = [
            collapse_whitespace(render_inline(cell, context)).replace("|", "\\|")
            for cell in row.children
            if isinstance(cell, Element) and cell.tag in {"td", "th"}
        ]
        if cells:
            rows.append(cells)
    if not rows:
        return []
    width = max(len(row) for row in rows)
    padded = [row + [""] * (width - len(row)) for row in rows]
    lines = [
        "| " + " | ".join(padded[0]) + " |",
        "| " + " | ".join(["---"] * width) + " |",
    ]
    lines.extend("| " + " | ".join(row) + " |" for row in padded[1:])
    return ["\n".join(lines)]


def render_chapter(
    root: Element,
    *,
    document_path: str,
    ordinal: int,
    assets: Assets,
    footnote_bodies: dict[str, str],
    consumed_keys: set[str],
    toc_title: str,
) -> str:
    """Render one spine document as a chapter owning exactly one level-1 heading.

    Returns an empty string for a document with no body left to render, which is
    how blank pages and an endnotes document whose notes were all pulled into
    their citing chapters stop producing empty chapters.
    """
    headings = [element for element in iter_elements(root) if element.tag in HEADING_TAGS]
    heading_title = element_text(headings[0]) if headings else ""
    title = heading_title or toc_title or f"Chapter {ordinal}"

    # Whatever headings survive have to land below the chapter heading, so the
    # shallowest of them becomes `##` and the rest keep their relative depth.
    # The heading that supplied the title is excluded: it is dropped rather than
    # repeated immediately under itself, and counting it would push every real
    # section one level deeper than it should be.
    surviving = headings[1:] if heading_title else headings
    shallowest = min((HEADING_TAGS[element.tag] for element in surviving), default=1)
    context = ChapterContext(
        document_path=document_path,
        assets=assets,
        ordinal=ordinal,
        footnote_bodies=footnote_bodies,
        consumed_keys=consumed_keys,
        heading_offset=2 - shallowest,
        skip_element=headings[0] if heading_title else None,
    )

    body = render_blocks(root, context)
    if not body and not context.footnotes:
        return ""
    blocks = [f"# {title}", *body]
    blocks.extend(f"[^{key}]: {text}" for key, text in context.footnotes)
    return "\n\n".join(block for block in blocks if block.strip())


# ---------------------------------------------------------------------------
# Book extraction
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ExtractionResult:
    markdown_path: Path
    chapters: int
    images: int


def output_stem(epub_path: Path) -> str:
    """The book's name with whitespace folded away.

    Both the Markdown file and its sidecar are named from this, and the sidecar
    name reaches the reader as the image URL. A space there would break Markdown
    link syntax and, worse, would stop the translation engine's `link_url`
    placeholder from matching -- leaving the path itself open to being
    "translated". Folding whitespace is enough; the rest of the name is already
    a legal file name because it came from one.
    """
    return re.sub(r"\s+", "_", epub_path.stem).strip("_") or "book"


def extract_book(
    epub_path: Path,
    output_dir: Path,
    progress: OperationProgress | None = None,
) -> ExtractionResult:
    output_dir.mkdir(parents=True, exist_ok=True)
    stem = output_stem(epub_path)
    markdown_path = output_dir / f"{stem}.md"
    sidecar_name = f"{stem}{SIDECAR_SUFFIX}"

    try:
        archive = ZipFile(epub_path)
    except BadZipFile as error:
        raise EpubExtractError(f"{epub_path.name} is not a readable EPUB archive") from error

    with archive:
        package = read_package(archive)
        toc_titles = read_toc_titles(archive, package)
        assets = Assets(
            directory=output_dir / sidecar_name,
            reference_prefix=sidecar_name,
            archive=archive,
        )

        documents: dict[str, Element] = {}
        order: list[str] = []
        for idref in package.spine:
            path = resolve_href(package.opf_path, package.manifest[idref].href)
            if path in documents:
                continue
            markup = read_text_entry(archive, path)
            if markup is None:
                logging.warning("Spine item %s is missing from the archive", path)
                continue
            documents[path] = parse_xhtml(markup)
            order.append(path)

        if progress is not None:
            progress.start("extracting", total=len(order))

        footnote_bodies = collect_footnote_bodies(documents, assets)
        consumed_keys: set[str] = set()

        chapters: list[str] = []
        for path in order:
            chapter = render_chapter(
                documents[path],
                document_path=path,
                ordinal=len(chapters) + 1,
                assets=assets,
                footnote_bodies=footnote_bodies,
                consumed_keys=consumed_keys,
                toc_title=toc_titles.get(path, ""),
            )
            if chapter.strip():
                chapters.append(chapter)
            if progress is not None:
                progress.advance("extracting")

        if not chapters:
            raise EpubExtractError(f"{epub_path.name} produced no chapter text")

        if progress is not None:
            progress.touch("assembling")
        markdown_path.write_text("\n\n".join(chapters) + "\n", encoding="utf-8")

    return ExtractionResult(
        markdown_path=markdown_path,
        chapters=len(chapters),
        images=len(assets.names),
    )


def epub_files(input_path: Path, book_filter: str = "") -> list[Path]:
    if input_path.is_file():
        candidates = [input_path]
    else:
        candidates = sorted(
            path
            for path in input_path.iterdir()
            if path.is_file() and path.suffix.lower() == ".epub"
        )
    if book_filter:
        needle = book_filter.lower()
        candidates = [path for path in candidates if needle in path.name.lower()]
    return candidates


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Extract EPUB spine documents into a merged Markdown file"
    )
    parser.add_argument("--input", required=True, help="An .epub file, or a directory of them")
    parser.add_argument("--output-dir", required=True, help="Directory to write Markdown into")
    parser.add_argument("--book", default="", help="Process only books matching this substring")
    arguments = parser.parse_args()

    input_path = Path(arguments.input).expanduser()
    if not input_path.exists():
        logging.error("Input path does not exist: %s", input_path)
        return 2
    if input_path.is_file() and input_path.suffix.lower() != ".epub":
        logging.error("Input file is not an EPUB: %s", input_path)
        return 2

    books = epub_files(input_path, arguments.book)
    if not books:
        logging.error("No EPUB files found under %s", input_path)
        return 2

    output_dir = Path(arguments.output_dir).expanduser()
    progress = OperationProgress.from_environment("extract", "chapters")
    progress.start("starting")

    failures: list[str] = []
    for book in books:
        logging.info("Extracting %s", book.name)
        try:
            result = extract_book(book, output_dir, progress)
        except EpubExtractError as error:
            logging.error("%s: %s", book.name, error)
            failures.append(book.name)
            continue
        logging.info(
            "%s -> %s (%s chapters, %s images)",
            book.name,
            result.markdown_path.name,
            result.chapters,
            result.images,
        )

    if failures:
        logging.error("Failed books: %s", ", ".join(failures))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
