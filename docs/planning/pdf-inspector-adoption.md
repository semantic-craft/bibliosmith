# Adopting pdf-inspector for the direct-text route

Status: proposal. Measured 2026-08-02 against the live `~/Zotero/storage` corpus
on an M-series Mac with `pdf-inspector==0.2.6` and `pymupdf==1.28.0`.

## What pdf-inspector is

[`firecrawl/pdf-inspector`](https://github.com/firecrawl/pdf-inspector), MIT, a
Rust library that classifies a PDF (`text_based` / `scanned` / `image_based` /
`mixed`) and converts born-digital PDFs to structured Markdown without OCR. It
recovers headings from font-size ratios, tables from PDF drawing operators plus
text alignment, lists, code blocks, links, and multi-column reading order. Its
only PDF dependency is `lopdf`; there is no model and no network call.

It ships prebuilt `abi3` wheels for macOS arm64/x86_64, manylinux x86_64/aarch64
and win_amd64, so both our macOS box and the Windows worker install it with a
plain `uv add` — no Rust toolchain, no `maturin`. There is also a crate and an
N-API binding, which matters for the Tauri launcher.

## Which of our flows it touches

Only the **direct-text (`pdf-text` / `direct_text`) route**. It is not an OCR
engine and does not compete with Baidu PaddleOCR-VL or MinerU, and it is
unrelated to the BabelDOC layout track. It has two contact points:

1. `packages/ocr/scripts/zotero_llm_worker.py` — `sample_text_layer()` decides
   the route; `process_text_route()` produces the Markdown.
2. `packages/ocr/scripts/pdf_to_html_paddleocr.py` plus
   `preview_local_pdf_folder()` in `book_pipeline.rs` — the local-folder route,
   which today has no text-layer check at all.

Because that Markdown is what the translation engine splits into chapter units
and what `build_bilingual_epub.py` turns into `<h1>`–`<h3>` and TOC entries, its
structure propagates all the way to the finished EPUB.

## What we do today

**Classification** — `sample_text_layer()` opens 5 pages (1, 2, 3, middle, last)
with PyMuPDF, counts non-space characters, and calls it text if the total clears
`min(600, pages × 80)`. A hand-rolled `text_layer_quality()` then looks for
fullwidth ASCII and private-use glyphs to flag a degraded Chinese text layer.

**Extraction** — `process_text_route()` calls
`page.get_text("text", sort=True)` per page and emits:

```markdown
## Page 1

<raw text>

## Page 2
...
```

That is the whole conversion. There is no heading detection, no table
detection, no list detection, no link detection, and `sort=True` is a
top-to-bottom sort, not column-aware reading order.

**Local folder** — `preview_local_pdf_folder()` hardcodes `route_kind:
"remote_paddleocr"` for every PDF it finds, and `pdf_to_html_paddleocr.py`
chunks the whole book straight to the Baidu OCR API. A born-digital PDF pays
full remote OCR.

## Are we worse? Measured, not assumed

### Structure: yes, badly

14 books where both engines produced Markdown (5,328 pages):

| Signal | ours | pdf-inspector |
|---|---|---|
| Real headings | **0** | 5,751 |
| `## Page N` scaffolding headings | 5,328 | 0 |
| Table rows | **0** | 4,194 |
| Links | 2 | 989 |
| Titles recovered | 0 | 5 / 14 |
| Text volume (non-space chars) | 12,674,594 | 12,707,175 |

Same text, and every structural signal the downstream EPUB needs is at zero on
our side. Worse than zero: the 5,328 `## Page N` lines are *fake* headings that
the EPUB builder faithfully renders as `<h2>`, so a 629-page book becomes 629
TOC entries named "Page N".

Chinese structure recovery is good where it parses — a CNKI article comes back
with `# 个人信息国家保护义务及展开`, `## 一、问题提出及界定`, `## 二、…`, with
footnote markers and footnote bodies separated.

### Extraction speed: yes, 5.4×

Same 14 books, 5,328 pages: ours 56.4s, pdf-inspector 10.5s. Per book the range
is 2.9×–14.4×.

**Classification speed is not a win for us**: 173 files, ours 7.6s vs 7.7s —
a wash, because our sampler only ever touches 5 pages. Firecrawl's ~10-50ms
detection claim is against engines that parse the whole document; it does not
translate into a saving here.

### Robustness: no, we are better

173 files from the live corpus. 10 are not PDFs at all (Zotero stored JSON) and
both engines correctly reject them. Of the 163 real PDFs:

- **19 (11.7%) fail pdf-inspector outright**, mostly
  `couldn't parse input: invalid file trailer`. **18 of the 19 are Chinese**
  (CNKI/wanfang-style downloads). PyMuPDF reads all of them.
- A PyMuPDF repair-save (`doc.tobytes(garbage=3, clean=True)`) before handing
  bytes to pdf-inspector **rescues only 9 of the 19**. The other 10 stay
  unusable. This gap is real and only half-mitigable.
- **CID fonts it cannot decode**: `广松涉_2013_资本论的哲学` returns
  `!"#$%&'()*+,-./0` from `extract_text` and empty Markdown. PyMuPDF decodes the
  same file into 357k characters of clean Chinese. To its credit pdf-inspector
  flags this rather than emitting mojibake into the Markdown, but PyMuPDF wins.
- **Layout stage bails on image-backed scans that have a good text layer**:
  `Esser_1972` decodes to 660k characters of clean German via `extract_text`,
  yet `process_pdf` returns empty Markdown and flags all 110 pages for OCR.

### `pages_needing_ocr` is not trustworthy on our corpus

Spot-checking `mixed` classifications, the flagged pages are 1 page out of
hundreds and our extraction gets real text on exactly those pages (967, 331,
509, 193 chars). Combined with the two whole-book false positives above, acting
on this field would have sent 591 pages of perfectly extractable book to paid
Baidu OCR.

**Rule that falls out of this: pdf-inspector never gets to escalate to paid OCR.**
PyMuPDF's character count stays the authority on "is there a text layer";
pdf-inspector is adopted purely as the *structure* engine.

### Decided: do not wire pdf-inspector into OCR routing

The obvious next thought is to feed `pages_needing_ocr` to PaddleOCR/MinerU and
OCR only the pages that need it. Measured over 173 files, this does not pay off
on our corpus and is explicitly **not** being built:

| Bucket | Count |
|---|---|
| A — clean text, no OCR pages | 97 |
| B — whole book flagged (real scans + CID-decode failures) | 10 |
| C — genuinely mixed: flagged pages really have no text layer | **4** |
| D — false positives: flagged pages have text PyMuPDF reads fine | **15** |

Two of the four C books (`Lessig_2008`, `狄乐达_2018`) are flagged 100% / 99%,
so they are ordinary scans our existing heuristic already routes to OCR. The
only true mixed books are `Lobel_2022` and `Stokes_2019`, needing **2 pages
each**. Per-page routing is therefore a *content-completeness* feature worth
about four pages across 173 books, not a cost saving.

Against that, bucket D is severe: `阿马蒂亚·森_2013` flags 304 of 346 pages,
`墨子刻_1996` 287 of 294, `梅利曼_2004` 172 of 196 — all with a 0% blank rate,
meaning PyMuPDF reads every one of them. The top eight D books alone are ~948
pages that would be uploaded to paid OCR for nothing. These are Chinese
translated academic titles: scan image plus a good OCR text layer, which
pdf-inspector systematically reads as "needs OCR".

Firecrawl's headline pitch — classify locally, skip OCR for the ~54% that don't
need it — is the one part that does **not** transfer here, because our corpus is
dominated by exactly the document shape its classifier misreads. What transfers
is the Markdown structure engine.

Worth noting the OCR backends already cover this ground themselves: `mineru.py`
sends `is_ocr` and per-file `page_ranges`, and MinerU's `pipeline`/`vlm`
backends do their own layout, table and formula analysis inside the API.
PaddleOCR-VL OCRs whatever pages we hand it, and we already chunk by page range.
The free pre-filter we actually want is our own PyMuPDF character count — it is
just not wired into the local-folder route yet, which is T5 and needs nothing
from pdf-inspector.

### A gap that is ours alone

The local-folder route sends **100%** of PDFs to paid Baidu OCR with no
text-layer check whatsoever. On this corpus 56% classify `text_based` and our
own heuristic calls 88% extractable. That is the largest single cost and latency
win available, and it does not depend on pdf-inspector at all — pdf-inspector
just makes the resulting Markdown worth having.

## Plan

Adopt as a **hybrid, not a replacement**. Chain per document:

```
pdf-inspector  →  parse error?  →  PyMuPDF repair-save  →  retry
               →  empty / mojibake Markdown?  →  PyMuPDF text fallback
               →  route escalation decisions stay with PyMuPDF char counts
```

Sequencing: T1 gates everything (nothing merges without corpus evidence). T2 is
the shared foundation. T3/T4 ship the Zotero route. T5 is independent of T3/T4
once T2 lands and carries the cost win. T6 is investigation only.

### T1 — Real-corpus conversion benchmark harness

Blocks every other ticket. A repo tool that runs N real PDFs from
`~/Zotero/storage` through both engines and reports parse-failure rate, real vs
scaffolding heading counts, table/link counts, character volume and wall time.
Hand-built fixtures must not be the acceptance evidence — the Chinese trailer
failures and the CID-font failure are invisible to clean fixtures.

- Files: new tool under `tools/`, output to `output/`.
- Done: harness reproduces the numbers in this document on the current code.

### T2 — Hybrid extractor module

Add `pdf-inspector` to `packages/ocr` dependencies and add a single-file
`packages/ocr/pdf_text.py`, matching the existing `mineru.py` / `paddle.py`
shape. One entry point returning Markdown plus which engine produced it and why,
implementing the fallback chain above. Reuse the existing
`text_layer_quality()` to detect mojibake in pdf-inspector's output.

- Must not expose `pages_needing_ocr` as a routing input.
- Done: on the T1 corpus, zero regressions versus PyMuPDF-only — no document
  ends up with less text than today.

### T3 — Switch the Zotero `pdf-text` route to T2

`process_text_route()` uses the new module. Drop the `## Page N` scaffolding.
Page anchors, if still needed downstream, become HTML comments rather than
headings so they cannot reach the EPUB TOC.

- Check the page-anchor work (`claude/page-anchor-108`) before changing the
  page-marker format.
- Done: a converted book's EPUB TOC shows real chapter names, not "Page N".

### T4 — Running-header / TOC pollution post-pass

pdf-inspector promotes running headers and footers to headings because they sit
in a distinct font — verified output includes `#### viii Editors' Introduction`
and `#### Editors' Introduction ix`. Shipping T3 without this trades 629 fake
"Page N" TOC entries for a few hundred fake running-header entries.

- Detect lines repeating across many pages at the same heading level and demote
  them.
- Done: TOC entry count for a known book is within a sane multiple of its real
  chapter count.

### T5 — Text-layer preflight for the local-folder route

The cost ticket. Two layers, one behavior — ship both or neither, or the
capability exists but nobody can reach it.

- Rust: `preview_local_pdf_folder()` stops hardcoding `remote_paddleocr` and
  classifies, so the route preview can show `direct_text` and the existing
  override tokens still apply.
- The classifier is the **existing PyMuPDF character-count check**, the same one
  `sample_text_layer()` already uses on the Zotero side — not pdf-inspector's,
  per the decision above.
- Python: `pdf_to_html_paddleocr.py` honours a direct-text route via T2 instead
  of chunking every book to Baidu.
- Scope: folder input only. Single-file input stays out (see
  `local-pdf-input-is-folder-only`).
- Done: a born-digital local book converts with no Baidu API call.

### T6 — Investigate `has_encoding_issues` as an extra dirty-text signal

Investigation, not a code change up front. pdf-inspector sets
`has_encoding_issues` where our hand-rolled fullwidth/private-use ratios are
guessing. If it agrees with the existing `blocked_dirty_text_layer` decisions on
the real corpus it can be OR-ed in as an additional signal — never as a
replacement, and never as an escalation trigger on its own.

- Done: a written agreement/disagreement table against currently-blocked items.

## Risks

- **Chinese parse failures (11.7%)** are the main one. T2's fallback chain
  contains it: those documents keep today's PyMuPDF behaviour and lose nothing.
  They also gain nothing, so the structural win is ~88% of the corpus, not 100%.
- **Second PDF engine to keep current.** Mitigated by `pdf-inspector` being a
  single dependency-light wheel with no model assets.
- **T3 changes user-visible Markdown**, so previously converted books differ
  from newly converted ones. Worth deciding whether anything gets re-converted.
