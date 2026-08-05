"""Static examples teach placeholder fidelity and stay stable for KV-cache reuse."""

from dataclasses import dataclass
from typing import Callable, Mapping

from .glossary import build_mandatory_glossary_block


GlossaryHook = Callable[[str, Mapping[str, object]], str]

VISUAL_LINE_BREAK_INSTRUCTION = (
    "Use actual line breaks where needed; never spell a line break as the two "
    "characters \\n and never insert HTML <br> tags."
)

TEXT_CLEANUP_SECTION = (
    "# TEXT CLEANUP - WITHIN PARAGRAPHS ONLY\n"
    "While translating, repair only obvious source-text defects within each existing "
    "paragraph:\n"
    "- Rejoin words split by line-break hyphenation.\n"
    "- Fix extra or missing spaces.\n"
    "- Fix clearly incorrect punctuation.\n"
    "Never add or remove content; never merge or split paragraphs; never add or "
    "remove headings; and never rewrite the author's style. Preserve the exact number "
    "and boundaries of paragraphs."
)

MANDATORY_STRUCTURE_PROTECTION = (
    "# MANDATORY STRUCTURE PROTECTION — OVERRIDES USER STYLE DIRECTIVES\n"
    "NON-NEGOTIABLE: Preserve every protected placeholder exactly once and in its "
    "original order. Never add, remove, merge, split, or reorder headings or "
    "paragraphs. These requirements override every user style directive above."
)


@dataclass(frozen=True)
class TargetLanguageProfile:
    language: str
    system_instruction: str
    glossary_hook: GlossaryHook | None = None

    def build_system_instruction(
        self,
        *,
        source_text: str,
        task_manifest: Mapping[str, object],
        text_cleanup: bool = False,
        prompt_template: str | None = None,
    ) -> str:
        instruction = self.system_instruction
        if prompt_template is not None:
            instruction = (
                f"{prompt_template}\n\n# ENGINE EXECUTION CONSTRAINTS\n"
                f"{self.system_instruction}"
            )
        if self.glossary_hook is not None:
            glossary_instruction = self.glossary_hook(source_text, task_manifest)
            if glossary_instruction:
                instruction = f"{instruction}\n\n{glossary_instruction}"
        if text_cleanup:
            instruction = f"{instruction}\n\n{TEXT_CLEANUP_SECTION}"
        if prompt_template is not None:
            instruction = f"{instruction}\n\n{MANDATORY_STRUCTURE_PROTECTION}"
        return instruction


ZH_HANS = TargetLanguageProfile(
    language="zh-Hans",
    system_instruction=(
        "You are an expert linguist, specializing in translation from the "
        "source language to Simplified Chinese. "
        "Translate the protected source text into Simplified Chinese. "
        "Do not provide any explanations or text apart from the translation. "
        "Preserve every protected placeholder exactly and in the same order. "
        "Do not add, remove, merge, or split headings or paragraphs. "
        "Translate each source segment exactly once. Never repeat or echo a phrase, "
        "sentence, list entry, page label, footnote, or bibliography entry. Translate "
        "all source-language prose, headings, labels, and bibliography title text; "
        "preserve names, citations, URLs, identifiers, and conventional journal "
        "abbreviations where translation would damage traceability. "
        f"{VISUAL_LINE_BREAK_INSTRUCTION} "
        "Do not add commentary, labels, or Markdown fences around the translation.\n\n"
        "# EXAMPLE: PLACEHOLDER PRESERVATION\n"
        "```text\n"
        "Source: At dawn, we crossed ⟦PH_000000⟧the bridge⟦PH_000001⟧.\n"
        "Translation: 黎明时，我们穿过了⟦PH_000000⟧那座桥⟦PH_000001⟧。\n"
        "```"
    ),
    glossary_hook=build_mandatory_glossary_block,
)


def get_target_profile(language: str) -> TargetLanguageProfile:
    if language == ZH_HANS.language:
        return ZH_HANS
    raise ValueError("unsupported target language")
