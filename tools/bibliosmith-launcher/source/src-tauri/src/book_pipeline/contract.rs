//! The wire vocabulary: status strings, schema version tags, policy versions and subprocess command labels. Values here are contracts with the state file, the frontend and the worker scripts -- changing a string is a breaking change, so they live apart from the code that reads them.

pub(crate) const STATUS_ROUTED: &str = "routed";
pub(crate) const STATUS_PENDING: &str = "pending";
pub(crate) const STATUS_READY: &str = "ready";
pub(crate) const STATUS_RUNNING: &str = "running";
pub(crate) const STATUS_WAITING_FOR_APPROVAL: &str = "waiting_for_approval";
pub(crate) const STATUS_COMPLETED: &str = "completed";
pub(crate) const STATUS_FAILED: &str = "failed";
pub(crate) const STATUS_BLOCKED: &str = "blocked";
pub(crate) const STATUS_PARTIAL: &str = "partial";
pub(crate) const STATUS_SKIPPED: &str = "skipped";
pub(crate) const STATUS_HANDOFF_RUNNING: &str = "handoff_running";
pub(crate) const STATUS_TRANSLATION_READY: &str = "translation_ready";
pub(crate) const MODE_CONVERT_THEN_TRANSLATE: &str = "convert_then_translate";
pub(crate) const MODE_TRANSLATE_ONLY: &str = "translate_only";
/// The second book track: BabelDOC translates the PDF in place and writes a
/// bilingual PDF that keeps the original layout. Only `direct_text` routes are
/// eligible, and the run is a single pass -- no split, no approval gates, no
/// EPUB build -- so it carries its own short stage list rather than falling
/// through to the translation one.
pub(crate) const MODE_LAYOUT_PRESERVING: &str = "layout_preserving";
pub(crate) const LAYOUT_PDF_COMMAND_LABEL: &str = "layout-preserving bilingual PDF";
/// How the active model slot reaches the BabelDOC wrapper. Mirrors the constants
/// at the top of `layout_pdf/cli.py`.
pub(crate) const LAYOUT_PDF_BASE_URL_ENV: &str = "LAYOUT_PDF_BASE_URL";
pub(crate) const LAYOUT_PDF_API_KEY_ENV: &str = "LAYOUT_PDF_API_KEY";
pub(crate) const LAYOUT_PDF_MODEL_ENV: &str = "LAYOUT_PDF_MODEL";
/// Warning kinds `packages/layout-pdf` may report. Mirrors the constants in
/// `layout_pdf/warnings.py`; a kind missing here is parsed and then dropped.
pub(crate) const LAYOUT_PDF_WARNING_KINDS: [&str; 2] = ["large_page", "other"];
/// Stated on every layout-track run because BabelDOC has no runtime warning for
/// it: its author and reference parsing is weak, so entries there can come back
/// merged into one paragraph.
pub(crate) const LAYOUT_PDF_REFERENCE_LIMITATION: &str =
    "BabelDOC known limitation: author and reference sections may come back with their paragraphs merged.";
/// Retired: conversion always continues into translation now, so nothing may be
/// enqueued with this mode. It stays spelled out because jobs stored before the
/// retirement still carry it, and reading them must not fall through to a
/// pipeline shape nobody named.
pub(crate) const MODE_CONVERSION_ONLY: &str = "conversion_only";
/// The modes a caller may enqueue. Anything else is refused at the queue
/// boundary rather than silently given a shape.
pub(crate) const ENQUEUEABLE_MODES: [&str; 3] = [
    MODE_CONVERT_THEN_TRANSLATE,
    MODE_TRANSLATE_ONLY,
    MODE_LAYOUT_PRESERVING,
];
pub(crate) const TRANSLATION_MODE_FAST: &str = "fast";
pub(crate) const TRANSLATION_MODE_EXPERT: &str = "expert";
pub(crate) const STATE_SCHEMA_VERSION: &str = "book-pipeline-state-v5";
pub(crate) const JOB_SCHEMA_VERSION: &str = "book-pipeline-job-v5";
pub(crate) const TERMINAL_EVENT_SCHEMA_VERSION: &str = "book-pipeline-terminal-event-v1";
pub(crate) const ITEM_INDEX_CONTRACT_VERSION: &str = "zfulltext-item-index-v1";
pub(crate) const CHUNK_CONTRACT_VERSION: &str = "zfulltext-chunk-v2";
pub(crate) const ITEM_INDEX_COMMAND_LABEL: &str = "Zotero item-scoped full-text index";
pub(crate) const ITEM_INDEX_PROFILE_COMMAND_LABEL: &str = "Zotero item index profile";
pub(crate) const ZOTERO_COLLECTION_SNAPSHOT_COMMAND_LABEL: &str = "Zotero collection snapshot";
pub(crate) const ZOTERO_COLLECTION_SNAPSHOT_SCHEMA: &str = "zotero-collection-snapshot-v1";
pub(crate) const ZOTERO_CONVERSION_COMMAND_LABEL: &str = "Zotero conversion worker";
pub(crate) const ZOTERO_ATTACH_COMMAND_LABEL: &str = "Zotero attachment upload";
/// Exit codes `zsearch` uses for its typed error categories (runtime, auth,
/// validation, not-found, network, conflict). The attach command accepts them
/// so it can fail with the CLI's own reason -- which the agent contract writes
/// to stdout, not stderr -- instead of a bare exit status. A code outside this
/// set, or a run that emits no envelope, still fails as an unexplained crash.
pub(crate) const ZOTERO_ATTACH_ACCEPTED_EXIT_CODES: [i32; 7] = [0, 1, 2, 3, 4, 5, 6];
/// Finished books that can be pushed back into Zotero as imported-file
/// attachments. Deliberately not every artifact: intermediate chapter files and
/// reports are working state, and the Markdown the conversion worker already
/// uploaded has its own attachment.
///
/// Both formats the redesign also names as deliverables are absent, and for the
/// same underlying reason -- neither is a single file this command could stand
/// behind:
///
/// - `reading_html` is registered one artifact **per chapter**
///   (`run_build_reading_stage`), so attaching it would upload one XHTML file
///   and record its key as though the HTML book had gone up. An HTML-only book
///   needs a packaged deliverable before it can be attached at all.
/// - The layout-preserving track's bilingual PDF is registered by extension as
///   the generic `pdf` kind, which any PDF under a job output root also gets --
///   a source PDF among them. Listing `pdf` would offer to attach a book's own
///   source back onto the item it came from.
///
/// Each joins the list once it has a whole-book artifact of its own.
pub(crate) const ZOTERO_ATTACHABLE_ARTIFACT_KINDS: [&str; 2] =
    ["reading_epub", "reading_bilingual_epub"];
pub(crate) const EPUB_EXTRACT_COMMAND_LABEL: &str = "EPUB chapter extraction";
/// Name suffixes marking a sidecar of supporting files for the Markdown beside
/// it: MinerU's per-part tree (`<stem>.mineru`) and the images the PaddleOCR
/// wrapper and the EPUB extractor pull out of a book (`<stem>_assets`). Both are
/// named from the Markdown's stem by the worker that writes them, both are
/// skipped when artifacts are collected, and both travel into the translation
/// project so the chapter rewrite can address them.
pub(crate) const MARKDOWN_SIDECAR_SUFFIXES: [&str; 2] = [".mineru", "_assets"];
pub(crate) const ZOTERO_WORKER_ATTACHMENT_EVIDENCE_SCHEMA: &str =
    "zotero-worker-attachment-evidence-v1";
pub(crate) const ZOTERO_WORKER_EXTRACTION_CONTRACT_VERSION: &str = "zotero-worker-extraction-v1";
pub(crate) const ZOTERO_WORKER_ATTACHMENT_EVIDENCE_MARKER: &str =
    "BOOK_PIPELINE_ATTACHMENT_EVIDENCE ";
/// The local PDF wrapper's routing plan, as it reports one line per book from
/// `--route-plan-only`. Same shape as the Zotero worker's attachment evidence:
/// a marker prefix, then one JSON object carrying its own schema version.
///
/// The plan is what stops the folder route from sending every PDF to the paid
/// engine. It is a forecast, not an instruction: the wrapper re-derives the same
/// decision from the same sampler when it actually runs, so a probe that could
/// not be run costs a pessimistic route chip in the preview and nothing else.
pub(crate) const LOCAL_PDF_ROUTE_PLAN_SCHEMA: &str = "local-pdf-route-plan-v1";
pub(crate) const LOCAL_PDF_ROUTE_PLAN_MARKER: &str = "BOOK_PIPELINE_LOCAL_PDF_ROUTE ";
pub(crate) const LOCAL_PDF_ROUTE_PLAN_COMMAND_LABEL: &str = "local PDF text-layer probe";
pub(crate) const MIGRATED_INTERRUPTED_OWNER: &str = "migrated-interrupted";
pub(crate) const SPLIT_POLICY_VERSION: &str = "split-policy-v4";
pub(crate) const STRUCTURE_POLICY_VERSION: &str = "structure-policy-v1";
pub(crate) const TASK_POLICY_VERSION: &str = "task-policy-v2";
pub(crate) const SOURCE_MAP_SCHEMA: &str = "local-reading-source-map-v2";
pub(crate) const PUBLICATION_MAP_SCHEMA: &str = "local-reading-publication-map-v1";
pub(crate) const TRANSLATION_UNIT_CONTROL_SCHEMA: &str =
    "local-reading-translation-unit-control-v2";
pub(crate) const TRANSLATION_TASK_SCHEMA: &str = "local-reading-translation-task-v2";
pub(crate) const TRANSLATION_ENGINE_RUN_SCHEMA: &str = "translation-engine-run-v1";
pub(crate) const TRANSLATION_ENGINE_REPORT_SCHEMA: &str = "translation-engine-report-v1";
pub(crate) const TRANSLATION_ENGINE_COMMAND_LABEL: &str = "translation engine";
pub(crate) const TRANSLATION_ENGINE_SAMPLE_SCHEMA: &str = "translation-engine-sample-v1";
pub(crate) const TRANSLATION_ENGINE_SAMPLE_REPORT_SCHEMA: &str =
    "translation-engine-sample-report-v1";
pub(crate) const TRANSLATION_ENGINE_SAMPLE_COMMAND_LABEL: &str = "translation engine sample";
pub(crate) const TRANSLATION_SAMPLE_COUNT: usize = 3;
pub(crate) const TRANSLATION_SAMPLE_CHARACTER_BUDGET: usize = 800;
/// Directory holding one child's engine comparison, under the job output root.
/// `collect_artifacts` skips it by this name: the comparison is evidence for a
/// routing decision, not conversion output, and the scan that walks the job
/// output tree would otherwise register it as the book's own artifacts.
pub(crate) const OCR_SAMPLE_DIR_NAME: &str = "ocr-sample";

/// Where `scripts/pdf_to_html_paddleocr.py` keeps its resumable chunk scratch,
/// relative to the job output directory. `collect_artifacts` skips it by name:
/// the chunk JSONL is working state the wrapper re-reads on resume, not a
/// deliverable, and a user reclaiming the disk would otherwise be left with
/// artifact rows pointing at files that no longer exist.
pub(crate) const WRAPPER_SCRATCH_DIR_NAME: &str = ".temp";
pub(crate) const OCR_SAMPLE_COMPARE_SCHEMA: &str = "ocr-sample-compare-v1";
pub(crate) const OCR_SAMPLE_COMPARE_REPORT_SCHEMA: &str = "ocr-sample-compare-report-v1";
pub(crate) const OCR_SAMPLE_COMPARE_COMMAND_LABEL: &str = "OCR sample compare";
pub(crate) const OCR_SAMPLE_ENGINE_PADDLEOCR: &str = "paddleocr";
pub(crate) const OCR_SAMPLE_ENGINE_MINERU: &str = "mineru";
/// How many interior pages each engine gets by default. Every sampled page is
/// a paid remote call against both engines, so the default stays small and the
/// caller raises it deliberately.
pub(crate) const OCR_SAMPLE_PAGE_COUNT: u32 = 3;
/// Mirrors MAX_SAMPLE_PAGES in packages/ocr/sample_compare.py. Both ends
/// enforce it: the UI cannot spend an unbounded amount by asking for more.
pub(crate) const OCR_SAMPLE_MAX_PAGES: u32 = 10;
/// Characters of Markdown kept per engine. Enough to judge layout handling in
/// a side-by-side panel without storing the sampled pages in full.
pub(crate) const OCR_SAMPLE_CHARACTER_BUDGET: usize = 4000;
pub(crate) const TRANSLATION_POLICY_VERSION: &str = "translation-policy-v10";
pub(crate) const TRANSLATION_ENGINE_MAX_TOKENS: u32 = 2_048;
pub(crate) const TRANSLATION_ENGINE_PLACEHOLDER_RETRIES: u32 = 1;
pub(crate) const TRANSLATION_HANDOFF_SCHEMA: &str = "local-reading-translation-handoff-v1";
pub(crate) const EXPERT_QA_HANDOFF_SCHEMA: &str = "local-reading-expert-qa-handoff-v1";
pub(crate) const EXPERT_QA_POLICY_VERSION: &str = "expert-qa-policy-v1";
pub(crate) const EXPERT_QA_FAST_RANDOM_SAMPLE_SIZE: usize = 1;
pub(crate) const EXPERT_QA_AGENT_PROFILE_ID: &str = "external-expert-qa";
pub(crate) const EXPERT_QA_SKILL_ID: &str = "expert-translation-quality";
pub(crate) const PROMOTION_MANIFEST_SCHEMA: &str = "local-reading-promotion-manifest-v1";
pub(crate) const READING_BUILD_COMMAND_LABEL: &str = "reading output builder";
pub(crate) const BILINGUAL_BUILD_COMMAND_LABEL: &str = "bilingual reading output builder";
pub(crate) const EPUBCHECK_COMMAND_LABEL: &str = "EPUBCheck";
pub(crate) const STRUCTURAL_READABILITY_COMMAND_LABEL: &str = "EPUB structural readability audit";
pub(crate) const STRUCTURAL_READABILITY_REPORT_SCHEMA: &str = "structural-readability-report-v1";
pub(crate) const DIGEST_BUILD_COMMAND_LABEL: &str = "BiblioSmith Digest";
pub(crate) const OUTPUT_FORMAT_MD: &str = "md";
pub(crate) const OUTPUT_FORMAT_HTML: &str = "html";
pub(crate) const OUTPUT_FORMAT_EPUB: &str = "epub";
pub(crate) const OUTPUT_FORMAT_BILINGUAL: &str = "bilingual";
pub(crate) const READING_VALIDATION_STATUS_START: &str =
    "<!-- book-pipeline:reading-validation:start -->";
pub(crate) const READING_VALIDATION_STATUS_END: &str =
    "<!-- book-pipeline:reading-validation:end -->";
pub(crate) const AGENT_EXECUTION_OWNER_PREFIX: &str = "agent:";
pub(crate) const SPLIT_BLOCKED_ERROR: &str = "source_changed_downstream_exists";
