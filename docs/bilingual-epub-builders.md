# The Two Bilingual EPUB Builders

Updated: 2026-07-26

This repository contains two bilingual EPUB builders. They are **separate tools
with separate input contracts and separate artifact names**, not an old and a
new version of one script. Neither can be deleted, and neither path may be
pointed at the other's script.

Both files were introduced by the same initial commit (`35401ef`). There is no
history that makes one of them a successor to the other.

## Comparison

| | Runner builder | Template builder |
| --- | --- | --- |
| Path | `tools/bibliosmith-launcher/source/scripts/build_bilingual_epub.py` | `template/epub_pipeline/common/scripts/build_bilingual_parallel_epub.py` |
| Who runs it | The launcher's `build_reading` stage, automatically | A person, through `npm run build:bilingual` |
| `--book-root` | Required | Optional; defaults to the parent of `scripts/` |
| Pairing input | `metadata/source_map.json` plus `chapters/final/*.md` | `qa/bilingual_parallel/alignment_map.json` |
| Alignment unit | Whole paragraph sequences, matched positionally | Explicit alignment units with paragraph IDs |
| Count mismatch | Falls back to whole-chapter blocks and prints `alignment=chapter-fallback` | Fails: a referenced paragraph ID must resolve |
| Edition switch | The job's output formats (`output_format_enabled(job, OUTPUT_FORMAT_BILINGUAL)`) | `state/pipeline_state.json`; a no-op when the bilingual edition is disabled |
| Artifact | `output/book_bilingual.epub` | `output/book_bilingual_parallel.epub` |
| Work directory | `output/bilingual_epub_work` | `output/epub_work_bilingual` |
| Frontmatter | Not included | `frontmatter/*.md` is rendered ahead of the aligned body |
| Tests | `tools/bibliosmith-launcher/source/scripts/tests/test_build_bilingual_epub.py` plus the Rust stage tests in `book_pipeline.rs` | `tests/test_build_bilingual_parallel_epub.py` |

## Why the launcher does not use the template builder

`prepare_bilingual_builder` in `src-tauri/src/book_pipeline.rs` copies the
runner builder into the project's `scripts/` directory and runs it with
`--book-root`; `run_build_reading_stage` then registers
`output/book_bilingual.epub` as the `reading_bilingual_epub` artifact, and the
Rust tests assert that exact filename.

Repointing that copy at the template builder would fail twice over: launcher
projects have no `qa/bilingual_parallel/alignment_map.json`, which the template
builder exits on, and the artifact it writes is named
`book_bilingual_parallel.epub`, which the runner does not look for.

## The asymmetry to be aware of

The `build_reading` stage takes its two builders from two different places:

- `prepare_reading_builder` copies `build_epub.js` and `run_python.js` from
  **`template/epub_pipeline/common/scripts/`**.
- `prepare_bilingual_builder` copies the bilingual builder from
  **`tools/bibliosmith-launcher/source/scripts/`**.

Before the rename both bilingual builders were called `build_bilingual_epub.py`,
so someone following the template directory by hand would silently get the other
algorithm and an artifact under a name the runner never registers. The template
builder's name now matches its artifact, which is what makes the two
distinguishable at a glance.

## Guarantees the runner path must keep

The bilingual build re-validates its inputs immediately before running, and this
behavior is not to be regressed:

- `metadata/source_map.json` is re-hashed and compared against the recorded
  `source_map` artifact hash; a change aborts the stage.
- Every `chapter_source` artifact is re-hashed and compared; a change aborts the
  stage.
- The builder script itself is hashed into `bilingualBuildScriptSha256`.
- A chapter whose source and target paragraph counts differ falls back to a
  whole-chapter block and reports `alignment=chapter-fallback`, which the
  launcher surfaces through its allowlisted worker markers.
