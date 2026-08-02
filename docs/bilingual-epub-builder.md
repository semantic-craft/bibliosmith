# Bilingual EPUB Builder

Updated: 2026-07-26

The Launcher owns one bilingual EPUB builder:

```text
tools/bibliosmith-launcher/source/scripts/build_bilingual_epub.py
```

`prepare_bilingual_builder` copies it into a local book project's `scripts/`
directory. The `build_reading` stage runs it with `--book-root` and registers
`output/reading/book_bilingual.epub` as the `reading_bilingual_epub` artifact.

## Contract

- Input alignment comes from `metadata/source_map.json` and matching
  `chapters/final/*.md` files.
- Equal paragraph counts produce paragraph-level source/target pairs.
- Paragraphs are separated by blank lines, except inside a fenced code block: a
  fence counts as one paragraph however many blank lines it holds, and renders
  as `<pre><code>` on both sides of its pair. Counting its parts separately
  would inflate one side's total and cost the whole chapter its pairing.
- A count mismatch falls back to a whole-chapter pair and reports
  `alignment=chapter-fallback`.
- The output is `output/reading/book_bilingual.epub`.
- The work directory is `output/bilingual_epub_work`.

## Integrity checks

Before building, the runner re-hashes the source map and every source chapter
against the recorded artifact hashes. The builder script hash is recorded as
`bilingualBuildScriptSha256`. A changed input therefore invalidates the prior
build rather than silently mixing revisions.

The single-language builder and its Python runtime resolver live beside this
file under `tools/bibliosmith-launcher/source/scripts/`; both are copied into
the same local project only when the reading build runs.
