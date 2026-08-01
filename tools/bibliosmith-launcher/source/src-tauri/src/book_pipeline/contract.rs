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
/// uploaded has its own attachment. BabelDOC's bilingual PDF joins this list
/// with the kind that ships alongside it (#99).
pub(crate) const ZOTERO_ATTACHABLE_ARTIFACT_KINDS: [&str; 3] =
    ["reading_epub", "reading_bilingual_epub", "reading_html"];
pub(crate) const ZOTERO_WORKER_ATTACHMENT_EVIDENCE_SCHEMA: &str =
    "zotero-worker-attachment-evidence-v1";
pub(crate) const ZOTERO_WORKER_EXTRACTION_CONTRACT_VERSION: &str = "zotero-worker-extraction-v1";
pub(crate) const ZOTERO_WORKER_ATTACHMENT_EVIDENCE_MARKER: &str =
    "BOOK_PIPELINE_ATTACHMENT_EVIDENCE ";
pub(crate) const MIGRATED_INTERRUPTED_OWNER: &str = "migrated-interrupted";
pub(crate) const SPLIT_POLICY_VERSION: &str = "split-policy-v3";
pub(crate) const TASK_POLICY_VERSION: &str = "task-policy-v1";
pub(crate) const SOURCE_MAP_SCHEMA: &str = "local-reading-source-map-v1";
pub(crate) const CHAPTER_CONTROL_SCHEMA: &str = "local-reading-chapter-control-v1";
pub(crate) const TRANSLATION_TASK_SCHEMA: &str = "local-reading-translation-task-v1";
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
pub(crate) const MAX_CUSTOM_INSTRUCTION_CHARACTERS: usize = 2000;
