from dataclasses import dataclass
import re


PLACEHOLDER_PATTERN = re.compile(r"⟦PH_\d{6}⟧")
_INLINE_MARKDOWN_PATTERN = (
    r"(?P<front_matter>\A---[ \t]*\n.*?\n---[ \t]*(?=\n|\Z))"
    r"|(?P<fenced_code>^[ \t]{0,3}(?P<fence>`{3,}|~{3,})[^\n]*"
    r"(?:\n.*?(?:^[ \t]{0,3}(?P=fence)[ \t]*(?=\n|\Z)|\Z)|\Z))"
    r"|(?P<code>(?P<ticks>`+)[^\n]*?(?P=ticks))"
    r"|(?P<display_math>\$\$.*?\$\$)"
    r"|(?P<inline_math>\$(?!\$)(?:\\.|[^$\n])+\$)"
    r"|(?P<link_url>(?<=\]\()[^)\s]+(?=\)))"
    r"|(?P<footnote>\[\^[^\]\n]+\])"
)
_PROTECTED_INLINE_MARKDOWN = re.compile(
    _INLINE_MARKDOWN_PATTERN,
    re.DOTALL | re.MULTILINE,
)
_STRUCTURE_MARKDOWN = re.compile(
    r"(?P<heading_prefix>^[ \t]{0,3}#{1,6}[ \t]+)"
    r"|(?P<paragraph_break>\n[ \t]*\n(?:[ \t]*\n)*)",
    re.MULTILINE,
)
_PROTECTED_MARKDOWN = re.compile(
    _INLINE_MARKDOWN_PATTERN
    + r"|(?P<heading_prefix>^[ \t]{0,3}#{1,6}[ \t]+)"
    + r"|(?P<paragraph_break>\n[ \t]*\n(?:[ \t]*\n)*)",
    re.DOTALL | re.MULTILINE,
)


@dataclass(frozen=True)
class ProtectedMarkdown:
    text: str
    replacements: tuple[tuple[str, str], ...]

    @property
    def placeholders(self) -> tuple[str, ...]:
        return tuple(placeholder for placeholder, _ in self.replacements)

    def restore(self, translated: str) -> str:
        if tuple(PLACEHOLDER_PATTERN.findall(translated)) != self.placeholders:
            raise ValueError("placeholder validation failed")
        restored = translated
        for placeholder, original in self.replacements:
            restored = restored.replace(placeholder, original)
        return restored

def protect_markdown(text: str) -> ProtectedMarkdown:
    return _protect(text, _PROTECTED_MARKDOWN)


def protect_markdown_for_chunking(text: str) -> ProtectedMarkdown:
    """Protect inline Markdown atoms before splitting, without inflating structure."""
    return _protect(text, _PROTECTED_INLINE_MARKDOWN)


def protect_chunk_structure(
    text: str, inline_protection: ProtectedMarkdown
) -> ProtectedMarkdown:
    """Add structure protection to a chunk that already has inline placeholders."""
    replacement_map = dict(inline_protection.replacements)
    existing = tuple(PLACEHOLDER_PATTERN.findall(text))
    if any(placeholder not in replacement_map for placeholder in existing):
        raise ValueError("chunk contains unknown placeholder syntax")
    parts: list[str] = []
    cursor = 0
    next_index = len(inline_protection.replacements)
    for match in _STRUCTURE_MARKDOWN.finditer(text):
        parts.append(text[cursor : match.start()])
        placeholder = f"⟦PH_{next_index:06d}⟧"
        next_index += 1
        parts.append(placeholder)
        replacement_map[placeholder] = match.group(0)
        cursor = match.end()
    parts.append(text[cursor:])
    protected_text = "".join(parts)
    replacements = tuple(
        (placeholder, replacement_map[placeholder])
        for placeholder in PLACEHOLDER_PATTERN.findall(protected_text)
    )
    return ProtectedMarkdown(protected_text, replacements)


def _protect(text: str, pattern: re.Pattern[str]) -> ProtectedMarkdown:
    if PLACEHOLDER_PATTERN.search(text):
        raise ValueError("source contains reserved placeholder syntax")
    replacements: list[tuple[str, str]] = []
    parts: list[str] = []
    cursor = 0
    for match in pattern.finditer(text):
        parts.append(text[cursor : match.start()])
        placeholder = f"⟦PH_{len(replacements):06d}⟧"
        parts.append(placeholder)
        replacements.append((placeholder, match.group(0)))
        cursor = match.end()
    parts.append(text[cursor:])
    return ProtectedMarkdown("".join(parts), tuple(replacements))
