use chrono::Local;
use fs4::{FileExt, TryLockError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    thread,
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

const STATUS_ROUTED: &str = "routed";
const STATUS_PENDING: &str = "pending";
const STATUS_READY: &str = "ready";
const STATUS_RUNNING: &str = "running";
const STATUS_WAITING_FOR_APPROVAL: &str = "waiting_for_approval";
const STATUS_COMPLETED: &str = "completed";
const STATUS_FAILED: &str = "failed";
const STATUS_BLOCKED: &str = "blocked";
const STATUS_PARTIAL: &str = "partial";
const STATUS_SKIPPED: &str = "skipped";
const STATUS_HANDOFF_RUNNING: &str = "handoff_running";
const STATUS_TRANSLATION_READY: &str = "translation_ready";
const MODE_CONVERT_THEN_TRANSLATE: &str = "convert_then_translate";
const MODE_TRANSLATE_ONLY: &str = "translate_only";
const TRANSLATION_MODE_FAST: &str = "fast";
const TRANSLATION_MODE_EXPERT: &str = "expert";
const STATE_SCHEMA_VERSION: &str = "book-pipeline-state-v5";
const JOB_SCHEMA_VERSION: &str = "book-pipeline-job-v5";
const TERMINAL_EVENT_SCHEMA_VERSION: &str = "book-pipeline-terminal-event-v1";
const ITEM_INDEX_CONTRACT_VERSION: &str = "zfulltext-item-index-v1";
const CHUNK_CONTRACT_VERSION: &str = "zfulltext-chunk-v1";
const ITEM_INDEX_COMMAND_LABEL: &str = "Zotero item-scoped full-text index";
const ITEM_INDEX_PROFILE_COMMAND_LABEL: &str = "Zotero item index profile";
const ZOTERO_COLLECTION_SNAPSHOT_COMMAND_LABEL: &str = "Zotero collection snapshot";
const ZOTERO_COLLECTION_SNAPSHOT_SCHEMA: &str = "zotero-collection-snapshot-v1";
const ZOTERO_CONVERSION_COMMAND_LABEL: &str = "Zotero conversion worker";
const ZOTERO_WORKER_ATTACHMENT_EVIDENCE_SCHEMA: &str = "zotero-worker-attachment-evidence-v1";
const ZOTERO_WORKER_EXTRACTION_CONTRACT_VERSION: &str = "zotero-worker-extraction-v1";
const ZOTERO_WORKER_ATTACHMENT_EVIDENCE_MARKER: &str = "BOOK_PIPELINE_ATTACHMENT_EVIDENCE ";
const MIGRATED_INTERRUPTED_OWNER: &str = "migrated-interrupted";
const SPLIT_POLICY_VERSION: &str = "split-policy-v1";
const TASK_POLICY_VERSION: &str = "task-policy-v1";
const SOURCE_MAP_SCHEMA: &str = "local-reading-source-map-v1";
const CHAPTER_CONTROL_SCHEMA: &str = "local-reading-chapter-control-v1";
const TRANSLATION_TASK_SCHEMA: &str = "local-reading-translation-task-v1";
const TRANSLATION_ENGINE_RUN_SCHEMA: &str = "translation-engine-run-v1";
const TRANSLATION_ENGINE_REPORT_SCHEMA: &str = "translation-engine-report-v1";
const TRANSLATION_ENGINE_COMMAND_LABEL: &str = "translation engine";
const TRANSLATION_ENGINE_SAMPLE_SCHEMA: &str = "translation-engine-sample-v1";
const TRANSLATION_ENGINE_SAMPLE_REPORT_SCHEMA: &str = "translation-engine-sample-report-v1";
const TRANSLATION_ENGINE_SAMPLE_COMMAND_LABEL: &str = "translation engine sample";
const TRANSLATION_SAMPLE_COUNT: usize = 3;
const TRANSLATION_SAMPLE_CHARACTER_BUDGET: usize = 800;
const TRANSLATION_POLICY_VERSION: &str = "translation-policy-v6";
const TRANSLATION_ENGINE_MAX_TOKENS: u32 = 2_048;
const TRANSLATION_ENGINE_PLACEHOLDER_RETRIES: u32 = 1;
const TRANSLATION_HANDOFF_SCHEMA: &str = "local-reading-translation-handoff-v1";
const EXPERT_QA_HANDOFF_SCHEMA: &str = "local-reading-expert-qa-handoff-v1";
const EXPERT_QA_POLICY_VERSION: &str = "expert-qa-policy-v1";
const EXPERT_QA_FAST_RANDOM_SAMPLE_SIZE: usize = 1;
const EXPERT_QA_AGENT_PROFILE_ID: &str = "external-expert-qa";
const EXPERT_QA_SKILL_ID: &str = "expert-translation-quality";
const PROMOTION_MANIFEST_SCHEMA: &str = "local-reading-promotion-manifest-v1";
const READING_BUILD_COMMAND_LABEL: &str = "reading output builder";
const BILINGUAL_BUILD_COMMAND_LABEL: &str = "bilingual reading output builder";
const EPUBCHECK_COMMAND_LABEL: &str = "EPUBCheck";
const DIGEST_BUILD_COMMAND_LABEL: &str = "BiblioSmith Digest";
const OUTPUT_FORMAT_MD: &str = "md";
const OUTPUT_FORMAT_HTML: &str = "html";
const OUTPUT_FORMAT_EPUB: &str = "epub";
const OUTPUT_FORMAT_BILINGUAL: &str = "bilingual";
const READING_VALIDATION_STATUS_START: &str = "<!-- book-pipeline:reading-validation:start -->";
const READING_VALIDATION_STATUS_END: &str = "<!-- book-pipeline:reading-validation:end -->";
const AGENT_EXECUTION_OWNER_PREFIX: &str = "agent:";
const SPLIT_BLOCKED_ERROR: &str = "source_changed_downstream_exists";
const MAX_CUSTOM_INSTRUCTION_CHARACTERS: usize = 2000;

fn default_output_formats() -> Vec<String> {
    [OUTPUT_FORMAT_MD, OUTPUT_FORMAT_HTML, OUTPUT_FORMAT_EPUB]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn normalize_output_formats(formats: &[String]) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for value in formats {
        let value = value.trim().to_ascii_lowercase();
        if !matches!(
            value.as_str(),
            OUTPUT_FORMAT_MD | OUTPUT_FORMAT_HTML | OUTPUT_FORMAT_EPUB | OUTPUT_FORMAT_BILINGUAL
        ) {
            return Err(format!(
                "outputFormats contains unsupported format {value}."
            ));
        }
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }
    if normalized.is_empty() {
        return Err("outputFormats must select at least one reading format.".into());
    }
    Ok(normalized)
}

fn normalize_custom_instruction(
    phase: &str,
    value: Option<String>,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    if value.chars().count() > MAX_CUSTOM_INSTRUCTION_CHARACTERS {
        return Err(format!(
            "custom_instructions_too_long: {phase} exceeds {MAX_CUSTOM_INSTRUCTION_CHARACTERS} characters."
        ));
    }
    Ok(Some(value))
}

fn normalize_custom_instructions(
    value: BookPipelineCustomInstructions,
) -> Result<Option<BookPipelineCustomInstructions>, String> {
    let normalized = BookPipelineCustomInstructions {
        translation: normalize_custom_instruction("translation", value.translation)?,
        reflection: normalize_custom_instruction("reflection", value.reflection)?,
    };
    Ok((normalized.translation.is_some() || normalized.reflection.is_some()).then_some(normalized))
}

fn validate_custom_instructions(
    value: Option<&BookPipelineCustomInstructions>,
) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    for (phase, instruction) in [
        ("translation", value.translation.as_deref()),
        ("reflection", value.reflection.as_deref()),
    ] {
        if instruction.is_some_and(|text| text.chars().count() > MAX_CUSTOM_INSTRUCTION_CHARACTERS)
        {
            return Err(format!(
                "custom_instructions_too_long: {phase} exceeds {MAX_CUSTOM_INSTRUCTION_CHARACTERS} characters."
            ));
        }
    }
    Ok(())
}

fn output_format_enabled(job: &BookPipelineJob, format: &str) -> bool {
    job.output_formats
        .iter()
        .any(|candidate| candidate == format)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineState {
    #[serde(default)]
    pub schema_version: String,
    #[serde(default)]
    pub revision: u64,
    pub jobs: Vec<BookPipelineJob>,
}

impl Default for BookPipelineState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION.into(),
            revision: 0,
            jobs: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineJob {
    #[serde(default)]
    pub schema_version: String,
    pub id: String,
    #[serde(default)]
    pub kind: String,
    pub mode: String,
    #[serde(default)]
    pub translation_mode: String,
    #[serde(default)]
    pub translation_profile_id: String,
    #[serde(default)]
    pub translation_config_id: String,
    #[serde(default)]
    pub translation_skill_ids: Vec<String>,
    #[serde(default)]
    pub second_pass_enabled: bool,
    #[serde(default)]
    pub text_cleanup: bool,
    #[serde(default)]
    pub digest_mode: bool,
    #[serde(default = "default_output_formats")]
    pub output_formats: Vec<String>,
    pub source: BookPipelineSource,
    pub route: Vec<BookPipelineRouteItem>,
    pub status: String,
    #[serde(default)]
    pub current_stage_id: String,
    pub current_step: String,
    pub last_error: Option<String>,
    pub log_summary: Vec<String>,
    pub artifacts: Vec<BookPipelineArtifact>,
    #[serde(default)]
    pub collection_items: Vec<BookPipelineCollectionItem>,
    pub output_dir: Option<String>,
    pub attempts: u32,
    #[serde(default)]
    pub stages: Vec<BookPipelineStage>,
    #[serde(default)]
    pub children: Vec<BookPipelineChildJob>,
    #[serde(default)]
    pub membership: Option<BookPipelineMembership>,
    #[serde(default)]
    pub summary: BookPipelineStatusSummary,
    #[serde(default)]
    pub progress: BookPipelineProgress,
    #[serde(default)]
    pub notification_deliveries: Vec<BookPipelineNotificationDelivery>,
    #[serde(default)]
    pub approval_references: Vec<BookPipelineApprovalReference>,
    #[serde(default)]
    pub navigation_targets: Vec<BookPipelineNavigationTarget>,
    #[serde(default)]
    pub open_target: Option<BookPipelineOpenTarget>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineStage {
    pub stage_id: String,
    pub status: String,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub safe_error: Option<BookPipelineSafeError>,
    #[serde(default)]
    pub contract_version: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub input_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    #[serde(default)]
    pub unit_summary: Option<BookPipelineUnitSummary>,
    #[serde(default)]
    pub approval_id: Option<String>,
    #[serde(default)]
    pub approval_request: Option<BookPipelineApprovalRequest>,
    #[serde(default)]
    pub execution_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_evidence: Option<BookPipelineIndexEvidence>,
    /// Auto-retry budget for this stage. `0` means "use the default policy", so
    /// state written before this field existed loads with the default rather
    /// than with a budget of zero.
    #[serde(default)]
    pub max_attempts: u32,
    /// Seconds to wait before attempt N+1. Empty means the default table.
    #[serde(default)]
    pub retry_backoff_seconds: Vec<u32>,
    /// Why the automatic retries stopped. Present only on a stage the runner has
    /// given up on, which is what distinguishes "failed, will retry itself" from
    /// "failed, waiting for you".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub give_up_reason: Option<String>,
    /// When the pending automatic retry becomes due. Written before the wait so
    /// the countdown is durable state a poll can read, not a silent sleep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineIndexEvidence {
    pub parent_item_key: String,
    pub source_sha256: String,
    pub chunk_count: u32,
    pub index_contract_version: String,
    pub chunk_contract_version: String,
    pub embedding_profile_id: String,
    pub completed_at: String,
    pub reused: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineAttachmentIdentity {
    pub collection_key: String,
    pub parent_item_key: String,
    pub pdf_attachment_key: String,
    pub parent_item_version: u64,
    pub attachment_version: u64,
    pub parent_date_modified: String,
    pub attachment_date_modified: String,
    pub content_type: String,
    pub link_mode: u32,
    pub attachment_path: String,
    pub path_exists: bool,
    pub file_size: Option<u64>,
    pub file_mtime_ns: Option<u64>,
    pub eligibility: String,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineCustomInstructions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineChildJob {
    pub id: String,
    pub parent_job_id: String,
    pub status: String,
    pub current_stage_id: String,
    pub source: BookPipelineSource,
    pub route: Vec<BookPipelineRouteItem>,
    #[serde(default)]
    pub stages: Vec<BookPipelineStage>,
    #[serde(default)]
    pub artifacts: Vec<BookPipelineArtifact>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub local_project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<BookPipelineAttachmentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_instructions: Option<BookPipelineCustomInstructions>,
    #[serde(default)]
    pub reader_evidence: Vec<BookPipelineReaderEvidence>,
}

/// The half of "validated in EPUBCheck **and a real reader**" that a machine
/// cannot run. It is evidence, not a gate: nothing here blocks promotion, and a
/// job with none is exactly as promotable as before.
///
/// The record names the artifact by kind and digest rather than by path, so it
/// carries nothing about the user's disk, and `conclusion` is a closed set
/// rather than free text so no private note can be typed into a durable,
/// exportable record. `stale` is derived, never stored by the caller: rebuild
/// the EPUB and the digest stops matching, so one reading session cannot vouch
/// for every later version of the book.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineReaderEvidence {
    pub reader: String,
    pub reader_version: String,
    pub artifact_kind: String,
    pub artifact_sha256: String,
    pub conclusion: String,
    pub recorded_at: String,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineMembership {
    pub revision: u32,
    pub frozen_at: Option<String>,
    pub discovery_stage_id: String,
    pub child_job_ids: Vec<String>,
    #[serde(default)]
    pub collection_key: String,
    #[serde(default)]
    pub snapshot_sha256: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineStatusSummary {
    pub total: u32,
    pub pending: u32,
    pub ready: u32,
    pub running: u32,
    pub waiting_for_approval: u32,
    pub blocked: u32,
    pub failed: u32,
    pub completed: u32,
    pub skipped: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineUnitSummary {
    pub total: u32,
    pub pending: u32,
    pub ready: u32,
    pub running: u32,
    pub blocked: u32,
    pub failed: u32,
    pub completed: u32,
    pub skipped: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineProgress {
    pub stage_total: u32,
    pub stage_completed: u32,
    pub percent: u8,
    pub active_stage_id: String,
    #[serde(default)]
    pub unit_summary: Option<BookPipelineUnitSummary>,
    /// Automatic attempts still available on the active stage. The Stages tab
    /// used to label every failure "retryable" with nothing behind it; this is
    /// the number that claim is actually worth.
    #[serde(default)]
    pub retry_attempts_remaining: u32,
    /// When the pending automatic retry is due, if one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at: Option<String>,
    /// Set once the runner has stopped retrying by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub give_up_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineTerminalEvent {
    pub schema_version: String,
    pub event_id: String,
    pub job_id: String,
    pub job_kind: String,
    pub status: String,
    pub current_stage_id: String,
    pub progress: BookPipelineProgress,
    pub summary: BookPipelineStatusSummary,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineNotificationDelivery {
    pub event_id: String,
    pub status: String,
    pub delivery_status: String,
    pub attempts: u32,
    #[serde(default)]
    pub delivered_at: Option<String>,
    #[serde(default)]
    pub safe_error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineApprovalReference {
    pub approval_id: String,
    pub gate_id: String,
    pub child_job_id: String,
    pub stage_id: String,
    pub decision: String,
    #[serde(default)]
    pub bound_artifact_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineApprovalRequest {
    pub gate_id: String,
    pub translation_mode: String,
    #[serde(default)]
    pub second_pass_enabled: bool,
    #[serde(default)]
    pub text_cleanup: bool,
    #[serde(default)]
    pub digest_mode: bool,
    #[serde(default = "default_output_formats")]
    pub output_formats: Vec<String>,
    pub provider_profile_id: Option<String>,
    pub agent_profile_id: Option<String>,
    pub config_id: String,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qa_policy: Option<String>,
    #[serde(default)]
    pub sample_evidence: BTreeMap<String, String>,
    #[serde(default)]
    pub bound_artifact_hashes: BTreeMap<String, String>,
    pub requested_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineSource {
    pub kind: String,
    pub title: Option<String>,
    pub path: Option<String>,
    pub selector: Option<String>,
    pub runner_behavior: Option<String>,
    pub translation_strategy: Option<String>,
    pub adapter_command: Option<String>,
    pub fake_zotero_items: Option<Vec<FakeZoteroItem>>,
    /// Per-route-item conversion overrides chosen in the wizard, keyed by route
    /// item id. These live on the source so durable Zotero collection jobs, whose
    /// route is recomputed from live worker evidence at run time, can re-apply
    /// them instead of silently reverting to the automatic route.
    #[serde(default)]
    pub route_overrides: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineRouteItem {
    pub id: String,
    pub title: String,
    pub source_kind: String,
    pub source_ref: String,
    pub route_kind: String,
    pub can_run: bool,
    pub blocked_reason: Option<String>,
    pub summary: String,
    /// Set when route_kind came from a user override rather than auto-routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_override: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineArtifact {
    #[serde(default)]
    pub artifact_id: String,
    pub kind: String,
    pub path: String,
    pub sha256: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub producer: BookPipelineArtifactProducer,
    #[serde(default)]
    pub input_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub source_refs: BookPipelineArtifactSourceRefs,
    #[serde(default)]
    pub privacy: String,
    #[serde(default)]
    pub validation: BookPipelineArtifactValidation,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    // One-way compatibility fields for v2 state. New evidence uses the
    // role-specific producer/sourceRefs records above.
    pub zotero_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_stage: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineArtifactProducer {
    pub child_job_id: Option<String>,
    pub stage_id: String,
    pub unit_id: Option<String>,
    pub attempt: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineArtifactSourceRefs {
    pub collection_key: Option<String>,
    pub parent_item_key: Option<String>,
    pub pdf_attachment_key: Option<String>,
    pub markdown_attachment_key: Option<String>,
    pub source_ref_sha256: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineArtifactValidation {
    pub exists: bool,
    pub nonempty: bool,
    pub hash_matches: bool,
    pub required_checks_passed: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineSafeError {
    pub code: String,
    pub summary: String,
    pub retryable: bool,
    pub attempt: u32,
    pub stage_id: String,
    pub unit_id: Option<String>,
    pub timestamp: String,
    pub diagnostic_artifact_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineNavigationTarget {
    pub target_id: String,
    pub kind: String,
    pub path: String,
    pub allowed_root: String,
    pub artifact_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineOpenTarget {
    pub target_id: String,
    pub kind: String,
    pub action_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedBookPipelineOpenTarget {
    path: PathBuf,
    kind: String,
    action_label: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineCollectionItem {
    pub id: String,
    pub title: String,
    pub route_kind: String,
    pub status: String,
    pub last_error: Option<String>,
    pub artifacts: Vec<BookPipelineArtifact>,
    pub attempts: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelinePreviewConfig {
    pub has_paddleocr_credentials: bool,
    pub has_mineru_credentials: bool,
    /// Wizard-selected conversion overrides, keyed by route item id. The same
    /// config struct feeds both preview and queue, so one field serves both.
    #[serde(default)]
    pub route_overrides: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineTranslationIntent {
    pub translation_mode: String,
    pub profile_id: String,
    pub config_id: String,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    #[serde(default)]
    pub second_pass_enabled: bool,
    #[serde(default)]
    pub text_cleanup: bool,
    #[serde(default)]
    pub digest_mode: bool,
    #[serde(default = "default_output_formats")]
    pub output_formats: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FakeZoteroItem {
    pub key: String,
    pub title: String,
    pub attachment_path: Option<String>,
    pub has_text_layer: bool,
    pub dirty_text_layer: bool,
    pub scanned: bool,
    pub already_converted: bool,
    pub prefer_mineru: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineZoteroDiscoveryResult {
    pub sources: Vec<BookPipelineSource>,
    pub log_summary: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineActionResult {
    pub ok: bool,
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineCleanupPreview {
    pub candidates: Vec<BookPipelineCleanupCandidate>,
    pub log_summary: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineCleanupCandidate {
    pub id: String,
    pub job_id: String,
    pub title: String,
    pub source_kind: String,
    pub source_ref: String,
    pub source_path: Option<String>,
    pub source_pdf_key: Option<String>,
    pub markdown_path: Option<String>,
    pub local_output_path: Option<String>,
    pub zotero_child_attachment_key: Option<String>,
    pub checks: Vec<BookPipelineCleanupEvidence>,
    pub can_approve: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineCleanupEvidence {
    pub kind: String,
    pub ok: bool,
    pub detail: String,
    pub path: Option<String>,
    pub zotero_key: Option<String>,
}

#[derive(Debug, Clone)]
struct RunnerOutput {
    log_summary: Vec<String>,
    artifacts: Vec<BookPipelineArtifact>,
    collection_items: Vec<BookPipelineCollectionItem>,
    output_dir: Option<PathBuf>,
    current_step: Option<String>,
}

#[derive(Debug, Clone)]
struct AttachmentRouteOutput {
    route: BookPipelineRouteItem,
    log_summary: Vec<String>,
    reused_artifact: Option<BookPipelineArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ZoteroWorkerAttachmentEvidence {
    schema_version: String,
    extraction_contract_version: String,
    status: String,
    route: String,
    pdf_attachment_key: String,
    parent_item_key: String,
    source_sha256: String,
    markdown_path: String,
    markdown_sha256: String,
    markdown_attachment_key: String,
}

#[derive(Debug, Clone)]
struct TranslationHandoffOutput {
    log_summary: Vec<String>,
    artifacts: Vec<BookPipelineArtifact>,
}

#[derive(Debug, Clone)]
struct ItemScopedIndexOutput {
    evidence: BookPipelineIndexEvidence,
    log_summary: Vec<String>,
}

#[derive(Debug, Clone)]
struct ItemScopedIndexInput {
    parent_item_key: String,
    source_sha256: String,
    chunk_contract_version: String,
    embedding_profile_id: String,
    markdown_artifact_id: String,
}

impl ItemScopedIndexInput {
    fn hashes(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("parentItemKey".into(), self.parent_item_key.clone()),
            ("markdownSha256".into(), self.source_sha256.clone()),
            (
                "chunkContractVersion".into(),
                self.chunk_contract_version.clone(),
            ),
            (
                "embeddingProfileId".into(),
                self.embedding_profile_id.clone(),
            ),
            (
                "markdownArtifactId".into(),
                self.markdown_artifact_id.clone(),
            ),
            (
                "indexContractVersion".into(),
                ITEM_INDEX_CONTRACT_VERSION.into(),
            ),
        ])
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemScopedIndexProfile {
    embedding_profile_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ZoteroCollectionSnapshot {
    schema_version: String,
    collection: ZoteroCollectionSnapshotIdentity,
    members: Vec<ZoteroCollectionSnapshotMember>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ZoteroCollectionSnapshotIdentity {
    key: String,
    name: String,
    version: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ZoteroCollectionSnapshotMember {
    collection_key: String,
    parent_item_key: String,
    parent_item_type: String,
    parent_item_version: u64,
    parent_date_modified: String,
    title: Option<String>,
    attachment_key: Option<String>,
    attachment_version: Option<u64>,
    attachment_date_modified: Option<String>,
    content_type: Option<String>,
    link_mode: Option<u32>,
    storage_path: Option<String>,
    attachment_path: Option<String>,
    path_exists: bool,
    file_size: Option<u64>,
    file_mtime_ns: Option<u64>,
    eligibility: String,
    reason: Option<String>,
}

trait PipelineRunner {
    fn run(&self, job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerOutput, String>;

    fn route_attachment(
        &self,
        _job: &BookPipelineJob,
        _child: &BookPipelineChildJob,
        _output_dir: &Path,
    ) -> Result<AttachmentRouteOutput, String> {
        Err("This pipeline runner does not provide per-attachment routing.".into())
    }

    fn extract_attachment(
        &self,
        _job: &BookPipelineJob,
        _child: &BookPipelineChildJob,
        _output_dir: &Path,
    ) -> Result<RunnerOutput, String> {
        Err("This pipeline runner does not provide per-attachment extraction.".into())
    }

    fn index(
        &self,
        _job: &BookPipelineJob,
        _child: &BookPipelineChildJob,
        _markdown: &BookPipelineArtifact,
        _output_dir: &Path,
    ) -> Result<ItemScopedIndexOutput, String> {
        Err("This pipeline runner does not provide item-scoped indexing.".into())
    }

    fn index_input(
        &self,
        _job: &BookPipelineJob,
        _child: &BookPipelineChildJob,
        _markdown: &BookPipelineArtifact,
        _output_dir: &Path,
    ) -> Result<ItemScopedIndexInput, String> {
        Err("This pipeline runner does not provide item-scoped index planning.".into())
    }
}

trait TranslationHandoffRunner {
    fn handoff(
        &self,
        job: &BookPipelineJob,
        artifact_path: Option<&str>,
        repo_root: &Path,
    ) -> Result<TranslationHandoffOutput, String>;

    fn handoff_attachment(
        &self,
        job: &BookPipelineJob,
        _child: &BookPipelineChildJob,
        artifact_path: &str,
        repo_root: &Path,
    ) -> Result<TranslationHandoffOutput, String> {
        self.handoff(job, Some(artifact_path), repo_root)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RunnerCommandKind {
    Fake,
    Process,
}

#[derive(Debug, Clone)]
struct RunnerCommand {
    kind: RunnerCommandKind,
    label: String,
    program: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: Option<PathBuf>,
    output_dir: PathBuf,
    attempts: u32,
    accepted_exit_codes: Vec<i32>,
}

#[derive(Debug, Clone, Default)]
struct RunnerCommandResult {
    stdout: String,
    stderr: String,
    log_summary: Vec<String>,
}

trait RunnerCommandExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String>;
}

struct SystemPipelineRunner;

struct LocalProjectHandoffRunner;

struct SystemCommandExecutor;

struct CommandPipelineRunner<E> {
    executor: E,
    book_ocr_conversion_root: Option<PathBuf>,
}

impl<E> CommandPipelineRunner<E> {
    fn new(executor: E) -> Self {
        Self {
            executor,
            book_ocr_conversion_root: None,
        }
    }

    // Production always resolves the conversion root from the pipeline config;
    // only the tests pin it to a fixture directory.
    #[cfg(test)]
    fn with_book_ocr_conversion_root(executor: E, root: PathBuf) -> Self {
        Self {
            executor,
            book_ocr_conversion_root: Some(root),
        }
    }
}

trait BookPipelineStateStore: Send + Sync {
    fn load(&self) -> Result<BookPipelineState, String>;
    fn save(&self, state: &BookPipelineState) -> Result<(), String>;
    fn job_output_dir(&self, job_id: &str) -> PathBuf;
    fn execution_owner(&self) -> Result<&str, String>;
}

#[derive(Debug)]
struct BookPipelineStore {
    state_path: PathBuf,
    output_root: PathBuf,
    execution_owner: String,
    execution_lease_root: PathBuf,
    execution_lease: Mutex<Option<Arc<File>>>,
}

struct BookPipelineStoreLock {
    path: PathBuf,
}

impl Drop for BookPipelineStoreLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl BookPipelineStore {
    fn default() -> Result<Self, String> {
        let state_dir = default_state_dir()?;
        Self::new(
            state_dir.join("jobs.json"),
            default_output_root()?,
            new_execution_owner(),
        )
    }

    #[cfg(test)]
    fn for_test(root: &Path) -> Self {
        Self::new(
            root.join("state").join("jobs.json"),
            root.join("output"),
            new_execution_owner(),
        )
        .expect("test Book Pipeline store should acquire its execution lease")
    }

    #[cfg(test)]
    fn for_test_with_owner(root: &Path, execution_owner: &str) -> Self {
        Self::new(
            root.join("state").join("jobs.json"),
            root.join("output"),
            execution_owner.into(),
        )
        .expect("test Book Pipeline store should acquire its execution lease")
    }

    fn new(
        state_path: PathBuf,
        output_root: PathBuf,
        execution_owner: String,
    ) -> Result<Self, String> {
        let state_root = state_path
            .parent()
            .ok_or_else(|| "Book Pipeline state path has no parent directory.".to_string())?;
        let execution_lease_root = state_root.join("execution-leases");
        Ok(Self {
            state_path,
            output_root,
            execution_owner,
            execution_lease_root,
            execution_lease: Mutex::new(None),
        })
    }

    fn read_state_unlocked(&self) -> Result<BookPipelineState, String> {
        if !self.state_path.exists() {
            return Ok(BookPipelineState::default());
        }
        let text = fs::read_to_string(&self.state_path).map_err(|err| err.to_string())?;
        serde_json::from_str(&text).map_err(|err| err.to_string())
    }

    fn acquire_lock(&self) -> Result<BookPipelineStoreLock, String> {
        let Some(parent) = self.state_path.parent() else {
            return Err("Book Pipeline state path has no parent directory.".into());
        };
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        let lock_path = self.state_path.with_extension("json.lock");
        for _ in 0..200 {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut lock_file) => {
                    let guard = BookPipelineStoreLock {
                        path: lock_path.clone(),
                    };
                    writeln!(lock_file, "{}", std::process::id()).map_err(|err| err.to_string())?;
                    return Ok(guard);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&lock_path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(30));
                    if stale {
                        let _ = fs::remove_file(&lock_path);
                    } else {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("Book Pipeline state store is busy.".into())
    }

    fn write_state_unlocked(&self, state: &BookPipelineState) -> Result<(), String> {
        let parent = self
            .state_path
            .parent()
            .ok_or_else(|| "Book Pipeline state path has no parent directory.".to_string())?;
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        let text = serde_json::to_string_pretty(state).map_err(|err| err.to_string())?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|err| err.to_string())?;
        temporary
            .write_all(text.as_bytes())
            .map_err(|err| err.to_string())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|err| err.to_string())?;
        temporary
            .persist(&self.state_path)
            .map_err(|err| err.error.to_string())?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|err| err.to_string())?;
        Ok(())
    }
}

impl BookPipelineStateStore for BookPipelineStore {
    fn load(&self) -> Result<BookPipelineState, String> {
        let mut state = self.read_state_unlocked()?;
        let needs_migration = state_needs_upgrade(&state);
        let needs_recovery = has_interrupted_running_stage(&state, &self.execution_lease_root);
        if needs_migration || needs_recovery {
            let _lock = self.acquire_lock()?;
            state = self.read_state_unlocked()?;
            let mut changed = false;
            let migrated = state_needs_upgrade(&state);
            if migrated {
                migrate_legacy_state(&mut state);
                mark_migrated_interrupted_stages(&mut state);
                changed = true;
            }
            if !migrated && recover_interrupted_stages(&mut state, &self.execution_lease_root) {
                changed = true;
            }
            if changed {
                state.revision = state.revision.saturating_add(1);
                derive_state(&mut state);
                validate_state(&state)?;
                self.write_state_unlocked(&state)?;
            }
        }
        derive_state(&mut state);
        validate_state(&state)?;
        Ok(state)
    }

    fn save(&self, state: &BookPipelineState) -> Result<(), String> {
        let _lock = self.acquire_lock()?;
        let mut previous = self.read_state_unlocked()?;
        if state_needs_upgrade(&previous) {
            migrate_legacy_state(&mut previous);
            mark_migrated_interrupted_stages(&mut previous);
        } else {
            derive_state(&mut previous);
        }
        if previous.revision != state.revision {
            return Err(format!(
                "Book Pipeline state changed concurrently: expected revision {}, found {}.",
                state.revision, previous.revision
            ));
        }

        let mut next = state.clone();
        next.schema_version = STATE_SCHEMA_VERSION.into();
        let mut migrated = false;
        for job in &mut next.jobs {
            if job_needs_upgrade(job) {
                migrate_legacy_job(job);
                migrated = true;
            }
        }
        if migrated {
            mark_migrated_interrupted_stages(&mut next);
        }
        derive_state(&mut next);
        validate_state(&next)?;
        validate_state_transitions(&previous, &next)?;
        next.revision = state.revision.saturating_add(1);
        self.write_state_unlocked(&next)
    }

    fn job_output_dir(&self, job_id: &str) -> PathBuf {
        self.output_root.join(job_id)
    }

    fn execution_owner(&self) -> Result<&str, String> {
        let mut lease = self
            .execution_lease
            .lock()
            .map_err(|_| "Book Pipeline execution lease is poisoned.".to_string())?;
        if lease.is_none() {
            *lease = Some(acquire_execution_lease(
                &self.execution_lease_root,
                &self.execution_owner,
            )?);
        }
        Ok(&self.execution_owner)
    }
}

impl Drop for BookPipelineStore {
    fn drop(&mut self) {
        let Ok(lease) = self.execution_lease.get_mut() else {
            return;
        };
        let Some(lease) = lease.take() else {
            return;
        };
        if Arc::strong_count(&lease) == 1 {
            let path = execution_lease_path(&self.execution_lease_root, &self.execution_owner);
            let _ = FileExt::unlock(lease.as_ref());
            drop(lease);
            let _ = fs::remove_file(path);
        }
    }
}

fn state_needs_upgrade(state: &BookPipelineState) -> bool {
    state.schema_version != STATE_SCHEMA_VERSION || state.jobs.iter().any(job_needs_upgrade)
}

fn job_needs_upgrade(job: &BookPipelineJob) -> bool {
    job.schema_version != JOB_SCHEMA_VERSION
        || job.translation_mode.is_empty()
        || (job.children.is_empty() && (!job.route.is_empty() || !job.collection_items.is_empty()))
        || job
            .children
            .iter()
            .any(|child| child.stages.iter().all(|stage| stage.stage_id != "index"))
        || job
            .stages
            .iter()
            .chain(job.children.iter().flat_map(|child| child.stages.iter()))
            .any(|stage| stage.contract_version.is_empty())
}

fn new_execution_owner() -> String {
    static OWNER_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = OWNER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("execution-{}-{started}-{sequence}", std::process::id())
}

fn execution_lease_path(root: &Path, execution_owner: &str) -> PathBuf {
    let digest = Sha256::digest(execution_owner.as_bytes());
    root.join(format!("{digest:x}.lock"))
}

fn acquire_execution_lease(root: &Path, execution_owner: &str) -> Result<Arc<File>, String> {
    static LEASES: OnceLock<Mutex<BTreeMap<PathBuf, Weak<File>>>> = OnceLock::new();
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let path = execution_lease_path(root, execution_owner);
    let mut leases = LEASES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "Book Pipeline execution lease registry is poisoned.".to_string())?;
    leases.retain(|_, lease| lease.strong_count() > 0);
    if let Some(lease) = leases.get(&path).and_then(Weak::upgrade) {
        return Ok(lease);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| error.to_string())?;
    match FileExt::try_lock(&file) {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(format!(
                "Book Pipeline execution owner {execution_owner} is already active."
            ));
        }
        Err(TryLockError::Error(error)) => return Err(error.to_string()),
    }
    let lease = Arc::new(file);
    leases.insert(path, Arc::downgrade(&lease));
    Ok(lease)
}

fn execution_lease_is_active(root: &Path, execution_owner: &str) -> bool {
    let path = execution_lease_path(root, execution_owner);
    let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
        return false;
    };
    match FileExt::try_lock(&file) {
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            drop(file);
            let _ = fs::remove_file(path);
            false
        }
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => true,
    }
}

fn has_interrupted_running_stage(state: &BookPipelineState, execution_lease_root: &Path) -> bool {
    state.jobs.iter().any(|job| {
        job.stages
            .iter()
            .chain(job.children.iter().flat_map(|child| child.stages.iter()))
            .any(|stage| running_stage_is_interrupted(stage, execution_lease_root))
    })
}

fn mark_migrated_interrupted_stages(state: &mut BookPipelineState) {
    for job in &mut state.jobs {
        for stage in job.stages.iter_mut().chain(
            job.children
                .iter_mut()
                .flat_map(|child| child.stages.iter_mut()),
        ) {
            if stage.status == STATUS_RUNNING && stage.execution_owner.is_none() {
                stage.execution_owner = Some(MIGRATED_INTERRUPTED_OWNER.into());
            }
        }
    }
}

fn running_stage_is_interrupted(stage: &BookPipelineStage, execution_lease_root: &Path) -> bool {
    if stage.status != STATUS_RUNNING {
        return false;
    }
    match stage.execution_owner.as_deref() {
        Some(owner) => !execution_lease_is_active(execution_lease_root, owner),
        None => true,
    }
}

fn recover_interrupted_stages(state: &mut BookPipelineState, execution_lease_root: &Path) -> bool {
    let mut recovered = false;
    let finished_at = now_label();
    for job in &mut state.jobs {
        for stage in job.stages.iter_mut().chain(
            job.children
                .iter_mut()
                .flat_map(|child| child.stages.iter_mut()),
        ) {
            if running_stage_is_interrupted(stage, execution_lease_root) {
                stage.status = STATUS_FAILED.into();
                stage.error = Some("Stage interrupted by launcher restart; retry is safe.".into());
                stage.finished_at = Some(finished_at.clone());
                stage.execution_owner = None;
                recovered = true;
            }
        }
    }
    recovered
}

fn migrate_legacy_state(state: &mut BookPipelineState) {
    state.schema_version = STATE_SCHEMA_VERSION.into();
    for job in &mut state.jobs {
        migrate_legacy_job(job);
    }
    derive_state(state);
}

fn migrate_legacy_job(job: &mut BookPipelineJob) {
    let legacy_status = job.status.clone();
    job.schema_version = JOB_SCHEMA_VERSION.into();
    if let Ok(output_formats) = normalize_output_formats(&job.output_formats) {
        job.output_formats = output_formats;
    }
    if job.translation_mode.is_empty() {
        job.translation_mode = TRANSLATION_MODE_FAST.into();
    }
    if job.kind.is_empty() {
        job.kind = if is_zotero_batch_source(&job.source) {
            "collection"
        } else {
            "single"
        }
        .into();
    }
    if job.children.is_empty() {
        job.children = if job.kind == "collection" {
            collection_children_from_job(job)
        } else {
            vec![legacy_child_from_job(job, &legacy_status)]
        };
    }
    if job.kind == "collection" {
        if job.stages.is_empty() {
            job.stages = vec![BookPipelineStage {
                stage_id: "discover".into(),
                status: STATUS_COMPLETED.into(),
                attempt: 1,
                error: None,
                contract_version: JOB_SCHEMA_VERSION.into(),
                ..BookPipelineStage::default()
            }];
        }
        if job.membership.is_none() {
            job.membership = Some(BookPipelineMembership {
                revision: 1,
                frozen_at: Some(job.updated_at.clone()),
                discovery_stage_id: "discover".into(),
                child_job_ids: job.children.iter().map(|child| child.id.clone()).collect(),
                collection_key: job.source.selector.clone().unwrap_or_default(),
                snapshot_sha256: String::new(),
            });
        }
        migrate_legacy_collection_execution_status(job, &legacy_status);
    }
    let digest_mode = job.digest_mode;
    for child in &mut job.children {
        ensure_item_index_stage(child);
        if child
            .stages
            .iter()
            .any(|stage| stage.stage_id == "validate_reading")
            && !child
                .stages
                .iter()
                .any(|stage| stage.stage_id == "build_digest")
        {
            child.stages.push(BookPipelineStage {
                stage_id: "build_digest".into(),
                status: if digest_mode {
                    STATUS_PENDING.into()
                } else {
                    STATUS_SKIPPED.into()
                },
                contract_version: JOB_SCHEMA_VERSION.into(),
                ..BookPipelineStage::default()
            });
        }
    }
    for stage in job.stages.iter_mut().chain(
        job.children
            .iter_mut()
            .flat_map(|child| child.stages.iter_mut()),
    ) {
        if stage.contract_version.is_empty() {
            stage.contract_version = JOB_SCHEMA_VERSION.into();
        }
    }
    derive_job(job);
}

fn collection_children_from_job(job: &BookPipelineJob) -> Vec<BookPipelineChildJob> {
    let mut children = Vec::new();
    for item in &job.collection_items {
        if let Some(route) = job.route.iter().find(|route| route.id == item.id) {
            children.push(collection_child_from_route(job, route, Some(item)));
        } else {
            let route = legacy_route_from_collection_item(job, item);
            children.push(collection_child_from_route(job, &route, Some(item)));
        }
    }
    children.extend(
        job.route
            .iter()
            .filter(|route| {
                route.route_kind != "translation_handoff"
                    && !job.collection_items.iter().any(|item| item.id == route.id)
            })
            .map(|route| collection_child_from_route(job, route, None)),
    );
    children
}

fn legacy_route_from_collection_item(
    job: &BookPipelineJob,
    item: &BookPipelineCollectionItem,
) -> BookPipelineRouteItem {
    let can_run = !matches!(item.status.as_str(), STATUS_BLOCKED | STATUS_SKIPPED);
    BookPipelineRouteItem {
        id: item.id.clone(),
        title: item.title.clone(),
        source_kind: "zotero_attachment".into(),
        source_ref: job
            .source
            .path
            .clone()
            .unwrap_or_else(|| format!("zotero://attachment/{}", item.id)),
        route_kind: item.route_kind.clone(),
        can_run,
        blocked_reason: (!can_run).then(|| item.last_error.clone()).flatten(),
        summary: "Recovered from legacy collection results.".into(),
        route_override: None,
    }
}

fn collection_child_from_route(
    job: &BookPipelineJob,
    route: &BookPipelineRouteItem,
    item: Option<&BookPipelineCollectionItem>,
) -> BookPipelineChildJob {
    let mut source = job.source.clone();
    source.kind = "zotero_attachment".into();
    source.title = Some(route.title.clone());
    source.path = Some(route.source_ref.clone());
    source.selector = Some(route.id.clone());
    source.fake_zotero_items = None;
    let status = item.map(|item| item.status.as_str());
    let (route_status, extract_status) = match status {
        Some(STATUS_COMPLETED) => (STATUS_COMPLETED, STATUS_COMPLETED),
        Some(STATUS_FAILED) => (STATUS_COMPLETED, STATUS_FAILED),
        Some(STATUS_BLOCKED) => (STATUS_BLOCKED, STATUS_PENDING),
        Some(STATUS_SKIPPED) => (STATUS_SKIPPED, STATUS_SKIPPED),
        Some(STATUS_RUNNING) => (STATUS_COMPLETED, STATUS_RUNNING),
        Some(STATUS_READY) => (STATUS_COMPLETED, STATUS_READY),
        _ if route.route_kind == "already_converted" => (STATUS_SKIPPED, STATUS_SKIPPED),
        _ if route.can_run => (STATUS_COMPLETED, STATUS_READY),
        _ => (STATUS_BLOCKED, STATUS_PENDING),
    };
    let mut stages = ordered_child_stage_ids(&job.mode, false)
        .into_iter()
        .map(|stage_id| BookPipelineStage {
            status: if stage_id == "route" {
                route_status.into()
            } else if stage_id == "extract" {
                extract_status.into()
            } else if (stage_id == "index" && item.is_some())
                || (stage_id == "build_digest" && !job.digest_mode)
            {
                STATUS_SKIPPED.into()
            } else {
                STATUS_PENDING.into()
            },
            stage_id: stage_id.into(),
            attempt: item.map(|item| item.attempts).unwrap_or(0),
            error: None,
            contract_version: stage_contract_version(stage_id).into(),
            ..BookPipelineStage::default()
        })
        .collect::<Vec<_>>();
    if extract_status == STATUS_SKIPPED {
        for stage in &mut stages {
            stage.status = STATUS_SKIPPED.into();
        }
    }
    let mut child = BookPipelineChildJob {
        id: format!("{}-{}", job.id, route.id),
        parent_job_id: job.id.clone(),
        status: STATUS_PENDING.into(),
        current_stage_id: "route".into(),
        source,
        route: vec![route.clone()],
        stages,
        artifacts: item.map(|item| item.artifacts.clone()).unwrap_or_default(),
        attempts: item.map(|item| item.attempts).unwrap_or(0),
        last_error: item.and_then(|item| item.last_error.clone()),
        local_project_root: None,
        source_identity: None,
        custom_instructions: None,
        reader_evidence: Vec::new(),
    };
    if let Some(stage) = child
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "extract")
    {
        stage.error = child.last_error.clone();
    }
    derive_child(&mut child);
    child
}

fn migrate_legacy_collection_execution_status(job: &mut BookPipelineJob, legacy_status: &str) {
    if !matches!(
        legacy_status,
        STATUS_HANDOFF_RUNNING | STATUS_TRANSLATION_READY
    ) {
        return;
    }
    let Some(index) = job
        .children
        .iter()
        .position(|child| {
            child
                .stages
                .iter()
                .find(|stage| stage.stage_id == "extract")
                .is_some_and(|stage| stage.status == STATUS_COMPLETED)
        })
        .or_else(|| {
            job.children
                .iter()
                .position(|child| child.status != STATUS_SKIPPED)
        })
    else {
        return;
    };
    let attempts = job.attempts.max(1);
    let digest_mode = job.digest_mode;
    let child = &mut job.children[index];
    set_stage_status(child, "route", STATUS_COMPLETED, None);
    set_stage_status(child, "extract", STATUS_COMPLETED, None);
    set_stage_status(child, "index", STATUS_SKIPPED, None);
    ensure_translation_stages(child, digest_mode);
    if legacy_status == STATUS_HANDOFF_RUNNING {
        if let Some(stage) = stage_mut(child, "handoff") {
            stage.status = STATUS_RUNNING.into();
            stage.error = None;
            stage.started_at = Some(now_label());
            stage.finished_at = None;
        }
    } else {
        set_stage_status(child, "handoff", STATUS_COMPLETED, None);
        set_stage_status(child, "split", STATUS_READY, None);
    }
    if let Some(stage) = stage_mut(
        child,
        if legacy_status == STATUS_HANDOFF_RUNNING {
            "handoff"
        } else {
            "split"
        },
    ) {
        stage.attempt = attempts;
    }
}

fn ordered_child_stage_ids(mode: &str, legacy_translation_state: bool) -> Vec<&'static str> {
    if should_handoff_after_run(mode) || legacy_translation_state {
        vec![
            "route",
            "extract",
            "index",
            "handoff",
            "split",
            "prepare",
            "approve_translation",
            "translate",
            "expert_qa",
            "approve_promotion",
            "promote",
            "build_reading",
            "validate_reading",
            "build_digest",
        ]
    } else {
        vec!["route", "extract", "index"]
    }
}

fn ensure_item_index_stage(child: &mut BookPipelineChildJob) {
    if child.stages.iter().any(|stage| stage.stage_id == "index") {
        return;
    }
    let Some(extract_index) = child
        .stages
        .iter()
        .position(|stage| stage.stage_id == "extract")
    else {
        return;
    };
    let extract_status = child.stages[extract_index].status.as_str();
    let downstream_started = child
        .stages
        .iter()
        .skip(extract_index + 1)
        .any(|stage| !matches!(stage.status.as_str(), STATUS_PENDING | STATUS_SKIPPED));
    let required = child_requires_item_index(&child.source);
    let status = if !required || extract_status == STATUS_SKIPPED || downstream_started {
        STATUS_SKIPPED
    } else if extract_status == STATUS_COMPLETED {
        STATUS_READY
    } else {
        STATUS_PENDING
    };
    child.stages.insert(
        extract_index + 1,
        BookPipelineStage {
            stage_id: "index".into(),
            status: status.into(),
            contract_version: ITEM_INDEX_CONTRACT_VERSION.into(),
            ..BookPipelineStage::default()
        },
    );
}

fn child_requires_item_index(source: &BookPipelineSource) -> bool {
    source.kind == "zotero_attachment"
}

fn stage_contract_version(stage_id: &str) -> &'static str {
    if stage_id == "index" {
        ITEM_INDEX_CONTRACT_VERSION
    } else {
        JOB_SCHEMA_VERSION
    }
}

fn legacy_child_from_job(job: &BookPipelineJob, legacy_status: &str) -> BookPipelineChildJob {
    let stage_ids = ordered_child_stage_ids(
        &job.mode,
        matches!(
            legacy_status,
            STATUS_HANDOFF_RUNNING | STATUS_TRANSLATION_READY
        ),
    );
    let current_stage_id = match legacy_status {
        STATUS_HANDOFF_RUNNING => "handoff",
        STATUS_TRANSLATION_READY => "split",
        STATUS_ROUTED => "extract",
        STATUS_BLOCKED => "route",
        _ if job.current_step.to_ascii_lowercase().contains("handoff") => "handoff",
        _ => "extract",
    };
    let current_status = match legacy_status {
        STATUS_ROUTED | STATUS_TRANSLATION_READY => STATUS_READY,
        STATUS_HANDOFF_RUNNING => STATUS_RUNNING,
        STATUS_COMPLETED => STATUS_COMPLETED,
        STATUS_FAILED => STATUS_FAILED,
        STATUS_BLOCKED => STATUS_BLOCKED,
        STATUS_PARTIAL => STATUS_PARTIAL,
        STATUS_RUNNING => STATUS_RUNNING,
        STATUS_SKIPPED => STATUS_SKIPPED,
        _ => STATUS_PENDING,
    };
    let current_index = stage_ids
        .iter()
        .position(|stage_id| *stage_id == current_stage_id)
        .unwrap_or(0);
    let stages = stage_ids
        .into_iter()
        .enumerate()
        .map(|(index, stage_id)| {
            let status = if (stage_id == "index"
                && (!child_requires_item_index(&job.source) || index < current_index))
                || (stage_id == "build_digest" && !job.digest_mode)
            {
                STATUS_SKIPPED
            } else if index < current_index {
                STATUS_COMPLETED
            } else if index == current_index {
                current_status
            } else {
                STATUS_PENDING
            };
            BookPipelineStage {
                stage_id: stage_id.into(),
                status: status.into(),
                attempt: if index <= current_index {
                    job.attempts
                } else {
                    0
                },
                error: (index == current_index)
                    .then(|| job.last_error.clone())
                    .flatten(),
                contract_version: stage_contract_version(stage_id).into(),
                ..BookPipelineStage::default()
            }
        })
        .collect();
    BookPipelineChildJob {
        id: format!("{}-child", job.id),
        parent_job_id: job.id.clone(),
        status: current_status.into(),
        current_stage_id: current_stage_id.into(),
        source: job.source.clone(),
        route: job.route.clone(),
        stages,
        artifacts: job.artifacts.clone(),
        attempts: job.attempts,
        last_error: job.last_error.clone(),
        local_project_root: None,
        source_identity: None,
        custom_instructions: None,
        reader_evidence: Vec::new(),
    }
}

fn derive_state(state: &mut BookPipelineState) {
    for job in &mut state.jobs {
        derive_job(job);
    }
}

/// Reader evidence is kept, never dropped — a re-run of `validate_reading` must
/// not quietly erase what someone sat down and checked. What changes is whether
/// it still applies: it does only while the digest it was taken against is still
/// the artifact's digest.
fn refresh_reader_evidence(child: &mut BookPipelineChildJob) {
    if child.reader_evidence.is_empty() {
        return;
    }
    let current = child
        .artifacts
        .iter()
        .filter(|artifact| artifact.producer_stage.as_deref() == Some("build_reading"))
        .filter_map(|artifact| {
            artifact
                .sha256
                .as_deref()
                .map(|sha256| (artifact.kind.clone(), sha256.to_string()))
        })
        .collect::<BTreeMap<_, _>>();
    for evidence in &mut child.reader_evidence {
        evidence.stale = current
            .get(&evidence.artifact_kind)
            .is_none_or(|sha256| *sha256 != evidence.artifact_sha256);
    }
}

/// The reader artifacts a person can actually open. Markdown and HTML are not
/// offered: "read it in Apple Books" is a claim about an EPUB.
const READER_EVIDENCE_ARTIFACT_KINDS: [&str; 2] = ["reading_epub", "reading_bilingual_epub"];
const READER_EVIDENCE_CONCLUSIONS: [&str; 2] = ["passed", "failed"];

fn record_reader_evidence(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    artifact_kind: &str,
    reader: &str,
    reader_version: &str,
    conclusion: &str,
) -> Result<BookPipelineJob, String> {
    let reader = reader.trim();
    let reader_version = reader_version.trim();
    if reader.is_empty() || reader_version.is_empty() {
        return Err("Reader evidence needs both a reader name and a reader version.".into());
    }
    if !READER_EVIDENCE_ARTIFACT_KINDS.contains(&artifact_kind) {
        return Err(format!(
            "Reader evidence can only be recorded against {}.",
            READER_EVIDENCE_ARTIFACT_KINDS.join(" or ")
        ));
    }
    if !READER_EVIDENCE_CONCLUSIONS.contains(&conclusion) {
        return Err(format!(
            "Reader evidence conclusion must be {}.",
            READER_EVIDENCE_CONCLUSIONS.join(" or ")
        ));
    }

    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
    let child = &mut state.jobs[job_index].children[child_index];
    // Read the digest here rather than taking it from the caller: evidence that
    // names a hash nobody verified would bind to nothing.
    let artifact_sha256 = child
        .artifacts
        .iter()
        .find(|artifact| {
            artifact.kind == artifact_kind
                && artifact.producer_stage.as_deref() == Some("build_reading")
        })
        .and_then(|artifact| artifact.sha256.clone())
        .ok_or_else(|| format!("This job has no built {artifact_kind} to record evidence for."))?;
    let evidence = BookPipelineReaderEvidence {
        reader: reader.to_string(),
        reader_version: reader_version.to_string(),
        artifact_kind: artifact_kind.to_string(),
        artifact_sha256,
        conclusion: conclusion.to_string(),
        recorded_at: now_label(),
        stale: false,
    };
    // One standing result per reader per artifact: re-reading the same book in
    // the same app supersedes rather than piles up.
    if let Some(existing) = child.reader_evidence.iter_mut().find(|candidate| {
        candidate.reader == evidence.reader && candidate.artifact_kind == evidence.artifact_kind
    }) {
        *existing = evidence;
    } else {
        child.reader_evidence.push(evidence);
    }

    state.jobs[job_index].current_step = format!("Recorded {reader} reader evidence");
    state.jobs[job_index].log_summary.push(format!(
        "Reader evidence recorded: {reader} {reader_version} — {conclusion}"
    ));
    state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let job = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(job)
}

fn derive_job(job: &mut BookPipelineJob) {
    enrich_job_evidence(job);
    for child in &mut job.children {
        derive_child(child);
    }
    job.summary = summarize_children(&job.children);

    if job.kind == "collection" {
        if let Some(discovery) = job
            .stages
            .iter()
            .find(|stage| stage.stage_id == "discover" && stage.status != STATUS_COMPLETED)
        {
            job.status = discovery.status.clone();
            job.current_stage_id = "discover".into();
        } else {
            job.status = aggregate_parent_status(&job.summary);
            job.current_stage_id = "children".into();
        }
    } else if let Some(child) = job.children.first() {
        job.status = child.status.clone();
        job.current_stage_id = child.current_stage_id.clone();
    }
    job.progress = derive_job_progress(job);
    sync_job_safe_errors(job);
    refresh_navigation_targets(job);
}

fn derive_job_progress(job: &BookPipelineJob) -> BookPipelineProgress {
    let stages = job
        .stages
        .iter()
        .chain(job.children.iter().flat_map(|child| child.stages.iter()))
        .collect::<Vec<_>>();
    let stage_total = stages.len() as u32;
    let stage_completed = stages
        .iter()
        .filter(|stage| matches!(stage.status.as_str(), STATUS_COMPLETED | STATUS_SKIPPED))
        .count() as u32;
    let active = [
        STATUS_RUNNING,
        STATUS_WAITING_FOR_APPROVAL,
        STATUS_READY,
        STATUS_FAILED,
        STATUS_BLOCKED,
        STATUS_PENDING,
    ]
    .into_iter()
    .find_map(|status| stages.iter().find(|stage| stage.status == status).copied());
    BookPipelineProgress {
        stage_total,
        stage_completed,
        percent: if stage_total == 0 {
            0
        } else {
            ((stage_completed * 100) / stage_total) as u8
        },
        active_stage_id: active
            .map(|stage| stage.stage_id.clone())
            .unwrap_or_else(|| job.current_stage_id.clone()),
        unit_summary: active.and_then(|stage| stage.unit_summary.clone()),
        retry_attempts_remaining: active.map(stage_attempts_remaining).unwrap_or_default(),
        next_retry_at: active.and_then(|stage| stage.next_retry_at.clone()),
        give_up_reason: active.and_then(|stage| stage.give_up_reason.clone()),
    }
}

fn derive_child(child: &mut BookPipelineChildJob) {
    refresh_reader_evidence(child);
    if let Some(stage) = child
        .stages
        .iter()
        .find(|stage| !matches!(stage.status.as_str(), STATUS_COMPLETED | STATUS_SKIPPED))
    {
        child.status = stage.status.clone();
        child.current_stage_id = stage.stage_id.clone();
        child.last_error = stage.error.clone().or_else(|| child.last_error.clone());
        return;
    }
    child.status = if child
        .stages
        .iter()
        .all(|stage| stage.status == STATUS_SKIPPED)
    {
        STATUS_SKIPPED.into()
    } else {
        STATUS_COMPLETED.into()
    };
    child.current_stage_id = child
        .stages
        .last()
        .map(|stage| stage.stage_id.clone())
        .unwrap_or_default();
}

fn enrich_job_evidence(job: &mut BookPipelineJob) {
    if job.output_dir.is_none() {
        job.output_dir = job
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "output_dir")
            .map(|artifact| artifact.path.clone());
    }
    job.artifacts
        .retain(|artifact| artifact.kind != "output_dir" && artifact.kind != "translation_project");
    for item in &mut job.collection_items {
        item.artifacts.retain(|artifact| {
            artifact.kind != "output_dir" && artifact.kind != "translation_project"
        });
    }

    let parent_source = job.source.clone();
    let created_at = job.created_at.clone();
    for child in &mut job.children {
        if child.local_project_root.is_none() {
            child.local_project_root = child
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == "translation_project")
                .map(|artifact| artifact.path.clone());
        }
        child.artifacts.retain(|artifact| {
            artifact.kind != "output_dir" && artifact.kind != "translation_project"
        });
        if child.local_project_root.is_none() {
            child.local_project_root =
                infer_project_root(&child.artifacts).map(|path| display_path(&path));
        }
        let source = child.source.clone();
        let source_identity = child.source_identity.clone();
        let stages = child.stages.clone();
        let child_id = child.id.clone();
        for artifact in &mut child.artifacts {
            enrich_artifact(
                artifact,
                Some(&child_id),
                &parent_source,
                &source,
                &stages,
                child.attempts,
                &created_at,
            );
            if let Some(source_identity) = source_identity.as_ref() {
                apply_attachment_identity(artifact, source_identity);
            }
        }
        for stage in &mut child.stages {
            stage.artifact_ids = child
                .artifacts
                .iter()
                .filter(|artifact| artifact.producer.stage_id == stage.stage_id)
                .map(|artifact| artifact.artifact_id.clone())
                .collect();
        }
    }

    let child_contexts = job
        .children
        .iter()
        .map(|child| {
            (
                child.id.clone(),
                child.source.clone(),
                child.source_identity.clone(),
                child.stages.clone(),
                child.attempts,
                child
                    .artifacts
                    .iter()
                    .map(|artifact| artifact.path.clone())
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<Vec<_>>();
    for artifact in &mut job.artifacts {
        let context = if artifact.kind == "collection_manifest" {
            None
        } else {
            child_contexts
                .iter()
                .find(|(_, _, _, _, _, paths)| paths.contains(&artifact.path))
                .or_else(|| child_contexts.first())
        };
        if let Some((child_id, source, source_identity, stages, attempts, _)) = context {
            enrich_artifact(
                artifact,
                Some(child_id),
                &parent_source,
                source,
                stages,
                *attempts,
                &created_at,
            );
            if let Some(source_identity) = source_identity.as_ref() {
                apply_attachment_identity(artifact, source_identity);
            }
        } else {
            enrich_artifact(
                artifact,
                None,
                &parent_source,
                &parent_source,
                &job.stages,
                job.attempts,
                &created_at,
            );
        }
    }
    for item in &mut job.collection_items {
        let context = child_contexts
            .iter()
            .find(|(_, source, _, _, _, _)| source.selector.as_deref() == Some(item.id.as_str()));
        for artifact in &mut item.artifacts {
            if let Some((child_id, source, source_identity, stages, attempts, _)) = context {
                enrich_artifact(
                    artifact,
                    Some(child_id),
                    &parent_source,
                    source,
                    stages,
                    *attempts,
                    &created_at,
                );
                if let Some(source_identity) = source_identity.as_ref() {
                    apply_attachment_identity(artifact, source_identity);
                }
            }
        }
    }
    for stage in &mut job.stages {
        stage.artifact_ids = job
            .artifacts
            .iter()
            .filter(|artifact| artifact.producer.stage_id == stage.stage_id)
            .map(|artifact| artifact.artifact_id.clone())
            .collect();
    }
}

fn infer_project_root(artifacts: &[BookPipelineArtifact]) -> Option<PathBuf> {
    if let Some(manifest) = artifacts
        .iter()
        .find(|artifact| artifact.kind == "source_manifest")
    {
        return Path::new(&manifest.path)
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
    }
    artifacts
        .iter()
        .find(|artifact| artifact.kind == "translation_source")
        .and_then(|artifact| Path::new(&artifact.path).parent())
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn enrich_artifact(
    artifact: &mut BookPipelineArtifact,
    child_job_id: Option<&str>,
    parent_source: &BookPipelineSource,
    source: &BookPipelineSource,
    stages: &[BookPipelineStage],
    fallback_attempt: u32,
    created_at: &str,
) {
    let path = PathBuf::from(&artifact.path);
    let metadata = fs::metadata(&path)
        .ok()
        .filter(|metadata| metadata.is_file());
    if artifact.sha256.is_none() && metadata.is_some() {
        artifact.sha256 = sha256_file(&path).ok();
    }
    if artifact.size_bytes.is_none() {
        artifact.size_bytes = metadata.as_ref().map(std::fs::Metadata::len);
    }
    let stage_id = artifact
        .producer_stage
        .clone()
        .filter(|stage| !stage.is_empty())
        .unwrap_or_else(|| artifact_default_stage(&artifact.kind).into());
    let stage = stages.iter().find(|stage| stage.stage_id == stage_id);
    if artifact.producer.stage_id.is_empty() {
        artifact.producer = BookPipelineArtifactProducer {
            child_job_id: child_job_id.map(str::to_string),
            stage_id: stage_id.clone(),
            unit_id: translation_artifact_unit_id(artifact).or_else(|| artifact_unit_id(artifact)),
            attempt: stage
                .map(|stage| stage.attempt)
                .unwrap_or(fallback_attempt)
                .max(1),
        };
    }
    artifact.producer_stage = Some(stage_id);
    if artifact.input_hashes.is_empty() {
        artifact.input_hashes = stage
            .map(|stage| stage.input_hashes.clone())
            .unwrap_or_default();
    }
    let source_ref_sha256 = source_reference_sha256(source);
    artifact
        .input_hashes
        .entry("sourceRefSha256".into())
        .or_insert_with(|| source_ref_sha256.clone());
    artifact.source_refs = BookPipelineArtifactSourceRefs {
        collection_key: matches!(
            parent_source.kind.as_str(),
            "zotero_collection" | "zotero_filter"
        )
        .then(|| parent_source.selector.clone())
        .flatten(),
        parent_item_key: None,
        pdf_attachment_key: (source.kind == "zotero_attachment")
            .then(|| source.selector.clone())
            .flatten(),
        markdown_attachment_key: artifact.zotero_key.clone(),
        source_ref_sha256,
    };
    if artifact.privacy.is_empty() {
        artifact.privacy = artifact_privacy(&artifact.kind).into();
    }
    let exists = metadata.is_some();
    let nonempty = metadata.as_ref().is_some_and(|metadata| metadata.len() > 0);
    let hash_matches = exists
        && artifact
            .sha256
            .as_deref()
            .is_some_and(|expected| sha256_file(&path).ok().as_deref() == Some(expected));
    artifact.validation = BookPipelineArtifactValidation {
        exists,
        nonempty,
        hash_matches,
        required_checks_passed: artifact
            .kind
            .contains("report")
            .then_some(hash_matches && nonempty),
    };
    if artifact.created_at.is_empty() {
        artifact.created_at = if created_at.is_empty() {
            now_label()
        } else {
            created_at.into()
        };
    }
    if artifact.artifact_id.is_empty() {
        let identity = format!(
            "{}\0{}\0{}",
            artifact.kind,
            artifact.path,
            artifact.sha256.as_deref().unwrap_or("missing")
        );
        artifact.artifact_id = format!("artifact-{}", sha256_str(&identity));
    }
}

fn apply_attachment_identity(
    artifact: &mut BookPipelineArtifact,
    identity: &BookPipelineAttachmentIdentity,
) {
    artifact.source_refs.collection_key = Some(identity.collection_key.clone());
    artifact.source_refs.parent_item_key = Some(identity.parent_item_key.clone());
    artifact.source_refs.pdf_attachment_key = Some(identity.pdf_attachment_key.clone());
}

fn source_reference_sha256(source: &BookPipelineSource) -> String {
    let value = format!(
        "{}\0{}\0{}",
        source.kind,
        source.selector.as_deref().unwrap_or(""),
        source.path.as_deref().unwrap_or("")
    );
    sha256_str(&value)
}

fn artifact_default_stage(kind: &str) -> &'static str {
    match kind {
        "collection_manifest" => "discover",
        "markdown" | "html" | "epub" | "metadata" | "index" => "extract",
        "translation_source"
        | "source_manifest"
        | "translation_draft"
        | "translation_reflection"
        | "translation_revised" => "handoff",
        "source_map" | "chapter_source" => "split",
        "glossary" | "style_profile" | "translation_task_manifest" => "prepare",
        "approval_packet" | "translation_sample_report" => "approve_translation",
        "chapter_translation" | "translation_run_manifest" | "translation_handoff" => "translate",
        "chapter_control" | "qa_report" | "expert_qa_handoff" => "expert_qa",
        "chapter_final" | "promotion_manifest" => "promote",
        "reading_markdown" | "reading_html" | "reading_epub" | "reading_bilingual_epub" => {
            "build_reading"
        }
        "qa_status" | "epubcheck_report" | "bilingual_epubcheck_report" | "layout_check_report" => {
            "validate_reading"
        }
        _ => "unknown",
    }
}

fn artifact_unit_id(artifact: &BookPipelineArtifact) -> Option<String> {
    let unit_kinds = [
        "chapter_source",
        "translation_task_manifest",
        "chapter_control",
        "chapter_final",
    ];
    unit_kinds
        .contains(&artifact.kind.as_str())
        .then(|| {
            Path::new(&artifact.path)
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .flatten()
}

fn artifact_privacy(kind: &str) -> &'static str {
    if kind.contains("diagnostic") {
        "redacted_diagnostic"
    } else if matches!(
        kind,
        "markdown"
            | "translation_source"
            | "translation_draft"
            | "translation_revised"
            | "chapter_source"
            | "chapter_translation"
            | "chapter_final"
            | "translation_sample_report"
            | "reading_markdown"
            | "reading_html"
            | "reading_epub"
            | "reading_bilingual_epub"
    ) {
        "private_text"
    } else {
        "private_metadata"
    }
}

fn sync_job_safe_errors(job: &mut BookPipelineJob) {
    for stage in job.stages.iter_mut().chain(
        job.children
            .iter_mut()
            .flat_map(|child| child.stages.iter_mut()),
    ) {
        if matches!(stage.status.as_str(), STATUS_FAILED | STATUS_BLOCKED) {
            if let Some(existing) = stage
                .safe_error
                .as_ref()
                .filter(|error| error.stage_id == stage.stage_id && error.attempt == stage.attempt)
            {
                stage.error = Some(existing.summary.clone());
                continue;
            }
            let fallback = if stage.status == STATUS_BLOCKED {
                format!("{} stage is blocked.", stage.stage_id)
            } else {
                format!("{} stage failed.", stage.stage_id)
            };
            let message = stage.error.as_deref().unwrap_or(&fallback);
            let safe_error = safe_error_from_message(&stage.stage_id, None, stage.attempt, message);
            stage.error = Some(safe_error.summary.clone());
            stage.safe_error = Some(safe_error);
        } else {
            stage.safe_error = None;
        }
    }
}

fn safe_error_from_message(
    stage_id: &str,
    unit_id: Option<&str>,
    attempt: u32,
    message: &str,
) -> BookPipelineSafeError {
    let lower = message.to_ascii_lowercase();
    // Broad: used only to classify the error as credentials-related, which is
    // a harmless category tag, not something that can leak a secret.
    let mentions_credentials = [
        "token",
        "api_key",
        "apikey",
        "api key",
        "api-key",
        "authorization",
        "bearer",
        "credential",
        "password",
        "secret",
        ".env",
        "x-amz-signature",
        "sig=",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    // Narrow: gates whether the summary text gets blanked out. A message can
    // mention credentials by name ("GEMINI_API_KEY not set") without carrying
    // an actual secret value, and should stay legible.
    let carries_a_leaked_value = message_carries_a_leaked_value(message);
    let has_private_payload = [
        "prompt",
        "provider payload",
        "provider request",
        "provider response",
        "source text",
        "translation text",
        "ocr text",
        "model response",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let code = if mentions_credentials {
        "missing_credentials"
    } else if lower.contains("source") && (lower.contains("missing") || lower.contains("not found"))
    {
        "source_missing"
    } else if lower.contains("rate limit")
        || lower.contains("quota")
        || lower.contains("provider limit")
    {
        "provider_limit"
    } else if lower.contains("empty") || lower.contains("no markdown") {
        "empty_output"
    } else if lower.contains("upload") {
        "upload_failed"
    } else if lower.contains("qa") || stage_id == "expert_qa" {
        "qa_blocked"
    } else if lower.contains("validation")
        || lower.contains("epubcheck")
        || stage_id == "validate_reading"
    {
        "validation_failed"
    } else if lower.contains("open_target_invalid") {
        "open_target_invalid"
    } else if lower.contains("interrupted") || lower.contains("restart") {
        "interrupted"
    } else if lower.contains("blocked") {
        "blocked"
    } else {
        "runner_failed"
    };
    let summary = if carries_a_leaked_value {
        "Sensitive credential or signed-request details were redacted.".into()
    } else if has_private_payload {
        "Private source, translation, prompt, or provider payload was redacted.".into()
    } else {
        redact_message_tail(message)
    };
    BookPipelineSafeError {
        code: code.into(),
        summary,
        retryable: !matches!(code, "qa_blocked"),
        attempt,
        stage_id: stage_id.into(),
        unit_id: unit_id.map(str::to_string),
        timestamp: now_label(),
        diagnostic_artifact_id: None,
    }
}

fn refresh_navigation_targets(job: &mut BookPipelineJob) {
    let mut targets = Vec::new();
    if let Some(output_dir) = job.output_dir.as_deref() {
        register_navigation_target(
            &mut targets,
            "workspace",
            Path::new(output_dir),
            Path::new(output_dir),
            None,
        );
        if job.kind == "collection" {
            if let Some(manifest) = job
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == "collection_manifest")
            {
                register_navigation_target(
                    &mut targets,
                    "collection_results",
                    Path::new(&manifest.path),
                    Path::new(output_dir),
                    Some(&manifest.artifact_id),
                );
                register_navigation_target(
                    &mut targets,
                    "verified_evidence",
                    Path::new(&manifest.path),
                    Path::new(output_dir),
                    Some(&manifest.artifact_id),
                );
            } else {
                register_navigation_target(
                    &mut targets,
                    "collection_results",
                    Path::new(output_dir),
                    Path::new(output_dir),
                    None,
                );
            }
        }
    }

    for child in &job.children {
        let Some(project_root) = child.local_project_root.as_deref().map(PathBuf::from) else {
            continue;
        };
        register_navigation_target(
            &mut targets,
            "project_workspace",
            &project_root,
            &project_root,
            None,
        );
        let validated = child
            .stages
            .iter()
            .any(|stage| stage.stage_id == "validate_reading" && stage.status == STATUS_COMPLETED);
        let reading_dirs = child
            .artifacts
            .iter()
            .filter(|artifact| {
                artifact.kind.starts_with("reading_") && artifact.validation.hash_matches
            })
            .filter_map(|artifact| Path::new(&artifact.path).parent().map(Path::to_path_buf))
            .collect::<Vec<_>>();
        if validated && !reading_dirs.is_empty() {
            let reading_dir = reading_dirs
                .iter()
                .find(|path| path.ends_with(Path::new("output").join("reading")))
                .cloned()
                .unwrap_or_else(|| reading_dirs[0].clone());
            register_navigation_target(
                &mut targets,
                "reading_output_directory",
                &reading_dir,
                &project_root,
                None,
            );
            register_navigation_target(
                &mut targets,
                "partial_results",
                &reading_dir,
                &project_root,
                None,
            );
        } else {
            register_navigation_target(
                &mut targets,
                "partial_results",
                &project_root,
                &project_root,
                None,
            );
        }

        let approval_kind = if child.current_stage_id == "approve_promotion" {
            "chapter_control"
        } else {
            "translation_task_manifest"
        };
        if let Some(artifact) = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "approval_packet")
            .or_else(|| {
                child
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == approval_kind)
            })
        {
            register_navigation_target(
                &mut targets,
                "approval_packet",
                Path::new(&artifact.path),
                &project_root,
                Some(&artifact.artifact_id),
            );
        }
        let blocker = child
            .artifacts
            .iter()
            .find(|artifact| matches!(artifact.kind.as_str(), "qa_status" | "chapter_control"))
            .filter(|artifact| artifact.validation.exists);
        if let Some(artifact) = blocker {
            register_navigation_target(
                &mut targets,
                "blocker_evidence",
                Path::new(&artifact.path),
                &project_root,
                Some(&artifact.artifact_id),
            );
        }
        let failure = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind.contains("diagnostic"))
            .filter(|artifact| artifact.validation.exists);
        if let Some(artifact) = failure {
            register_navigation_target(
                &mut targets,
                "failure_evidence",
                Path::new(&artifact.path),
                &project_root,
                Some(&artifact.artifact_id),
            );
        }
        if let Some(artifact) = child.artifacts.iter().find(|artifact| {
            artifact.validation.hash_matches
                && artifact.kind != "source_pdf_reference"
                && child.source.path.as_deref() != Some(artifact.path.as_str())
        }) {
            register_navigation_target(
                &mut targets,
                "verified_evidence",
                Path::new(&artifact.path),
                &project_root,
                Some(&artifact.artifact_id),
            );
        }
    }
    job.navigation_targets = targets;
    job.open_target = select_book_pipeline_open_target(job);
}

fn register_navigation_target(
    targets: &mut Vec<BookPipelineNavigationTarget>,
    kind: &str,
    path: &Path,
    allowed_root: &Path,
    artifact_id: Option<&str>,
) {
    let target_id = format!(
        "target-{}",
        sha256_str(&format!("{kind}\0{}", display_path(path)))
    );
    if targets.iter().any(|target| target.target_id == target_id) {
        return;
    }
    targets.push(BookPipelineNavigationTarget {
        target_id,
        kind: kind.into(),
        path: display_path(path),
        allowed_root: display_path(allowed_root),
        artifact_id: artifact_id.map(str::to_string),
    });
}

fn select_book_pipeline_open_target(job: &BookPipelineJob) -> Option<BookPipelineOpenTarget> {
    let find = |kinds: &[&str]| {
        kinds.iter().find_map(|kind| {
            job.navigation_targets
                .iter()
                .find(|target| target.kind == *kind)
        })
    };
    let (target, action_label) = match job.status.as_str() {
        STATUS_PENDING | STATUS_READY | STATUS_RUNNING => {
            (find(&["project_workspace", "workspace"]), "Open workspace")
        }
        STATUS_WAITING_FOR_APPROVAL => (
            find(&["approval_packet", "project_workspace", "workspace"]),
            "Review approval",
        ),
        STATUS_BLOCKED => (
            find(&["blocker_evidence", "project_workspace", "workspace"]),
            "Review blocker",
        ),
        STATUS_FAILED => (
            find(&["failure_evidence", "project_workspace", "workspace"]),
            "Open failure evidence",
        ),
        STATUS_PARTIAL => (
            if job.kind == "collection" {
                find(&["collection_results", "workspace"])
            } else {
                find(&["partial_results", "project_workspace", "workspace"])
            },
            "Inspect partial results",
        ),
        STATUS_COMPLETED if job.kind == "collection" => (
            find(&["collection_results", "workspace"]),
            "Open collection results",
        ),
        STATUS_COMPLETED => {
            if let Some(target) = find(&["reading_output_directory"]) {
                (Some(target), "Open reading output")
            } else {
                (find(&["project_workspace", "workspace"]), "Open workspace")
            }
        }
        STATUS_SKIPPED => (find(&["verified_evidence"]), "Open verified evidence"),
        _ => (None, "Open workspace"),
    };
    target.map(|target| BookPipelineOpenTarget {
        target_id: target.target_id.clone(),
        kind: target.kind.clone(),
        action_label: action_label.into(),
    })
}

fn summarize_children(children: &[BookPipelineChildJob]) -> BookPipelineStatusSummary {
    let mut summary = BookPipelineStatusSummary {
        total: children.len() as u32,
        ..BookPipelineStatusSummary::default()
    };
    for child in children {
        match child.status.as_str() {
            STATUS_PENDING => summary.pending += 1,
            STATUS_READY => summary.ready += 1,
            STATUS_RUNNING => summary.running += 1,
            STATUS_WAITING_FOR_APPROVAL => summary.waiting_for_approval += 1,
            STATUS_BLOCKED => summary.blocked += 1,
            STATUS_FAILED => summary.failed += 1,
            STATUS_COMPLETED => summary.completed += 1,
            STATUS_SKIPPED => summary.skipped += 1,
            STATUS_PARTIAL => {}
            _ => summary.pending += 1,
        }
    }
    summary
}

fn aggregate_parent_status(summary: &BookPipelineStatusSummary) -> String {
    if summary.running > 0 {
        return STATUS_RUNNING.into();
    }
    if summary.waiting_for_approval > 0 {
        return STATUS_WAITING_FOR_APPROVAL.into();
    }
    if summary.ready > 0 {
        return STATUS_READY.into();
    }
    if summary.total > 0
        && summary.completed > 0
        && summary.completed + summary.skipped == summary.total
    {
        return STATUS_COMPLETED.into();
    }
    if summary.completed + summary.skipped > 0 && summary.failed + summary.blocked > 0 {
        return STATUS_PARTIAL.into();
    }
    let non_skipped = summary.total.saturating_sub(summary.skipped);
    if non_skipped > 0 && summary.failed == non_skipped {
        return STATUS_FAILED.into();
    }
    if non_skipped > 0
        && summary.completed == 0
        && summary.failed + summary.blocked == non_skipped
        && summary.blocked > 0
    {
        return STATUS_BLOCKED.into();
    }
    if summary.total > 0 && summary.skipped == summary.total {
        return STATUS_SKIPPED.into();
    }
    STATUS_PENDING.into()
}

fn validate_state(state: &BookPipelineState) -> Result<(), String> {
    for job in &state.jobs {
        if !matches!(
            job.translation_mode.as_str(),
            TRANSLATION_MODE_FAST | TRANSLATION_MODE_EXPERT
        ) {
            return Err(format!(
                "Invalid Book Pipeline translation mode {} in job {}",
                job.translation_mode, job.id
            ));
        }
        if job.translation_mode == TRANSLATION_MODE_EXPERT && job.second_pass_enabled {
            return Err(format!(
                "Expert Book Pipeline job {} cannot enable the fast reflection second pass",
                job.id
            ));
        }
        if job.translation_mode == TRANSLATION_MODE_EXPERT && job.text_cleanup {
            return Err(format!(
                "Expert Book Pipeline job {} cannot enable fast text cleanup",
                job.id
            ));
        }
        let normalized_output_formats = normalize_output_formats(&job.output_formats)?;
        if normalized_output_formats != job.output_formats {
            return Err(format!(
                "Book Pipeline job {} has non-canonical outputFormats",
                job.id
            ));
        }
        if job.digest_mode && !output_format_enabled(job, OUTPUT_FORMAT_EPUB) {
            return Err(format!(
                "Book Pipeline job {} requires epub in outputFormats when digestMode is enabled",
                job.id
            ));
        }
        let mut artifact_ids = BTreeMap::new();
        for artifact in job_artifact_refs(job) {
            validate_artifact_contract(job, artifact)?;
            if let Some(existing) = artifact_ids.insert(&artifact.artifact_id, artifact) {
                if existing != artifact {
                    return Err(format!(
                        "Artifact ID {} has conflicting {} records in job {}",
                        artifact.artifact_id,
                        artifact_immutable_difference(existing, artifact),
                        job.id
                    ));
                }
            }
        }
        for stage in &job.stages {
            if stage.stage_id != "discover"
                || !is_stage_status(&stage.status)
                || stage.contract_version.is_empty()
                || (stage.status == STATUS_RUNNING) != stage.execution_owner.is_some()
            {
                return Err(format!(
                    "Invalid parent stage {} in collection job {}",
                    stage.stage_id, job.id
                ));
            }
        }
        if job.kind == "collection" {
            let membership = job
                .membership
                .as_ref()
                .ok_or_else(|| format!("Collection job {} is missing frozen membership", job.id))?;
            if membership.revision == 0 || membership.discovery_stage_id != "discover" {
                return Err(format!(
                    "Collection job {} has invalid frozen membership metadata",
                    job.id
                ));
            }
            let child_ids = job
                .children
                .iter()
                .map(|child| child.id.as_str())
                .collect::<Vec<_>>();
            let unique_ids = child_ids
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            if unique_ids.len() != child_ids.len()
                || membership.child_job_ids
                    != job
                        .children
                        .iter()
                        .map(|child| child.id.clone())
                        .collect::<Vec<_>>()
            {
                return Err(format!(
                    "Collection job {} has child state outside its frozen membership",
                    job.id
                ));
            }
            if !membership.snapshot_sha256.is_empty() {
                let valid_snapshot_hash = membership.snapshot_sha256.len() == 64
                    && membership
                        .snapshot_sha256
                        .chars()
                        .all(|ch| ch.is_ascii_hexdigit());
                let discover = job
                    .stages
                    .iter()
                    .find(|stage| stage.stage_id == "discover")
                    .ok_or_else(|| format!("Collection job {} has no discovery stage", job.id))?;
                if membership.collection_key.is_empty()
                    || job.source.selector.as_deref() != Some(membership.collection_key.as_str())
                    || !valid_snapshot_hash
                    || !job.collection_items.is_empty()
                    || discover.input_hashes.get("collectionKey")
                        != Some(&membership.collection_key)
                    || discover.input_hashes.get("snapshotSha256")
                        != Some(&membership.snapshot_sha256)
                {
                    return Err(format!(
                        "Collection job {} has invalid durable snapshot identity",
                        job.id
                    ));
                }
                let mut attachment_keys = BTreeSet::new();
                for child in &job.children {
                    let identity = child.source_identity.as_ref().ok_or_else(|| {
                        format!(
                            "Collection child {} has no durable attachment identity",
                            child.id
                        )
                    })?;
                    let expected_id_suffix =
                        format!("-r{}-{}", membership.revision, identity.pdf_attachment_key);
                    if identity.collection_key != membership.collection_key
                        || identity.parent_item_key.is_empty()
                        || identity.pdf_attachment_key.is_empty()
                        || !attachment_keys.insert(identity.pdf_attachment_key.clone())
                        || child.source.kind != "zotero_attachment"
                        || child.source.selector.as_deref()
                            != Some(identity.pdf_attachment_key.as_str())
                        || child.source.path.as_deref()
                            != non_empty(Some(identity.attachment_path.as_str()))
                        || !child.id.ends_with(&expected_id_suffix)
                    {
                        return Err(format!(
                            "Collection child {} does not match its frozen attachment identity",
                            child.id
                        ));
                    }
                }
            }
        }
        for child in &job.children {
            validate_custom_instructions(child.custom_instructions.as_ref())?;
            if child.parent_job_id != job.id {
                return Err(format!(
                    "Book Pipeline child {} does not belong to parent {}",
                    child.id, job.id
                ));
            }
            let mut previous_order = None;
            let mut prerequisites_complete = true;
            let child_artifact_ids = child
                .artifacts
                .iter()
                .map(|artifact| artifact.artifact_id.as_str())
                .collect::<BTreeSet<_>>();
            for stage in &child.stages {
                let order = ordered_stage_index(&stage.stage_id).ok_or_else(|| {
                    format!(
                        "Unknown Book Pipeline stage {} in child {}",
                        stage.stage_id, child.id
                    )
                })?;
                if previous_order.is_some_and(|previous| order <= previous) {
                    return Err(format!(
                        "Book Pipeline stages are not strictly ordered in child {}",
                        child.id
                    ));
                }
                if !is_stage_status(&stage.status) {
                    return Err(format!(
                        "Invalid Book Pipeline stage status {} for {}",
                        stage.status, stage.stage_id
                    ));
                }
                if stage.contract_version.is_empty() {
                    return Err(format!(
                        "Book Pipeline stage {} in child {} has no contract version",
                        stage.stage_id, child.id
                    ));
                }
                validate_index_stage_contract(stage, child)?;
                if !has_valid_child_execution_owner(stage) {
                    return Err(format!(
                        "Book Pipeline stage {} in child {} has invalid execution ownership",
                        stage.stage_id, child.id
                    ));
                }
                if stage
                    .artifact_ids
                    .iter()
                    .any(|artifact_id| !child_artifact_ids.contains(artifact_id.as_str()))
                {
                    return Err(format!(
                        "Book Pipeline stage {} in child {} references an unknown artifact ID",
                        stage.stage_id, child.id
                    ));
                }
                if !prerequisites_complete
                    && matches!(
                        stage.status.as_str(),
                        STATUS_READY
                            | STATUS_RUNNING
                            | STATUS_WAITING_FOR_APPROVAL
                            | STATUS_BLOCKED
                            | STATUS_FAILED
                            | STATUS_COMPLETED
                    )
                {
                    return Err(format!(
                        "Book Pipeline stage {} in child {} has an incomplete prerequisite",
                        stage.stage_id, child.id
                    ));
                }
                prerequisites_complete &=
                    matches!(stage.status.as_str(), STATUS_COMPLETED | STATUS_SKIPPED);
                previous_order = Some(order);
            }
        }
    }
    Ok(())
}

fn validate_index_stage_contract(
    stage: &BookPipelineStage,
    child: &BookPipelineChildJob,
) -> Result<(), String> {
    if stage.stage_id != "index" {
        if stage.index_evidence.is_some() {
            return Err(format!(
                "Non-index stage {} in child {} contains index evidence",
                stage.stage_id, child.id
            ));
        }
        return Ok(());
    }
    if stage.contract_version != ITEM_INDEX_CONTRACT_VERSION {
        return Err(format!(
            "Index stage in child {} has an unsupported contract version",
            child.id
        ));
    }
    let planned_input = item_index_input_from_stage(child).map_err(|error| {
        if stage.status == STATUS_COMPLETED {
            format!(
                "Index stage in child {} has mismatched completed index evidence: {error}",
                child.id
            )
        } else {
            error
        }
    })?;
    if stage.status == STATUS_RUNNING && planned_input.is_none() {
        return Err(format!(
            "Running index stage in child {} has no persisted input identity",
            child.id
        ));
    }
    if stage.status != STATUS_COMPLETED {
        if stage.index_evidence.is_some() {
            return Err(format!(
                "Incomplete index stage in child {} contains completed index evidence",
                child.id
            ));
        }
        return Ok(());
    }
    let evidence = stage.index_evidence.as_ref().ok_or_else(|| {
        format!(
            "Index stage in child {} has no completed index evidence",
            child.id
        )
    })?;
    let planned_input = planned_input.ok_or_else(|| {
        format!(
            "Completed index stage in child {} has no persisted input identity",
            child.id
        )
    })?;
    let valid_sha256 = evidence.source_sha256.len() == 64
        && evidence
            .source_sha256
            .chars()
            .all(|ch| ch.is_ascii_hexdigit());
    let hashes_match = planned_input.parent_item_key == evidence.parent_item_key
        && planned_input.source_sha256 == evidence.source_sha256
        && planned_input.chunk_contract_version == evidence.chunk_contract_version
        && planned_input.embedding_profile_id == evidence.embedding_profile_id;
    if evidence.parent_item_key.trim().is_empty()
        || !valid_sha256
        || evidence.chunk_count == 0
        || evidence.index_contract_version != ITEM_INDEX_CONTRACT_VERSION
        || evidence.chunk_contract_version != CHUNK_CONTRACT_VERSION
        || evidence.embedding_profile_id.trim().is_empty()
        || evidence.completed_at.trim().is_empty()
        || !hashes_match
    {
        return Err(format!(
            "Index stage in child {} has mismatched completed index evidence",
            child.id
        ));
    }
    Ok(())
}

fn item_index_input_from_stage(
    child: &BookPipelineChildJob,
) -> Result<Option<ItemScopedIndexInput>, String> {
    let Some(stage) = stage_ref(child, "index") else {
        return Ok(None);
    };
    if stage.input_hashes.is_empty() {
        return Ok(None);
    }
    let field = |name: &str| {
        stage
            .input_hashes
            .get(name)
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Index stage in child {} has incomplete persisted input identity",
                    child.id
                )
            })
    };
    if field("indexContractVersion")? != ITEM_INDEX_CONTRACT_VERSION {
        return Err(format!(
            "Index stage in child {} has mismatched input contract identity",
            child.id
        ));
    }
    let input = ItemScopedIndexInput {
        parent_item_key: field("parentItemKey")?,
        source_sha256: field("markdownSha256")?,
        chunk_contract_version: field("chunkContractVersion")?,
        embedding_profile_id: field("embeddingProfileId")?,
        markdown_artifact_id: field("markdownArtifactId")?,
    };
    let valid_sha256 = input.source_sha256.len() == 64
        && input.source_sha256.chars().all(|ch| ch.is_ascii_hexdigit());
    let artifact_matches = child.artifacts.iter().any(|artifact| {
        artifact.artifact_id == input.markdown_artifact_id
            && artifact.kind == "markdown"
            && artifact.sha256.as_deref() == Some(input.source_sha256.as_str())
    });
    if !valid_sha256 || input.chunk_contract_version != CHUNK_CONTRACT_VERSION || !artifact_matches
    {
        return Err(format!(
            "Index stage in child {} has invalid persisted input identity",
            child.id
        ));
    }
    Ok(Some(input))
}

fn job_artifact_refs(job: &BookPipelineJob) -> Vec<&BookPipelineArtifact> {
    job.artifacts
        .iter()
        .chain(job.children.iter().flat_map(|child| child.artifacts.iter()))
        .chain(
            job.collection_items
                .iter()
                .flat_map(|item| item.artifacts.iter()),
        )
        .collect()
}

fn validate_artifact_contract(
    job: &BookPipelineJob,
    artifact: &BookPipelineArtifact,
) -> Result<(), String> {
    let expected_id = format!(
        "artifact-{}",
        sha256_str(&format!(
            "{}\0{}\0{}",
            artifact.kind,
            artifact.path,
            artifact.sha256.as_deref().unwrap_or("missing")
        ))
    );
    if artifact.artifact_id != expected_id
        || artifact.sha256.is_none()
        || artifact.producer.stage_id.is_empty()
        || artifact.producer.stage_id == "unknown"
        || artifact.producer.attempt == 0
        || artifact.input_hashes.is_empty()
        || artifact.source_refs.source_ref_sha256.is_empty()
        || artifact.privacy.is_empty()
        || artifact.created_at.is_empty()
    {
        return Err(format!(
            "Artifact {} in job {} has incomplete provenance",
            artifact.path, job.id
        ));
    }
    if artifact.validation.exists
        && (artifact.size_bytes.is_none() || !Path::new(&artifact.path).is_file())
    {
        return Err(format!(
            "Artifact {} in job {} is not a registered file",
            artifact.path, job.id
        ));
    }
    Ok(())
}

fn ordered_stage_index(stage_id: &str) -> Option<usize> {
    [
        "route",
        "extract",
        "index",
        "handoff",
        "split",
        "prepare",
        "approve_translation",
        "translate",
        "expert_qa",
        "approve_promotion",
        "promote",
        "build_reading",
        "validate_reading",
        "build_digest",
    ]
    .iter()
    .position(|candidate| *candidate == stage_id)
}

fn is_stage_status(status: &str) -> bool {
    matches!(
        status,
        STATUS_PENDING
            | STATUS_READY
            | STATUS_RUNNING
            | STATUS_WAITING_FOR_APPROVAL
            | STATUS_BLOCKED
            | STATUS_FAILED
            | STATUS_COMPLETED
            | STATUS_SKIPPED
    )
}

fn is_agent_handoff_waiting(stage: &BookPipelineStage) -> bool {
    stage.status == STATUS_BLOCKED
        && matches!(stage.stage_id.as_str(), "translate" | "expert_qa")
        && stage
            .execution_owner
            .as_deref()
            .is_some_and(|owner| owner.starts_with(AGENT_EXECUTION_OWNER_PREFIX))
}

fn has_valid_child_execution_owner(stage: &BookPipelineStage) -> bool {
    match stage.status.as_str() {
        STATUS_RUNNING => stage.execution_owner.is_some(),
        STATUS_BLOCKED => stage.execution_owner.is_none() || is_agent_handoff_waiting(stage),
        _ => stage.execution_owner.is_none(),
    }
}

trait BookPipelineNotificationSink {
    fn deliver(&self, event: &BookPipelineTerminalEvent) -> Result<(), String>;
}

struct SystemWebhookSink {
    endpoint: String,
}

impl SystemWebhookSink {
    fn from_config() -> Option<Self> {
        std::env::var("BOOK_PIPELINE_WEBHOOK_URL")
            .ok()
            .or_else(|| {
                let env_path = local_reading_repo_root().ok()?.join(".env");
                let raw = fs::read_to_string(env_path).ok()?;
                dotenv_value(&raw, "BOOK_PIPELINE_WEBHOOK_URL")
            })
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|endpoint| Self { endpoint })
    }
}

fn dotenv_value(raw: &str, key: &str) -> Option<String> {
    raw.lines().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (candidate, value) = line.split_once('=')?;
        if candidate.trim() != key {
            return None;
        }
        let value = value.trim();
        Some(
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .or_else(|| {
                    value
                        .strip_prefix('\'')
                        .and_then(|value| value.strip_suffix('\''))
                })
                .unwrap_or(value)
                .to_string(),
        )
    })
}

impl BookPipelineNotificationSink for SystemWebhookSink {
    fn deliver(&self, event: &BookPipelineTerminalEvent) -> Result<(), String> {
        if !(self.endpoint.starts_with("https://") || self.endpoint.starts_with("http://")) {
            return Err("webhook_endpoint_invalid".into());
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "webhook_client_unavailable".to_string())?;
        for attempt in 0..3 {
            let delivered = client
                .post(&self.endpoint)
                .header("Idempotency-Key", &event.event_id)
                .json(event)
                .send()
                .is_ok_and(|response| response.status().is_success());
            if delivered {
                return Ok(());
            }
            if attempt < 2 {
                thread::sleep(Duration::from_millis(200));
            }
        }
        Err("webhook_delivery_failed".into())
    }
}

fn is_terminal_job_status(status: &str) -> bool {
    matches!(
        status,
        STATUS_COMPLETED | STATUS_FAILED | STATUS_BLOCKED | STATUS_PARTIAL | STATUS_SKIPPED
    )
}

/// ADR 0002 promises one webhook per terminal *outcome*, so the event identity is
/// the outcome — nothing else. Folding the snapshot fields in made it one webhook
/// per (outcome, timestamp) instead: a job that failed, was retried and failed
/// again hashed a fresh `updated_at` and `attempts` into a new id and notified
/// twice. Those fields stay in the payload, where the ADR does want them; they
/// just do not decide identity. Keeping the id stable also means a redelivery
/// after a failed attempt carries the `Idempotency-Key` the receiver already saw.
fn terminal_event(job: &BookPipelineJob) -> BookPipelineTerminalEvent {
    let identity = format!(
        "{}\0{}\0{}\0{}",
        TERMINAL_EVENT_SCHEMA_VERSION, job.id, job.kind, job.status,
    );
    BookPipelineTerminalEvent {
        schema_version: TERMINAL_EVENT_SCHEMA_VERSION.into(),
        event_id: sha256_str(&identity),
        job_id: job.id.clone(),
        job_kind: job.kind.clone(),
        status: job.status.clone(),
        current_stage_id: job.current_stage_id.clone(),
        progress: job.progress.clone(),
        summary: job.summary.clone(),
        updated_at: job.updated_at.clone(),
    }
}

fn dispatch_terminal_notification<S, N>(
    store: &S,
    sink: &N,
    job_id: &str,
) -> Result<BookPipelineJob, String>
where
    S: BookPipelineStateStore,
    N: BookPipelineNotificationSink,
{
    let mut state = store.load()?;
    let job_index = state
        .jobs
        .iter()
        .position(|job| job.id == job_id)
        .ok_or_else(|| format!("Book Pipeline job {job_id} was not found."))?;
    let event = terminal_event(&state.jobs[job_index]);
    if !is_terminal_job_status(&event.status) {
        return Ok(state.jobs[job_index].clone());
    }
    if state.jobs[job_index]
        .notification_deliveries
        .iter()
        .any(|delivery| {
            delivery.event_id == event.event_id && delivery.delivery_status == "delivered"
        })
    {
        return Ok(state.jobs[job_index].clone());
    }

    let delivered = sink.deliver(&event);
    let existing = state.jobs[job_index]
        .notification_deliveries
        .iter_mut()
        .find(|delivery| delivery.event_id == event.event_id);
    let delivery = BookPipelineNotificationDelivery {
        event_id: event.event_id,
        status: event.status,
        delivery_status: if delivered.is_ok() {
            "delivered".into()
        } else {
            "failed".into()
        },
        attempts: existing
            .as_ref()
            .map(|delivery| delivery.attempts.saturating_add(1))
            .unwrap_or(1),
        delivered_at: delivered.is_ok().then(now_label),
        safe_error: delivered.err(),
    };
    if let Some(existing) = existing {
        *existing = delivery;
    } else {
        state.jobs[job_index].notification_deliveries.push(delivery);
    }
    store.save(&state)?;
    store
        .load()?
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .ok_or_else(|| format!("Book Pipeline job {job_id} was not found after notification."))
}

fn dispatch_configured_terminal_notification(
    store: &BookPipelineStore,
    job: BookPipelineJob,
) -> Result<BookPipelineJob, String> {
    let Some(sink) = SystemWebhookSink::from_config() else {
        return Ok(job);
    };
    dispatch_terminal_notification(store, &sink, &job.id)
}

fn validate_state_transitions(
    previous: &BookPipelineState,
    next: &BookPipelineState,
) -> Result<(), String> {
    for next_job in &next.jobs {
        let Some(previous_job) = previous.jobs.iter().find(|job| job.id == next_job.id) else {
            continue;
        };
        if previous_job.membership.is_some() && previous_job.membership != next_job.membership {
            return Err(format!(
                "Collection job {} cannot change frozen membership",
                next_job.id
            ));
        }
        for next_artifact in job_artifact_refs(next_job) {
            let Some(previous_artifact) = job_artifact_refs(previous_job)
                .into_iter()
                .find(|artifact| artifact.artifact_id == next_artifact.artifact_id)
            else {
                continue;
            };
            if !artifact_immutable_fields_equal(previous_artifact, next_artifact) {
                return Err(format!(
                    "Artifact {} was mutated instead of registered with a new ID ({})",
                    next_artifact.artifact_id,
                    artifact_immutable_difference(previous_artifact, next_artifact)
                ));
            }
        }
        for next_stage in &next_job.stages {
            let Some(previous_stage) = previous_job
                .stages
                .iter()
                .find(|stage| stage.stage_id == next_stage.stage_id)
            else {
                continue;
            };
            if !is_allowed_stage_transition(previous_stage, next_stage) {
                return Err(format!(
                    "Invalid Book Pipeline parent stage transition for {}: {} -> {}",
                    next_stage.stage_id, previous_stage.status, next_stage.status
                ));
            }
        }
        for next_child in &next_job.children {
            let Some(previous_child) = previous_job
                .children
                .iter()
                .find(|child| child.id == next_child.id)
            else {
                continue;
            };
            for next_stage in &next_child.stages {
                let Some(previous_stage) = previous_child
                    .stages
                    .iter()
                    .find(|stage| stage.stage_id == next_stage.stage_id)
                else {
                    continue;
                };
                if !is_allowed_stage_transition(previous_stage, next_stage) {
                    return Err(format!(
                        "Invalid Book Pipeline stage transition for {}: {} -> {}",
                        next_stage.stage_id, previous_stage.status, next_stage.status
                    ));
                }
            }
        }
    }
    Ok(())
}

fn artifact_immutable_difference(
    previous: &BookPipelineArtifact,
    next: &BookPipelineArtifact,
) -> &'static str {
    if previous.kind != next.kind {
        "kind"
    } else if previous.path != next.path {
        "path"
    } else if previous.sha256 != next.sha256 {
        "sha256"
    } else if previous.size_bytes != next.size_bytes {
        "sizeBytes"
    } else if previous.producer != next.producer {
        "producer"
    } else if previous.input_hashes != next.input_hashes {
        "inputHashes"
    } else if previous.source_refs != next.source_refs {
        "sourceRefs"
    } else if previous.privacy != next.privacy {
        "privacy"
    } else if previous.created_at != next.created_at {
        "createdAt"
    } else {
        "identity"
    }
}

fn artifact_immutable_fields_equal(
    previous: &BookPipelineArtifact,
    next: &BookPipelineArtifact,
) -> bool {
    previous.artifact_id == next.artifact_id
        && previous.kind == next.kind
        && previous.path == next.path
        && previous.sha256 == next.sha256
        && previous.size_bytes == next.size_bytes
        && previous.producer == next.producer
        && previous.input_hashes == next.input_hashes
        && previous.source_refs == next.source_refs
        && previous.privacy == next.privacy
        && previous.created_at == next.created_at
}

fn is_allowed_stage_transition(previous: &BookPipelineStage, next: &BookPipelineStage) -> bool {
    if matches!(
        previous.status.as_str(),
        STATUS_READY | STATUS_FAILED | STATUS_BLOCKED
    ) && next.status == STATUS_PENDING
        && next
            .input_hashes
            .contains_key("translationApprovalBindingSha256")
    {
        return previous.input_hashes != next.input_hashes;
    }
    if previous.stage_id == "index"
        && previous.status == STATUS_READY
        && next.status == STATUS_FAILED
    {
        return true;
    }
    if previous.stage_id.starts_with("approve_")
        && previous.status == STATUS_READY
        && next.status == STATUS_COMPLETED
    {
        return next.approval_id.is_some();
    }
    if previous.status == STATUS_COMPLETED && next.status != STATUS_COMPLETED {
        let invalidated = previous.contract_version != next.contract_version
            || previous.input_hashes != next.input_hashes;
        return invalidated
            && matches!(
                next.status.as_str(),
                STATUS_PENDING | STATUS_READY | STATUS_BLOCKED | STATUS_SKIPPED
            );
    }
    if matches!(
        previous.status.as_str(),
        STATUS_READY | STATUS_WAITING_FOR_APPROVAL
    ) && next.status == STATUS_PENDING
    {
        return previous.contract_version != next.contract_version
            || previous.input_hashes != next.input_hashes;
    }
    if is_agent_handoff_waiting(previous) && next.status == STATUS_RUNNING {
        return true;
    }
    previous.status == next.status
        || matches!(
            (previous.status.as_str(), next.status.as_str()),
            (
                STATUS_PENDING,
                STATUS_READY | STATUS_RUNNING | STATUS_BLOCKED | STATUS_SKIPPED
            ) | (
                STATUS_READY,
                STATUS_RUNNING | STATUS_BLOCKED | STATUS_SKIPPED
            ) | (
                STATUS_RUNNING,
                STATUS_COMPLETED | STATUS_FAILED | STATUS_BLOCKED | STATUS_WAITING_FOR_APPROVAL
            ) | (
                STATUS_WAITING_FOR_APPROVAL,
                STATUS_COMPLETED | STATUS_BLOCKED | STATUS_READY
            ) | (STATUS_BLOCKED, STATUS_READY | STATUS_SKIPPED)
                | (
                    STATUS_FAILED,
                    STATUS_READY | STATUS_RUNNING | STATUS_SKIPPED
                )
        )
}

#[tauri::command]
pub fn get_book_pipeline_state() -> Result<BookPipelineState, String> {
    BookPipelineStore::default()?.load()
}

#[tauri::command]
pub fn preview_book_pipeline_route(
    source: BookPipelineSource,
    mode: String,
    config: Option<BookPipelinePreviewConfig>,
) -> Result<Vec<BookPipelineRouteItem>, String> {
    preview_book_pipeline_route_with_executor(
        &SystemCommandExecutor,
        &source,
        &mode,
        config.unwrap_or_default(),
    )
}

fn preview_book_pipeline_route_with_executor<E: RunnerCommandExecutor>(
    executor: &E,
    source: &BookPipelineSource,
    mode: &str,
    config: BookPipelinePreviewConfig,
) -> Result<Vec<BookPipelineRouteItem>, String> {
    if source.kind == "zotero_collection" && source.fake_zotero_items.is_none() {
        let collection_key = non_empty(source.selector.as_deref())
            .ok_or_else(|| "Zotero collection source is missing a collection key.".to_string())?;
        let snapshot = read_zotero_collection_snapshot(executor, collection_key)?;
        return Ok(snapshot
            .members
            .iter()
            .filter_map(collection_snapshot_route)
            .collect());
    }
    if is_zotero_source(source) && source.fake_zotero_items.is_none() {
        return preview_zotero_route_from_worker(
            executor,
            source,
            config,
            20,
            &book_ocr_conversion_root(),
        );
    }
    Ok(preview_route(source, mode, config))
}

#[tauri::command]
pub fn queue_book_pipeline_job(
    source: BookPipelineSource,
    mode: String,
    translation_intent: BookPipelineTranslationIntent,
    config: Option<BookPipelinePreviewConfig>,
) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    queue_job_with_translation_intent(
        &store,
        source,
        mode,
        translation_intent,
        config.unwrap_or_default(),
    )
}

#[tauri::command]
pub fn save_book_pipeline_custom_instructions(
    job_id: String,
    child_id: Option<String>,
    custom_instructions: BookPipelineCustomInstructions,
) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    save_book_custom_instructions(&store, &job_id, child_id.as_deref(), custom_instructions)
}

fn save_book_custom_instructions(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    custom_instructions: BookPipelineCustomInstructions,
) -> Result<BookPipelineJob, String> {
    let custom_instructions = normalize_custom_instructions(custom_instructions)?;
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
    let approval_order = ordered_stage_index("approve_translation").unwrap_or_default();
    let child = &state.jobs[job_index].children[child_index];
    if child.custom_instructions == custom_instructions {
        return Ok(state.jobs[job_index].clone());
    }
    let later_stage_is_active = child.stages.iter().any(|stage| {
        ordered_stage_index(&stage.stage_id).is_some_and(|order| order > approval_order)
            && (matches!(
                stage.status.as_str(),
                STATUS_RUNNING | STATUS_WAITING_FOR_APPROVAL
            ) || is_agent_handoff_waiting(stage))
    });
    if later_stage_is_active {
        return Err("custom_instructions_locked: translation or a later stage is active.".into());
    }

    state.jobs[job_index].children[child_index].custom_instructions = custom_instructions;
    ready_translation_approval_gate(&mut state.jobs[job_index], child_index);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let saved = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(saved)
}

#[tauri::command]
pub fn run_book_pipeline_job(job_id: String) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    let job = run_job_to_quiescence(&store, &SystemPipelineRunner, &job_id)?;
    dispatch_configured_terminal_notification(&store, job)
}

#[tauri::command]
pub fn retry_book_pipeline_job(job_id: String) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    let job = retry_job_to_quiescence(&store, &SystemPipelineRunner, &job_id)?;
    dispatch_configured_terminal_notification(&store, job)
}

/// Remove a job from the shelf. Files on disk (extraction output, the local
/// reading project, Zotero attachments) are deliberately left untouched — this
/// only forgets the job, so a re-queued book can reuse the converted Markdown.
#[tauri::command]
pub fn delete_book_pipeline_job(
    job_id: String,
    explicit_approval: bool,
) -> Result<BookPipelineState, String> {
    let store = BookPipelineStore::default()?;
    delete_job(&store, &job_id, explicit_approval)
}

fn delete_job(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    explicit_approval: bool,
) -> Result<BookPipelineState, String> {
    if !explicit_approval {
        return Err("Explicit approval is required to delete a Book Pipeline job.".into());
    }
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    if job_is_actively_running(&state.jobs[job_index]) {
        return Err(
            "This book is currently running; wait for the active stage to finish first.".into(),
        );
    }
    state.jobs.remove(job_index);
    store.save(&state)?;
    store.load()
}

fn job_is_actively_running(job: &BookPipelineJob) -> bool {
    matches!(job.status.as_str(), STATUS_RUNNING | STATUS_HANDOFF_RUNNING)
        || job.children.iter().any(|child| {
            child
                .stages
                .iter()
                .any(|stage| stage.status == STATUS_RUNNING)
        })
}

#[tauri::command]
pub fn advance_book_pipeline_job(
    job_id: String,
    child_id: Option<String>,
    invalidate_downstream: Option<bool>,
) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    let job = advance_job(
        &store,
        &job_id,
        child_id.as_deref(),
        invalidate_downstream.unwrap_or(false),
    )?;
    dispatch_configured_terminal_notification(&store, job)
}

fn approve_job_gate(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    stage_id: &str,
    explicit_approval: bool,
) -> Result<BookPipelineJob, String> {
    if !explicit_approval {
        return Err("Explicit Book Pipeline gate approval is required.".into());
    }
    if !matches!(stage_id, "approve_translation" | "approve_promotion") {
        return Err("Only a ready translation or promotion gate can be approved.".into());
    }

    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
    let approved = match stage_id {
        "approve_translation" => approve_translation_gate(&mut state.jobs[job_index], child_index),
        "approve_promotion" => approve_promotion_gate(&mut state.jobs[job_index], child_index),
        _ => false,
    };
    if !approved {
        return Err(format!(
            "The {stage_id} gate is not ready or its artifact binding changed."
        ));
    }

    state.jobs[job_index].current_step = format!("Approved {stage_id} gate");
    state.jobs[job_index]
        .log_summary
        .push(format!("Explicit {stage_id} approval recorded"));
    state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let job = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(job)
}

#[tauri::command]
pub fn approve_book_pipeline_gate(
    job_id: String,
    child_id: Option<String>,
    stage_id: String,
    explicit_approval: bool,
) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    approve_job_gate(
        &store,
        &job_id,
        child_id.as_deref(),
        &stage_id,
        explicit_approval,
    )
}

/// Re-route a book the pipeline held back, in place. Before this the Overview
/// tab offered the same three choices as the wizard's preflight step but with
/// every button disabled, so a held book could only be dealt with by deleting it
/// and queueing it again — which for a collection took the whole batch with it.
#[tauri::command]
pub fn set_book_pipeline_route_override(
    job_id: String,
    child_id: Option<String>,
    route_item_id: String,
    route_override: String,
    config: Option<BookPipelinePreviewConfig>,
) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    set_route_override(
        &store,
        &job_id,
        child_id.as_deref(),
        &route_item_id,
        &route_override,
        &config.unwrap_or_default(),
    )
}

fn set_route_override(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    route_item_id: &str,
    route_override: &str,
    config: &BookPipelinePreviewConfig,
) -> Result<BookPipelineJob, String> {
    if !ROUTE_OVERRIDE_TOKENS.contains(&route_override) {
        return Err(format!(
            "Route override must be one of {}.",
            ROUTE_OVERRIDE_TOKENS.join(", ")
        ));
    }
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
    let child = &mut state.jobs[job_index].children[child_index];
    if !child.route.iter().any(|item| item.id == route_item_id) {
        return Err(format!(
            "This book has no route item {route_item_id} to override."
        ));
    }
    if stage_ref(child, "extract").is_some_and(|stage| stage.status == STATUS_RUNNING) {
        return Err("Cannot re-route a book while its extraction is running.".into());
    }

    if route_override == "auto" {
        child.source.route_overrides.remove(route_item_id);
    } else {
        child
            .source
            .route_overrides
            .insert(route_item_id.to_string(), route_override.to_string());
    }
    // The same routing the wizard's preflight step uses, so a decision made after
    // the book was held lands on exactly the contract a decision made before it
    // would have. Credentials are re-read here, not remembered from queue time.
    let overrides = child.source.route_overrides.clone();
    apply_route_overrides(&mut child.route, &overrides, Some(config));
    // `apply_route_overrides` only ever forces a route, so dropping one cannot be
    // undone in place — the automatic decision has to be recomputed from live
    // worker evidence. Readying `route` is what schedules that, and it may well
    // hold the book again, which is the point of asking for `auto`.
    let runnable = route_override == "auto"
        || child
            .route
            .iter()
            .any(|item| item.id == route_item_id && item.can_run);
    // A re-route that is still not runnable — forcing a provider whose
    // credentials are missing — must leave the book held rather than readying a
    // stage that would fail on the next spawn.
    if runnable {
        // Ready, not completed: routing re-runs against live worker evidence and
        // re-applies the override that now lives on the source, so the decision
        // goes through the same path a queue-time choice does. (`blocked ->
        // completed` is not a legal stage transition either.)
        set_stage_status(child, "route", STATUS_READY, None);
        // Extraction stays pending behind it: `extract` may not be ready before
        // `route` has completed, and routing is exactly what has to run again.
        set_stage_status(child, "extract", STATUS_PENDING, None);
        child.last_error = None;
    } else {
        let held = child
            .route
            .iter()
            .find(|item| item.id == route_item_id)
            .and_then(|item| item.blocked_reason.clone())
            .unwrap_or_else(|| "The requested route is not runnable.".into());
        set_stage_status(child, "route", STATUS_BLOCKED, Some(held.clone()));
        set_stage_status(child, "extract", STATUS_PENDING, None);
        child.last_error = Some(held);
    }

    state.jobs[job_index].current_step = format!("Route override: {route_override}");
    state.jobs[job_index].log_summary.push(format!(
        "Route {route_item_id} overridden to {route_override}"
    ));
    state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let job = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(job)
}

/// Record that a person opened the built book in a real reader. Optional by
/// design: never calling this leaves promotion exactly as it was.
#[tauri::command]
pub fn record_book_pipeline_reader_evidence(
    job_id: String,
    child_id: Option<String>,
    artifact_kind: String,
    reader: String,
    reader_version: String,
    conclusion: String,
) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    record_reader_evidence(
        &store,
        &job_id,
        child_id.as_deref(),
        &artifact_kind,
        &reader,
        &reader_version,
        &conclusion,
    )
}

/// Sampling is a "try before you decide" action, so by default it leaves the
/// job's own translation provider alone: the caller passes `apply_to_job` only
/// for the explicit "translate this book with this model" action, which goes
/// through `set_book_pipeline_translation_provider` in the normal case.
#[tauri::command]
pub fn run_book_pipeline_translation_sample(
    job_id: String,
    child_id: String,
    provider_profile_id: String,
    provider_config_id: String,
    apply_to_job: bool,
) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    run_translation_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        &provider_profile_id,
        &provider_config_id,
        apply_to_job,
        &SystemCommandExecutor,
    )
}

/// Adopt a provider slot as the job's own, which is what the full-book run will
/// use. Separate from sampling so that trying a model out cannot silently
/// redirect the book; the approval gate is rebound here exactly as it is after a
/// sample, so an approval that predates the change does not survive it.
#[tauri::command]
pub fn set_book_pipeline_translation_provider(
    job_id: String,
    child_id: String,
    provider_profile_id: String,
    provider_config_id: String,
) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    set_translation_provider_in_store(
        &store,
        &job_id,
        Some(&child_id),
        &provider_profile_id,
        &provider_config_id,
    )
}

#[tauri::command]
pub fn choose_book_pipeline_pdf_folder() -> Result<Option<BookPipelineSource>, String> {
    let Some(folder) = rfd::FileDialog::new()
        .set_title("Select local PDF folder")
        .pick_folder()
    else {
        return Ok(None);
    };
    Ok(Some(BookPipelineSource {
        kind: "local_pdf_folder".into(),
        title: folder
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string()),
        path: Some(display_path(&folder)),
        selector: None,
        runner_behavior: None,
        translation_strategy: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    }))
}

#[tauri::command]
pub fn choose_book_pipeline_markdown_source() -> Result<Option<BookPipelineSource>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Select Markdown or source-text file")
        .add_filter("Source text", &["md", "markdown", "txt"])
        .pick_file()
    else {
        return Ok(None);
    };
    Ok(Some(BookPipelineSource {
        kind: "markdown_source".into(),
        title: path
            .file_stem()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string()),
        path: Some(display_path(&path)),
        selector: None,
        runner_behavior: None,
        translation_strategy: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    }))
}

#[tauri::command]
pub fn discover_book_pipeline_zotero_sources(
    source: BookPipelineSource,
    limit: Option<u32>,
) -> Result<BookPipelineZoteroDiscoveryResult, String> {
    discover_zotero_sources(&SystemCommandExecutor, &source, limit.unwrap_or(20))
}

#[tauri::command]
pub fn open_book_pipeline_output(job_id: String) -> Result<BookPipelineActionResult, String> {
    let store = BookPipelineStore::default()?;
    let state = store.load()?;
    let job = state
        .jobs
        .iter()
        .find(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found.".to_string())?;
    let mut allowed_roots = vec![store.job_output_dir(&job.id)];
    if let Ok(repo_root) = local_reading_repo_root() {
        allowed_roots.push(repo_root.join("books").join("local"));
    }
    let resolved = resolve_book_pipeline_open_target(job, &allowed_roots)?;
    open::that(&resolved.path).map_err(|err| err.to_string())?;
    Ok(BookPipelineActionResult {
        ok: true,
        message: format!(
            "{}: {}",
            resolved.action_label,
            display_path(&resolved.path)
        ),
        path: Some(display_path(&resolved.path)),
    })
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineArtifactExcerpt {
    pub artifact_id: String,
    pub kind: String,
    pub excerpt: String,
    pub truncated: bool,
}

const ARTIFACT_EXCERPT_DEFAULT_CHARS: usize = 800;
const ARTIFACT_EXCERPT_MAX_CHARS: usize = 4000;

#[tauri::command]
pub fn read_book_pipeline_artifact_excerpt(
    job_id: String,
    artifact_id: String,
    max_chars: Option<usize>,
) -> Result<BookPipelineArtifactExcerpt, String> {
    let store = BookPipelineStore::default()?;
    let state = store.load()?;
    let job = state
        .jobs
        .iter()
        .find(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found.".to_string())?;
    let mut allowed_roots = vec![store.job_output_dir(&job.id)];
    if let Ok(repo_root) = local_reading_repo_root() {
        allowed_roots.push(repo_root.join("books").join("local"));
    }
    read_artifact_excerpt(job, &artifact_id, max_chars, &allowed_roots)
}

#[tauri::command]
pub fn read_book_pipeline_translation_sample(
    job_id: String,
    child_id: String,
) -> Result<BookPipelineTranslationSampleReport, String> {
    let store = BookPipelineStore::default()?;
    let state = store.load()?;
    let job = state
        .jobs
        .iter()
        .find(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found.".to_string())?;
    read_translation_sample_report(job, &child_id)
}

/// Read the head of a registered artifact for on-screen preview. The content is
/// private_text: it is returned to the UI only and must never be logged.
fn read_artifact_excerpt(
    job: &BookPipelineJob,
    artifact_id: &str,
    max_chars: Option<usize>,
    allowed_roots: &[PathBuf],
) -> Result<BookPipelineArtifactExcerpt, String> {
    let artifact = find_job_artifact(job, artifact_id).ok_or_else(|| {
        "artifact_excerpt_invalid: the artifact is not registered on this job.".to_string()
    })?;
    let path = fs::canonicalize(&artifact.path)
        .map_err(|_| "artifact_excerpt_invalid: the artifact file is missing.".to_string())?;
    let allowed = allowed_roots.iter().any(|allowed_root| {
        fs::canonicalize(allowed_root)
            .ok()
            .is_some_and(|allowed_root| path.starts_with(allowed_root))
    });
    if !allowed {
        return Err(
            "artifact_excerpt_invalid: the artifact is outside the job/project allowlist.".into(),
        );
    }
    let content = fs::read_to_string(&path)
        .map_err(|_| "artifact_excerpt_invalid: the artifact is not readable text.".to_string())?;
    let limit = max_chars
        .unwrap_or(ARTIFACT_EXCERPT_DEFAULT_CHARS)
        .clamp(1, ARTIFACT_EXCERPT_MAX_CHARS);
    let excerpt: String = content.chars().take(limit).collect();
    let truncated = excerpt.len() < content.len();
    Ok(BookPipelineArtifactExcerpt {
        artifact_id: artifact.artifact_id.clone(),
        kind: artifact.kind.clone(),
        excerpt,
        truncated,
    })
}

#[tauri::command]
pub fn export_book_pipeline_diagnostic(
    job_id: String,
    profile: String,
) -> Result<serde_json::Value, String> {
    let store = BookPipelineStore::default()?;
    let state = store.load()?;
    let job = state
        .jobs
        .iter()
        .find(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found.".to_string())?;
    build_book_pipeline_diagnostic(job, &profile)
}

/// The same three profiles, written to a folder the user picks, because a
/// diagnostic bundle is only useful if it can be attached to a report. The
/// command above returns the value in-process and stays as it is.
#[tauri::command]
pub fn save_book_pipeline_diagnostic(
    job_id: String,
    profile: String,
) -> Result<BookPipelineActionResult, String> {
    let store = BookPipelineStore::default()?;
    let state = store.load()?;
    let job = state
        .jobs
        .iter()
        .find(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found.".to_string())?;
    // Build before the dialog: an unsupported profile should fail immediately
    // rather than after the user has picked a folder.
    let document = build_book_pipeline_diagnostic(job, &profile)?;
    let Some(folder) = rfd::FileDialog::new()
        .set_title("Save Book Pipeline diagnostic bundle")
        .pick_folder()
    else {
        return Ok(BookPipelineActionResult {
            ok: false,
            message: "Diagnostic export cancelled.".into(),
            path: None,
        });
    };
    let path = write_book_pipeline_diagnostic(&folder, &job.id, &profile, &document)?;
    Ok(BookPipelineActionResult {
        ok: true,
        message: format!("Diagnostic bundle written to {}", display_path(&path)),
        path: Some(display_path(&path)),
    })
}

fn write_book_pipeline_diagnostic(
    dir: &Path,
    job_id: &str,
    profile: &str,
    document: &serde_json::Value,
) -> Result<PathBuf, String> {
    // The id is generated as "job-<nanos>", but a bundle must not be able to
    // write outside the folder the user picked even if an imported job carries
    // something else.
    let safe_job_id: String = job_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = dir.join(format!(
        "bibliosmith-diagnostic-{safe_job_id}-{profile}.json"
    ));
    let body = serde_json::to_string_pretty(document).map_err(|err| err.to_string())? + "\n";
    fs::write(&path, body).map_err(|err| err.to_string())?;
    Ok(path)
}

fn resolve_book_pipeline_open_target(
    job: &BookPipelineJob,
    allowed_roots: &[PathBuf],
) -> Result<ResolvedBookPipelineOpenTarget, String> {
    let selected = job.open_target.as_ref().ok_or_else(|| {
        "open_target_invalid: no registered target is available for this job state.".to_string()
    })?;
    let target = job
        .navigation_targets
        .iter()
        .find(|target| target.target_id == selected.target_id && target.kind == selected.kind)
        .ok_or_else(|| {
            "open_target_invalid: the selected target is not registered on this job.".to_string()
        })?;
    let registered_root = fs::canonicalize(&target.allowed_root).map_err(|_| {
        "open_target_invalid: the registered target root does not exist.".to_string()
    })?;
    let allowed = allowed_roots.iter().any(|allowed_root| {
        fs::canonicalize(allowed_root)
            .ok()
            .is_some_and(|allowed_root| registered_root.starts_with(allowed_root))
    });
    if !allowed {
        return Err(
            "open_target_invalid: the registered root is outside the job/project allowlist.".into(),
        );
    }
    let path = fs::canonicalize(&target.path).map_err(|_| {
        "open_target_invalid: the registered target is missing; no ancestor fallback was used."
            .to_string()
    })?;
    if !path.starts_with(&registered_root) {
        return Err("open_target_invalid: target traversal escaped its registered root.".into());
    }
    if let Some(artifact_id) = target.artifact_id.as_deref() {
        let artifact = find_job_artifact(job, artifact_id).ok_or_else(|| {
            "open_target_invalid: target references an unregistered artifact.".to_string()
        })?;
        if Path::new(&artifact.path) != Path::new(&target.path) {
            return Err("open_target_invalid: target path does not match its artifact.".into());
        }
        if let Some(expected) = artifact.sha256.as_deref() {
            let actual = sha256_file(&path).map_err(|_| {
                "open_target_invalid: target artifact could not be validated.".to_string()
            })?;
            if actual != expected {
                return Err("open_target_invalid: target artifact hash changed.".into());
            }
        }
    }
    Ok(ResolvedBookPipelineOpenTarget {
        path,
        kind: selected.kind.clone(),
        action_label: selected.action_label.clone(),
    })
}

fn find_job_artifact<'a>(
    job: &'a BookPipelineJob,
    artifact_id: &str,
) -> Option<&'a BookPipelineArtifact> {
    job.artifacts
        .iter()
        .chain(job.children.iter().flat_map(|child| child.artifacts.iter()))
        .chain(
            job.collection_items
                .iter()
                .flat_map(|item| item.artifacts.iter()),
        )
        .find(|artifact| artifact.artifact_id == artifact_id)
}

fn build_book_pipeline_diagnostic(
    job: &BookPipelineJob,
    profile: &str,
) -> Result<serde_json::Value, String> {
    if !matches!(profile, "local-full" | "redacted-support" | "public-issue") {
        return Err(format!(
            "Unsupported Book Pipeline diagnostic profile: {profile}"
        ));
    }
    let include_safe_error_summary = profile != "public-issue";
    let stages = job
        .stages
        .iter()
        .map(|stage| diagnostic_stage_value(stage, None, include_safe_error_summary))
        .chain(job.children.iter().enumerate().flat_map(|(index, child)| {
            child.stages.iter().map(move |stage| {
                diagnostic_stage_value(stage, Some(index + 1), include_safe_error_summary)
            })
        }))
        .collect::<Vec<_>>();
    let mut root = serde_json::json!({
        "schemaVersion": JOB_SCHEMA_VERSION,
        "profile": profile,
        "status": job.status,
        "currentStageId": job.current_stage_id,
        "stages": stages,
    });
    if profile == "public-issue" {
        return Ok(root);
    }

    let mut seen = BTreeSet::new();
    let artifacts = job
        .artifacts
        .iter()
        .chain(job.children.iter().flat_map(|child| child.artifacts.iter()))
        .filter(|artifact| seen.insert(artifact.artifact_id.clone()))
        .map(|artifact| {
            if profile == "local-full" {
                serde_json::to_value(artifact).unwrap_or_else(|_| serde_json::json!({}))
            } else {
                serde_json::json!({
                    "artifactId": artifact.artifact_id,
                    "kind": artifact.kind,
                    "path": diagnostic_redacted_path(job, &artifact.path),
                    "sha256": artifact.sha256,
                    "sizeBytes": artifact.size_bytes,
                    "producer": artifact.producer,
                    "inputHashes": artifact.input_hashes,
                    "privacy": artifact.privacy,
                    "validation": artifact.validation,
                })
            }
        })
        .collect::<Vec<_>>();
    root["artifacts"] = serde_json::Value::Array(artifacts);
    Ok(root)
}

fn diagnostic_stage_value(
    stage: &BookPipelineStage,
    child_ordinal: Option<usize>,
    include_safe_error_summary: bool,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "stageId": stage.stage_id,
        "status": stage.status,
        "attempt": stage.attempt,
    });
    if let Some(ordinal) = child_ordinal {
        value["child"] = serde_json::json!(format!("child-{ordinal:03}"));
    }
    if let Some(error) = stage.safe_error.as_ref() {
        value["error"] = serde_json::json!({
            "code": error.code,
            "retryable": error.retryable,
            "stageId": error.stage_id,
            "unitId": error.unit_id,
        });
        if include_safe_error_summary {
            value["error"]["summary"] = serde_json::json!(error.summary);
        }
    }
    value
}

fn diagnostic_redacted_path(job: &BookPipelineJob, path: &str) -> String {
    let path = Path::new(path);
    if let Some(output_dir) = job.output_dir.as_deref().map(Path::new) {
        if let Ok(relative) = path.strip_prefix(output_dir) {
            return display_path(&Path::new("<JOB_ROOT>").join(relative));
        }
    }
    for project_root in job
        .children
        .iter()
        .filter_map(|child| child.local_project_root.as_deref())
        .map(Path::new)
    {
        if let Ok(relative) = path.strip_prefix(project_root) {
            return display_path(&Path::new("<PROJECT_ROOT>").join(relative));
        }
    }
    "<PRIVATE_PATH>".into()
}

#[tauri::command]
pub fn handoff_book_pipeline_markdown(
    job_id: String,
    artifact_path: Option<String>,
) -> Result<BookPipelineJob, String> {
    let store = BookPipelineStore::default()?;
    handoff_job_markdown(
        &store,
        &job_id,
        artifact_path.as_deref(),
        &local_reading_repo_root()?,
    )
}

#[tauri::command]
pub fn preview_book_pipeline_cleanup() -> Result<BookPipelineCleanupPreview, String> {
    let store = BookPipelineStore::default()?;
    preview_cleanup_candidates(&store)
}

#[tauri::command]
pub fn approve_book_pipeline_cleanup(
    candidate_id: String,
    explicit_approval: bool,
) -> Result<BookPipelineActionResult, String> {
    let store = BookPipelineStore::default()?;
    approve_cleanup_candidate(&store, &candidate_id, explicit_approval)
}

fn validate_translation_intent(intent: &BookPipelineTranslationIntent) -> Result<(), String> {
    if !matches!(
        intent.translation_mode.as_str(),
        TRANSLATION_MODE_FAST | TRANSLATION_MODE_EXPERT
    ) {
        return Err("translationMode must be fast or expert.".into());
    }
    if intent.profile_id.trim().is_empty() {
        return Err("translationIntent.profileId is required.".into());
    }
    if intent.config_id.trim().is_empty() {
        return Err("translationIntent.configId is required.".into());
    }
    if intent
        .skill_ids
        .iter()
        .any(|skill_id| skill_id.trim().is_empty())
    {
        return Err("translationIntent.skillIds cannot contain an empty ID.".into());
    }
    if intent.translation_mode == TRANSLATION_MODE_FAST && !intent.skill_ids.is_empty() {
        return Err("Fast translation mode cannot declare agent skill IDs.".into());
    }
    if intent.translation_mode == TRANSLATION_MODE_EXPERT && intent.skill_ids.is_empty() {
        return Err("Expert translation mode requires at least one skill ID.".into());
    }
    if intent.translation_mode == TRANSLATION_MODE_EXPERT && intent.second_pass_enabled {
        return Err(
            "The reflection second pass is only available in fast translation mode.".into(),
        );
    }
    if intent.translation_mode == TRANSLATION_MODE_EXPERT && intent.text_cleanup {
        return Err("Text cleanup is only available in fast translation mode.".into());
    }
    let output_formats = normalize_output_formats(&intent.output_formats)?;
    if intent.digest_mode
        && !output_formats
            .iter()
            .any(|format| format == OUTPUT_FORMAT_EPUB)
    {
        return Err("digestMode requires epub in outputFormats.".into());
    }
    Ok(())
}

fn queue_job_with_translation_intent(
    store: &dyn BookPipelineStateStore,
    source: BookPipelineSource,
    mode: String,
    translation_intent: BookPipelineTranslationIntent,
    config: BookPipelinePreviewConfig,
) -> Result<BookPipelineJob, String> {
    queue_job_with_translation_intent_and_executor(
        store,
        &SystemCommandExecutor,
        source,
        mode,
        translation_intent,
        config,
    )
}

fn queue_job_with_translation_intent_and_executor<E: RunnerCommandExecutor>(
    store: &dyn BookPipelineStateStore,
    executor: &E,
    source: BookPipelineSource,
    mode: String,
    translation_intent: BookPipelineTranslationIntent,
    config: BookPipelinePreviewConfig,
) -> Result<BookPipelineJob, String> {
    // The wizard sends overrides on the transient config; persist them onto the
    // source so they survive into the stored job and the run-time route stage.
    let mut source = source;
    if !config.route_overrides.is_empty() {
        source.route_overrides = config.route_overrides.clone();
    }
    let source = source;
    if source.kind == "zotero_collection" && source.fake_zotero_items.is_none() {
        return queue_zotero_collection_snapshot_job(
            store,
            executor,
            source,
            mode,
            translation_intent,
        );
    }
    queue_standard_job_with_translation_intent(
        store,
        executor,
        source,
        mode,
        translation_intent,
        config,
    )
}

fn queue_standard_job_with_translation_intent<E: RunnerCommandExecutor>(
    store: &dyn BookPipelineStateStore,
    executor: &E,
    source: BookPipelineSource,
    mode: String,
    translation_intent: BookPipelineTranslationIntent,
    config: BookPipelinePreviewConfig,
) -> Result<BookPipelineJob, String> {
    queue_standard_job_for_root(
        store,
        executor,
        source,
        mode,
        translation_intent,
        config,
        &book_ocr_conversion_root(),
    )
}

fn queue_standard_job_for_root<E: RunnerCommandExecutor>(
    store: &dyn BookPipelineStateStore,
    executor: &E,
    source: BookPipelineSource,
    mode: String,
    translation_intent: BookPipelineTranslationIntent,
    config: BookPipelinePreviewConfig,
    root: &Path,
) -> Result<BookPipelineJob, String> {
    validate_translation_intent(&translation_intent)?;
    let output_formats = normalize_output_formats(&translation_intent.output_formats)?;
    // A live Zotero source must queue the same worker-discovered route the
    // wizard previewed; the offline preview only ever routes explicit
    // discovery evidence and would otherwise queue phantom children.
    let route = if is_zotero_source(&source) && source.fake_zotero_items.is_none() {
        preview_zotero_route_from_worker(executor, &source, config, 20, root)?
    } else {
        preview_route(&source, &mode, config)
    };
    let mut state = store.load()?;
    let now = now_label();
    let runnable = route_is_runnable_for_source(&source, &route);
    let job_id = new_job_id();
    let output_dir = store.job_output_dir(&job_id);
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let mut job = BookPipelineJob {
        schema_version: String::new(),
        id: job_id,
        kind: String::new(),
        mode,
        translation_mode: translation_intent.translation_mode,
        translation_profile_id: translation_intent.profile_id,
        translation_config_id: translation_intent.config_id,
        translation_skill_ids: translation_intent.skill_ids,
        second_pass_enabled: translation_intent.second_pass_enabled,
        text_cleanup: translation_intent.text_cleanup,
        digest_mode: translation_intent.digest_mode,
        output_formats,
        source,
        route,
        status: if runnable {
            STATUS_ROUTED
        } else {
            STATUS_BLOCKED
        }
        .into(),
        current_stage_id: String::new(),
        current_step: "Route preview recorded".into(),
        last_error: None,
        log_summary: vec!["Route preview recorded".into()],
        artifacts: Vec::new(),
        collection_items: Vec::new(),
        output_dir: Some(display_path(&output_dir)),
        attempts: 0,
        stages: Vec::new(),
        children: Vec::new(),
        membership: None,
        summary: BookPipelineStatusSummary::default(),
        progress: BookPipelineProgress::default(),
        notification_deliveries: Vec::new(),
        approval_references: Vec::new(),
        navigation_targets: Vec::new(),
        open_target: None,
        created_at: now.clone(),
        updated_at: now,
    };
    migrate_legacy_job(&mut job);
    state.jobs.insert(0, job.clone());
    store.save(&state)?;
    Ok(job)
}

fn queue_zotero_collection_snapshot_job<E: RunnerCommandExecutor>(
    store: &dyn BookPipelineStateStore,
    executor: &E,
    mut source: BookPipelineSource,
    mode: String,
    translation_intent: BookPipelineTranslationIntent,
) -> Result<BookPipelineJob, String> {
    validate_translation_intent(&translation_intent)?;
    let output_formats = normalize_output_formats(&translation_intent.output_formats)?;
    let collection_key = non_empty(source.selector.as_deref())
        .ok_or_else(|| "Zotero collection source is missing a collection key.".to_string())?
        .to_string();
    let snapshot = read_zotero_collection_snapshot(executor, &collection_key)?;
    let snapshot_json = serde_json::to_string(&snapshot)
        .map_err(|error| format!("Could not serialize Zotero collection snapshot: {error}"))?;
    let snapshot_sha256 = sha256_str(&snapshot_json);
    let mut state = store.load()?;
    if let Some(existing) = state.jobs.iter().find(|job| {
        job.membership.as_ref().is_some_and(|membership| {
            membership.collection_key == collection_key
                && membership.snapshot_sha256 == snapshot_sha256
        })
    }) {
        return Ok(existing.clone());
    }

    let membership_revision = state
        .jobs
        .iter()
        .filter_map(|job| job.membership.as_ref())
        .filter(|membership| membership.collection_key == collection_key)
        .map(|membership| membership.revision)
        .max()
        .unwrap_or_default()
        .saturating_add(1);
    let now = now_label();
    let job_id = new_job_id();
    let output_dir = store.job_output_dir(&job_id);
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let manifest_path = output_dir.join("collection-snapshot.json");
    let manifest = serde_json::json!({
        "schemaVersion": ZOTERO_COLLECTION_SNAPSHOT_SCHEMA,
        "snapshotSha256": snapshot_sha256,
        "membershipRevision": membership_revision,
        "collection": snapshot.collection,
        "members": snapshot.members,
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("Could not serialize collection manifest: {error}"))?;
    fs::write(&manifest_path, manifest_bytes).map_err(|error| error.to_string())?;

    source.title = Some(snapshot.collection.name.clone());
    source.selector = Some(collection_key.clone());
    let children = snapshot
        .members
        .iter()
        .filter_map(|member| {
            collection_snapshot_child(
                &job_id,
                membership_revision,
                &collection_key,
                member,
                &mode,
                translation_intent.digest_mode,
                &source.route_overrides,
            )
        })
        .collect::<Vec<_>>();
    let eligible_count = children
        .iter()
        .filter(|child| {
            child
                .source_identity
                .as_ref()
                .is_some_and(|identity| identity.eligibility == "eligible_pdf")
        })
        .count();
    let outcome_counts = snapshot_outcome_counts(&snapshot);
    let discover_status = if eligible_count == 0 {
        STATUS_BLOCKED
    } else {
        STATUS_COMPLETED
    };
    let discover_error = (eligible_count == 0).then(|| "no_eligible_pdf".to_string());
    let discover_stage = BookPipelineStage {
        stage_id: "discover".into(),
        status: discover_status.into(),
        attempt: 1,
        error: discover_error.clone(),
        contract_version: JOB_SCHEMA_VERSION.into(),
        input_hashes: BTreeMap::from([
            ("collectionKey".into(), collection_key.clone()),
            ("snapshotSha256".into(), snapshot_sha256.clone()),
            (
                "collectionVersion".into(),
                snapshot.collection.version.to_string(),
            ),
        ]),
        ..BookPipelineStage::default()
    };
    let route = children
        .iter()
        .flat_map(|child| child.route.iter().cloned())
        .collect::<Vec<_>>();
    let child_job_ids = children.iter().map(|child| child.id.clone()).collect();
    let manifest_artifact =
        required_stage_artifact("collection_manifest", &manifest_path, "discover")?;
    let mut job = BookPipelineJob {
        schema_version: JOB_SCHEMA_VERSION.into(),
        id: job_id,
        kind: "collection".into(),
        mode,
        translation_mode: translation_intent.translation_mode,
        translation_profile_id: translation_intent.profile_id,
        translation_config_id: translation_intent.config_id,
        translation_skill_ids: translation_intent.skill_ids,
        second_pass_enabled: translation_intent.second_pass_enabled,
        text_cleanup: translation_intent.text_cleanup,
        digest_mode: translation_intent.digest_mode,
        output_formats,
        source,
        route,
        status: discover_status.into(),
        current_stage_id: "discover".into(),
        current_step: "Collection membership frozen".into(),
        last_error: discover_error,
        log_summary: vec![format!(
            "Collection snapshot frozen: revision={membership_revision} attachments={} eligible={eligible_count} outcomes={}",
            children.len(),
            outcome_counts
                .iter()
                .map(|(outcome, count)| format!("{outcome}:{count}"))
                .collect::<Vec<_>>()
                .join(",")
        )],
        artifacts: vec![manifest_artifact],
        collection_items: Vec::new(),
        output_dir: Some(display_path(&output_dir)),
        attempts: 0,
        stages: vec![discover_stage],
        children,
        membership: Some(BookPipelineMembership {
            revision: membership_revision,
            frozen_at: Some(now.clone()),
            discovery_stage_id: "discover".into(),
            child_job_ids,
            collection_key: collection_key.clone(),
            snapshot_sha256: snapshot_sha256.clone(),
        }),
        summary: BookPipelineStatusSummary::default(),
        progress: BookPipelineProgress::default(),
        notification_deliveries: Vec::new(),
        approval_references: Vec::new(),
        navigation_targets: Vec::new(),
        open_target: None,
        created_at: now.clone(),
        updated_at: now,
    };
    derive_job(&mut job);
    state.jobs.insert(0, job.clone());
    validate_state(&state)?;
    if let Err(error) = store.save(&state) {
        let _ = fs::remove_dir_all(&output_dir);
        if let Ok(current) = store.load() {
            if let Some(existing) = current.jobs.iter().find(|stored| {
                stored.membership.as_ref().is_some_and(|membership| {
                    membership.collection_key == collection_key
                        && membership.snapshot_sha256 == snapshot_sha256
                })
            }) {
                return Ok(existing.clone());
            }
        }
        return Err(error);
    }
    Ok(job)
}

fn build_zotero_collection_snapshot_command(collection_key: &str) -> Result<RunnerCommand, String> {
    let repo_root = local_reading_repo_root()?;
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: ZOTERO_COLLECTION_SNAPSHOT_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args: vec![
            "run".into(),
            "--package".into(),
            "zotero-cli-agent".into(),
            "zsearch".into(),
            "collection-snapshot".into(),
            collection_key.into(),
        ],
        env: Vec::new(),
        cwd: Some(repo_root.clone()),
        output_dir: repo_root,
        attempts: 1,
        accepted_exit_codes: vec![0],
    })
}

fn read_zotero_collection_snapshot<E: RunnerCommandExecutor>(
    executor: &E,
    collection_key: &str,
) -> Result<ZoteroCollectionSnapshot, String> {
    let command = build_zotero_collection_snapshot_command(collection_key)?;
    let command_result = executor.execute(&command).map_err(|error| {
        redact_runner_message(&format!("Zotero collection snapshot failed: {error}"))
    })?;
    let snapshot: ZoteroCollectionSnapshot = serde_json::from_str(command_result.stdout.trim())
        .map_err(|_| "Zotero collection snapshot returned invalid JSON.".to_string())?;
    validate_zotero_collection_snapshot(&snapshot, collection_key)?;
    Ok(snapshot)
}

fn validate_zotero_collection_snapshot(
    snapshot: &ZoteroCollectionSnapshot,
    requested_collection_key: &str,
) -> Result<(), String> {
    if snapshot.schema_version != ZOTERO_COLLECTION_SNAPSHOT_SCHEMA
        || snapshot.collection.key != requested_collection_key
        || snapshot.collection.name.trim().is_empty()
    {
        return Err("Zotero collection snapshot identity does not match the request.".into());
    }
    let mut previous_identity: Option<(String, String)> = None;
    let mut attachment_keys = BTreeSet::new();
    for member in &snapshot.members {
        if member.collection_key != requested_collection_key
            || member.parent_item_key.trim().is_empty()
            || member.parent_item_type.trim().is_empty()
            || member.parent_date_modified.trim().is_empty()
        {
            return Err("Zotero collection snapshot contains incomplete parent identity.".into());
        }
        let identity = (
            member.parent_item_key.clone(),
            member.attachment_key.clone().unwrap_or_default(),
        );
        if previous_identity
            .as_ref()
            .is_some_and(|previous| previous > &identity)
        {
            return Err("Zotero collection snapshot membership is not stably ordered.".into());
        }
        previous_identity = Some(identity);
        match member.attachment_key.as_deref() {
            Some(attachment_key) => {
                if attachment_key.trim().is_empty()
                    || !attachment_keys.insert(attachment_key.to_string())
                    || member.attachment_version.is_none()
                    || member
                        .attachment_date_modified
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty())
                    || member.link_mode.is_none()
                    || !matches!(
                        member.eligibility.as_str(),
                        "eligible_pdf"
                            | "unsupported_content_type"
                            | "unresolved_path"
                            | "missing_file"
                    )
                {
                    return Err(
                        "Zotero collection snapshot contains incomplete attachment identity."
                            .into(),
                    );
                }
                let has_resolved_path = member
                    .attachment_path
                    .as_deref()
                    .is_some_and(|path| !path.trim().is_empty());
                let has_file_evidence =
                    member.file_size.is_some() && member.file_mtime_ns.is_some();
                let has_no_file_evidence =
                    member.file_size.is_none() && member.file_mtime_ns.is_none();
                let has_reason = member
                    .reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty());
                let is_pdf = snapshot_member_has_pdf_evidence(member);
                let evidence_is_consistent = match member.eligibility.as_str() {
                    "eligible_pdf" => {
                        is_pdf
                            && snapshot_member_path_mode_is_safe(member)
                            && member.path_exists
                            && has_resolved_path
                            && has_file_evidence
                            && member.reason.is_none()
                    }
                    "missing_file" => {
                        is_pdf
                            && snapshot_member_path_mode_is_safe(member)
                            && !member.path_exists
                            && has_resolved_path
                            && has_no_file_evidence
                            && has_reason
                    }
                    "unresolved_path" => {
                        is_pdf
                            && !member.path_exists
                            && !has_resolved_path
                            && has_no_file_evidence
                            && has_reason
                    }
                    "unsupported_content_type" => {
                        !is_pdf
                            && has_reason
                            && (!member.path_exists || (has_resolved_path && has_file_evidence))
                            && (member.path_exists || has_no_file_evidence)
                    }
                    _ => false,
                };
                if !evidence_is_consistent {
                    return Err(
                        "Zotero collection snapshot contains inconsistent attachment evidence."
                            .into(),
                    );
                }
            }
            None => {
                if member.eligibility != "no_attachment"
                    || member.attachment_version.is_some()
                    || member.attachment_date_modified.is_some()
                    || member.content_type.is_some()
                    || member.link_mode.is_some()
                    || member.storage_path.is_some()
                    || member.attachment_path.is_some()
                    || member.path_exists
                    || member.file_size.is_some()
                    || member.file_mtime_ns.is_some()
                    || member
                        .reason
                        .as_deref()
                        .is_none_or(|reason| reason.trim().is_empty())
                {
                    return Err(
                        "Zotero collection snapshot has an invalid no-attachment outcome.".into(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn snapshot_member_has_pdf_evidence(member: &ZoteroCollectionSnapshotMember) -> bool {
    let content_type = member
        .content_type
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !content_type.is_empty() {
        return content_type == "application/pdf";
    }
    member
        .storage_path
        .as_deref()
        .or(member.attachment_path.as_deref())
        .is_some_and(|path| path.to_ascii_lowercase().ends_with(".pdf"))
}

fn snapshot_member_path_mode_is_safe(member: &ZoteroCollectionSnapshotMember) -> bool {
    let Some(storage_path) = member.storage_path.as_deref() else {
        return false;
    };
    if let Some(relative) = storage_path.strip_prefix("storage:") {
        let path = Path::new(relative);
        let Some(attachment_key) = member.attachment_key.as_deref() else {
            return false;
        };
        let Some(resolved_path) = member.attachment_path.as_deref() else {
            return false;
        };
        let resolved_path = Path::new(resolved_path);
        let expected_suffix = Path::new(attachment_key).join(relative);
        return matches!(member.link_mode, Some(0 | 1))
            && !relative.is_empty()
            && !path.is_absolute()
            && resolved_path.is_absolute()
            && resolved_path.ends_with(expected_suffix)
            && !path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::RootDir
                )
            });
    }
    member.link_mode == Some(2)
        && Path::new(storage_path).is_absolute()
        && member.attachment_path.as_deref() == Some(storage_path)
}

fn collection_snapshot_child(
    parent_job_id: &str,
    membership_revision: u32,
    collection_key: &str,
    member: &ZoteroCollectionSnapshotMember,
    mode: &str,
    digest_mode: bool,
    route_overrides: &BTreeMap<String, String>,
) -> Option<BookPipelineChildJob> {
    let attachment_key = member.attachment_key.as_deref()?;
    let eligible = member.eligibility == "eligible_pdf";
    let route = collection_snapshot_route(member)?;
    let title = route.title.clone();
    let reason = member.reason.clone();
    let stages = ordered_child_stage_ids(mode, false)
        .into_iter()
        .map(|stage_id| BookPipelineStage {
            stage_id: stage_id.into(),
            status: if stage_id == "route" {
                if eligible {
                    STATUS_READY.into()
                } else {
                    STATUS_BLOCKED.into()
                }
            } else if stage_id == "build_digest" && !digest_mode {
                STATUS_SKIPPED.into()
            } else {
                STATUS_PENDING.into()
            },
            error: (stage_id == "route" && !eligible)
                .then(|| reason.clone().unwrap_or_else(|| member.eligibility.clone())),
            contract_version: stage_contract_version(stage_id).into(),
            ..BookPipelineStage::default()
        })
        .collect();
    let mut child = BookPipelineChildJob {
        id: format!("{parent_job_id}-r{membership_revision}-{attachment_key}"),
        parent_job_id: parent_job_id.into(),
        status: STATUS_PENDING.into(),
        current_stage_id: "route".into(),
        source: BookPipelineSource {
            kind: "zotero_attachment".into(),
            title: Some(title),
            path: member.attachment_path.clone(),
            selector: Some(attachment_key.into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            // Carry only this attachment's override. The child's route is
            // recomputed from live worker evidence at run time, and this is the
            // only place the user's choice survives to be re-applied there.
            route_overrides: route_overrides
                .get(attachment_key)
                .map(|token| BTreeMap::from([(attachment_key.to_string(), token.clone())]))
                .unwrap_or_default(),
        },
        route: vec![route],
        stages,
        artifacts: Vec::new(),
        attempts: 0,
        last_error: (!eligible)
            .then(|| reason.clone().unwrap_or_else(|| member.eligibility.clone())),
        local_project_root: None,
        source_identity: Some(BookPipelineAttachmentIdentity {
            collection_key: collection_key.into(),
            parent_item_key: member.parent_item_key.clone(),
            pdf_attachment_key: attachment_key.into(),
            parent_item_version: member.parent_item_version,
            attachment_version: member.attachment_version.unwrap_or_default(),
            parent_date_modified: member.parent_date_modified.clone(),
            attachment_date_modified: member.attachment_date_modified.clone().unwrap_or_default(),
            content_type: member.content_type.clone().unwrap_or_default(),
            link_mode: member.link_mode.unwrap_or_default(),
            attachment_path: member.attachment_path.clone().unwrap_or_default(),
            path_exists: member.path_exists,
            file_size: member.file_size,
            file_mtime_ns: member.file_mtime_ns,
            eligibility: member.eligibility.clone(),
            reason: member.reason.clone(),
        }),
        custom_instructions: None,
        reader_evidence: Vec::new(),
    };
    derive_child(&mut child);
    Some(child)
}

fn collection_snapshot_route(
    member: &ZoteroCollectionSnapshotMember,
) -> Option<BookPipelineRouteItem> {
    let attachment_key = member.attachment_key.as_deref()?;
    let eligible = member.eligibility == "eligible_pdf";
    let title = member
        .title
        .clone()
        .unwrap_or_else(|| format!("Zotero attachment {attachment_key}"));
    let reason = member.reason.clone();
    Some(BookPipelineRouteItem {
        id: attachment_key.into(),
        title,
        source_kind: "zotero_attachment".into(),
        source_ref: member
            .attachment_path
            .clone()
            .unwrap_or_else(|| format!("zotero://attachment/{attachment_key}")),
        route_kind: if eligible {
            "pending_route".into()
        } else {
            member.eligibility.clone()
        },
        can_run: eligible,
        blocked_reason: (!eligible)
            .then(|| reason.clone().unwrap_or_else(|| member.eligibility.clone())),
        summary: if eligible {
            "Attachment is frozen and ready for route selection.".into()
        } else {
            reason.unwrap_or_else(|| "Attachment is not eligible for routing.".into())
        },
        route_override: None,
    })
}

#[cfg(test)]
fn queue_job(
    store: &dyn BookPipelineStateStore,
    source: BookPipelineSource,
    mode: String,
    config: BookPipelinePreviewConfig,
) -> Result<BookPipelineJob, String> {
    queue_job_with_translation_intent(
        store,
        source,
        mode,
        BookPipelineTranslationIntent {
            translation_mode: TRANSLATION_MODE_FAST.into(),
            profile_id: "fake-provider-profile".into(),
            config_id: "fake-provider-config".into(),
            skill_ids: Vec::new(),
            second_pass_enabled: false,
            text_cleanup: false,
            digest_mode: false,
            output_formats: default_output_formats(),
        },
        config,
    )
}

fn handoff_job_markdown(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    artifact_path: Option<&str>,
    repo_root: &Path,
) -> Result<BookPipelineJob, String> {
    handoff_job_markdown_with_runner(
        store,
        job_id,
        artifact_path,
        repo_root,
        &LocalProjectHandoffRunner,
    )
}

fn handoff_job_markdown_with_runner(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    artifact_path: Option<&str>,
    repo_root: &Path,
    handoff_runner: &dyn TranslationHandoffRunner,
) -> Result<BookPipelineJob, String> {
    let mut state = store.load()?;
    let index = state
        .jobs
        .iter()
        .position(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found.".to_string())?;
    state.jobs[index].current_step = "Preparing translation handoff".into();
    state.jobs[index].last_error = None;
    state.jobs[index]
        .log_summary
        .push("Translation handoff started".into());
    state.jobs[index].updated_at = now_label();
    let execution_owner = store.execution_owner()?;
    let handoff_child_id = mark_handoff_running(&mut state.jobs[index], execution_owner)
        .ok_or_else(|| "No completed extraction is ready for translation handoff.".to_string())?;
    store.save(&state)?;

    let result = handoff_runner.handoff(&state.jobs[index], artifact_path, repo_root);
    let mut state = store.load()?;
    let index = state
        .jobs
        .iter()
        .position(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found after handoff.".to_string())?;
    match result {
        Ok(handoff) => {
            state.jobs[index].current_step = "Translation handoff ready".into();
            state.jobs[index].last_error = None;
            state.jobs[index]
                .artifacts
                .extend(handoff.artifacts.clone());
            state.jobs[index]
                .log_summary
                .extend(handoff.log_summary.clone());
            mark_handoff_finished(
                &mut state.jobs[index],
                Some(&handoff_child_id),
                Ok(&handoff),
            );
        }
        Err(error) => {
            state.jobs[index].current_step = "Translation handoff failed".into();
            state.jobs[index].last_error = Some(redact_runner_message(&error));
            state.jobs[index]
                .log_summary
                .push(redact_runner_message(&format!(
                    "Translation handoff failed: {error}"
                )));
            mark_handoff_finished(&mut state.jobs[index], Some(&handoff_child_id), Err(&error));
        }
    }
    state.jobs[index].log_summary = trim_log_summary(&state.jobs[index].log_summary);
    state.jobs[index].updated_at = now_label();
    let job = state.jobs[index].clone();
    store.save(&state)?;
    Ok(job)
}

fn preview_cleanup_candidates(
    store: &dyn BookPipelineStateStore,
) -> Result<BookPipelineCleanupPreview, String> {
    let state = store.load()?;
    let candidates = cleanup_candidates_from_jobs(&state.jobs);
    Ok(BookPipelineCleanupPreview {
        log_summary: vec![format!(
            "Found {} cleanup candidate(s) from Book Pipeline job history",
            candidates.len()
        )],
        candidates,
    })
}

fn approve_cleanup_candidate(
    store: &dyn BookPipelineStateStore,
    candidate_id: &str,
    explicit_approval: bool,
) -> Result<BookPipelineActionResult, String> {
    if !explicit_approval {
        return Err("Explicit cleanup approval is required.".into());
    }
    let preview = preview_cleanup_candidates(store)?;
    let candidate = preview
        .candidates
        .into_iter()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| "Cleanup candidate not found.".to_string())?;
    if !candidate.can_approve {
        let missing = candidate
            .checks
            .iter()
            .filter(|check| !check.ok)
            .map(|check| check.kind.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Cleanup approval blocked; missing evidence: {missing}"
        ));
    }

    let mut state = store.load()?;
    let index = state
        .jobs
        .iter()
        .position(|job| job.id == candidate.job_id)
        .ok_or_else(|| "Book Pipeline job not found for cleanup approval.".to_string())?;
    let now = now_label();
    state.jobs[index].log_summary.push(format!(
        "Cleanup approval recorded at {now} for {}; existing cleanup wrapper remains the deletion path",
        candidate.source_ref
    ));
    state.jobs[index].log_summary = trim_log_summary(&state.jobs[index].log_summary);
    state.jobs[index].updated_at = now;
    store.save(&state)?;

    Ok(BookPipelineActionResult {
        ok: true,
        message:
            "Cleanup approval recorded. The launcher did not delete any source PDF; existing cleanup scripts remain the deletion path."
                .into(),
        path: candidate.source_path.or(candidate.markdown_path),
    })
}

fn cleanup_candidates_from_jobs(jobs: &[BookPipelineJob]) -> Vec<BookPipelineCleanupCandidate> {
    jobs.iter().filter_map(cleanup_candidate_for_job).collect()
}

fn cleanup_candidate_for_job(job: &BookPipelineJob) -> Option<BookPipelineCleanupCandidate> {
    if !is_zotero_source(&job.source)
        && !job
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown")
    {
        return None;
    }
    let markdown = job
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown");
    let markdown_path = markdown.map(|artifact| artifact.path.clone());
    let zotero_child_attachment_key = markdown.and_then(|artifact| artifact.zotero_key.clone());
    let local_output_path = cleanup_local_output_path(job);
    let checks = vec![
        cleanup_markdown_check(markdown),
        cleanup_local_output_check(job),
        cleanup_zotero_child_check(zotero_child_attachment_key.as_deref()),
    ];
    let can_approve = checks.iter().all(|check| check.ok);
    let source_ref = job
        .source
        .selector
        .clone()
        .or_else(|| job.source.path.clone())
        .unwrap_or_else(|| job.id.clone());

    Some(BookPipelineCleanupCandidate {
        id: format!("cleanup-{}", job.id),
        job_id: job.id.clone(),
        title: source_title(&job.source),
        source_kind: job.source.kind.clone(),
        source_ref,
        source_path: job.source.path.clone(),
        source_pdf_key: job.source.selector.clone(),
        markdown_path,
        local_output_path,
        zotero_child_attachment_key,
        checks,
        can_approve,
    })
}

fn cleanup_markdown_check(markdown: Option<&BookPipelineArtifact>) -> BookPipelineCleanupEvidence {
    let Some(markdown) = markdown else {
        return cleanup_evidence(
            "markdown_output",
            false,
            "Missing Markdown artifact.",
            None,
            None,
        );
    };
    cleanup_artifact_file_check(
        "markdown_output",
        markdown,
        "Markdown output exists and checksum matches.",
        "Markdown artifact is missing, empty, or checksum mismatched.",
    )
}

fn cleanup_local_output_check(job: &BookPipelineJob) -> BookPipelineCleanupEvidence {
    if let Some(path) = cleanup_output_dir_path(job) {
        let path_buf = PathBuf::from(&path);
        if path_buf.is_dir() {
            return cleanup_evidence(
                "local_output",
                true,
                "Local output directory exists.",
                Some(path),
                None,
            );
        }
    }
    if let Some(artifact) = job.artifacts.iter().find(|artifact| {
        matches!(
            artifact.kind.as_str(),
            "html" | "epub" | "translation_source"
        )
    }) {
        return cleanup_artifact_file_check(
            "local_output",
            artifact,
            "Local deliverable exists and checksum matches.",
            "Local deliverable is missing, empty, or checksum mismatched.",
        );
    }
    cleanup_evidence(
        "local_output",
        false,
        "Missing local output directory or deliverable artifact.",
        None,
        None,
    )
}

fn cleanup_zotero_child_check(zotero_key: Option<&str>) -> BookPipelineCleanupEvidence {
    match non_empty(zotero_key) {
        Some(key) => cleanup_evidence(
            "zotero_child_attachment",
            true,
            "Zotero Markdown child attachment key is recorded.",
            None,
            Some(key.to_string()),
        ),
        None => cleanup_evidence(
            "zotero_child_attachment",
            false,
            "Missing Zotero Markdown child attachment key.",
            None,
            None,
        ),
    }
}

fn cleanup_artifact_file_check(
    kind: &str,
    artifact: &BookPipelineArtifact,
    ok_detail: &str,
    missing_detail: &str,
) -> BookPipelineCleanupEvidence {
    let path = PathBuf::from(&artifact.path);
    let ok = artifact.sha256.as_deref().is_some_and(|expected| {
        path.is_file()
            && path.metadata().is_ok_and(|metadata| metadata.len() > 0)
            && sha256_file(&path).is_ok_and(|actual| actual == expected)
    });
    cleanup_evidence(
        kind,
        ok,
        if ok { ok_detail } else { missing_detail },
        Some(artifact.path.clone()),
        artifact.zotero_key.clone(),
    )
}

fn cleanup_output_dir_path(job: &BookPipelineJob) -> Option<String> {
    job.output_dir.clone().or_else(|| {
        job.artifacts
            .iter()
            .find(|artifact| artifact.kind == "output_dir")
            .map(|artifact| artifact.path.clone())
    })
}

fn cleanup_local_output_path(job: &BookPipelineJob) -> Option<String> {
    cleanup_output_dir_path(job).or_else(|| {
        job.artifacts
            .iter()
            .find(|artifact| {
                matches!(
                    artifact.kind.as_str(),
                    "html" | "epub" | "translation_source"
                )
            })
            .map(|artifact| artifact.path.clone())
    })
}

fn cleanup_evidence(
    kind: &str,
    ok: bool,
    detail: &str,
    path: Option<String>,
    zotero_key: Option<String>,
) -> BookPipelineCleanupEvidence {
    BookPipelineCleanupEvidence {
        kind: kind.into(),
        ok,
        detail: detail.into(),
        path,
        zotero_key,
    }
}

fn stage_mut<'a>(
    child: &'a mut BookPipelineChildJob,
    stage_id: &str,
) -> Option<&'a mut BookPipelineStage> {
    child
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == stage_id)
}

fn set_stage_status(
    child: &mut BookPipelineChildJob,
    stage_id: &str,
    status: &str,
    error: Option<String>,
) {
    debug_assert_ne!(status, STATUS_RUNNING);
    let now = now_label();
    if let Some(stage) = stage_mut(child, stage_id) {
        if matches!(
            status,
            STATUS_COMPLETED | STATUS_FAILED | STATUS_BLOCKED | STATUS_SKIPPED
        ) {
            stage.finished_at = Some(now);
        }
        stage.status = status.into();
        stage.error = error;
        stage.execution_owner = None;
    }
}

fn set_agent_handoff_waiting(
    child: &mut BookPipelineChildJob,
    stage_id: &str,
    agent_profile_id: &str,
) {
    if let Some(stage) = stage_mut(child, stage_id) {
        stage.status = STATUS_BLOCKED.into();
        stage.error = None;
        stage.finished_at = Some(now_label());
        stage.execution_owner = Some(format!("{AGENT_EXECUTION_OWNER_PREFIX}{agent_profile_id}"));
    }
}

fn start_stage(child: &mut BookPipelineChildJob, stage_id: &str, execution_owner: &str) {
    let now = now_label();
    if let Some(stage) = stage_mut(child, stage_id) {
        if stage.status != STATUS_RUNNING {
            stage.attempt = stage.attempt.saturating_add(1);
            stage.started_at = Some(now);
            stage.finished_at = None;
        }
        stage.status = STATUS_RUNNING.into();
        stage.error = None;
        stage.execution_owner = Some(execution_owner.into());
        // A stage that is running again is not a stage anyone has given up on:
        // an operator-driven advance restarts the automatic budget from here.
        stage.give_up_reason = None;
        stage.next_retry_at = None;
    }
}

/// Stage-level auto-retry budget. The translation engine already retries
/// individual chunks inside one run
/// (`packages/translation-engine/src/translation_engine/pipeline.py`), so the
/// runner deliberately counts *stages* rather than chunks — layering the two
/// counters would charge one flaky provider call twice and multiply the real
/// ceiling by the engine's own ladder.
const DEFAULT_STAGE_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_STAGE_RETRY_BACKOFF_SECONDS: [u32; 2] = [2, 5];
const GIVE_UP_RETRY_BUDGET_EXHAUSTED: &str = "retry_budget_exhausted";
const GIVE_UP_NOT_RETRYABLE: &str = "not_retryable";

fn stage_max_attempts(stage: &BookPipelineStage) -> u32 {
    if stage.max_attempts > 0 {
        stage.max_attempts
    } else {
        DEFAULT_STAGE_MAX_ATTEMPTS
    }
}

fn stage_retry_backoff_seconds(stage: &BookPipelineStage, attempt: u32) -> u32 {
    let table: &[u32] = if stage.retry_backoff_seconds.is_empty() {
        &DEFAULT_STAGE_RETRY_BACKOFF_SECONDS
    } else {
        &stage.retry_backoff_seconds
    };
    // Attempt 1 waits the first entry; past the end the last entry repeats, so a
    // short table means a flat wait rather than no wait.
    table
        .get(attempt.saturating_sub(1) as usize)
        .or_else(|| table.last())
        .copied()
        .unwrap_or_default()
}

fn stage_attempts_remaining(stage: &BookPipelineStage) -> u32 {
    if stage.give_up_reason.is_some() {
        return 0;
    }
    stage_max_attempts(stage).saturating_sub(stage.attempt)
}

/// Decide what happens to a stage that just failed. Runs after `derive_job` has
/// classified the failure, because `safe_error.retryable` is what it turns on:
/// a non-retryable failure must not burn an attempt or wait a backoff.
fn schedule_stage_retry(child: &mut BookPipelineChildJob, stage_id: &str) -> Option<u32> {
    let stage = stage_mut(child, stage_id)?;
    if stage.status != STATUS_FAILED {
        stage.give_up_reason = None;
        stage.next_retry_at = None;
        return None;
    }
    // Pin the resolved budget onto the stage the first time it matters, so the
    // policy is self-describing state rather than a default the UI has to know.
    stage.max_attempts = stage_max_attempts(stage);
    if stage.retry_backoff_seconds.is_empty() {
        stage.retry_backoff_seconds = DEFAULT_STAGE_RETRY_BACKOFF_SECONDS.to_vec();
    }
    let retryable = stage
        .safe_error
        .as_ref()
        .is_some_and(|error| error.retryable);
    if !retryable {
        stage.give_up_reason = Some(GIVE_UP_NOT_RETRYABLE.into());
        stage.next_retry_at = None;
        return None;
    }
    if stage.attempt >= stage_max_attempts(stage) {
        stage.give_up_reason = Some(GIVE_UP_RETRY_BUDGET_EXHAUSTED.into());
        stage.next_retry_at = None;
        return None;
    }
    let seconds = stage_retry_backoff_seconds(stage, stage.attempt);
    stage.give_up_reason = None;
    stage.next_retry_at = Some(offset_label(seconds));
    Some(seconds)
}

fn mark_route_blocked(job: &mut BookPipelineJob, error: &str) {
    for child in &mut job.children {
        if child.status == STATUS_READY || child.status == STATUS_PENDING {
            set_stage_status(child, "route", STATUS_BLOCKED, Some(error.into()));
        }
    }
    derive_job(job);
}

fn mark_extract_running(job: &mut BookPipelineJob, execution_owner: &str) -> usize {
    let mut started = 0;
    for child in &mut job.children {
        let runnable = stage_mut(child, "extract")
            .is_some_and(|stage| matches!(stage.status.as_str(), STATUS_READY | STATUS_FAILED));
        if runnable {
            start_stage(child, "extract", execution_owner);
            child.attempts = child.attempts.saturating_add(1);
            started += 1;
        }
    }
    derive_job(job);
    started
}

fn mark_extract_failed(job: &mut BookPipelineJob, error: &str) {
    for child in &mut job.children {
        if stage_mut(child, "extract").is_some_and(|stage| stage.status == STATUS_RUNNING) {
            set_stage_status(child, "extract", STATUS_FAILED, Some(error.into()));
            child.last_error = Some(error.into());
        }
    }
    derive_job(job);
}

fn ensure_translation_stages(child: &mut BookPipelineChildJob, digest_mode: bool) {
    let existing = child
        .stages
        .iter()
        .map(|stage| stage.stage_id.clone())
        .collect::<Vec<_>>();
    let extract_completed =
        stage_mut(child, "extract").is_some_and(|stage| stage.status == STATUS_COMPLETED);
    for stage_id in ordered_child_stage_ids(MODE_CONVERT_THEN_TRANSLATE, true) {
        if existing.iter().any(|candidate| candidate == stage_id) {
            continue;
        }
        child.stages.push(BookPipelineStage {
            stage_id: stage_id.into(),
            status: if stage_id == "index" && child.source.kind != "zotero_attachment" {
                STATUS_SKIPPED.into()
            } else if stage_id == "index" && extract_completed {
                STATUS_READY.into()
            } else if stage_id == "build_digest" && !digest_mode {
                STATUS_SKIPPED.into()
            } else {
                STATUS_PENDING.into()
            },
            contract_version: stage_contract_version(stage_id).into(),
            ..BookPipelineStage::default()
        });
    }
    if stage_mut(child, "extract").is_some_and(|stage| stage.status == STATUS_COMPLETED)
        && stage_mut(child, "index")
            .is_some_and(|stage| matches!(stage.status.as_str(), STATUS_COMPLETED | STATUS_SKIPPED))
        && stage_mut(child, "handoff").is_some_and(|stage| stage.status == STATUS_PENDING)
    {
        set_stage_status(child, "handoff", STATUS_READY, None);
    }
}

fn mark_handoff_running(job: &mut BookPipelineJob, execution_owner: &str) -> Option<String> {
    let digest_mode = job.digest_mode;
    let index = job.children.iter().position(|child| {
        child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .is_some_and(|stage| stage.status == STATUS_COMPLETED)
            && child
                .stages
                .iter()
                .find(|stage| stage.stage_id == "index")
                .is_some_and(|stage| {
                    matches!(stage.status.as_str(), STATUS_COMPLETED | STATUS_SKIPPED)
                })
            && child
                .stages
                .iter()
                .find(|stage| stage.stage_id == "handoff")
                .map(|stage| {
                    matches!(
                        stage.status.as_str(),
                        STATUS_PENDING | STATUS_READY | STATUS_FAILED
                    )
                })
                .unwrap_or(true)
    })?;
    let child = &mut job.children[index];
    ensure_translation_stages(child, digest_mode);
    start_stage(child, "handoff", execution_owner);
    let child_id = child.id.clone();
    derive_job(job);
    Some(child_id)
}

/// `mark_handoff_finished` can only find a child it already started, and nothing
/// started here. The failure belongs on the child whose extraction did complete:
/// that is the handoff the user will retry, and pinning it there keeps the
/// completed extraction out of the retry.
fn mark_handoff_unavailable(job: &mut BookPipelineJob, error: &str) {
    let digest_mode = job.digest_mode;
    let Some(child) = job.children.iter_mut().find(|child| {
        stage_ref(child, "extract").is_some_and(|stage| stage.status == STATUS_COMPLETED)
            && stage_ref(child, "handoff").is_none_or(|stage| stage.status != STATUS_COMPLETED)
    }) else {
        // Nothing extracted successfully, so the per-child failures
        // `apply_runner_output_to_children` just recorded are the real story.
        derive_job(job);
        return;
    };
    ensure_translation_stages(child, digest_mode);
    set_stage_status(child, "handoff", STATUS_FAILED, Some(error.to_string()));
    child.last_error = Some(error.to_string());
    derive_job(job);
}

fn mark_handoff_finished(
    job: &mut BookPipelineJob,
    child_id: Option<&str>,
    result: Result<&TranslationHandoffOutput, &str>,
) {
    let index = child_id
        .and_then(|child_id| job.children.iter().position(|child| child.id == child_id))
        .or_else(|| {
            job.children.iter().position(|child| {
                child
                    .stages
                    .iter()
                    .find(|stage| stage.stage_id == "handoff")
                    .is_some_and(|stage| stage.status == STATUS_RUNNING)
            })
        });
    let Some(index) = index else {
        return;
    };
    let child = &mut job.children[index];
    match result {
        Ok(handoff) => {
            set_stage_status(child, "handoff", STATUS_COMPLETED, None);
            set_stage_status(child, "split", STATUS_READY, None);
            child.artifacts.extend(handoff.artifacts.clone());
            child.last_error = None;
        }
        Err(error) => {
            let redacted = redact_runner_message(error);
            set_stage_status(child, "handoff", STATUS_FAILED, Some(redacted.clone()));
            child.last_error = Some(redacted);
        }
    }
    derive_job(job);
}

fn apply_runner_output_to_children(job: &mut BookPipelineJob) {
    if !job.collection_items.is_empty() {
        for item in &job.collection_items {
            let Some(child) = job
                .children
                .iter_mut()
                .find(|child| child.source.selector.as_deref() == Some(item.id.as_str()))
            else {
                continue;
            };
            child.attempts = item.attempts;
            merge_artifacts(&mut child.artifacts, &item.artifacts);
            child.last_error = item.last_error.clone();
            let route_blocked =
                stage_mut(child, "route").is_some_and(|stage| stage.status == STATUS_BLOCKED);
            if item.status == STATUS_BLOCKED && route_blocked {
                set_stage_status(child, "route", STATUS_BLOCKED, item.last_error.clone());
                set_stage_status(child, "extract", STATUS_PENDING, None);
            } else {
                set_stage_status(child, "extract", &item.status, item.last_error.clone());
            }
        }
    } else if let Some(child) = job.children.first_mut() {
        set_stage_status(child, "extract", STATUS_COMPLETED, None);
        child.artifacts = job.artifacts.clone();
        child.attempts = job.attempts;
        child.last_error = None;
    }
    prepare_item_index_after_extract(job);
    derive_job(job);
}

fn prepare_item_index_after_extract(job: &mut BookPipelineJob) {
    for child in &mut job.children {
        let extract_completed =
            stage_mut(child, "extract").is_some_and(|stage| stage.status == STATUS_COMPLETED);
        if !extract_completed {
            continue;
        }
        if child_requires_item_index(&child.source) {
            if stage_mut(child, "index").is_some_and(|stage| stage.status == STATUS_PENDING) {
                set_stage_status(child, "index", STATUS_READY, None);
            }
        } else {
            set_stage_status(child, "index", STATUS_SKIPPED, None);
        }
    }
}

fn merge_artifacts(existing: &mut Vec<BookPipelineArtifact>, incoming: &[BookPipelineArtifact]) {
    for artifact in incoming {
        let already_registered = existing.iter().any(|candidate| {
            candidate.kind == artifact.kind
                && candidate.path == artifact.path
                && candidate.sha256 == artifact.sha256
                && candidate.zotero_key == artifact.zotero_key
        });
        if !already_registered {
            existing.push(artifact.clone());
        }
    }
}

fn run_job(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    job_id: &str,
) -> Result<BookPipelineJob, String> {
    run_job_with_handoff(store, runner, &LocalProjectHandoffRunner, job_id, None)
}

fn retry_job(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    job_id: &str,
) -> Result<BookPipelineJob, String> {
    retry_job_with_handoff(store, runner, &LocalProjectHandoffRunner, job_id, None)
}

fn run_job_to_quiescence(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    job_id: &str,
) -> Result<BookPipelineJob, String> {
    let state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    if !is_durable_collection_job(&state.jobs[job_index]) {
        drop(state);
        return run_job(store, runner, job_id);
    }
    drop(state);
    run_job_to_quiescence_with_handoff(store, runner, &LocalProjectHandoffRunner, job_id, None)
}

fn retry_job_to_quiescence(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    job_id: &str,
) -> Result<BookPipelineJob, String> {
    let state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    if !is_durable_collection_job(&state.jobs[job_index]) {
        drop(state);
        return retry_job(store, runner, job_id);
    }
    drop(state);
    retry_job_to_quiescence_with_handoff(store, runner, &LocalProjectHandoffRunner, job_id, None)
}

fn runnable_item_index_child(job: &BookPipelineJob, child_id: Option<&str>) -> Option<usize> {
    job.children.iter().position(|child| {
        child_id.is_none_or(|child_id| child.id == child_id)
            && stage_ref(child, "extract").is_some_and(|stage| stage.status == STATUS_COMPLETED)
            && stage_ref(child, "index")
                .is_some_and(|stage| matches!(stage.status.as_str(), STATUS_READY | STATUS_FAILED))
    })
}

fn stage_ref<'a>(child: &'a BookPipelineChildJob, stage_id: &str) -> Option<&'a BookPipelineStage> {
    child.stages.iter().find(|stage| stage.stage_id == stage_id)
}

fn run_item_index_stage(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    job_id: &str,
    child_id: Option<&str>,
) -> Result<BookPipelineJob, String> {
    let (planned_job, planned_child, markdown) = {
        let state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index =
            runnable_item_index_child(&state.jobs[job_index], child_id).ok_or_else(|| {
                "No eligible item-scoped index stage is ready to run or retry.".to_string()
            })?;
        let job = state.jobs[job_index].clone();
        let child = job.children[child_index].clone();
        let Some(markdown) = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .cloned()
        else {
            return finish_item_index_preflight_failure(
                store,
                job_id,
                &child.id,
                "Completed extraction has no Markdown artifact to index.",
            );
        };
        (job, child, markdown)
    };
    let index_input = match item_index_input_from_stage(&planned_child)? {
        Some(existing) => existing,
        None => match runner.index_input(
            &planned_job,
            &planned_child,
            &markdown,
            &store.job_output_dir(job_id),
        ) {
            Ok(input) => input,
            Err(error) => {
                return finish_item_index_preflight_failure(
                    store,
                    job_id,
                    &planned_child.id,
                    &error,
                );
            }
        },
    };

    let (running_job, running_child) = {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index =
            runnable_item_index_child(&state.jobs[job_index], Some(&planned_child.id)).ok_or_else(
                || "No eligible item-scoped index stage is ready to run or retry.".to_string(),
            )?;
        let stage = stage_mut(&mut state.jobs[job_index].children[child_index], "index")
            .ok_or_else(|| "Book Pipeline index stage disappeared before running.".to_string())?;
        stage.input_hashes = index_input.hashes();
        let execution_owner = store.execution_owner()?;
        start_stage(
            &mut state.jobs[job_index].children[child_index],
            "index",
            execution_owner,
        );
        state.jobs[job_index].current_step = "Running item-scoped index stage".into();
        state.jobs[job_index].last_error = None;
        state.jobs[job_index].updated_at = now_label();
        derive_job(&mut state.jobs[job_index]);
        let running_job = state.jobs[job_index].clone();
        let running_child = state.jobs[job_index].children[child_index].clone();
        store.save(&state)?;
        (running_job, running_child)
    };

    let result = runner
        .index(
            &running_job,
            &running_child,
            &markdown,
            &store.job_output_dir(job_id),
        )
        .and_then(|output| {
            if output.evidence.parent_item_key != index_input.parent_item_key
                || output.evidence.source_sha256 != index_input.source_sha256
                || output.evidence.chunk_contract_version != index_input.chunk_contract_version
                || output.evidence.embedding_profile_id != index_input.embedding_profile_id
            {
                Err(
                    "Zotero item index evidence does not match the persisted input identity."
                        .into(),
                )
            } else {
                Ok(output)
            }
        });
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], Some(&running_child.id))?;
    match result {
        Ok(output) => {
            let stage = stage_mut(&mut state.jobs[job_index].children[child_index], "index")
                .ok_or_else(|| {
                    "Book Pipeline index stage disappeared while running.".to_string()
                })?;
            stage.index_evidence = Some(output.evidence.clone());
            set_stage_status(
                &mut state.jobs[job_index].children[child_index],
                "index",
                STATUS_COMPLETED,
                None,
            );
            if stage_ref(&state.jobs[job_index].children[child_index], "handoff")
                .is_some_and(|stage| stage.status == STATUS_PENDING)
            {
                set_stage_status(
                    &mut state.jobs[job_index].children[child_index],
                    "handoff",
                    STATUS_READY,
                    None,
                );
            }
            state.jobs[job_index].children[child_index].last_error = None;
            state.jobs[job_index].current_step =
                if should_handoff_after_run(&state.jobs[job_index].mode) {
                    "Item-scoped index completed".into()
                } else {
                    "Completed".into()
                };
            state.jobs[job_index].last_error = None;
            state.jobs[job_index].log_summary.extend(output.log_summary);
        }
        Err(error) => {
            let redacted = redact_runner_message(&error);
            set_stage_status(
                &mut state.jobs[job_index].children[child_index],
                "index",
                STATUS_FAILED,
                Some(redacted.clone()),
            );
            state.jobs[job_index].children[child_index].last_error = Some(redacted.clone());
            state.jobs[job_index].current_step = "Item-scoped index failed".into();
            state.jobs[job_index].last_error = Some(redacted);
        }
    }
    state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let finished = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(finished)
}

fn finish_item_index_preflight_failure(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: &str,
    error: &str,
) -> Result<BookPipelineJob, String> {
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = runnable_item_index_child(&state.jobs[job_index], Some(child_id))
        .ok_or_else(|| "No eligible item-scoped index stage is ready to fail.".to_string())?;
    let redacted = redact_runner_message(error);
    let now = now_label();
    let stage = stage_mut(&mut state.jobs[job_index].children[child_index], "index")
        .ok_or_else(|| "Book Pipeline index stage disappeared during preflight.".to_string())?;
    stage.attempt = stage.attempt.saturating_add(1);
    stage.started_at = Some(now);
    set_stage_status(
        &mut state.jobs[job_index].children[child_index],
        "index",
        STATUS_FAILED,
        Some(redacted.clone()),
    );
    state.jobs[job_index].children[child_index].last_error = Some(redacted.clone());
    state.jobs[job_index].current_step = "Item-scoped index failed".into();
    state.jobs[job_index].last_error = Some(redacted);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let finished = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(finished)
}

fn run_item_index_stages(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    job_id: &str,
) -> Result<BookPipelineJob, String> {
    let child_ids = {
        let state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        state.jobs[job_index]
            .children
            .iter()
            .filter(|child| {
                stage_ref(child, "extract").is_some_and(|stage| stage.status == STATUS_COMPLETED)
                    && stage_ref(child, "index").is_some_and(|stage| {
                        matches!(stage.status.as_str(), STATUS_READY | STATUS_FAILED)
                    })
            })
            .map(|child| child.id.clone())
            .collect::<Vec<_>>()
    };
    let mut indexed = None;
    for child_id in child_ids {
        indexed = Some(run_item_index_stage(
            store,
            runner,
            job_id,
            Some(&child_id),
        )?);
    }
    let mut indexed = indexed.ok_or_else(|| {
        "No eligible item-scoped index stage is ready to run or retry.".to_string()
    })?;
    let has_index_failure = indexed
        .children
        .iter()
        .any(|child| stage_ref(child, "index").is_some_and(|stage| stage.status == STATUS_FAILED));
    if indexed.kind == "collection"
        && !should_handoff_after_run(&indexed.mode)
        && !has_index_failure
    {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        state.jobs[job_index].current_step = collection_summary_step(&state.jobs[job_index]);
        state.jobs[job_index].updated_at = now_label();
        indexed = state.jobs[job_index].clone();
        store.save(&state)?;
    }
    Ok(indexed)
}

fn collection_summary_step(job: &BookPipelineJob) -> String {
    let count = |status: &str| {
        job.collection_items
            .iter()
            .filter(|item| item.status == status)
            .count()
    };
    format!(
        "Collection summary: completed={} failed={} blocked={} skipped={}",
        count(STATUS_COMPLETED),
        count(STATUS_FAILED),
        count(STATUS_BLOCKED),
        count(STATUS_SKIPPED)
    )
}

fn continue_after_item_index(
    store: &dyn BookPipelineStateStore,
    handoff_runner: &dyn TranslationHandoffRunner,
    job_id: &str,
    repo_root: Option<&Path>,
    indexed: BookPipelineJob,
) -> Result<BookPipelineJob, String> {
    let index_failed = indexed
        .children
        .iter()
        .any(|child| stage_ref(child, "index").is_some_and(|stage| stage.status == STATUS_FAILED));
    if index_failed || !should_handoff_after_run(&indexed.mode) {
        return Ok(indexed);
    }
    match repo_root {
        Some(repo_root) => {
            handoff_job_markdown_with_runner(store, job_id, None, repo_root, handoff_runner)
        }
        None => {
            let repo_root = local_reading_repo_root()?;
            handoff_job_markdown_with_runner(store, job_id, None, &repo_root, handoff_runner)
        }
    }
}

fn is_durable_collection_job(job: &BookPipelineJob) -> bool {
    job.kind == "collection"
        && job
            .membership
            .as_ref()
            .is_some_and(|membership| !membership.snapshot_sha256.is_empty())
}

fn durable_collection_stage_to_run(job: &BookPipelineJob) -> Option<(String, &'static str)> {
    for child in &job.children {
        for stage_id in ["route", "extract", "index", "handoff"] {
            if stage_ref(child, stage_id).is_some_and(|stage| stage.status == STATUS_READY) {
                return Some((child.id.clone(), stage_id));
            }
        }
    }
    None
}

fn durable_collection_stage_to_retry(job: &BookPipelineJob) -> Option<(String, &'static str)> {
    for child in &job.children {
        for stage_id in ["route", "extract", "index", "handoff"] {
            if stage_ref(child, stage_id).is_some_and(|stage| stage.status == STATUS_FAILED) {
                return Some((child.id.clone(), stage_id));
            }
        }
    }
    durable_collection_stage_to_run(job)
}

fn frozen_attachment_input_hashes(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
) -> Result<BTreeMap<String, String>, String> {
    let membership = job
        .membership
        .as_ref()
        .ok_or_else(|| "Durable collection job is missing frozen membership.".to_string())?;
    let identity = child.source_identity.as_ref().ok_or_else(|| {
        "Durable collection child is missing its frozen attachment identity.".to_string()
    })?;
    Ok(BTreeMap::from([
        ("collectionKey".into(), identity.collection_key.clone()),
        ("parentItemKey".into(), identity.parent_item_key.clone()),
        (
            "pdfAttachmentKey".into(),
            identity.pdf_attachment_key.clone(),
        ),
        (
            "attachmentVersion".into(),
            identity.attachment_version.to_string(),
        ),
        ("contentType".into(), identity.content_type.clone()),
        (
            "fileSize".into(),
            identity.file_size.unwrap_or_default().to_string(),
        ),
        (
            "fileMtimeNs".into(),
            identity.file_mtime_ns.unwrap_or_default().to_string(),
        ),
        ("membershipRevision".into(), membership.revision.to_string()),
        ("snapshotSha256".into(), membership.snapshot_sha256.clone()),
    ]))
}

fn file_mtime_ns(metadata: &fs::Metadata) -> Result<u64, String> {
    let nanos = metadata
        .modified()
        .map_err(|_| "Could not read frozen source modification time.".to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Frozen source modification time is invalid.".to_string())?
        .as_nanos();
    u64::try_from(nanos).map_err(|_| "Frozen source modification time is out of range.".into())
}

fn validate_frozen_attachment_source(
    child: &BookPipelineChildJob,
    expected_sha256: Option<&str>,
) -> Result<String, String> {
    let identity = child.source_identity.as_ref().ok_or_else(|| {
        "Durable collection child is missing its frozen attachment identity.".to_string()
    })?;
    let path = Path::new(&identity.attachment_path);
    let metadata = fs::metadata(path)
        .map_err(|_| "frozen source attachment is no longer available.".to_string())?;
    if !metadata.is_file()
        || identity.file_size != Some(metadata.len())
        || identity.file_mtime_ns != Some(file_mtime_ns(&metadata)?)
    {
        return Err("frozen source attachment changed after collection discovery.".into());
    }
    let observed_sha256 = sha256_file(path)?;
    if expected_sha256.is_some_and(|expected| expected != observed_sha256) {
        return Err("frozen source attachment changed after route selection.".into());
    }
    Ok(observed_sha256)
}

fn block_durable_collection_stage(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: &str,
    stage_id: &str,
    error: &str,
) -> Result<BookPipelineJob, String> {
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], Some(child_id))?;
    let redacted = redact_runner_message(error);
    set_stage_status(
        &mut state.jobs[job_index].children[child_index],
        stage_id,
        STATUS_BLOCKED,
        Some(redacted.clone()),
    );
    state.jobs[job_index].children[child_index].last_error = Some(redacted.clone());
    state.jobs[job_index].current_step = format!("Attachment {stage_id} blocked");
    state.jobs[job_index].last_error = Some(redacted);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let blocked = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(blocked)
}

fn run_durable_collection_route_stage(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    job_id: &str,
    child_id: &str,
) -> Result<BookPipelineJob, String> {
    let source_sha256 = {
        let state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], Some(child_id))?;
        match validate_frozen_attachment_source(&state.jobs[job_index].children[child_index], None)
        {
            Ok(source_sha256) => source_sha256,
            Err(error) => {
                drop(state);
                return block_durable_collection_stage(store, job_id, child_id, "route", &error);
            }
        }
    };
    let (running_job, running_child) = {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], Some(child_id))?;
        if stage_ref(&state.jobs[job_index].children[child_index], "route")
            .is_none_or(|stage| !matches!(stage.status.as_str(), STATUS_READY | STATUS_FAILED))
        {
            return Err("No durable attachment route stage is ready to run.".into());
        }
        let mut input_hashes = frozen_attachment_input_hashes(
            &state.jobs[job_index],
            &state.jobs[job_index].children[child_index],
        )?;
        input_hashes.insert("sourceSha256".into(), source_sha256.clone());
        stage_mut(&mut state.jobs[job_index].children[child_index], "route")
            .ok_or_else(|| "Durable attachment route stage disappeared.".to_string())?
            .input_hashes = input_hashes;
        let execution_owner = store.execution_owner()?;
        start_stage(
            &mut state.jobs[job_index].children[child_index],
            "route",
            execution_owner,
        );
        state.jobs[job_index].children[child_index].attempts = state.jobs[job_index].children
            [child_index]
            .attempts
            .saturating_add(1);
        state.jobs[job_index].current_step = "Running attachment route stage".into();
        state.jobs[job_index].last_error = None;
        state.jobs[job_index].updated_at = now_label();
        derive_job(&mut state.jobs[job_index]);
        let running_job = state.jobs[job_index].clone();
        let running_child = state.jobs[job_index].children[child_index].clone();
        store.save(&state)?;
        (running_job, running_child)
    };

    let result = runner
        .route_attachment(&running_job, &running_child, &store.job_output_dir(job_id))
        .and_then(|output| {
            validate_frozen_attachment_source(&running_child, Some(&source_sha256))?;
            Ok(output)
        });
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], Some(child_id))?;
    match result {
        Ok(output) => {
            let AttachmentRouteOutput {
                mut route,
                log_summary,
                reused_artifact,
            } = output;
            // The durable collection path re-derives the route from live worker
            // evidence, which would otherwise discard the conversion override the
            // user picked in the wizard. Re-apply it here so the stored route and
            // the runner command agree with what the wizard displayed.
            apply_route_overrides(
                std::slice::from_mut(&mut route),
                &running_child.source.route_overrides,
                None,
            );
            let expected_attachment_key = state.jobs[job_index].children[child_index]
                .source_identity
                .as_ref()
                .map(|identity| identity.pdf_attachment_key.as_str())
                .unwrap_or_default();
            if route.id != expected_attachment_key {
                let error = "Attachment route evidence does not match the frozen child.";
                set_stage_status(
                    &mut state.jobs[job_index].children[child_index],
                    "route",
                    STATUS_FAILED,
                    Some(error.into()),
                );
                state.jobs[job_index].children[child_index].last_error = Some(error.into());
                state.jobs[job_index].current_step = "Attachment route failed".into();
                state.jobs[job_index].last_error = Some(error.into());
            } else {
                let route_decision_sha256 = sha256_str(&format!(
                    "{}\0{}\0{}\0{}",
                    route.id, route.route_kind, route.can_run, source_sha256
                ));
                if let Some(stage) =
                    stage_mut(&mut state.jobs[job_index].children[child_index], "route")
                {
                    stage
                        .input_hashes
                        .insert("routeKind".into(), route.route_kind.clone());
                    stage
                        .input_hashes
                        .insert("routeDecisionSha256".into(), route_decision_sha256);
                }
                state.jobs[job_index].children[child_index].route = vec![route.clone()];
                if let Some(parent_route) = state.jobs[job_index]
                    .route
                    .iter_mut()
                    .find(|candidate| candidate.id == route.id)
                {
                    *parent_route = route.clone();
                }
                state.jobs[job_index].log_summary.extend(log_summary);

                if route.route_kind == "already_converted" {
                    if let Some(artifact) = reused_artifact {
                        merge_artifacts(
                            &mut state.jobs[job_index].children[child_index].artifacts,
                            std::slice::from_ref(&artifact),
                        );
                        merge_artifacts(
                            &mut state.jobs[job_index].artifacts,
                            std::slice::from_ref(&artifact),
                        );
                        set_stage_status(
                            &mut state.jobs[job_index].children[child_index],
                            "route",
                            STATUS_COMPLETED,
                            None,
                        );
                        set_stage_status(
                            &mut state.jobs[job_index].children[child_index],
                            "extract",
                            STATUS_READY,
                            None,
                        );
                        state.jobs[job_index].children[child_index].last_error = None;
                        state.jobs[job_index].current_step =
                            "Verified attachment extraction ready for durable reuse".into();
                        state.jobs[job_index].last_error = None;
                    } else {
                        let error = "Already-completed route has no verified Markdown artifact.";
                        set_stage_status(
                            &mut state.jobs[job_index].children[child_index],
                            "route",
                            STATUS_FAILED,
                            Some(error.into()),
                        );
                        state.jobs[job_index].children[child_index].last_error = Some(error.into());
                        state.jobs[job_index].current_step = "Attachment route failed".into();
                        state.jobs[job_index].last_error = Some(error.into());
                    }
                } else if route.can_run {
                    set_stage_status(
                        &mut state.jobs[job_index].children[child_index],
                        "route",
                        STATUS_COMPLETED,
                        None,
                    );
                    set_stage_status(
                        &mut state.jobs[job_index].children[child_index],
                        "extract",
                        STATUS_READY,
                        None,
                    );
                    state.jobs[job_index].children[child_index].last_error = None;
                    state.jobs[job_index].current_step = "Attachment route completed".into();
                    state.jobs[job_index].last_error = None;
                } else {
                    let error = route
                        .blocked_reason
                        .unwrap_or_else(|| "Attachment route is not runnable.".into());
                    let redacted = redact_runner_message(&error);
                    set_stage_status(
                        &mut state.jobs[job_index].children[child_index],
                        "route",
                        STATUS_BLOCKED,
                        Some(redacted.clone()),
                    );
                    state.jobs[job_index].children[child_index].last_error = Some(redacted.clone());
                    state.jobs[job_index].current_step = "Attachment route blocked".into();
                    state.jobs[job_index].last_error = Some(redacted);
                }
            }
        }
        Err(error) => {
            let redacted = redact_runner_message(&error);
            set_stage_status(
                &mut state.jobs[job_index].children[child_index],
                "route",
                STATUS_FAILED,
                Some(redacted.clone()),
            );
            state.jobs[job_index].children[child_index].last_error = Some(redacted.clone());
            state.jobs[job_index].current_step = "Attachment route failed".into();
            state.jobs[job_index].last_error = Some(redacted);
        }
    }
    state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let finished = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(finished)
}

fn validate_durable_collection_extract_output(
    child: &BookPipelineChildJob,
    output: &RunnerOutput,
) -> Result<(), String> {
    if !output.collection_items.is_empty() {
        return Err("Per-attachment extraction returned forbidden collection batch state.".into());
    }
    let identity = child.source_identity.as_ref().ok_or_else(|| {
        "Durable collection child is missing its frozen attachment identity.".to_string()
    })?;
    let markdown = output
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .ok_or_else(|| "Per-attachment extraction produced no Markdown artifact.".to_string())?;
    if markdown.zotero_key.as_deref().is_none_or(str::is_empty) {
        return Err("Per-attachment extraction did not record a Markdown attachment key.".into());
    }
    let markdown_path = Path::new(&markdown.path);
    let metadata = fs::metadata(markdown_path)
        .map_err(|_| "Extracted Markdown artifact is not available.".to_string())?;
    let expected_sha256 = markdown
        .sha256
        .as_deref()
        .ok_or_else(|| "Extracted Markdown artifact has no SHA-256.".to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || sha256_file(markdown_path)? != expected_sha256
    {
        return Err("Extracted Markdown artifact failed validation.".into());
    }
    if markdown_frontmatter_value(Path::new(&markdown.path), "parent_item_key")
        .is_some_and(|parent_item_key| parent_item_key != identity.parent_item_key)
    {
        return Err("Extracted Markdown does not match the frozen parent item identity.".into());
    }
    Ok(())
}

fn run_durable_collection_extract_stage(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    job_id: &str,
    child_id: &str,
) -> Result<BookPipelineJob, String> {
    let source_sha256 = {
        let state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], Some(child_id))?;
        let expected_sha256 = stage_ref(&state.jobs[job_index].children[child_index], "route")
            .and_then(|stage| stage.input_hashes.get("sourceSha256"))
            .ok_or_else(|| "Completed attachment route has no source fingerprint.".to_string())?;
        match validate_frozen_attachment_source(
            &state.jobs[job_index].children[child_index],
            Some(expected_sha256),
        ) {
            Ok(source_sha256) => source_sha256,
            Err(error) => {
                drop(state);
                return block_durable_collection_stage(store, job_id, child_id, "extract", &error);
            }
        }
    };
    let (running_job, running_child, output_dir) = {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], Some(child_id))?;
        if stage_ref(&state.jobs[job_index].children[child_index], "extract")
            .is_none_or(|stage| !matches!(stage.status.as_str(), STATUS_READY | STATUS_FAILED))
        {
            return Err("No durable attachment extraction stage is ready to run.".into());
        }
        let mut input_hashes = frozen_attachment_input_hashes(
            &state.jobs[job_index],
            &state.jobs[job_index].children[child_index],
        )?;
        input_hashes.insert("sourceSha256".into(), source_sha256.clone());
        let route_kind = state.jobs[job_index].children[child_index]
            .route
            .iter()
            .find(|route| route.can_run || route.route_kind == "already_converted")
            .map(|route| route.route_kind.clone())
            .ok_or_else(|| "Durable attachment has no completed runnable route.".to_string())?;
        input_hashes.insert("routeKind".into(), route_kind);
        stage_mut(&mut state.jobs[job_index].children[child_index], "extract")
            .ok_or_else(|| "Durable attachment extract stage disappeared.".to_string())?
            .input_hashes = input_hashes;
        let execution_owner = store.execution_owner()?;
        start_stage(
            &mut state.jobs[job_index].children[child_index],
            "extract",
            execution_owner,
        );
        state.jobs[job_index].children[child_index].attempts = state.jobs[job_index].children
            [child_index]
            .attempts
            .saturating_add(1);
        state.jobs[job_index].current_step = "Running attachment extract stage".into();
        state.jobs[job_index].last_error = None;
        state.jobs[job_index].updated_at = now_label();
        derive_job(&mut state.jobs[job_index]);
        let running_job = state.jobs[job_index].clone();
        let running_child = state.jobs[job_index].children[child_index].clone();
        let output_component = state.jobs[job_index].children[child_index]
            .source_identity
            .as_ref()
            .map(|identity| clean_path_component(&identity.pdf_attachment_key))
            .filter(|component| !component.is_empty())
            .ok_or_else(|| "Durable attachment has no safe output identity.".to_string())?;
        let output_dir = store.job_output_dir(job_id).join(output_component);
        store.save(&state)?;
        (running_job, running_child, output_dir)
    };

    let result = if running_child
        .route
        .iter()
        .any(|route| route.route_kind == "already_converted")
    {
        let markdown = running_child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .cloned()
            .ok_or_else(|| {
                "Already-completed route lost its verified Markdown artifact.".to_string()
            });
        markdown.and_then(|markdown| {
            let output = RunnerOutput {
                log_summary: vec!["Verified attachment extraction reused".into()],
                artifacts: vec![markdown],
                collection_items: Vec::new(),
                output_dir: None,
                current_step: None,
            };
            validate_frozen_attachment_source(&running_child, Some(&source_sha256))?;
            validate_durable_collection_extract_output(&running_child, &output)?;
            Ok(output)
        })
    } else {
        runner
            .extract_attachment(&running_job, &running_child, &output_dir)
            .and_then(|output| {
                validate_frozen_attachment_source(&running_child, Some(&source_sha256))?;
                validate_durable_collection_extract_output(&running_child, &output)?;
                Ok(output)
            })
    };
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], Some(child_id))?;
    match result {
        Ok(output) => {
            merge_artifacts(
                &mut state.jobs[job_index].children[child_index].artifacts,
                &output.artifacts,
            );
            merge_artifacts(&mut state.jobs[job_index].artifacts, &output.artifacts);
            set_stage_status(
                &mut state.jobs[job_index].children[child_index],
                "extract",
                STATUS_COMPLETED,
                None,
            );
            set_stage_status(
                &mut state.jobs[job_index].children[child_index],
                "index",
                STATUS_READY,
                None,
            );
            state.jobs[job_index].children[child_index].last_error = None;
            state.jobs[job_index].current_step =
                "Attachment extraction completed; item-scoped index ready".into();
            state.jobs[job_index].last_error = None;
            state.jobs[job_index].log_summary.extend(output.log_summary);
        }
        Err(error) => {
            let redacted = redact_runner_message(&error);
            set_stage_status(
                &mut state.jobs[job_index].children[child_index],
                "extract",
                STATUS_FAILED,
                Some(redacted.clone()),
            );
            state.jobs[job_index].children[child_index].last_error = Some(redacted.clone());
            state.jobs[job_index].current_step = "Attachment extraction failed".into();
            state.jobs[job_index].last_error = Some(redacted);
        }
    }
    state.jobs[job_index].collection_items.clear();
    state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let finished = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(finished)
}

fn indexed_markdown_for_handoff(
    child: &BookPipelineChildJob,
) -> Result<BookPipelineArtifact, String> {
    let index_stage = stage_ref(child, "index")
        .filter(|stage| stage.status == STATUS_COMPLETED)
        .ok_or_else(|| "Attachment has no completed item-scoped index.".to_string())?;
    let input = item_index_input_from_stage(child)?.ok_or_else(|| {
        "Completed item-scoped index has no persisted input identity.".to_string()
    })?;
    let evidence = index_stage
        .index_evidence
        .as_ref()
        .ok_or_else(|| "Completed item-scoped index has no evidence.".to_string())?;
    let markdown = child
        .artifacts
        .iter()
        .find(|artifact| {
            artifact.artifact_id == input.markdown_artifact_id && artifact.kind == "markdown"
        })
        .ok_or_else(|| "indexed Markdown artifact is no longer registered.".to_string())?;
    let (parent_item_key, source_sha256) = validated_item_index_artifact(child, markdown)
        .map_err(|_| "indexed Markdown artifact changed after indexing.".to_string())?;
    if parent_item_key != input.parent_item_key
        || source_sha256 != input.source_sha256
        || evidence.parent_item_key != input.parent_item_key
        || evidence.source_sha256 != input.source_sha256
    {
        return Err("indexed Markdown artifact does not match completed index evidence.".into());
    }
    Ok(markdown.clone())
}

fn run_durable_collection_handoff_stage(
    store: &dyn BookPipelineStateStore,
    handoff_runner: &dyn TranslationHandoffRunner,
    job_id: &str,
    child_id: &str,
    repo_root: Option<&Path>,
) -> Result<BookPipelineJob, String> {
    {
        let state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], Some(child_id))?;
        if let Err(error) =
            indexed_markdown_for_handoff(&state.jobs[job_index].children[child_index])
        {
            drop(state);
            return block_durable_collection_stage(store, job_id, child_id, "handoff", &error);
        }
    }
    let (running_job, running_child, markdown_path) = {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], Some(child_id))?;
        let child = &state.jobs[job_index].children[child_index];
        if stage_ref(child, "handoff")
            .is_none_or(|stage| !matches!(stage.status.as_str(), STATUS_READY | STATUS_FAILED))
            || stage_ref(child, "index").is_none_or(|stage| stage.status != STATUS_COMPLETED)
        {
            return Err("No indexed durable attachment handoff is ready to run.".into());
        }
        let markdown_path = indexed_markdown_for_handoff(child)?.path;
        let input_hashes = stage_ref(child, "index")
            .map(|stage| stage.input_hashes.clone())
            .unwrap_or_default();
        stage_mut(&mut state.jobs[job_index].children[child_index], "handoff")
            .ok_or_else(|| "Durable attachment handoff stage disappeared.".to_string())?
            .input_hashes = input_hashes;
        let execution_owner = store.execution_owner()?;
        start_stage(
            &mut state.jobs[job_index].children[child_index],
            "handoff",
            execution_owner,
        );
        state.jobs[job_index].children[child_index].attempts = state.jobs[job_index].children
            [child_index]
            .attempts
            .saturating_add(1);
        state.jobs[job_index].current_step = "Running attachment handoff stage".into();
        state.jobs[job_index].last_error = None;
        state.jobs[job_index].updated_at = now_label();
        derive_job(&mut state.jobs[job_index]);
        let running_job = state.jobs[job_index].clone();
        let running_child = state.jobs[job_index].children[child_index].clone();
        store.save(&state)?;
        (running_job, running_child, markdown_path)
    };

    let result = indexed_markdown_for_handoff(&running_child).and_then(|_| match repo_root {
        Some(repo_root) => handoff_runner.handoff_attachment(
            &running_job,
            &running_child,
            &markdown_path,
            repo_root,
        ),
        None => local_reading_repo_root().and_then(|repo_root| {
            handoff_runner.handoff_attachment(
                &running_job,
                &running_child,
                &markdown_path,
                &repo_root,
            )
        }),
    });
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    match result {
        Ok(output) => {
            merge_artifacts(&mut state.jobs[job_index].artifacts, &output.artifacts);
            state.jobs[job_index]
                .log_summary
                .extend(output.log_summary.clone());
            mark_handoff_finished(&mut state.jobs[job_index], Some(child_id), Ok(&output));
            state.jobs[job_index].current_step = "Attachment handoff completed".into();
            state.jobs[job_index].last_error = None;
        }
        Err(error) => {
            let redacted = redact_runner_message(&error);
            mark_handoff_finished(
                &mut state.jobs[job_index],
                Some(child_id),
                Err(redacted.as_str()),
            );
            state.jobs[job_index].current_step = "Attachment handoff failed".into();
            state.jobs[job_index].last_error = Some(redacted);
        }
    }
    state.jobs[job_index].collection_items.clear();
    state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    let finished = state.jobs[job_index].clone();
    store.save(&state)?;
    Ok(finished)
}

fn run_durable_collection_child_stage(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    handoff_runner: &dyn TranslationHandoffRunner,
    job_id: &str,
    repo_root: Option<&Path>,
    retry_failed: bool,
) -> Result<BookPipelineJob, String> {
    let state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let next_stage = if retry_failed {
        durable_collection_stage_to_retry(&state.jobs[job_index])
    } else {
        durable_collection_stage_to_run(&state.jobs[job_index])
    };
    let Some((child_id, stage_id)) = next_stage else {
        return Ok(state.jobs[job_index].clone());
    };
    drop(state);
    match stage_id {
        "route" => run_durable_collection_route_stage(store, runner, job_id, &child_id),
        "extract" => run_durable_collection_extract_stage(store, runner, job_id, &child_id),
        "index" => run_item_index_stage(store, runner, job_id, Some(&child_id)),
        "handoff" => run_durable_collection_handoff_stage(
            store,
            handoff_runner,
            job_id,
            &child_id,
            repo_root,
        ),
        _ => unreachable!("durable collection scheduler returned an unsupported stage"),
    }
}

fn run_job_with_handoff(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    handoff_runner: &dyn TranslationHandoffRunner,
    job_id: &str,
    repo_root: Option<&Path>,
) -> Result<BookPipelineJob, String> {
    run_job_with_handoff_mode(store, runner, handoff_runner, job_id, repo_root, false)
}

fn retry_job_with_handoff(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    handoff_runner: &dyn TranslationHandoffRunner,
    job_id: &str,
    repo_root: Option<&Path>,
) -> Result<BookPipelineJob, String> {
    run_job_with_handoff_mode(store, runner, handoff_runner, job_id, repo_root, true)
}

fn run_job_to_quiescence_with_handoff(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    handoff_runner: &dyn TranslationHandoffRunner,
    job_id: &str,
    repo_root: Option<&Path>,
) -> Result<BookPipelineJob, String> {
    loop {
        let state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        if !is_durable_collection_job(&state.jobs[job_index]) {
            drop(state);
            return run_job_with_handoff(store, runner, handoff_runner, job_id, repo_root);
        }
        if durable_collection_stage_to_run(&state.jobs[job_index]).is_none() {
            return Ok(state.jobs[job_index].clone());
        }
        drop(state);
        run_job_with_handoff(store, runner, handoff_runner, job_id, repo_root)?;
    }
}

fn retry_job_to_quiescence_with_handoff(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    handoff_runner: &dyn TranslationHandoffRunner,
    job_id: &str,
    repo_root: Option<&Path>,
) -> Result<BookPipelineJob, String> {
    let state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    if !is_durable_collection_job(&state.jobs[job_index]) {
        drop(state);
        return retry_job_with_handoff(store, runner, handoff_runner, job_id, repo_root);
    }
    drop(state);
    retry_job_with_handoff(store, runner, handoff_runner, job_id, repo_root)?;
    run_job_to_quiescence_with_handoff(store, runner, handoff_runner, job_id, repo_root)
}

fn run_job_with_handoff_mode(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    handoff_runner: &dyn TranslationHandoffRunner,
    job_id: &str,
    repo_root: Option<&Path>,
    retry_failed_durable_stage: bool,
) -> Result<BookPipelineJob, String> {
    let mut state = store.load()?;
    let index = state
        .jobs
        .iter()
        .position(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found.".to_string())?;

    if is_durable_collection_job(&state.jobs[index]) {
        drop(state);
        return run_durable_collection_child_stage(
            store,
            runner,
            handoff_runner,
            job_id,
            repo_root,
            retry_failed_durable_stage,
        );
    }

    if collection_awaits_attachment_routing(&state.jobs[index]) {
        return Ok(state.jobs[index].clone());
    }

    if !route_is_runnable_for_source(&state.jobs[index].source, &state.jobs[index].route) {
        state.jobs[index].current_step = "Blocked by route preview".into();
        state.jobs[index].last_error = Some("No safe runnable route item is available.".into());
        mark_route_blocked(
            &mut state.jobs[index],
            "No safe runnable route item is available.",
        );
        state.jobs[index].updated_at = now_label();
        let blocked = state.jobs[index].clone();
        store.save(&state)?;
        return Ok(blocked);
    }

    if runnable_item_index_child(&state.jobs[index], None).is_some() {
        drop(state);
        let indexed = run_item_index_stages(store, runner, job_id)?;
        return continue_after_item_index(store, handoff_runner, job_id, repo_root, indexed);
    }

    let execution_owner = store.execution_owner()?;
    let started = mark_extract_running(&mut state.jobs[index], execution_owner);
    if started == 0 {
        return Err("No eligible extraction stage is ready to run or retry.".into());
    }
    state.jobs[index].current_step = "Running selected conversion route".into();
    state.jobs[index].last_error = None;
    state.jobs[index].attempts += 1;
    let attempts = state.jobs[index].attempts;
    state.jobs[index]
        .log_summary
        .push(format!("Attempt {attempts} started"));
    state.jobs[index].updated_at = now_label();
    let running_job = state.jobs[index].clone();
    store.save(&state)?;

    let output_dir = store.job_output_dir(&running_job.id);
    let result = runner.run(&running_job, &output_dir);
    let mut state = store.load()?;
    let index = state
        .jobs
        .iter()
        .position(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found after runner completed.".to_string())?;
    let convert_then_translate = state.jobs[index].mode == MODE_CONVERT_THEN_TRANSLATE;
    let should_handoff = should_handoff_after_run(&state.jobs[index].mode);

    match result {
        Ok(output) => {
            state.jobs[index].last_error = None;
            state.jobs[index].artifacts = output.artifacts;
            state.jobs[index].collection_items = output.collection_items;
            state.jobs[index].output_dir = output.output_dir.map(|path| display_path(&path));
            state.jobs[index].log_summary.extend(output.log_summary);
            apply_runner_output_to_children(&mut state.jobs[index]);
            if runnable_item_index_child(&state.jobs[index], None).is_some() {
                state.jobs[index].current_step =
                    "Extraction completed; item-scoped index ready".into();
                state.jobs[index].log_summary = trim_log_summary(&state.jobs[index].log_summary);
                state.jobs[index].updated_at = now_label();
                store.save(&state)?;
                let indexed = run_item_index_stages(store, runner, job_id)?;
                return continue_after_item_index(
                    store,
                    handoff_runner,
                    job_id,
                    repo_root,
                    indexed,
                );
            }
            if should_handoff {
                state.jobs[index].current_step = "Preparing translation handoff".into();
                state.jobs[index]
                    .log_summary
                    .push(if convert_then_translate {
                        "Conversion completed; translation handoff started".into()
                    } else {
                        "Source preparation completed; translation handoff started".into()
                    });
                let execution_owner = store.execution_owner()?;
                // Everything the runner produced is still only in memory here.
                // Returning through `?` would drop the whole conversion along
                // with it — and leave the extract stage `running` on disk from
                // the save above — so the user had to re-OCR just to retry a
                // handoff. Persist the run, then record the handoff as failed.
                let Some(handoff_child_id) =
                    mark_handoff_running(&mut state.jobs[index], execution_owner)
                else {
                    let error = "No completed extraction is ready for translation handoff.";
                    state.jobs[index].current_step = "Translation handoff failed".into();
                    state.jobs[index].last_error = Some(error.into());
                    state.jobs[index]
                        .log_summary
                        .push(format!("Translation handoff failed: {error}"));
                    mark_handoff_unavailable(&mut state.jobs[index], error);
                    state.jobs[index].log_summary =
                        trim_log_summary(&state.jobs[index].log_summary);
                    state.jobs[index].updated_at = now_label();
                    let finished = state.jobs[index].clone();
                    store.save(&state)?;
                    return Ok(finished);
                };
                state.jobs[index].log_summary = trim_log_summary(&state.jobs[index].log_summary);
                state.jobs[index].updated_at = now_label();
                let handoff_job = state.jobs[index].clone();
                store.save(&state)?;

                let handoff_result = if let Some(repo_root) = repo_root {
                    handoff_runner.handoff(&handoff_job, None, repo_root)
                } else {
                    local_reading_repo_root().and_then(|repo_root| {
                        handoff_runner.handoff(&handoff_job, None, &repo_root)
                    })
                };
                let mut state = store.load()?;
                let index = state
                    .jobs
                    .iter()
                    .position(|job| job.id == job_id)
                    .ok_or_else(|| {
                        "Book Pipeline job not found after handoff completed.".to_string()
                    })?;
                match handoff_result {
                    Ok(handoff) => {
                        state.jobs[index].current_step = "Translation handoff ready".into();
                        state.jobs[index].last_error = None;
                        state.jobs[index]
                            .artifacts
                            .extend(handoff.artifacts.clone());
                        state.jobs[index]
                            .log_summary
                            .extend(handoff.log_summary.clone());
                        mark_handoff_finished(
                            &mut state.jobs[index],
                            Some(&handoff_child_id),
                            Ok(&handoff),
                        );
                    }
                    Err(error) => {
                        state.jobs[index].current_step = "Translation handoff failed".into();
                        state.jobs[index].last_error = Some(redact_runner_message(&error));
                        state.jobs[index]
                            .log_summary
                            .push(redact_runner_message(&format!(
                                "Translation handoff failed: {error}"
                            )));
                        mark_handoff_finished(
                            &mut state.jobs[index],
                            Some(&handoff_child_id),
                            Err(&error),
                        );
                    }
                }
                state.jobs[index].log_summary = trim_log_summary(&state.jobs[index].log_summary);
                state.jobs[index].updated_at = now_label();
                let finished = state.jobs[index].clone();
                store.save(&state)?;
                return Ok(finished);
            }
            state.jobs[index].current_step =
                output.current_step.unwrap_or_else(|| "Completed".into());
        }
        Err(error) => {
            state.jobs[index].current_step = if convert_then_translate {
                "Conversion failed"
            } else if should_handoff {
                "Source preparation failed"
            } else {
                "Failed"
            }
            .into();
            state.jobs[index].last_error = Some(redact_runner_message(&error));
            state.jobs[index]
                .log_summary
                .push(redact_runner_message(&format!("Runner failed: {error}")));
            mark_extract_failed(&mut state.jobs[index], &redact_runner_message(&error));
        }
    }
    state.jobs[index].log_summary = trim_log_summary(&state.jobs[index].log_summary);
    state.jobs[index].updated_at = now_label();
    let finished = state.jobs[index].clone();
    store.save(&state)?;
    Ok(finished)
}

fn collection_awaits_attachment_routing(job: &BookPipelineJob) -> bool {
    job.kind == "collection"
        && job
            .membership
            .as_ref()
            .is_some_and(|membership| !membership.snapshot_sha256.is_empty())
        && job.children.iter().any(|child| {
            stage_ref(child, "route").is_some_and(|stage| stage.status == STATUS_READY)
        })
        && job.children.iter().all(|child| {
            stage_ref(child, "extract").is_some_and(|stage| stage.status == STATUS_PENDING)
        })
}

fn should_handoff_after_run(mode: &str) -> bool {
    mode == MODE_CONVERT_THEN_TRANSLATE || mode == MODE_TRANSLATE_ONLY
}

impl PipelineRunner for SystemPipelineRunner {
    fn run(&self, job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerOutput, String> {
        CommandPipelineRunner::new(SystemCommandExecutor).run(job, output_dir)
    }

    fn route_attachment(
        &self,
        job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        output_dir: &Path,
    ) -> Result<AttachmentRouteOutput, String> {
        CommandPipelineRunner::new(SystemCommandExecutor).route_attachment(job, child, output_dir)
    }

    fn extract_attachment(
        &self,
        job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        output_dir: &Path,
    ) -> Result<RunnerOutput, String> {
        CommandPipelineRunner::new(SystemCommandExecutor).extract_attachment(job, child, output_dir)
    }

    fn index(
        &self,
        job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        markdown: &BookPipelineArtifact,
        output_dir: &Path,
    ) -> Result<ItemScopedIndexOutput, String> {
        CommandPipelineRunner::new(SystemCommandExecutor).index(job, child, markdown, output_dir)
    }

    fn index_input(
        &self,
        job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        markdown: &BookPipelineArtifact,
        output_dir: &Path,
    ) -> Result<ItemScopedIndexInput, String> {
        CommandPipelineRunner::new(SystemCommandExecutor)
            .index_input(job, child, markdown, output_dir)
    }
}

impl TranslationHandoffRunner for LocalProjectHandoffRunner {
    fn handoff(
        &self,
        job: &BookPipelineJob,
        artifact_path: Option<&str>,
        repo_root: &Path,
    ) -> Result<TranslationHandoffOutput, String> {
        create_translation_handoff_project(job, artifact_path, repo_root)
    }

    fn handoff_attachment(
        &self,
        job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        artifact_path: &str,
        repo_root: &Path,
    ) -> Result<TranslationHandoffOutput, String> {
        create_translation_handoff_project_with_title(
            job,
            Some(artifact_path),
            repo_root,
            child.source.title.as_deref(),
        )
    }
}

impl<E: RunnerCommandExecutor> PipelineRunner for CommandPipelineRunner<E> {
    fn run(&self, job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerOutput, String> {
        if job.source.kind == "markdown_source" {
            return run_markdown_source_job(job, output_dir);
        }
        if is_zotero_batch_source(&job.source) {
            let root = self
                .book_ocr_conversion_root
                .as_deref()
                .map(Path::to_path_buf)
                .unwrap_or_else(book_ocr_conversion_root);
            return run_zotero_batch_job(&self.executor, job, output_dir, &root);
        }

        let command = if let Some(root) = self.book_ocr_conversion_root.as_deref() {
            build_runner_command_with_root(job, output_dir, Some(root))?
        } else {
            build_runner_command(job, output_dir)?
        };
        execute_conversion_command(&self.executor, command)
    }

    fn route_attachment(
        &self,
        _job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        _output_dir: &Path,
    ) -> Result<AttachmentRouteOutput, String> {
        let root = self
            .book_ocr_conversion_root
            .as_deref()
            .map(Path::to_path_buf)
            .unwrap_or_else(book_ocr_conversion_root);
        route_zotero_attachment_from_worker(&self.executor, child, &root)
    }

    fn extract_attachment(
        &self,
        _job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        output_dir: &Path,
    ) -> Result<RunnerOutput, String> {
        let root = self
            .book_ocr_conversion_root
            .as_deref()
            .map(Path::to_path_buf)
            .unwrap_or_else(book_ocr_conversion_root);
        let command = build_zotero_child_conversion_command_for_root(child, output_dir, &root)?;
        execute_conversion_command(&self.executor, command)
    }

    fn index(
        &self,
        job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        markdown: &BookPipelineArtifact,
        output_dir: &Path,
    ) -> Result<ItemScopedIndexOutput, String> {
        let mut command = build_zotero_item_index_command(job, child, markdown, output_dir)?;
        inject_embedding_credential(&mut command);
        let command_result = self.executor.execute(&command)?;
        let evidence: BookPipelineIndexEvidence =
            serde_json::from_str(command_result.stdout.trim())
                .map_err(|_| "Zotero item index returned invalid JSON evidence.".to_string())?;
        let expected_sha256 = markdown
            .sha256
            .as_deref()
            .ok_or_else(|| "Zotero Markdown artifact has no SHA-256.".to_string())?;
        if evidence.parent_item_key
            != markdown_parent_item_key(markdown).ok_or_else(|| {
                "Zotero Markdown artifact has no parent_item_key metadata.".to_string()
            })?
            || evidence.source_sha256 != expected_sha256
            || evidence.index_contract_version != ITEM_INDEX_CONTRACT_VERSION
            || evidence.chunk_contract_version != CHUNK_CONTRACT_VERSION
            || evidence.embedding_profile_id
                != item_index_input_from_stage(child)?
                    .ok_or_else(|| {
                        "Zotero item index has no persisted input identity.".to_string()
                    })?
                    .embedding_profile_id
            || evidence.completed_at.trim().is_empty()
        {
            return Err("Zotero item index evidence does not match the requested artifact.".into());
        }
        let mut log_summary = vec![format!(
            "Item-scoped index completed: parent={} chunks={} reused={}",
            evidence.parent_item_key, evidence.chunk_count, evidence.reused
        )];
        log_summary.extend(redact_log_lines(&command_result.log_summary));
        Ok(ItemScopedIndexOutput {
            evidence,
            log_summary,
        })
    }

    fn index_input(
        &self,
        _job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        markdown: &BookPipelineArtifact,
        output_dir: &Path,
    ) -> Result<ItemScopedIndexInput, String> {
        let (parent_item_key, source_sha256) = validated_item_index_artifact(child, markdown)?;
        let mut command = build_zotero_item_index_profile_command(child, output_dir)?;
        inject_embedding_credential(&mut command);
        let command_result = self.executor.execute(&command)?;
        let profile: ItemScopedIndexProfile = serde_json::from_str(command_result.stdout.trim())
            .map_err(|_| "Zotero item index profile returned invalid JSON.".to_string())?;
        if profile.embedding_profile_id.trim().is_empty() {
            return Err("Zotero item index profile is empty.".into());
        }
        Ok(ItemScopedIndexInput {
            parent_item_key,
            source_sha256,
            chunk_contract_version: CHUNK_CONTRACT_VERSION.into(),
            embedding_profile_id: profile.embedding_profile_id,
            markdown_artifact_id: markdown.artifact_id.clone(),
        })
    }
}

fn execute_conversion_command<E: RunnerCommandExecutor>(
    executor: &E,
    mut command: RunnerCommand,
) -> Result<RunnerOutput, String> {
    inject_ocr_credentials(&mut command);
    fs::create_dir_all(&command.output_dir).map_err(|err| err.to_string())?;
    let command_result = executor.execute(&command)?;
    let zotero_key = extract_zotero_attachment_key(&command_result);
    if command.label == ZOTERO_CONVERSION_COMMAND_LABEL && zotero_key.is_none() {
        return Err(
            "Zotero extraction did not report the required Markdown attachment key.".into(),
        );
    }
    let mut artifacts = scan_artifacts(&command.output_dir)?;
    if let Some(key) = &zotero_key {
        for artifact in artifacts
            .iter_mut()
            .filter(|artifact| artifact.kind == "markdown")
        {
            artifact.zotero_key = Some(key.clone());
        }
    }
    let mut log_summary = vec![format!("Runner command prepared: {}", command.label)];
    log_summary.extend(redact_log_lines(&command_result.log_summary));
    log_summary.extend(parse_allowlisted_worker_markers(
        &command_result.stdout,
        &[command.output_dir.as_path()],
    ));
    log_summary.extend(parse_allowlisted_worker_markers(
        &command_result.stderr,
        &[command.output_dir.as_path()],
    ));
    if let Some(key) = zotero_key {
        log_summary.push(format!("Zotero Markdown attachment recorded: {key}"));
    }
    Ok(RunnerOutput {
        log_summary: trim_log_summary(&log_summary),
        artifacts,
        collection_items: Vec::new(),
        output_dir: Some(command.output_dir),
        current_step: None,
    })
}

fn build_zotero_item_index_command(
    _job: &BookPipelineJob,
    child: &BookPipelineChildJob,
    markdown: &BookPipelineArtifact,
    output_dir: &Path,
) -> Result<RunnerCommand, String> {
    let (parent_item_key, markdown_sha256) = validated_item_index_artifact(child, markdown)?;
    let index_input = item_index_input_from_stage(child)?
        .ok_or_else(|| "Zotero item index has no persisted input identity.".to_string())?;
    if index_input.markdown_artifact_id != markdown.artifact_id
        || index_input.parent_item_key != parent_item_key
        || index_input.source_sha256 != markdown_sha256
    {
        return Err(
            "Zotero item index artifact does not match its persisted input identity.".into(),
        );
    }
    let markdown_path = PathBuf::from(&markdown.path);
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: ITEM_INDEX_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args: vec![
            "run".into(),
            "--package".into(),
            "zotero-cli-agent".into(),
            "zfulltext".into(),
            "index".into(),
            "--parent-item-key".into(),
            parent_item_key,
            "--markdown".into(),
            display_path(&markdown_path),
            "--sha256".into(),
            markdown_sha256,
            "--chunk-contract-version".into(),
            CHUNK_CONTRACT_VERSION.into(),
            "--embedding-profile-id".into(),
            index_input.embedding_profile_id,
        ],
        env: Vec::new(),
        cwd: Some(local_reading_repo_root()?),
        output_dir: output_dir.to_path_buf(),
        attempts: child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "index")
            .map(|stage| stage.attempt)
            .unwrap_or_default(),
        accepted_exit_codes: vec![0],
    })
}

fn build_zotero_item_index_profile_command(
    child: &BookPipelineChildJob,
    output_dir: &Path,
) -> Result<RunnerCommand, String> {
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: ITEM_INDEX_PROFILE_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args: vec![
            "run".into(),
            "--package".into(),
            "zotero-cli-agent".into(),
            "zfulltext".into(),
            "profile".into(),
        ],
        env: Vec::new(),
        cwd: Some(local_reading_repo_root()?),
        output_dir: output_dir.to_path_buf(),
        attempts: child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "index")
            .map(|stage| stage.attempt.saturating_add(1))
            .unwrap_or(1),
        accepted_exit_codes: vec![0],
    })
}

/// Inject the Keychain-stored key for the Zotero full-text search embedding
/// backend. A no-op when no key is stored, so indexing still falls back to a
/// key in the repository-root .env.
fn inject_embedding_credential(command: &mut RunnerCommand) {
    if let Some((key_env, value)) = crate::embedding_settings::resolve_credential_env() {
        command.env.push((key_env, value));
    }
}

/// Inject the Keychain-stored PaddleOCR / MinerU tokens into an OCR worker
/// command. A no-op when nothing is stored; the worker's own repo-root .env
/// lookup remains the fallback (it never overrides an injected variable).
/// Routing/discovery commands need this too — the worker decides between
/// paddle-ocr and missing-paddleocr-token routes by looking at these vars.
fn inject_ocr_credentials(command: &mut RunnerCommand) {
    for (key_env, value) in crate::ocr_settings::resolve_credential_env() {
        command.env.push((key_env, value));
    }
}

fn validated_item_index_artifact(
    child: &BookPipelineChildJob,
    markdown: &BookPipelineArtifact,
) -> Result<(String, String), String> {
    if child.source.kind != "zotero_attachment" {
        return Err("Item-scoped indexing is only available for Zotero attachments.".into());
    }
    let markdown_path = PathBuf::from(&markdown.path);
    let markdown_sha256 = markdown
        .sha256
        .as_deref()
        .ok_or_else(|| "Zotero Markdown artifact has no SHA-256.".to_string())?;
    if !markdown.validation.exists
        || !markdown.validation.nonempty
        || !markdown.validation.hash_matches
        || sha256_file(&markdown_path)? != markdown_sha256
    {
        return Err("Zotero Markdown artifact failed validation before indexing.".into());
    }
    let parent_item_key = markdown_parent_item_key(markdown)
        .ok_or_else(|| "Zotero Markdown artifact has no parent_item_key metadata.".to_string())?;
    Ok((parent_item_key, markdown_sha256.into()))
}

fn markdown_parent_item_key(markdown: &BookPipelineArtifact) -> Option<String> {
    markdown
        .source_refs
        .parent_item_key
        .clone()
        .or_else(|| markdown_frontmatter_value(Path::new(&markdown.path), "parent_item_key"))
}

fn markdown_frontmatter_value(path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        let Some((candidate, value)) = line.split_once(':') else {
            continue;
        };
        if candidate.trim() == key {
            let value = value.trim().trim_matches(['\"', '\'']);
            if !value.is_empty() && value != "null" {
                return Some(value.into());
            }
        }
    }
    None
}

fn run_zotero_batch_job<E: RunnerCommandExecutor>(
    executor: &E,
    job: &BookPipelineJob,
    output_dir: &Path,
    root: &Path,
) -> Result<RunnerOutput, String> {
    fs::create_dir_all(output_dir).map_err(|err| err.to_string())?;
    let retry_failed_ids: Vec<String> = if job.attempts > 0 {
        job.collection_items
            .iter()
            .filter(|item| item.status == STATUS_FAILED)
            .map(|item| item.id.clone())
            .collect()
    } else {
        Vec::new()
    };
    let retry_only_failed = !retry_failed_ids.is_empty();
    let mut collection_items = Vec::new();
    let mut log_summary = vec![format!(
        "Runner command prepared: Zotero batch worker for {}",
        source_title(&job.source)
    )];

    for route in job
        .route
        .iter()
        .filter(|route| route.route_kind != "translation_handoff")
    {
        if retry_only_failed && !retry_failed_ids.iter().any(|id| id == &route.id) {
            if let Some(previous) = job
                .collection_items
                .iter()
                .find(|item| item.id == route.id)
                .cloned()
            {
                collection_items.push(previous);
            }
            continue;
        }

        if route.route_kind == "already_converted" {
            log_summary.push(format!(
                "Collection item skipped: {} already converted",
                route.id
            ));
            collection_items.push(collection_item_from_route(
                route,
                "skipped",
                route.blocked_reason.clone(),
                Vec::new(),
                job.attempts,
            ));
            continue;
        }

        if !route.can_run {
            let reason = route
                .blocked_reason
                .clone()
                .unwrap_or_else(|| route.summary.clone());
            log_summary.push(redact_runner_message(&format!(
                "Collection item blocked: {} {reason}",
                route.id
            )));
            collection_items.push(collection_item_from_route(
                route,
                STATUS_BLOCKED,
                Some(reason),
                Vec::new(),
                job.attempts,
            ));
            continue;
        }

        let item_output_dir = output_dir.join(clean_path_component(&route.id));
        let item_job = zotero_item_job(job, route);
        let mut command =
            build_zotero_conversion_command_for_root(&item_job, &item_output_dir, root)?;
        inject_ocr_credentials(&mut command);
        fs::create_dir_all(&command.output_dir).map_err(|err| err.to_string())?;
        match executor.execute(&command) {
            Ok(command_result) => {
                let zotero_key = extract_zotero_attachment_key(&command_result);
                let mut artifacts = scan_artifacts(&command.output_dir)?;
                if let Some(key) = &zotero_key {
                    for artifact in artifacts
                        .iter_mut()
                        .filter(|artifact| artifact.kind == "markdown")
                    {
                        artifact.zotero_key = Some(key.clone());
                    }
                }
                log_summary.push(format!(
                    "Collection item completed: {} via {}",
                    route.id, route.route_kind
                ));
                log_summary.extend(redact_log_lines(&command_result.log_summary));
                log_summary.extend(parse_allowlisted_worker_markers(
                    &command_result.stdout,
                    &[command.output_dir.as_path()],
                ));
                log_summary.extend(parse_allowlisted_worker_markers(
                    &command_result.stderr,
                    &[command.output_dir.as_path()],
                ));
                if let Some(key) = zotero_key {
                    log_summary.push(format!("Zotero Markdown attachment recorded: {key}"));
                }
                collection_items.push(collection_item_from_route(
                    route,
                    STATUS_COMPLETED,
                    None,
                    artifacts,
                    job.attempts,
                ));
            }
            Err(error) => {
                let redacted = redact_runner_message(&error);
                log_summary.push(redact_runner_message(&format!(
                    "Collection item failed: {} {error}",
                    route.id
                )));
                let artifacts = scan_artifacts(&command.output_dir)?;
                collection_items.push(collection_item_from_route(
                    route,
                    STATUS_FAILED,
                    Some(redacted),
                    artifacts,
                    job.attempts,
                ));
            }
        }
    }

    let mut artifacts = Vec::new();
    for item in &collection_items {
        artifacts.extend(item.artifacts.clone());
    }
    let completed = collection_items
        .iter()
        .filter(|item| item.status == STATUS_COMPLETED)
        .count();
    let failed = collection_items
        .iter()
        .filter(|item| item.status == STATUS_FAILED)
        .count();
    let blocked = collection_items
        .iter()
        .filter(|item| item.status == STATUS_BLOCKED)
        .count();
    let skipped = collection_items
        .iter()
        .filter(|item| item.status == "skipped")
        .count();
    let current_step = format!(
        "Collection summary: completed={completed} failed={failed} blocked={blocked} skipped={skipped}"
    );
    let manifest_path = output_dir.join("collection-summary.json");
    let manifest = serde_json::json!({
        "schema": "book-pipeline-collection-summary-v1",
        "jobId": job.id,
        "counts": {
            "completed": completed,
            "failed": failed,
            "blocked": blocked,
            "skipped": skipped,
        },
        "items": collection_items.iter().map(|item| serde_json::json!({
            "id": item.id,
            "routeKind": item.route_kind,
            "status": item.status,
        })).collect::<Vec<_>>(),
    });
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())? + "\n",
    )
    .map_err(|error| error.to_string())?;
    artifacts.push(required_stage_artifact(
        "collection_manifest",
        &manifest_path,
        "discover",
    )?);
    log_summary.push(current_step.clone());
    Ok(RunnerOutput {
        log_summary: trim_log_summary(&log_summary),
        artifacts,
        collection_items,
        output_dir: Some(output_dir.to_path_buf()),
        current_step: Some(current_step),
    })
}

fn collection_item_from_route(
    route: &BookPipelineRouteItem,
    status: &str,
    last_error: Option<String>,
    artifacts: Vec<BookPipelineArtifact>,
    attempts: u32,
) -> BookPipelineCollectionItem {
    BookPipelineCollectionItem {
        id: route.id.clone(),
        title: route.title.clone(),
        route_kind: route.route_kind.clone(),
        status: status.into(),
        last_error,
        artifacts,
        attempts,
    }
}

fn zotero_item_job(parent: &BookPipelineJob, route: &BookPipelineRouteItem) -> BookPipelineJob {
    let mut source = parent.source.clone();
    source.kind = "zotero_attachment".into();
    source.title = Some(route.title.clone());
    source.path = Some(route.source_ref.clone());
    source.selector = Some(route.id.clone());
    source.fake_zotero_items = None;
    BookPipelineJob {
        schema_version: JOB_SCHEMA_VERSION.into(),
        id: format!("{}-{}", parent.id, route.id),
        kind: "single".into(),
        mode: parent.mode.clone(),
        translation_mode: parent.translation_mode.clone(),
        translation_profile_id: parent.translation_profile_id.clone(),
        translation_config_id: parent.translation_config_id.clone(),
        translation_skill_ids: parent.translation_skill_ids.clone(),
        second_pass_enabled: parent.second_pass_enabled,
        text_cleanup: parent.text_cleanup,
        digest_mode: parent.digest_mode,
        output_formats: parent.output_formats.clone(),
        source,
        route: vec![BookPipelineRouteItem {
            source_kind: "zotero_attachment".into(),
            ..route.clone()
        }],
        status: STATUS_RUNNING.into(),
        current_stage_id: "extract".into(),
        current_step: format!("Running collection item {}", route.id),
        last_error: None,
        log_summary: Vec::new(),
        artifacts: Vec::new(),
        collection_items: Vec::new(),
        output_dir: None,
        attempts: parent.attempts,
        stages: Vec::new(),
        children: Vec::new(),
        membership: None,
        summary: BookPipelineStatusSummary::default(),
        progress: BookPipelineProgress::default(),
        notification_deliveries: Vec::new(),
        approval_references: Vec::new(),
        navigation_targets: Vec::new(),
        open_target: None,
        created_at: parent.created_at.clone(),
        updated_at: now_label(),
    }
}

impl RunnerCommandExecutor for SystemCommandExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        match command.kind {
            RunnerCommandKind::Fake => run_fake_command(command),
            RunnerCommandKind::Process => run_process_command(command),
        }
    }
}

fn build_runner_command(job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerCommand, String> {
    build_runner_command_with_root(job, output_dir, None)
}

fn build_runner_command_with_root(
    job: &BookPipelineJob,
    output_dir: &Path,
    book_ocr_root: Option<&Path>,
) -> Result<RunnerCommand, String> {
    match job.source.kind.as_str() {
        "fake" => build_fake_runner_command(job, output_dir),
        "external_adapter" => build_external_adapter_command(job, output_dir),
        "local_pdf_folder" => {
            if let Some(root) = book_ocr_root {
                build_local_pdf_folder_command_for_root(job, output_dir, root)
            } else {
                build_local_pdf_folder_command(job, output_dir)
            }
        }
        "zotero_attachment" | "zotero_collection" | "zotero_filter" => {
            if let Some(root) = book_ocr_root {
                build_zotero_conversion_command_for_root(job, output_dir, root)
            } else {
                build_zotero_conversion_command(job, output_dir)
            }
        }
        other => Err(format!("Unsupported Book Pipeline source kind: {other}")),
    }
}

fn build_fake_runner_command(
    job: &BookPipelineJob,
    output_dir: &Path,
) -> Result<RunnerCommand, String> {
    let behavior = job
        .source
        .runner_behavior
        .clone()
        .unwrap_or_else(|| "succeed".into());
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Fake,
        label: "fake Book Pipeline runner".into(),
        program: PathBuf::from("__fake_book_pipeline_runner__"),
        args: vec![
            "--behavior".into(),
            behavior,
            "--title".into(),
            source_title(&job.source),
            "--output-dir".into(),
            display_path(output_dir),
        ],
        env: Vec::new(),
        cwd: None,
        output_dir: output_dir.to_path_buf(),
        attempts: job.attempts,
        accepted_exit_codes: vec![0],
    })
}

fn run_markdown_source_job(
    job: &BookPipelineJob,
    output_dir: &Path,
) -> Result<RunnerOutput, String> {
    let source_path = markdown_source_path(&job.source)?;
    ensure_markdown_source_allowed(&source_path)?;
    fs::create_dir_all(output_dir).map_err(|err| err.to_string())?;
    let copied = output_dir.join("source.md");
    fs::copy(&source_path, &copied).map_err(|err| {
        format!(
            "Failed to stage Markdown source {}: {err}",
            display_path(&source_path)
        )
    })?;
    let artifacts = scan_artifacts(output_dir)?;
    Ok(RunnerOutput {
        log_summary: vec![format!(
            "Markdown source staged for translation handoff: {}",
            display_path(&source_path)
        )],
        artifacts,
        collection_items: Vec::new(),
        output_dir: Some(output_dir.to_path_buf()),
        current_step: None,
    })
}

fn build_external_adapter_command(
    job: &BookPipelineJob,
    output_dir: &Path,
) -> Result<RunnerCommand, String> {
    let program = non_empty(job.source.adapter_command.as_deref())
        .or_else(|| non_empty(job.source.path.as_deref()))
        .ok_or_else(|| "External adapter source is missing a command path.".to_string())?;
    let program = PathBuf::from(program);
    if !program.is_file() {
        return Err(format!(
            "External adapter command not found at {}",
            display_path(&program)
        ));
    }
    let mut args = Vec::new();
    if let Some(input) = non_empty(job.source.path.as_deref()) {
        if Path::new(input) != program {
            args.push("--input".into());
            args.push(input.to_string());
        }
    }
    args.push("--output-dir".into());
    args.push(display_path(output_dir));
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: "external Book Pipeline adapter".into(),
        program,
        args,
        env: Vec::new(),
        cwd: None,
        output_dir: output_dir.to_path_buf(),
        attempts: job.attempts,
        accepted_exit_codes: vec![0],
    })
}

/// The OCR and Zotero wrappers import PyMuPDF, pypdf, requests and markdown,
/// which the `ocr` workspace member declares and uv installs into the repo-root
/// `.venv`. A bare interpreter only ever found them where a machine happened to
/// have them installed globally, so these scripts go through `uv run --package`
/// exactly like the translation, index and digest stages already do. The command
/// runs from the OCR root, from which uv discovers the enclosing workspace.
fn ocr_python_args(script: &Path) -> Vec<String> {
    vec![
        "run".into(),
        "--package".into(),
        "ocr".into(),
        "python".into(),
        display_path(script),
    ]
}

fn build_local_pdf_folder_command(
    job: &BookPipelineJob,
    output_dir: &Path,
) -> Result<RunnerCommand, String> {
    build_local_pdf_folder_command_for_root(job, output_dir, &book_ocr_conversion_root())
}

fn build_local_pdf_folder_command_for_root(
    job: &BookPipelineJob,
    output_dir: &Path,
    root: &Path,
) -> Result<RunnerCommand, String> {
    let input_dir = job
        .source
        .path
        .as_deref()
        .ok_or_else(|| "Local PDF folder source is missing a path.".to_string())?;
    let script = root.join("scripts").join("pdf_to_html_paddleocr.py");
    if !script.is_file() {
        return Err(format!(
            "Local PDF conversion wrapper not found at {}",
            display_path(&script)
        ));
    }
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: "local PDF conversion wrapper".into(),
        program: PathBuf::from("uv"),
        args: {
            let mut args = ocr_python_args(&script);
            args.extend([
                "--input-dir".into(),
                input_dir.into(),
                "--output-dir".into(),
                display_path(output_dir),
            ]);
            args
        },
        env: Vec::new(),
        cwd: Some(root.to_path_buf()),
        output_dir: output_dir.to_path_buf(),
        attempts: job.attempts,
        accepted_exit_codes: vec![0],
    })
}

fn build_zotero_conversion_command(
    job: &BookPipelineJob,
    output_dir: &Path,
) -> Result<RunnerCommand, String> {
    build_zotero_conversion_command_for_root(job, output_dir, &book_ocr_conversion_root())
}

fn build_zotero_conversion_command_for_root(
    job: &BookPipelineJob,
    output_dir: &Path,
    root: &Path,
) -> Result<RunnerCommand, String> {
    let route = runnable_zotero_route(job)?;
    build_zotero_conversion_command_for_source(&job.source, &route, job.attempts, output_dir, root)
}

fn build_zotero_child_conversion_command_for_root(
    child: &BookPipelineChildJob,
    output_dir: &Path,
    root: &Path,
) -> Result<RunnerCommand, String> {
    let route = child
        .route
        .iter()
        .find(|route| route.can_run && route.route_kind != "translation_handoff")
        .ok_or_else(|| "Durable collection child has no runnable attachment route.".to_string())?;
    let mut command = build_zotero_worker_conversion_command_for_source(
        &child.source,
        route,
        child.attempts,
        output_dir,
        root,
    )?;
    command.args.push("--preserve-source".into());
    Ok(command)
}

fn build_zotero_conversion_command_for_source(
    source: &BookPipelineSource,
    route: &BookPipelineRouteItem,
    attempts: u32,
    output_dir: &Path,
    root: &Path,
) -> Result<RunnerCommand, String> {
    if route.route_kind == "mineru" {
        return build_mineru_command_for_root(source, attempts, output_dir, root, route);
    }
    build_zotero_worker_conversion_command_for_source(source, route, attempts, output_dir, root)
}

fn build_zotero_worker_conversion_command_for_source(
    source: &BookPipelineSource,
    route: &BookPipelineRouteItem,
    attempts: u32,
    output_dir: &Path,
    root: &Path,
) -> Result<RunnerCommand, String> {
    let script = root.join("scripts").join("zotero_llm_worker.py");
    if !script.is_file() {
        return Err(format!(
            "Zotero worker not found at {}",
            display_path(&script)
        ));
    }
    let mut args = ocr_python_args(&script);
    match source.kind.as_str() {
        "zotero_attachment" => {
            let selector = non_empty(source.selector.as_deref())
                .ok_or_else(|| "Zotero attachment source is missing a selector.".to_string())?;
            args.push("--attachment-key".into());
            args.push(selector.to_string());
        }
        "zotero_filter" => {
            if let Some(parent_type) = parent_item_type_filter(source.selector.as_deref()) {
                args.push("--parent-item-type".into());
                args.push(parent_type);
            }
            if let Some(query) = zotero_query_filter(source.selector.as_deref()) {
                args.push("--query".into());
                args.push(query);
            }
            args.push("--limit".into());
            args.push("20".into());
        }
        "zotero_collection" => {
            return Err(
                "Zotero collection execution is not supported by the current worker CLI.".into(),
            );
        }
        _ => return Err("Unsupported Zotero source kind.".into()),
    }
    match route.route_kind.as_str() {
        "direct_text" => args.push("--force-text".into()),
        "remote_paddleocr" => args.push("--force-ocr".into()),
        "mineru" => args.push("--force-mineru".into()),
        "blocked_dirty_text_layer" | "already_converted" | "missing_credentials" => {
            return Err(format!(
                "Zotero route {} is not runnable: {}",
                route.route_kind,
                route
                    .blocked_reason
                    .clone()
                    .unwrap_or_else(|| route.summary.clone())
            ));
        }
        other => return Err(format!("Unsupported Zotero route kind: {other}")),
    }
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: ZOTERO_CONVERSION_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args,
        env: vec![("OCR_OUTPUT_ROOT".into(), display_path(output_dir))],
        cwd: Some(root.to_path_buf()),
        output_dir: output_dir.to_path_buf(),
        attempts,
        accepted_exit_codes: vec![0],
    })
}

fn build_mineru_command_for_root(
    source: &BookPipelineSource,
    attempts: u32,
    output_dir: &Path,
    root: &Path,
    route: &BookPipelineRouteItem,
) -> Result<RunnerCommand, String> {
    let script = mineru_script_for_root(root)?;
    let selector = non_empty(source.selector.as_deref())
        .or_else(|| non_empty(Some(route.id.as_str())))
        .ok_or_else(|| "MinerU source is missing a selector.".to_string())?;
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: "MinerU extraction wrapper".into(),
        program: PathBuf::from("uv"),
        args: {
            let mut args = ocr_python_args(&script);
            args.extend([
                "--attachment-key".into(),
                selector.to_string(),
                "--output-dir".into(),
                display_path(output_dir),
            ]);
            args
        },
        env: vec![("OCR_OUTPUT_ROOT".into(), display_path(output_dir))],
        cwd: Some(root.to_path_buf()),
        output_dir: output_dir.to_path_buf(),
        attempts,
        accepted_exit_codes: vec![0],
    })
}

fn mineru_script_for_root(root: &Path) -> Result<PathBuf, String> {
    for candidate in [
        root.join("mineru.py"),
        root.join("scripts").join("mineru.py"),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "MinerU wrapper not found at {} or {}",
        display_path(&root.join("mineru.py")),
        display_path(&root.join("scripts").join("mineru.py"))
    ))
}

fn build_zotero_discovery_command_for_root(
    source: &BookPipelineSource,
    limit: u32,
    root: &Path,
) -> Result<RunnerCommand, String> {
    let script = root.join("scripts").join("zotero_llm_worker.py");
    if !script.is_file() {
        return Err(format!(
            "Zotero worker not found at {}",
            display_path(&script)
        ));
    }
    let mut args = ocr_python_args(&script);
    args.extend([
        "--dry-run".into(),
        "--limit".into(),
        limit.clamp(1, 100).to_string(),
    ]);
    if source.kind == "zotero_attachment" {
        if let Some(selector) = non_empty(source.selector.as_deref()) {
            args.push("--attachment-key".into());
            args.push(selector.to_string());
            args.push("--pipeline-route".into());
        }
    }
    if source.kind == "zotero_filter" {
        if let Some(parent_type) = parent_item_type_filter(source.selector.as_deref()) {
            args.push("--parent-item-type".into());
            args.push(parent_type);
        }
        if let Some(query) = zotero_query_filter(source.selector.as_deref()) {
            args.push("--query".into());
            args.push(query);
        }
    }
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: "Zotero discovery dry-run".into(),
        program: PathBuf::from("uv"),
        args,
        env: Vec::new(),
        cwd: Some(root.to_path_buf()),
        output_dir: root
            .join("output")
            .join("book-pipeline")
            .join("zotero-discovery"),
        attempts: 0,
        accepted_exit_codes: vec![0],
    })
}

fn discover_zotero_sources<E: RunnerCommandExecutor>(
    executor: &E,
    source: &BookPipelineSource,
    limit: u32,
) -> Result<BookPipelineZoteroDiscoveryResult, String> {
    if source.kind == "zotero_collection" && source.fake_zotero_items.is_none() {
        return discover_zotero_collection_snapshot(executor, source);
    }
    discover_zotero_sources_with_root(executor, source, limit, &book_ocr_conversion_root())
}

fn discover_zotero_sources_with_root<E: RunnerCommandExecutor>(
    executor: &E,
    source: &BookPipelineSource,
    limit: u32,
    root: &Path,
) -> Result<BookPipelineZoteroDiscoveryResult, String> {
    if let Some(items) = &source.fake_zotero_items {
        return Ok(BookPipelineZoteroDiscoveryResult {
            sources: fake_zotero_sources(source, items),
            log_summary: vec![format!(
                "Loaded {} fake Zotero discovery item(s)",
                items.len()
            )],
        });
    }
    if source.kind == "zotero_collection" {
        return discover_zotero_collection_snapshot(executor, source);
    }

    let mut command = build_zotero_discovery_command_for_root(source, limit, root)?;
    inject_ocr_credentials(&mut command);
    let command_result = executor
        .execute(&command)
        .map_err(|err| redact_runner_message(&format!("Zotero discovery failed: {err}")))?;
    let mut log_summary = vec![format!("Runner command prepared: {}", command.label)];
    log_summary.extend(redact_log_lines(&command_result.log_summary));
    log_summary.extend(parse_allowlisted_worker_markers(
        &command_result.stdout,
        &[command.output_dir.as_path()],
    ));
    log_summary.extend(parse_allowlisted_worker_markers(
        &command_result.stderr,
        &[command.output_dir.as_path()],
    ));

    let mut sources = parse_zotero_discovery_sources(
        source,
        &format!("{}\n{}", command_result.stdout, command_result.stderr),
    );
    if sources.is_empty() {
        if let Some(selection) = zotero_selection_source(source) {
            sources.push(selection);
        }
    }
    Ok(BookPipelineZoteroDiscoveryResult {
        sources,
        log_summary: trim_log_summary(&log_summary),
    })
}

fn discover_zotero_collection_snapshot<E: RunnerCommandExecutor>(
    executor: &E,
    source: &BookPipelineSource,
) -> Result<BookPipelineZoteroDiscoveryResult, String> {
    let collection_key = non_empty(source.selector.as_deref())
        .ok_or_else(|| "Zotero collection source is missing a collection key.".to_string())?;
    let snapshot = read_zotero_collection_snapshot(executor, collection_key)?;
    let attachment_count = snapshot
        .members
        .iter()
        .filter(|member| member.attachment_key.is_some())
        .count();
    let eligible_count = snapshot
        .members
        .iter()
        .filter(|member| member.eligibility == "eligible_pdf")
        .count();
    let outcomes = snapshot_outcome_counts(&snapshot)
        .iter()
        .map(|(outcome, count)| format!("{outcome}:{count}"))
        .collect::<Vec<_>>()
        .join(",");
    Ok(BookPipelineZoteroDiscoveryResult {
        sources: vec![BookPipelineSource {
            kind: "zotero_collection".into(),
            title: Some(snapshot.collection.name),
            path: None,
            selector: Some(snapshot.collection.key),
            runner_behavior: None,
            translation_strategy: source.translation_strategy.clone(),
            adapter_command: source.adapter_command.clone(),
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        }],
        log_summary: vec![format!(
            "Collection snapshot discovered: members={} attachments={attachment_count} eligible={eligible_count} outcomes={outcomes}",
            snapshot.members.len()
        )],
    })
}

fn snapshot_outcome_counts(snapshot: &ZoteroCollectionSnapshot) -> BTreeMap<String, usize> {
    snapshot
        .members
        .iter()
        .fold(BTreeMap::new(), |mut counts, member| {
            *counts.entry(member.eligibility.clone()).or_default() += 1;
            counts
        })
}

fn preview_zotero_route_from_worker<E: RunnerCommandExecutor>(
    executor: &E,
    source: &BookPipelineSource,
    config: BookPipelinePreviewConfig,
    limit: u32,
    root: &Path,
) -> Result<Vec<BookPipelineRouteItem>, String> {
    let discovery = discover_zotero_sources_with_root(executor, source, limit, root)?;
    let mut routes = Vec::new();
    for discovered_source in discovery.sources {
        if discovered_source.fake_zotero_items.is_some() {
            routes.extend(preview_zotero_source(&discovered_source, config.clone()));
        }
    }
    if routes.is_empty() {
        return Ok(vec![zotero_undiscovered_route_item(source)]);
    }
    let overrides = if config.route_overrides.is_empty() {
        &source.route_overrides
    } else {
        &config.route_overrides
    };
    apply_route_overrides(&mut routes, overrides, Some(&config));
    Ok(routes)
}

fn parse_zotero_worker_attachment_evidence(
    text: &str,
    expected_attachment_key: &str,
) -> Result<Option<ZoteroWorkerAttachmentEvidence>, String> {
    let mut parsed = Vec::new();
    for line in text.lines() {
        let Some((_, payload)) = line.split_once(ZOTERO_WORKER_ATTACHMENT_EVIDENCE_MARKER) else {
            continue;
        };
        let evidence: ZoteroWorkerAttachmentEvidence = serde_json::from_str(payload.trim())
            .map_err(|_| "Zotero worker returned invalid attachment evidence.".to_string())?;
        if evidence.schema_version != ZOTERO_WORKER_ATTACHMENT_EVIDENCE_SCHEMA {
            return Err(
                "Zotero worker returned an unsupported attachment evidence contract.".into(),
            );
        }
        parsed.push(evidence);
    }
    if parsed.len() > 1 {
        return Err("Zotero worker returned duplicate attachment evidence.".into());
    }
    let Some(evidence) = parsed.pop() else {
        return Ok(None);
    };
    if evidence.pdf_attachment_key != expected_attachment_key {
        return Err(
            "Zotero worker attachment evidence does not match the frozen attachment.".into(),
        );
    }
    Ok(Some(evidence))
}

fn reused_markdown_artifact_from_evidence(
    child: &BookPipelineChildJob,
    evidence: &ZoteroWorkerAttachmentEvidence,
) -> Result<BookPipelineArtifact, String> {
    let identity = child.source_identity.as_ref().ok_or_else(|| {
        "Durable collection child is missing its frozen attachment identity.".to_string()
    })?;
    if evidence.status != "already_completed"
        || evidence.extraction_contract_version != ZOTERO_WORKER_EXTRACTION_CONTRACT_VERSION
        || evidence.parent_item_key != identity.parent_item_key
        || evidence.pdf_attachment_key != identity.pdf_attachment_key
        || evidence.route.trim().is_empty()
        || evidence.source_sha256 != sha256_file(Path::new(&identity.attachment_path))?
    {
        return Err(
            "Completed worker evidence does not match the extraction contract or frozen attachment identity."
                .into(),
        );
    }
    let markdown_path = Path::new(&evidence.markdown_path);
    if !markdown_path.is_absolute()
        || evidence.markdown_sha256.len() != 64
        || !evidence
            .markdown_sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit())
        || clean_zotero_key(&evidence.markdown_attachment_key).as_deref()
            != Some(evidence.markdown_attachment_key.as_str())
    {
        return Err("Completed worker evidence has an invalid Markdown identity.".into());
    }
    let mut artifact = required_stage_artifact("markdown", markdown_path, "extract")?;
    if artifact.sha256.as_deref() != Some(evidence.markdown_sha256.as_str()) {
        return Err("Completed worker Markdown hash does not match the persisted evidence.".into());
    }
    if markdown_frontmatter_value(markdown_path, "parent_item_key")
        .is_some_and(|parent_item_key| parent_item_key != identity.parent_item_key)
    {
        return Err("Completed worker Markdown does not match the frozen parent item.".into());
    }
    artifact.zotero_key = Some(evidence.markdown_attachment_key.clone());
    artifact.input_hashes.insert(
        "workerEvidenceContract".into(),
        evidence.schema_version.clone(),
    );
    artifact.input_hashes.insert(
        "workerExtractionContract".into(),
        evidence.extraction_contract_version.clone(),
    );
    artifact
        .input_hashes
        .insert("sourceSha256".into(), evidence.source_sha256.clone());
    artifact
        .input_hashes
        .insert("reusedRoute".into(), evidence.route.clone());
    Ok(artifact)
}

fn route_zotero_attachment_from_worker<E: RunnerCommandExecutor>(
    executor: &E,
    child: &BookPipelineChildJob,
    root: &Path,
) -> Result<AttachmentRouteOutput, String> {
    let identity = child.source_identity.as_ref().ok_or_else(|| {
        "Durable collection child is missing its frozen attachment identity.".to_string()
    })?;
    let mut command = build_zotero_discovery_command_for_root(&child.source, 1, root)?;
    inject_ocr_credentials(&mut command);
    let command_result = executor
        .execute(&command)
        .map_err(|error| redact_runner_message(&format!("Attachment route failed: {error}")))?;
    let combined = format!("{}\n{}", command_result.stdout, command_result.stderr);
    let mut routes = parse_zotero_discovery_sources(&child.source, &combined)
        .into_iter()
        .filter_map(|mut source| source.fake_zotero_items.take().map(|items| (source, items)))
        .flat_map(|(source, items)| {
            items.into_iter().map(move |item| {
                preview_zotero_item(
                    &source,
                    item,
                    &BookPipelinePreviewConfig {
                        has_paddleocr_credentials: true,
                        has_mineru_credentials: true,
                        route_overrides: BTreeMap::new(),
                    },
                )
            })
        })
        .filter(|route| route.id == identity.pdf_attachment_key)
        .collect::<Vec<_>>();
    if routes.len() != 1 {
        return Err(
            "Per-attachment route evidence did not identify the frozen attachment exactly once."
                .into(),
        );
    }
    let mut route = routes.remove(0);
    if worker_plan_route_for_attachment(&combined, &identity.pdf_attachment_key).as_deref()
        == Some("missing-paddleocr-token")
    {
        route.route_kind = "missing_credentials".into();
        route.can_run = false;
        route.blocked_reason = Some("PaddleOCR is unavailable for this attachment.".into());
        route.summary = "Scanned attachment is blocked before extraction.".into();
    }
    route.source_kind = "zotero_attachment".into();
    route.source_ref = child
        .source
        .path
        .clone()
        .unwrap_or_else(|| format!("zotero://attachment/{}", identity.pdf_attachment_key));
    let evidence =
        parse_zotero_worker_attachment_evidence(&combined, &identity.pdf_attachment_key)?;
    let reused_artifact = if route.route_kind == "already_converted" {
        Some(reused_markdown_artifact_from_evidence(
            child,
            evidence.as_ref().ok_or_else(|| {
                "Already-completed route is missing verified Markdown evidence.".to_string()
            })?,
        )?)
    } else {
        None
    };
    Ok(AttachmentRouteOutput {
        log_summary: vec![format!(
            "Attachment route selected: key={} route={}",
            identity.pdf_attachment_key, route.route_kind
        )],
        route,
        reused_artifact,
    })
}

fn worker_plan_route_for_attachment(text: &str, expected_attachment_key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (_, plan) = line.split_once("PLAN ")?;
        let mut tokens = plan.split_whitespace();
        (tokens.next()? == expected_attachment_key)
            .then(|| plan_token_value(plan, "route=").map(str::to_string))?
    })
}

fn run_fake_command(command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
    let behavior = command_arg_value(command, "--behavior").unwrap_or("succeed");
    if behavior == "always_fail" || (behavior == "fail_once" && command.attempts <= 1) {
        return Err("Fake CLI runner failed intentionally.".into());
    }

    fs::create_dir_all(&command.output_dir).map_err(|err| err.to_string())?;
    let markdown = command.output_dir.join("fake-source.md");
    fs::write(
        &markdown,
        format!(
            "# {}\n\nGenerated by the fake Book Pipeline CLI runner.\n",
            command_arg_value(command, "--title").unwrap_or("Fake source")
        ),
    )
    .map_err(|err| err.to_string())?;
    Ok(RunnerCommandResult {
        stdout: "fake runner wrote Markdown output".into(),
        stderr: String::new(),
        log_summary: vec!["Fake CLI runner produced Markdown output".into()],
    })
}

/// Directories a desktop launch is missing from `PATH`. A macOS `.app` started
/// from Finder inherits the launchd default `/usr/bin:/bin:/usr/sbin:/sbin`,
/// which holds none of the places Homebrew, uv, nvm, volta or rustup install
/// into, so a bare `uv`/`node` fails to spawn even though the identical command
/// works from a terminal.
fn extra_program_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(unix)]
    dirs.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
            .into_iter()
            .map(PathBuf::from),
    );
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".cargo").join("bin"));
        dirs.push(home.join(".bun").join("bin"));
        dirs.push(home.join(".volta").join("bin"));
        dirs.extend(nvm_bin_dirs(&home.join(".nvm")));
    }
    #[cfg(target_os = "windows")]
    if let Some(appdata) = env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    dirs
}

/// nvm keeps every Node under a version directory, so unlike the other roots its
/// `bin` cannot be written as a constant. The installed versions are the ground
/// truth; `~/.nvm/alias/default` only chooses among them, and because it may name
/// something not installed (a bare `22`, an `lts/*`, a version since removed) the
/// remaining installs stay behind it newest-first rather than leaving no Node.
fn nvm_bin_dirs(nvm_root: &Path) -> Vec<PathBuf> {
    let mut versions = fs::read_dir(nvm_root.join("versions").join("node"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("bin").is_dir())
        .collect::<Vec<_>>();
    versions.sort_by_key(|path| std::cmp::Reverse(nvm_version_key(path)));
    if let Some(position) = nvm_default_alias(nvm_root).and_then(|alias| {
        versions
            .iter()
            .position(|path| nvm_version_matches(path, &alias))
    }) {
        versions.swap(0, position);
    }
    versions.into_iter().map(|path| path.join("bin")).collect()
}

fn nvm_default_alias(nvm_root: &Path) -> Option<String> {
    let alias = fs::read_to_string(nvm_root.join("alias").join("default")).ok()?;
    let alias = alias.trim().trim_start_matches('v').to_string();
    (!alias.is_empty()).then_some(alias)
}

fn nvm_version_name(path: &Path) -> &str {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(|name| name.trim_start_matches('v'))
        .unwrap_or_default()
}

/// `v9.1.0` sorts below `v24.0.0`, which a plain string comparison gets backwards.
fn nvm_version_key(path: &Path) -> Vec<u64> {
    nvm_version_name(path)
        .split('.')
        .map(|part| part.parse().unwrap_or_default())
        .collect()
}

/// A `22` alias means the `v22.*` install, not `v22` exactly and not `v220.*`.
fn nvm_version_matches(path: &Path, alias: &str) -> bool {
    let name = nvm_version_name(path);
    name == alias
        || name
            .strip_prefix(alias)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// `PATH` first so a user who put a specific toolchain in front keeps it; the
/// desktop-missing roots only fill the gap behind it.
fn program_search_dirs_from(path_var: Option<&OsStr>, extra: Vec<PathBuf>) -> Vec<PathBuf> {
    let inherited = path_var
        .map(|value| env::split_paths(value).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut dirs = Vec::new();
    let mut seen = BTreeSet::new();
    for dir in inherited.into_iter().chain(extra) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if seen.insert(dir.clone()) {
            dirs.push(dir);
        }
    }
    dirs
}

fn program_search_dirs() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        program_search_dirs_from(env::var_os("PATH").as_deref(), extra_program_dirs())
    })
}

fn program_file_names(name: &str) -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        if Path::new(name).extension().is_some() {
            return vec![name.to_string()];
        }
        ["exe", "cmd", "bat"]
            .iter()
            .map(|extension| format!("{name}.{extension}"))
            .chain(std::iter::once(name.to_string()))
            .collect()
    }
    #[cfg(not(target_os = "windows"))]
    {
        vec![name.to_string()]
    }
}

fn lookup_program_in(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let file_names = program_file_names(name);
    dirs.iter().find_map(|dir| {
        file_names
            .iter()
            .map(|file_name| dir.join(file_name))
            .find(|candidate| candidate.is_file())
    })
}

/// A program that already carries a directory component is an explicit choice
/// and is left alone; a bare name is looked up in the search dirs, and staying
/// as the bare name when nothing matches keeps the spawn error readable.
fn resolve_runner_program_in(dirs: &[PathBuf], program: &Path) -> PathBuf {
    if program
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        return program.to_path_buf();
    }
    let Some(name) = program.file_name().and_then(OsStr::to_str) else {
        return program.to_path_buf();
    };
    lookup_program_in(dirs, name).unwrap_or_else(|| program.to_path_buf())
}

/// The launcher downloads a private JRE and CPython, and `run_epubcheck.js` /
/// `run_python.js` already honour `BIBLIOSMITH_JAVA` / `BIBLIOSMITH_PYTHON`;
/// resolving the same way here keeps the Rust EPUBCheck and bilingual-build
/// calls on that contract instead of whatever interpreter a desktop session
/// inherited. Resolved per call, not cached: preparing a runtime from Settings
/// mid-session has to take effect on the next stage rather than after a restart.
fn resolve_runner_program(program: &Path) -> PathBuf {
    let managed = match program.as_os_str().to_str() {
        Some("java") => crate::managed_java_executable(),
        Some("python3") => crate::managed_python_executable(),
        _ => None,
    };
    if let Some(path) = managed {
        return path;
    }
    resolve_runner_program_in(program_search_dirs(), program)
}

/// The child needs the same search path: `uv` has to find a Python, `node` has
/// to find its own tooling, and neither inherits anything useful from a desktop
/// launch. `command.env` is applied afterwards so a command can still override.
fn runner_path_env_value() -> Option<OsString> {
    env::join_paths(program_search_dirs()).ok()
}

fn run_process_command(command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
    let program = resolve_runner_program(&command.program);
    let mut process = Command::new(&program);
    process.args(&command.args);
    if let Some(path_value) = runner_path_env_value() {
        process.env("PATH", path_value);
    }
    process.envs(command.env.iter().map(|(key, value)| (key, value)));
    if let Some(cwd) = &command.cwd {
        process.current_dir(cwd);
    }
    let output = process.output().map_err(|err| {
        format!(
            "Failed to start {} with {}: {err}",
            command.label,
            display_path(&program)
        )
    })?;
    command_result_from_output(command, output)
}

fn command_result_from_output(
    command: &RunnerCommand,
    output: Output,
) -> Result<RunnerCommandResult, String> {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let status_code = output.status.code();
    if status_code.is_some_and(|code| command.accepted_exit_codes.contains(&code)) {
        let outcome = if output.status.success() {
            "completed"
        } else {
            "reported unit failures"
        };
        return Ok(RunnerCommandResult {
            stdout,
            stderr,
            log_summary: vec![format!("{} {outcome}", command.label)],
        });
    }
    let status = status_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".into());
    let detail = stderr_tail(&stderr);
    if detail.is_empty() {
        Err(format!("{} exited with status {status}.", command.label))
    } else {
        Err(format!(
            "{} exited with status {status}: {detail}",
            command.label
        ))
    }
}

/// The last few non-empty stderr lines, redacted and joined into one line.
/// Python tracebacks put the actual exception on the final line, so this
/// usually tells "missing key" from "network error" without the caller
/// having to go read a log file for the raw exit-status error above.
fn stderr_tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let tail_start = lines.len().saturating_sub(3);
    lines[tail_start..]
        .iter()
        .map(|line| redact_stderr_line(line))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Unlike `redact_runner_message` (built for verbose log summaries, where any
/// mention of "key" is reason enough to hide the whole line), this only
/// redacts a line that looks like it is carrying an actual secret — an auth
/// header or a `KEY=value` assignment — so "GEMINI_API_KEY not set" stays
/// legible instead of being blanked out along with the credentials it never
/// contained.
fn redact_stderr_line(line: &str) -> String {
    if message_carries_a_leaked_value(line) {
        "[redacted]".to_string()
    } else {
        line.to_string()
    }
}

/// Whether a message looks like it is carrying an actual secret value — an
/// auth header, a signed-request signature, or a `KEY=value` assignment — as
/// opposed to merely naming a credential. Used to decide whether an error
/// message needs to be blanked out entirely: "GEMINI_API_KEY not set" names
/// no secret and should stay legible, while "Authorization: Bearer sk-..."
/// must not reach the UI intact.
fn message_carries_a_leaked_value(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("bearer ")
        || lower.contains("authorization:")
        || lower.contains("x-goog-api-key")
        || lower.contains("x-goog-signature")
        || lower.contains("x-amz-signature")
        || lower.contains("sig=")
        || (text.contains('=')
            && (lower.contains("key")
                || lower.contains("token")
                || lower.contains("secret")
                || lower.contains("signature")))
}

fn extract_zotero_attachment_key(result: &RunnerCommandResult) -> Option<String> {
    for text in result
        .log_summary
        .iter()
        .map(String::as_str)
        .chain([result.stdout.as_str(), result.stderr.as_str()])
    {
        if let Some(key) = extract_zotero_attachment_key_from_text(text) {
            return Some(key);
        }
    }
    None
}

fn extract_zotero_attachment_key_from_text(text: &str) -> Option<String> {
    for line in text.lines() {
        let Some((_, payload)) = line.split_once(ZOTERO_WORKER_ATTACHMENT_EVIDENCE_MARKER) else {
            continue;
        };
        let Ok(evidence) = serde_json::from_str::<ZoteroWorkerAttachmentEvidence>(payload.trim())
        else {
            continue;
        };
        if evidence.schema_version == ZOTERO_WORKER_ATTACHMENT_EVIDENCE_SCHEMA {
            if let Some(key) = clean_zotero_key(&evidence.markdown_attachment_key) {
                return Some(key);
            }
        }
    }
    for marker in [
        "zotero_attachment_key=",
        "markdown_attachment=",
        "markdown_attachment_key=",
        "to Zotero attachment ",
    ] {
        let Some((_, after)) = text.split_once(marker) else {
            continue;
        };
        if let Some(key) = clean_zotero_key(after) {
            return Some(key);
        }
    }
    None
}

fn clean_zotero_key(value: &str) -> Option<String> {
    let key: String = value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ':' | '=' | ',' | ';'))
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric())
        .collect();
    (key.len() >= 3).then_some(key)
}

fn command_arg_value<'a>(command: &'a RunnerCommand, key: &str) -> Option<&'a str> {
    command
        .args
        .windows(2)
        .find(|pair| pair[0] == key)
        .map(|pair| pair[1].as_str())
}

fn parse_allowlisted_worker_markers(value: &str, allowed_roots: &[&Path]) -> Vec<String> {
    let mut markers = Vec::new();
    for line in value.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(key) = extract_zotero_attachment_key_from_text(line) {
            let marker = format!("worker marker: markdown_attachment_key={key}");
            if !markers.contains(&marker) {
                markers.push(marker);
            }
            continue;
        }
        if let Some(alignment) = line
            .split_whitespace()
            .find_map(|token| token.strip_prefix("alignment="))
            .filter(|alignment| matches!(*alignment, "paragraph" | "chapter-fallback"))
        {
            let marker = format!("worker marker: alignment={alignment}");
            if !markers.contains(&marker) {
                markers.push(marker);
            }
            continue;
        }
        let Some(payload) = line.strip_prefix("BOOK_PIPELINE_MARKER ") else {
            continue;
        };
        let mut fields = Vec::new();
        for token in payload.split_whitespace() {
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            let allowed = match key {
                "status" => matches!(
                    value,
                    STATUS_PENDING
                        | STATUS_READY
                        | STATUS_RUNNING
                        | STATUS_WAITING_FOR_APPROVAL
                        | STATUS_BLOCKED
                        | STATUS_FAILED
                        | STATUS_COMPLETED
                        | STATUS_PARTIAL
                        | STATUS_SKIPPED
                ),
                "count" => !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()),
                "sha256" => value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit()),
                "path" => {
                    let path = Path::new(value);
                    path.is_absolute()
                        && !path
                            .components()
                            .any(|component| component == std::path::Component::ParentDir)
                        && allowed_roots.iter().any(|root| path.starts_with(root))
                }
                _ => false,
            };
            if allowed {
                fields.push(format!("{key}={value}"));
            }
        }
        if !fields.is_empty() {
            markers.push(format!("worker marker: {}", fields.join(" ")));
        }
    }
    markers
}

fn redact_log_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| redact_runner_message(line))
        .collect()
}

fn parse_zotero_discovery_sources(
    requested_source: &BookPipelineSource,
    text: &str,
) -> Vec<BookPipelineSource> {
    let mut sources = Vec::new();
    if let Some(selection) = zotero_selection_source(requested_source) {
        if requested_source.kind == "zotero_collection" || requested_source.kind == "zotero_filter"
        {
            sources.push(selection);
        }
    }
    for line in text.lines() {
        let Some(source) =
            parse_zotero_plan_source(line).or_else(|| parse_zotero_completed_source(line))
        else {
            continue;
        };
        if !sources
            .iter()
            .any(|existing| existing.kind == source.kind && existing.selector == source.selector)
        {
            sources.push(source);
        }
    }
    sources
}

fn parse_zotero_plan_source(line: &str) -> Option<BookPipelineSource> {
    let (_, plan) = line.split_once("PLAN ")?;
    let key = non_empty(plan.split_whitespace().next())?;
    let route = plan_token_value(plan, "route=").unwrap_or("pdf-text");
    let route = route.replace('_', "-");
    let title = plan
        .split_once(" title=")
        .map(|(_, title)| title.trim())
        .map(strip_worker_markers)
        .and_then(|title| non_empty(Some(title)))
        .unwrap_or(key);
    let fingerprint = source_fingerprint_token(plan);
    let dirty_text_layer = route == "needs-mineru" || route == "blocked-dirty-text-layer";
    let already_converted = route == "already-converted" || route == "skipped-completed";
    let prefer_mineru = route == "mineru";
    let item = FakeZoteroItem {
        key: key.to_string(),
        title: title.to_string(),
        attachment_path: Some(zotero_source_ref(key, fingerprint.as_deref(), None)),
        has_text_layer: route == "pdf-text" || dirty_text_layer || already_converted,
        dirty_text_layer,
        scanned: route == "paddle-ocr" || route == "remote-paddleocr" || prefer_mineru,
        already_converted,
        prefer_mineru,
    };
    Some(BookPipelineSource {
        kind: "zotero_attachment".into(),
        title: Some(title.to_string()),
        path: None,
        selector: Some(key.to_string()),
        runner_behavior: None,
        translation_strategy: None,
        adapter_command: None,
        fake_zotero_items: Some(vec![item]),
        route_overrides: BTreeMap::new(),
    })
}

fn parse_zotero_completed_source(line: &str) -> Option<BookPipelineSource> {
    let (_, skipped) = line.split_once("SKIP completed ")?;
    let mut parts = skipped.splitn(2, char::is_whitespace);
    let key = non_empty(parts.next())?;
    let title = parts
        .next()
        .map(strip_worker_markers)
        .and_then(|title| non_empty(Some(title)))
        .unwrap_or(key);
    let fingerprint = source_fingerprint_token(skipped);
    let output_path = worker_token_value(skipped, "output_path=");
    let item = FakeZoteroItem {
        key: key.to_string(),
        title: title.to_string(),
        attachment_path: Some(zotero_source_ref(key, fingerprint.as_deref(), output_path)),
        has_text_layer: true,
        dirty_text_layer: false,
        scanned: false,
        already_converted: true,
        prefer_mineru: false,
    };
    Some(BookPipelineSource {
        kind: "zotero_attachment".into(),
        title: Some(title.to_string()),
        path: None,
        selector: Some(key.to_string()),
        runner_behavior: None,
        translation_strategy: None,
        adapter_command: None,
        fake_zotero_items: Some(vec![item]),
        route_overrides: BTreeMap::new(),
    })
}

fn source_fingerprint_token(text: &str) -> Option<String> {
    worker_token_value(text, "source_md5=")
        .or_else(|| worker_token_value(text, "source_pdf_md5="))
        .map(str::to_string)
}

fn worker_token_value<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.split_whitespace()
        .find_map(|token| token.strip_prefix(prefix))
        .and_then(|value| non_empty(Some(value.trim_matches(|ch| matches!(ch, ',' | ';')))))
}

fn strip_worker_markers(value: &str) -> &str {
    let marker_index = [
        " source_md5=",
        " source_pdf_md5=",
        " output_path=",
        " zotero_attachment_key=",
    ]
    .iter()
    .filter_map(|marker| value.find(marker))
    .min()
    .unwrap_or(value.len());
    value[..marker_index].trim()
}

fn zotero_source_ref(key: &str, fingerprint: Option<&str>, output_path: Option<&str>) -> String {
    let base = output_path.filter(|path| !path.is_empty()).unwrap_or("");
    let base = if base.is_empty() {
        format!("zotero://attachment/{key}")
    } else {
        base.to_string()
    };
    match fingerprint {
        Some(fingerprint) => format!("{base}#source_md5={fingerprint}"),
        None => base,
    }
}

fn plan_token_value<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.split_whitespace()
        .find_map(|token| token.strip_prefix(prefix))
        .and_then(|value| non_empty(Some(value)))
}

fn fake_zotero_sources(
    requested_source: &BookPipelineSource,
    items: &[FakeZoteroItem],
) -> Vec<BookPipelineSource> {
    if requested_source.kind == "zotero_collection" || requested_source.kind == "zotero_filter" {
        return vec![BookPipelineSource {
            kind: requested_source.kind.clone(),
            title: requested_source
                .title
                .clone()
                .or_else(|| requested_source.selector.clone()),
            path: None,
            selector: requested_source.selector.clone(),
            runner_behavior: None,
            translation_strategy: requested_source.translation_strategy.clone(),
            adapter_command: requested_source.adapter_command.clone(),
            fake_zotero_items: Some(items.to_vec()),
            route_overrides: BTreeMap::new(),
        }];
    }
    items
        .iter()
        .cloned()
        .map(|item| BookPipelineSource {
            kind: "zotero_attachment".into(),
            title: Some(item.title.clone()),
            path: item.attachment_path.clone(),
            selector: Some(item.key.clone()),
            runner_behavior: None,
            translation_strategy: requested_source.translation_strategy.clone(),
            adapter_command: requested_source.adapter_command.clone(),
            fake_zotero_items: Some(vec![item]),
            route_overrides: BTreeMap::new(),
        })
        .collect()
}

fn zotero_selection_source(source: &BookPipelineSource) -> Option<BookPipelineSource> {
    if !matches!(
        source.kind.as_str(),
        "zotero_attachment" | "zotero_collection" | "zotero_filter"
    ) {
        return None;
    }
    Some(BookPipelineSource {
        kind: source.kind.clone(),
        title: source.title.clone().or_else(|| source.selector.clone()),
        path: None,
        selector: source.selector.clone(),
        runner_behavior: None,
        translation_strategy: source.translation_strategy.clone(),
        adapter_command: source.adapter_command.clone(),
        fake_zotero_items: source.fake_zotero_items.clone(),
        route_overrides: BTreeMap::new(),
    })
}

fn parent_item_type_filter(selector: Option<&str>) -> Option<String> {
    let selector = non_empty(selector)?;
    for prefix in [
        "parent_item_type=",
        "parent-item-type=",
        "parent_item_type:",
        "parent-item-type:",
        "itemType=",
        "itemType:",
    ] {
        if let Some(value) = selector.strip_prefix(prefix) {
            return non_empty(Some(value)).map(str::to_string);
        }
    }
    None
}

/// A title/creator/year search entered through the Zotero search box (mode
/// zotero_filter). Recognises the same query= / q= / title= prefixes the
/// selector already uses for itemType=, so the free-text field stays one
/// mechanism rather than growing a second syntax.
fn zotero_query_filter(selector: Option<&str>) -> Option<String> {
    let selector = non_empty(selector)?;
    for prefix in ["query=", "query:", "q=", "q:", "title=", "title:"] {
        if let Some(value) = selector.strip_prefix(prefix) {
            return non_empty(Some(value)).map(str::to_string);
        }
    }
    None
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Route kinds that represent a conversion decision the user is allowed to
/// override. Anything else (an unsupported source, a missing file, the synthetic
/// translation handoff row) is structural and must stay as routed.
/// The tokens the wizard's preflight dropdown already offers. The in-place
/// re-route accepts exactly the same set, plus `auto` to drop an override.
const ROUTE_OVERRIDE_TOKENS: [&str; 5] = ["auto", "direct", "paddle", "mineru", "keep"];

const OVERRIDABLE_ROUTE_KINDS: [&str; 6] = [
    "direct_text",
    "remote_paddleocr",
    "mineru",
    "already_converted",
    "blocked_dirty_text_layer",
    "missing_credentials",
];

/// Apply the wizard's per-item conversion overrides to a freshly computed route.
///
/// An override never grants capability the machine does not have: forcing an OCR
/// engine without its credentials leaves the item blocked exactly as auto-routing
/// would have. This runs for durable Zotero jobs too, where the route is
/// recomputed from live worker evidence at run time — without re-applying here
/// the user's choice would be displayed and then silently ignored.
/// `credentials` is `Some` when the caller can still check whether an OCR engine
/// is configured (preview and queue time) and `None` at run time, where the
/// wizard already rejected credential-less overrides and the stored route is
/// merely being re-derived.
fn apply_route_overrides(
    route: &mut [BookPipelineRouteItem],
    overrides: &BTreeMap<String, String>,
    credentials: Option<&BookPipelinePreviewConfig>,
) {
    if overrides.is_empty() {
        return;
    }
    for item in route.iter_mut() {
        let Some(token) = overrides.get(&item.id).map(String::as_str) else {
            continue;
        };
        if token.is_empty() || token == "auto" {
            continue;
        }
        if !OVERRIDABLE_ROUTE_KINDS.contains(&item.route_kind.as_str()) {
            continue;
        }
        let (route_kind, has_credentials) = match token {
            "direct" => ("direct_text", None),
            "paddle" => (
                "remote_paddleocr",
                credentials.map(|config| config.has_paddleocr_credentials),
            ),
            "mineru" => (
                "mineru",
                credentials.map(|config| config.has_mineru_credentials),
            ),
            "keep" => ("already_converted", None),
            _ => continue,
        };
        item.route_override = Some(token.to_string());
        if has_credentials == Some(false) {
            item.route_kind = "missing_credentials".into();
            item.can_run = false;
            item.blocked_reason = Some(format!(
                "Forced {token} conversion needs its credentials, which are not configured."
            ));
            item.summary = format!(
                "Override requested {token}, but its credentials are missing, so this item stays held."
            );
            continue;
        }
        item.route_kind = route_kind.into();
        item.can_run = route_kind != "already_converted";
        item.blocked_reason = None;
        item.summary = match token {
            "keep" => {
                "Override: keep the existing conversion result and skip re-converting.".into()
            }
            _ => format!("Override: forced {route_kind} instead of the automatic route."),
        };
    }
}

fn preview_route(
    source: &BookPipelineSource,
    mode: &str,
    config: BookPipelinePreviewConfig,
) -> Vec<BookPipelineRouteItem> {
    let override_config = config.clone();
    let mut route = match source.kind.as_str() {
        "fake" => vec![BookPipelineRouteItem {
            id: "fake-source".into(),
            title: source_title(source),
            source_kind: "fake".into(),
            source_ref: source
                .selector
                .clone()
                .unwrap_or_else(|| "fake://source".into()),
            route_kind: "direct_text".into(),
            can_run: true,
            blocked_reason: None,
            summary: format!("Fake source will run in {mode} mode through the fake CLI runner."),
            route_override: None,
        }],
        "markdown_source" => preview_markdown_source(source),
        "external_adapter" => preview_external_adapter(source),
        "local_pdf_folder" => preview_local_pdf_folder(source),
        "zotero_attachment" | "zotero_collection" | "zotero_filter" => {
            preview_zotero_source(source, config)
        }
        _ => vec![BookPipelineRouteItem {
            id: "unsupported-source".into(),
            title: source_title(source),
            source_kind: source.kind.clone(),
            source_ref: source.selector.clone().unwrap_or_default(),
            route_kind: "blocked".into(),
            can_run: false,
            blocked_reason: Some("Unsupported source type.".into()),
            summary: "This source type is not implemented yet.".into(),
            route_override: None,
        }],
    };
    // Overrides come either fresh from the wizard (config) or from the source
    // they were persisted on when the job was queued. Apply before the handoff
    // row is pushed: an override that holds every item must also withdraw the
    // handoff, which route_is_runnable_for_source below recomputes.
    let overrides = if override_config.route_overrides.is_empty() {
        &source.route_overrides
    } else {
        &override_config.route_overrides
    };
    apply_route_overrides(&mut route, overrides, Some(&override_config));
    if should_handoff_after_run(mode) && route_is_runnable_for_source(source, &route) {
        route.push(translation_handoff_route_item(source));
    }
    route
}

fn route_is_runnable_for_source(
    source: &BookPipelineSource,
    route: &[BookPipelineRouteItem],
) -> bool {
    if route.is_empty() {
        return false;
    }
    if is_zotero_batch_source(source) {
        return route
            .iter()
            .any(|item| item.route_kind != "translation_handoff" && item.can_run);
    }
    if is_zotero_source(source) {
        return route.iter().all(|item| item.can_run);
    }
    route.iter().any(|item| item.can_run)
}

fn runnable_zotero_route(job: &BookPipelineJob) -> Result<BookPipelineRouteItem, String> {
    if !is_zotero_source(&job.source) {
        return Err("Job is not a Zotero source.".into());
    }
    if !route_is_runnable_for_source(&job.source, &job.route) {
        return Err("Zotero job is blocked by route preview.".into());
    }
    job.route
        .iter()
        .find(|item| item.can_run)
        .cloned()
        .ok_or_else(|| "Zotero job has no runnable route.".to_string())
}

fn translation_handoff_route_item(source: &BookPipelineSource) -> BookPipelineRouteItem {
    BookPipelineRouteItem {
        id: "translation-handoff".into(),
        title: "Local reading project handoff".into(),
        source_kind: source.kind.clone(),
        source_ref: "books/local/zh-Hans".into(),
        route_kind: "translation_handoff".into(),
        can_run: true,
        blocked_reason: None,
        summary: "Cleaned Markdown will be copied into a local reading project after conversion."
            .into(),
        route_override: None,
    }
}

fn preview_markdown_source(source: &BookPipelineSource) -> Vec<BookPipelineRouteItem> {
    let title = source_title(source);
    let source_ref = source.path.clone().unwrap_or_default();
    let path = source.path.as_deref().map(PathBuf::from);
    let (can_run, blocked_reason, summary) = match path.as_deref() {
        Some(path) if !path.exists() => (
            false,
            Some("Selected Markdown source does not exist.".into()),
            "Choose an existing Markdown or source-text file before translation.".into(),
        ),
        Some(path) if !path.is_file() => (
            false,
            Some("Selected Markdown source is not a file.".into()),
            "Translate-only expects a single source Markdown or text file.".into(),
        ),
        Some(path) if !is_markdown_source_file(path) => (
            false,
            Some("Selected source is not Markdown or plain text.".into()),
            "Accepted translate-only inputs are .md, .markdown, and .txt files.".into(),
        ),
        Some(path) if is_runtime_staging_path(path) => (
            false,
            Some("OCR runtime staging paths are rejected by default.".into()),
            "Choose the cleaned Markdown artifact or final source text, not a temporary OCR staging file.".into(),
        ),
        Some(_) => (
            true,
            None,
            "Selected source text will be copied into a local reading project without OCR.".into(),
        ),
        None => (
            false,
            Some("Choose a Markdown or source-text file before translation.".into()),
            "Translate-only needs an explicit local source file.".into(),
        ),
    };
    vec![BookPipelineRouteItem {
        id: "markdown-source".into(),
        title,
        source_kind: "markdown_source".into(),
        source_ref,
        route_kind: "translation_ready".into(),
        can_run,
        blocked_reason,
        summary,
        route_override: None,
    }]
}

fn preview_external_adapter(source: &BookPipelineSource) -> Vec<BookPipelineRouteItem> {
    let command = non_empty(source.adapter_command.as_deref())
        .or_else(|| non_empty(source.path.as_deref()))
        .unwrap_or("");
    let path = Path::new(command);
    let (can_run, blocked_reason, summary) = if command.is_empty() {
        (
            false,
            Some("External adapter command path is missing.".into()),
            "Register a local adapter command before running this route.".into(),
        )
    } else if !path.is_file() {
        (
            false,
            Some("External adapter command was not found.".into()),
            "The adapter route is available once the configured command exists on disk.".into(),
        )
    } else {
        (
            true,
            None,
            "External adapter will run with --input and --output-dir, then Book Pipeline will normalize artifacts.".into(),
        )
    };
    vec![BookPipelineRouteItem {
        id: "external-adapter".into(),
        title: source_title(source),
        source_kind: "external_adapter".into(),
        source_ref: command.into(),
        route_kind: "external_adapter".into(),
        can_run,
        blocked_reason,
        summary,
        route_override: None,
    }]
}

fn preview_local_pdf_folder(source: &BookPipelineSource) -> Vec<BookPipelineRouteItem> {
    let folder = source.path.as_deref().unwrap_or("");
    let pdfs = pdf_files(folder);
    if pdfs.is_empty() {
        return vec![BookPipelineRouteItem {
            id: "local-pdf-folder".into(),
            title: source_title(source),
            source_kind: "local_pdf_folder".into(),
            source_ref: folder.into(),
            route_kind: "remote_paddleocr".into(),
            can_run: !folder.is_empty(),
            blocked_reason: folder
                .is_empty()
                .then(|| "Choose a folder before running conversion.".into()),
            summary:
                "The existing local PDF conversion wrapper will decide the PDF extraction details."
                    .into(),
            route_override: None,
        }];
    }

    pdfs.into_iter()
        .enumerate()
        .map(|(index, path)| BookPipelineRouteItem {
            id: format!("local-pdf-{}", index + 1),
            title: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("PDF")
                .to_string(),
            source_kind: "local_pdf_folder".into(),
            source_ref: display_path(&path),
            route_kind: "remote_paddleocr".into(),
            can_run: true,
            blocked_reason: None,
            summary: "Run through scripts/pdf_to_html_paddleocr.py from packages/ocr.".into(),
            route_override: None,
        })
        .collect()
}

fn preview_zotero_source(
    source: &BookPipelineSource,
    config: BookPipelinePreviewConfig,
) -> Vec<BookPipelineRouteItem> {
    // A Zotero route can only be previewed from discovery evidence; without it
    // the item stays blocked. Never fabricate placeholder attachments here —
    // they would be queued as real children and fail against the live worker.
    let Some(items) = &source.fake_zotero_items else {
        return vec![zotero_undiscovered_route_item(source)];
    };
    items
        .iter()
        .cloned()
        .map(|item| preview_zotero_item(source, item, &config))
        .collect()
}

fn zotero_undiscovered_route_item(source: &BookPipelineSource) -> BookPipelineRouteItem {
    BookPipelineRouteItem {
        id: "zotero-no-attachments".into(),
        title: source_title(source),
        source_kind: source.kind.clone(),
        source_ref: source.selector.clone().unwrap_or_default(),
        // Not the bare "blocked" kind: the frontend labels route kinds by exact
        // match and only falls back to the raw wire string, so an empty
        // discovery would show untranslated English. routeTone still tints it
        // as a block because it matches on the "blocked" prefix.
        route_kind: "blocked_no_attachment".into(),
        can_run: false,
        blocked_reason: Some("No matching Zotero attachment was discovered for this source.".into()),
        summary: "Adjust the search or filter, or select a specific attachment from bibliographic discovery.".into(),
        route_override: None,
    }
}

fn is_zotero_source(source: &BookPipelineSource) -> bool {
    matches!(
        source.kind.as_str(),
        "zotero_attachment" | "zotero_collection" | "zotero_filter"
    )
}

fn is_zotero_batch_source(source: &BookPipelineSource) -> bool {
    matches!(source.kind.as_str(), "zotero_collection" | "zotero_filter")
}

fn preview_zotero_item(
    source: &BookPipelineSource,
    item: FakeZoteroItem,
    config: &BookPipelinePreviewConfig,
) -> BookPipelineRouteItem {
    let (route_kind, can_run, blocked_reason, summary) = if item.already_converted {
        (
            "already_converted",
            false,
            Some("Converted Markdown already exists for this attachment.".into()),
            "Already converted; no full conversion will start from preview.".into(),
        )
    } else if item.dirty_text_layer {
        (
            "blocked_dirty_text_layer",
            false,
            Some("Dirty embedded text layer detected; route requires manual MinerU review.".into()),
            "Blocked to avoid silently converting degraded Chinese text.".into(),
        )
    } else if item.prefer_mineru {
        if config.has_mineru_credentials {
            (
                "mineru",
                true,
                None,
                "Route preview selects MinerU for this layout-sensitive item.".into(),
            )
        } else {
            (
                "missing_credentials",
                false,
                Some("MinerU credentials are missing.".into()),
                "MinerU candidate is blocked until credentials are configured.".into(),
            )
        }
    } else if item.has_text_layer && !item.scanned {
        (
            "direct_text",
            true,
            None,
            "Direct embedded text extraction can run without remote OCR credentials.".into(),
        )
    } else if config.has_paddleocr_credentials {
        (
            "remote_paddleocr",
            true,
            None,
            "Scanned or low-text PDF will use the existing remote PaddleOCR workflow.".into(),
        )
    } else {
        (
            "missing_credentials",
            false,
            Some("Remote PaddleOCR credentials are missing.".into()),
            "Scanned or low-text PDF is blocked until OCR credentials are configured.".into(),
        )
    };

    BookPipelineRouteItem {
        id: item.key.clone(),
        title: item.title,
        source_kind: source.kind.clone(),
        source_ref: item
            .attachment_path
            .clone()
            .unwrap_or_else(|| format!("zotero://{}", item.key)),
        route_kind: route_kind.into(),
        can_run,
        blocked_reason,
        summary,
        route_override: None,
    }
}

fn scan_artifacts(output_dir: &Path) -> Result<Vec<BookPipelineArtifact>, String> {
    let mut artifacts = Vec::new();
    collect_artifacts(output_dir, &mut artifacts)?;
    Ok(artifacts)
}

fn collect_artifacts(dir: &Path, artifacts: &mut Vec<BookPipelineArtifact>) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_artifacts(&path, artifacts)?;
            continue;
        }
        let Some(kind) = artifact_kind(&path) else {
            continue;
        };
        artifacts.push(BookPipelineArtifact {
            kind: kind.into(),
            path: display_path(&path),
            sha256: Some(sha256_file(&path)?),
            zotero_key: None,
            producer_stage: Some("extract".into()),
            ..BookPipelineArtifact::default()
        });
    }
    Ok(())
}

fn create_translation_handoff_project(
    job: &BookPipelineJob,
    artifact_path: Option<&str>,
    repo_root: &Path,
) -> Result<TranslationHandoffOutput, String> {
    create_translation_handoff_project_with_title(
        job,
        artifact_path,
        repo_root,
        job.source.title.as_deref(),
    )
}

fn create_translation_handoff_project_with_title(
    job: &BookPipelineJob,
    artifact_path: Option<&str>,
    repo_root: &Path,
    project_title: Option<&str>,
) -> Result<TranslationHandoffOutput, String> {
    let markdown = selected_markdown_artifact(job, artifact_path)?;
    let markdown_path = PathBuf::from(&markdown.path);
    if !markdown_path.is_file() {
        return Err(format!(
            "Markdown artifact is not available at {}",
            display_path(&markdown_path)
        ));
    }
    let target_dir = repo_root.join("books").join("local").join("zh-Hans");
    let project_slug = clean_project_slug(
        project_title
            .or_else(|| markdown_path.file_stem().and_then(|name| name.to_str()))
            .unwrap_or("book_pipeline_handoff"),
    );
    let project_root = target_dir.join(format!(
        "{:03}_{}",
        next_local_project_number(&target_dir)?,
        project_slug
    ));
    create_local_project_contract(&project_root)?;
    let original = project_root.join("source").join("original.md");
    let source = project_root.join("source").join("source.md");
    fs::copy(&markdown_path, &original).map_err(|err| err.to_string())?;
    fs::copy(&markdown_path, &source).map_err(|err| err.to_string())?;
    let source_sha256 = sha256_file(&markdown_path)?;
    write_source_manifest(
        &project_root,
        &markdown_path,
        &source_sha256,
        "cleaned_markdown_ready",
    )?;
    let manifest = project_root.join("metadata").join("source_manifest.json");
    let mut artifacts = vec![
        BookPipelineArtifact {
            kind: "translation_source".into(),
            path: display_path(&source),
            sha256: Some(sha256_file(&source)?),
            zotero_key: markdown.zotero_key.clone(),
            producer_stage: Some("handoff".into()),
            ..BookPipelineArtifact::default()
        },
        BookPipelineArtifact {
            kind: "source_manifest".into(),
            path: display_path(&manifest),
            sha256: Some(sha256_file(&manifest)?),
            zotero_key: markdown.zotero_key.clone(),
            producer_stage: Some("handoff".into()),
            ..BookPipelineArtifact::default()
        },
    ];
    let mut log_summary = vec![format!(
        "Translation handoff ready at {}",
        display_path(&project_root)
    )];
    if uses_reflection_translation_strategy(job) {
        let reflection_artifacts = write_reflection_translation_artifacts(&project_root)?;
        artifacts.extend(reflection_artifacts);
        log_summary.push(
            "Reflection translation strategy recorded: draft, reflection, and revised phases are ready."
                .into(),
        );
    }
    Ok(TranslationHandoffOutput {
        log_summary,
        artifacts,
    })
}

fn uses_reflection_translation_strategy(job: &BookPipelineJob) -> bool {
    job.source
        .translation_strategy
        .as_deref()
        .map(|strategy| {
            matches!(
                strategy.trim().to_ascii_lowercase().as_str(),
                "reflection" | "reflection_translation" | "reflect"
            )
        })
        .unwrap_or(false)
}

fn write_reflection_translation_artifacts(
    project_root: &Path,
) -> Result<Vec<BookPipelineArtifact>, String> {
    let draft = project_root
        .join("chapters")
        .join("translated")
        .join("000_reflection_draft.md");
    let reflection = project_root.join("qa").join("reflection_strategy.md");
    let revised = project_root
        .join("chapters")
        .join("final")
        .join("000_reflection_revised.md");
    fs::write(
        &draft,
        "# Translation Draft\n\nReflection strategy phase: draft translation pending.\n",
    )
    .map_err(|err| err.to_string())?;
    fs::write(
        &reflection,
        "# Reflection Notes\n\nReview terminology, omissions, register, and paragraph-level fidelity before revision.\n",
    )
    .map_err(|err| err.to_string())?;
    fs::write(
        &revised,
        "# Revised Translation\n\nReflection strategy phase: revised translation will be produced here.\n",
    )
    .map_err(|err| err.to_string())?;
    Ok(vec![
        BookPipelineArtifact {
            kind: "translation_draft".into(),
            path: display_path(&draft),
            sha256: Some(sha256_file(&draft)?),
            zotero_key: None,
            producer_stage: Some("handoff".into()),
            ..BookPipelineArtifact::default()
        },
        BookPipelineArtifact {
            kind: "translation_reflection".into(),
            path: display_path(&reflection),
            sha256: Some(sha256_file(&reflection)?),
            zotero_key: None,
            producer_stage: Some("handoff".into()),
            ..BookPipelineArtifact::default()
        },
        BookPipelineArtifact {
            kind: "translation_revised".into(),
            path: display_path(&revised),
            sha256: Some(sha256_file(&revised)?),
            zotero_key: None,
            producer_stage: Some("handoff".into()),
            ..BookPipelineArtifact::default()
        },
    ])
}

fn selected_markdown_artifact<'a>(
    job: &'a BookPipelineJob,
    artifact_path: Option<&str>,
) -> Result<&'a BookPipelineArtifact, String> {
    if let Some(path) = artifact_path {
        return job
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown" && artifact.path == path)
            .ok_or_else(|| "Selected Markdown artifact was not found on this job.".to_string());
    }
    job.artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .ok_or_else(|| "This job has no cleaned Markdown artifact to hand off.".to_string())
}

fn create_local_project_contract(project_root: &Path) -> Result<(), String> {
    for relative in [
        "source",
        "chapters/src",
        "chapters/translated",
        "chapters/final",
        "glossary",
        "metadata",
        "qa/chapter_controls",
        "notes",
        "output/reading",
    ] {
        fs::create_dir_all(project_root.join(relative)).map_err(|err| err.to_string())?;
    }
    fs::write(
        project_root.join("glossary").join("terms.csv"),
        "source,translation,category,note\n",
    )
    .map_err(|err| err.to_string())?;
    fs::write(
        project_root.join("metadata").join("style_profile.md"),
        "# Style Profile\n\n- Source language: auto\n- Target language: zh-Hans\n- Project type: book\n",
    )
    .map_err(|err| err.to_string())?;
    fs::write(
        project_root.join("qa").join("status.md"),
        "# QA Status\n\n- extraction: cleaned_markdown_ready\n- split: pending\n- translation: pending\n- reading output: pending\n",
    )
    .map_err(|err| err.to_string())?;
    fs::write(
        project_root.join("AGENTS.md"),
        "# Book Project Instructions\n\n- Use `skills/local-book-reading-pipeline/SKILL.md` from the repository root.\n- Put source chapters in `chapters/src/`, drafts in `chapters/translated/`, final text in `chapters/final/`.\n",
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn write_source_manifest(
    project_root: &Path,
    markdown_path: &Path,
    sha256: &str,
    extraction_status: &str,
) -> Result<(), String> {
    let manifest = serde_json::json!({
        "schema": "local-reading-source-manifest-v1",
        "project_type": "book",
        "source_file_name": markdown_path.file_name().and_then(|name| name.to_str()).unwrap_or("source.md"),
        "stored_source_path": "source/original.md",
        "source_sha256": sha256,
        "source_format": "md",
        "source_language": "auto",
        "target_language": "zh-Hans",
        "extraction_status": extraction_status,
        "notes": "Created by Book Pipeline translation handoff from a cleaned Markdown artifact.",
    });
    fs::write(
        project_root.join("metadata").join("source_manifest.json"),
        serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())? + "\n",
    )
    .map_err(|err| err.to_string())
}

fn artifact_kind(path: &Path) -> Option<&'static str> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("md") | Some("markdown") => Some("markdown"),
        Some("html") | Some("htm") => Some("html"),
        Some("epub") => Some("epub"),
        Some("json") | Some("jsonl") => Some("metadata"),
        Some("idx") | Some("index") => Some("index"),
        _ if is_index_artifact(path) => Some("index"),
        _ => None,
    }
}

fn is_index_artifact(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let file_name = file_name.to_ascii_lowercase();
    file_name == "index"
        || file_name.ends_with(".index")
        || file_name.contains("_index.")
        || file_name.contains("-index.")
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn next_local_project_number(target_dir: &Path) -> Result<u32, String> {
    let mut highest = 0;
    let entries = match fs::read_dir(target_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(1),
    };
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        if !entry.path().is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some((prefix, _)) = name.split_once('_') else {
            continue;
        };
        if let Ok(number) = prefix.parse::<u32>() {
            highest = highest.max(number);
        }
    }
    Ok(highest + 1)
}

fn clean_project_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_separator = false;
    for ch in value.chars() {
        let replacement = if ch.is_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            ch
        } else {
            '_'
        };
        if replacement == '_' {
            if previous_separator {
                continue;
            }
            previous_separator = true;
        } else {
            previous_separator = false;
        }
        slug.push(replacement);
        if slug.len() >= 80 {
            break;
        }
    }
    let slug = slug.trim_matches(|ch| matches!(ch, ' ' | '.' | '_' | '-'));
    if slug.is_empty() {
        "book_pipeline_handoff".into()
    } else {
        slug.into()
    }
}

fn clean_path_component(value: &str) -> String {
    clean_project_slug(value)
}

fn pdf_files(folder: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut pdfs: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        })
        .collect();
    pdfs.sort();
    pdfs
}

fn markdown_source_path(source: &BookPipelineSource) -> Result<PathBuf, String> {
    let path = source
        .path
        .as_deref()
        .ok_or_else(|| "Markdown source is missing a path.".to_string())?;
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err(format!(
            "Markdown source is not available at {}",
            display_path(&path)
        ));
    }
    if !is_markdown_source_file(&path) {
        return Err("Markdown source must be a .md, .markdown, or .txt file.".into());
    }
    Ok(path)
}

fn ensure_markdown_source_allowed(path: &Path) -> Result<(), String> {
    if is_runtime_staging_path(path) {
        return Err("OCR runtime staging paths are rejected by default.".into());
    }
    Ok(())
}

fn is_markdown_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "txt"
            )
        })
        .unwrap_or(false)
}

fn is_runtime_staging_path(path: &Path) -> bool {
    let normalized = display_path(path).replace('\\', "/").to_ascii_lowercase();
    normalized.contains("/.state/staging/") || normalized.contains("/ocr_output/.state/")
}

fn default_state_dir() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir()
        .or_else(dirs::config_local_dir)
        .ok_or_else(|| "Could not locate local app data directory.".to_string())?;
    Ok(base
        .join("BiblioSmith")
        .join("launcher")
        .join("book-pipeline"))
}

fn default_output_root() -> Result<PathBuf, String> {
    let ocr_root = book_ocr_conversion_root();
    if ocr_root.is_dir() {
        return Ok(ocr_root.join("output").join("book-pipeline"));
    }
    Ok(default_state_dir()?.join("output"))
}

fn book_ocr_conversion_root() -> PathBuf {
    let fallback_root = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Projects")
        .join("book-ocr-conversion");

    match local_reading_repo_root() {
        Ok(repo_root) => {
            let monorepo_root = repo_root.join("packages").join("ocr");
            if monorepo_root.is_dir() {
                return monorepo_root;
            }
            eprintln!(
                "Warning: monorepo OCR package not found at {}; falling back to legacy OCR root at {}.",
                display_path(&monorepo_root),
                display_path(&fallback_root)
            );
        }
        Err(error) => eprintln!(
            "Warning: could not locate the local reading repository ({error}); falling back to legacy OCR root at {}.",
            display_path(&fallback_root)
        ),
    }

    fallback_root
}

fn local_reading_repo_root() -> Result<PathBuf, String> {
    // The installed app's repo root is a runtime choice (configured repoRoot,
    // then BIBLIOSMITH_HOME), never the machine that happened to compile the
    // binary. CARGO_MANIFEST_DIR is a build-time constant baked into the
    // executable, so it only points at the right tree by coincidence (e.g. a
    // self-hosted CI runner's own checkout sitting on the same disk); it stays
    // here purely as the dev/test fallback when no runtime config exists yet.
    if let Some(repo_root) = crate::configured_repo_root() {
        return existing_repo_root(repo_root);
    }
    if let Some(repo_root) = crate::bibliosmith_home_repo_root() {
        return existing_repo_root(repo_root);
    }

    let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    start
        .ancestors()
        .find(|path| {
            path.join("AGENTS.md").is_file()
                && path
                    .join("tools")
                    .join("create_local_book_project.py")
                    .is_file()
        })
        .map(Path::to_path_buf)
        .ok_or_else(|| "Could not locate bibliosmith repo root.".to_string())
}

/// The repo root is the cwd of nearly every runner command, and the default one
/// (`~/BiblioSmith`) is a guess that need not exist. Left unchecked the spawn
/// itself fails and the user reads an errno about a directory they never chose,
/// so the missing root is named here together with the settings that fix it.
fn existing_repo_root(repo_root: PathBuf) -> Result<PathBuf, String> {
    if repo_root.is_dir() {
        return Ok(repo_root);
    }
    Err(format!(
        "BiblioSmith 仓库目录不存在：{}。请在设置里选择本地 bibliosmith 仓库目录，或把 BIBLIOSMITH_HOME 指向已有的仓库。",
        display_path(&repo_root)
    ))
}

fn source_title(source: &BookPipelineSource) -> String {
    source
        .title
        .clone()
        .or_else(|| {
            source
                .path
                .as_ref()
                .and_then(|path| file_name_label(Path::new(path)))
        })
        .or_else(|| source.selector.clone())
        .unwrap_or_else(|| "Book Pipeline source".into())
}

fn file_name_label(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn now_label() -> String {
    Local::now().to_rfc3339()
}

/// A timestamp `seconds` from now, in the same format as `now_label`, so the UI
/// can render a due time with the parser it already uses.
fn offset_label(seconds: u32) -> String {
    (Local::now() + chrono::Duration::seconds(seconds.into())).to_rfc3339()
}

fn new_job_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("job-{nanos}")
}

fn new_approval_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("approval-{nanos}")
}

fn trim_log_summary(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .rev()
        .take(20)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn redact_runner_message(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if message_carries_a_leaked_value(message) {
        return "Sensitive credential or signed-request details were redacted.".into();
    }
    if lower.contains("prompt")
        || lower.contains("provider payload")
        || lower.contains("provider request")
        || lower.contains("provider response")
        || lower.contains("source text")
        || lower.contains("translation text")
        || lower.contains("ocr text")
        || lower.contains("model response")
    {
        return "Private source, translation, prompt, or provider payload was redacted.".into();
    }
    redact_message_tail(message)
}

/// The safe remainder of message redaction once the caller has already ruled
/// out a leaked credential and a private payload: hide signed/private URLs,
/// then cap length. Shared so `safe_error_from_message` (which uses its own
/// narrower credential gate, since a final error summary naming a missing key
/// should stay legible) doesn't fall through into `redact_runner_message`'s
/// broader keyword check and get re-redacted.
fn redact_message_tail(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if (lower.contains("https://") || lower.contains("http://")) && lower.contains('?') {
        return "A private or signed URL was redacted.".into();
    }
    let first_line = message.lines().next().unwrap_or_default().trim();
    first_line.chars().take(400).collect()
}

// ---- Staged-gates runner: deterministic split + prepare stages (issue #38) ----

struct SplitPlan {
    primary_heading_level: usize,
    chapters: Vec<SplitChapter>,
}

struct SplitChapter {
    ordinal: usize,
    id: String,
    title: String,
    start_line: usize,
    end_line: usize,
    text: String,
    blocks: Vec<SplitBlock>,
}

struct SplitBlock {
    id: String,
    start_line: usize,
    end_line: usize,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceMapDocument {
    schema: String,
    split_policy_version: String,
    source_markdown_sha256: String,
    source_path: String,
    primary_heading_level: usize,
    chapters: Vec<SourceMapChapter>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceMapChapter {
    id: String,
    ordinal: usize,
    title: String,
    source_start_line: usize,
    source_end_line: usize,
    chapter_source_path: String,
    chapter_source_sha256: String,
    blocks: Vec<SourceMapBlock>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceMapBlock {
    id: String,
    source_start_line: usize,
    source_end_line: usize,
    sha256: String,
}

struct StageRunOutput {
    artifacts: Vec<BookPipelineArtifact>,
    artifact_kinds: Vec<&'static str>,
    input_hashes: BTreeMap<String, String>,
    log_summary: Vec<String>,
    unit_summary: Option<BookPipelineUnitSummary>,
    error: Option<String>,
}

enum StageRunOutcome {
    Finished(StageRunOutput),
    WaitingForAgent {
        output: StageRunOutput,
        agent_profile_id: String,
    },
}

impl StageRunOutcome {
    fn finished(output: StageRunOutput) -> Self {
        Self::Finished(output)
    }
}

fn sha256_str(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Deterministically split cleaned Markdown into ordered chapters plus paragraph
/// blocks. The policy is: split at the shallowest ATX heading level present, keep
/// any leading preamble as a front-matter chapter, and treat a heading-free
/// document as a single chapter. Fenced code blocks are never scanned for
/// headings so their `#` lines cannot create phantom chapters.
fn split_source_markdown(text: &str) -> SplitPlan {
    let lines: Vec<&str> = text.lines().collect();
    let mut in_fence = false;
    let heading_levels: Vec<Option<usize>> = lines
        .iter()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
                return None;
            }
            if in_fence {
                return None;
            }
            atx_heading_level(line)
        })
        .collect();
    let primary = heading_levels.iter().flatten().min().copied();
    let chapters = match primary {
        None => {
            if lines.iter().all(|line| line.trim().is_empty()) {
                Vec::new()
            } else {
                vec![build_chapter(1, "Chapter 1", 1, lines.len(), &lines)]
            }
        }
        Some(level) => split_at_headings(&lines, &heading_levels, level),
    };
    SplitPlan {
        primary_heading_level: primary.unwrap_or(0),
        chapters,
    }
}

fn atx_heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let hashes = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if (1..=6).contains(&hashes) {
        let rest = &trimmed[hashes..];
        if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
            return Some(hashes);
        }
    }
    None
}

fn heading_title(line: &str) -> String {
    line.trim_start().trim_start_matches('#').trim().to_string()
}

fn split_at_headings(
    lines: &[&str],
    heading_levels: &[Option<usize>],
    primary: usize,
) -> Vec<SplitChapter> {
    let boundaries: Vec<usize> = heading_levels
        .iter()
        .enumerate()
        .filter_map(|(index, level)| (*level == Some(primary)).then_some(index))
        .collect();
    let mut chapters = Vec::new();
    let mut ordinal = 0;
    let first = boundaries.first().copied().unwrap_or(0);
    if first > 0 && lines[..first].iter().any(|line| !line.trim().is_empty()) {
        ordinal += 1;
        chapters.push(build_chapter(
            ordinal,
            "Front Matter",
            1,
            first,
            &lines[..first],
        ));
    }
    for (position, &start) in boundaries.iter().enumerate() {
        let end = boundaries.get(position + 1).copied().unwrap_or(lines.len());
        ordinal += 1;
        let title = heading_title(lines[start]);
        let title = if title.is_empty() {
            format!("Chapter {ordinal}")
        } else {
            title
        };
        chapters.push(build_chapter(
            ordinal,
            &title,
            start + 1,
            end,
            &lines[start..end],
        ));
    }
    chapters
}

fn build_chapter(
    ordinal: usize,
    title: &str,
    start_line: usize,
    end_line: usize,
    slice: &[&str],
) -> SplitChapter {
    let id = format!("chapter_{ordinal:03}");
    let text = if slice.is_empty() {
        String::new()
    } else {
        format!("{}\n", slice.join("\n"))
    };
    let blocks = paragraph_blocks(slice, start_line, &id);
    SplitChapter {
        ordinal,
        id,
        title: title.to_string(),
        start_line,
        end_line,
        text,
        blocks,
    }
}

fn paragraph_blocks(slice: &[&str], slice_start_line: usize, chapter_id: &str) -> Vec<SplitBlock> {
    let mut blocks = Vec::new();
    let mut index = 0;
    let mut ordinal = 0;
    while index < slice.len() {
        if slice[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let start = index;
        while index < slice.len() && !slice[index].trim().is_empty() {
            index += 1;
        }
        ordinal += 1;
        let block_text = format!("{}\n", slice[start..index].join("\n"));
        blocks.push(SplitBlock {
            id: format!("{chapter_id}_block_{ordinal:03}"),
            start_line: slice_start_line + start,
            end_line: slice_start_line + index - 1,
            sha256: sha256_str(&block_text),
        });
    }
    blocks
}

fn project_root_from_child(child: &BookPipelineChildJob) -> Result<PathBuf, String> {
    child
        .local_project_root
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| infer_project_root(&child.artifacts))
        .ok_or_else(|| "This child has no translation project to advance.".to_string())
}

fn replace_stage_artifacts(
    existing: &mut Vec<BookPipelineArtifact>,
    kinds: &[&str],
    mut incoming: Vec<BookPipelineArtifact>,
) {
    reuse_identical_artifact_records(existing, &mut incoming);
    existing.retain(|artifact| !kinds.contains(&artifact.kind.as_str()));
    existing.extend(incoming);
}

fn reuse_identical_artifact_records(
    existing: &[BookPipelineArtifact],
    incoming: &mut [BookPipelineArtifact],
) {
    for artifact in incoming {
        if let Some(current) = existing.iter().find(|current| {
            current.kind == artifact.kind
                && current.path == artifact.path
                && current.sha256 == artifact.sha256
        }) {
            *artifact = current.clone();
        }
    }
}

fn translation_artifact_unit_id(artifact: &BookPipelineArtifact) -> Option<String> {
    let file_name = Path::new(&artifact.path).file_name()?.to_str()?;
    match artifact.kind.as_str() {
        "chapter_translation" => file_name.strip_suffix(".md").map(str::to_string),
        "chapter_translation_degraded" => {
            file_name.strip_suffix(".degraded.md").map(str::to_string)
        }
        _ => None,
    }
}

fn replace_translation_stage_artifacts(
    existing: &mut Vec<BookPipelineArtifact>,
    mut incoming: Vec<BookPipelineArtifact>,
) {
    reuse_identical_artifact_records(existing, &mut incoming);
    let replaced_unit_ids = incoming
        .iter()
        .filter_map(translation_artifact_unit_id)
        .collect::<BTreeSet<_>>();
    existing.retain(|artifact| {
        if matches!(
            artifact.kind.as_str(),
            "translation_run_manifest" | "translation_handoff"
        ) {
            return false;
        }
        translation_artifact_unit_id(artifact)
            .map(|unit_id| !replaced_unit_ids.contains(&unit_id))
            .unwrap_or(true)
    });
    existing.extend(incoming);
}

fn replace_expert_qa_stage_artifacts(
    existing: &mut Vec<BookPipelineArtifact>,
    kinds: &[&str],
    incoming: Vec<BookPipelineArtifact>,
) {
    let mut qa_artifacts = Vec::new();
    for artifact in incoming {
        if artifact.kind == "chapter_translation" {
            if let Some(current) = existing
                .iter_mut()
                .find(|current| current.kind == artifact.kind && current.path == artifact.path)
            {
                if current.sha256 != artifact.sha256 {
                    *current = artifact;
                }
            } else {
                existing.push(artifact);
            }
        } else {
            qa_artifacts.push(artifact);
        }
    }
    replace_stage_artifacts(existing, kinds, qa_artifacts);
}

fn clear_generated_files(dir: &Path, prefix: &str, suffix: &str) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with(prefix) && name.ends_with(suffix) {
                fs::remove_file(entry.path()).map_err(|err| err.to_string())?;
            }
        }
    }
    Ok(())
}

/// Split stage (order 4): deterministically write `chapters/src/` and a source
/// map with chapter/block traceability, registering the `source_map` and
/// `chapter_source` artifacts with SHA-256. Idempotency key is the source
/// Markdown SHA-256 plus the split policy version.
fn run_split_stage(child: &BookPipelineChildJob) -> Result<StageRunOutput, String> {
    let project_root = project_root_from_child(child)?;
    let source_md = project_root.join("source").join("source.md");
    let source_text = fs::read_to_string(&source_md)
        .map_err(|_| "Source Markdown is missing; run the handoff stage first.".to_string())?;
    let source_sha256 = sha256_str(&source_text);
    let plan = split_source_markdown(&source_text);

    let src_dir = project_root.join("chapters").join("src");
    fs::create_dir_all(&src_dir).map_err(|err| err.to_string())?;
    clear_generated_files(&src_dir, "chapter_", ".md")?;

    let mut artifacts = Vec::new();
    let mut map_chapters = Vec::new();
    for chapter in &plan.chapters {
        let file_name = format!("{}.md", chapter.id);
        let chapter_path = src_dir.join(&file_name);
        fs::write(&chapter_path, &chapter.text).map_err(|err| err.to_string())?;
        let chapter_sha256 = sha256_str(&chapter.text);
        artifacts.push(BookPipelineArtifact {
            kind: "chapter_source".into(),
            path: display_path(&chapter_path),
            sha256: Some(chapter_sha256.clone()),
            zotero_key: None,
            producer_stage: Some("split".into()),
            ..BookPipelineArtifact::default()
        });
        map_chapters.push(SourceMapChapter {
            id: chapter.id.clone(),
            ordinal: chapter.ordinal,
            title: chapter.title.clone(),
            source_start_line: chapter.start_line,
            source_end_line: chapter.end_line,
            chapter_source_path: format!("chapters/src/{file_name}"),
            chapter_source_sha256: chapter_sha256,
            blocks: chapter
                .blocks
                .iter()
                .map(|block| SourceMapBlock {
                    id: block.id.clone(),
                    source_start_line: block.start_line,
                    source_end_line: block.end_line,
                    sha256: block.sha256.clone(),
                })
                .collect(),
        });
    }

    let document = SourceMapDocument {
        schema: SOURCE_MAP_SCHEMA.into(),
        split_policy_version: SPLIT_POLICY_VERSION.into(),
        source_markdown_sha256: source_sha256.clone(),
        source_path: "source/source.md".into(),
        primary_heading_level: plan.primary_heading_level,
        chapters: map_chapters,
    };
    let map_json = serde_json::to_string_pretty(&document).map_err(|err| err.to_string())? + "\n";
    let map_dir = project_root.join("metadata");
    fs::create_dir_all(&map_dir).map_err(|err| err.to_string())?;
    let map_path = map_dir.join("source_map.json");
    fs::write(&map_path, &map_json).map_err(|err| err.to_string())?;
    artifacts.insert(
        0,
        BookPipelineArtifact {
            kind: "source_map".into(),
            path: display_path(&map_path),
            sha256: Some(sha256_str(&map_json)),
            zotero_key: None,
            producer_stage: Some("split".into()),
            ..BookPipelineArtifact::default()
        },
    );

    let mut input_hashes = BTreeMap::new();
    input_hashes.insert("sourceMarkdownSha256".into(), source_sha256);
    input_hashes.insert("splitPolicyVersion".into(), SPLIT_POLICY_VERSION.into());
    Ok(StageRunOutput {
        artifacts,
        artifact_kinds: vec!["source_map", "chapter_source"],
        input_hashes,
        log_summary: vec![format!(
            "Split source into {} chapter(s)",
            plan.chapters.len()
        )],
        unit_summary: None,
        error: None,
    })
}

/// Prepare stage (order 5): seed glossary/style, per-chapter controls, and
/// provider-independent task manifests, registering the `glossary`,
/// `style_profile`, `chapter_control`, and `translation_task_manifest`
/// artifacts. Idempotency key is the source-map hash plus glossary/style hashes
/// plus the task policy version. No private source text enters these records.
fn run_prepare_stage(child: &BookPipelineChildJob) -> Result<StageRunOutput, String> {
    let project_root = project_root_from_child(child)?;
    let map_path = project_root.join("metadata").join("source_map.json");
    let map_json = fs::read_to_string(&map_path)
        .map_err(|_| "Source map is missing; run the split stage first.".to_string())?;
    let source_map_sha256 = sha256_str(&map_json);
    let document: SourceMapDocument =
        serde_json::from_str(&map_json).map_err(|err| err.to_string())?;

    let glossary_path = project_root.join("glossary").join("terms.csv");
    if !glossary_path.is_file() {
        fs::create_dir_all(glossary_path.parent().unwrap()).map_err(|err| err.to_string())?;
        fs::write(&glossary_path, "source,translation,category,note\n")
            .map_err(|err| err.to_string())?;
    }
    let style_path = project_root.join("metadata").join("style_profile.md");
    if !style_path.is_file() {
        fs::create_dir_all(style_path.parent().unwrap()).map_err(|err| err.to_string())?;
        fs::write(
            &style_path,
            "# Style Profile\n\n- Source language: auto\n- Target language: zh-Hans\n- Project type: book\n",
        )
        .map_err(|err| err.to_string())?;
    }
    let glossary_sha256 = sha256_file(&glossary_path)?;
    let style_sha256 = sha256_file(&style_path)?;

    let controls_dir = project_root.join("qa").join("chapter_controls");
    let tasks_dir = project_root.join("qa").join("tasks");
    fs::create_dir_all(&controls_dir).map_err(|err| err.to_string())?;
    fs::create_dir_all(&tasks_dir).map_err(|err| err.to_string())?;
    clear_generated_files(&controls_dir, "chapter_", ".json")?;
    clear_generated_files(&tasks_dir, "chapter_", ".json")?;

    let mut artifacts = vec![
        BookPipelineArtifact {
            kind: "glossary".into(),
            path: display_path(&glossary_path),
            sha256: Some(glossary_sha256.clone()),
            zotero_key: None,
            producer_stage: Some("prepare".into()),
            ..BookPipelineArtifact::default()
        },
        BookPipelineArtifact {
            kind: "style_profile".into(),
            path: display_path(&style_path),
            sha256: Some(style_sha256.clone()),
            zotero_key: None,
            producer_stage: Some("prepare".into()),
            ..BookPipelineArtifact::default()
        },
    ];
    for chapter in &document.chapters {
        let control = serde_json::json!({
            "schema": CHAPTER_CONTROL_SCHEMA,
            "chapterId": chapter.id,
            "targetLanguage": "zh-Hans",
            "status": "pending",
            "checks": {
                "targetOnlyReading": "pending",
                "sourceFidelity": "pending",
                "polysemyBackCheck": "pending",
                "proseRebuild": "pending",
                "closure": "pending"
            },
            "unresolvedPolysemy": 0
        });
        let control_json =
            serde_json::to_string_pretty(&control).map_err(|err| err.to_string())? + "\n";
        let control_path = controls_dir.join(format!("{}.json", chapter.id));
        fs::write(&control_path, &control_json).map_err(|err| err.to_string())?;
        artifacts.push(BookPipelineArtifact {
            kind: "chapter_control".into(),
            path: display_path(&control_path),
            sha256: Some(sha256_str(&control_json)),
            zotero_key: None,
            producer_stage: Some("prepare".into()),
            ..BookPipelineArtifact::default()
        });

        let task = serde_json::json!({
            "schema": TRANSLATION_TASK_SCHEMA,
            "taskPolicyVersion": TASK_POLICY_VERSION,
            "chapterId": chapter.id,
            "targetLanguage": "zh-Hans",
            "sourceChapterPath": chapter.chapter_source_path,
            "sourceChapterSha256": chapter.chapter_source_sha256,
            "sourceStartLine": chapter.source_start_line,
            "sourceEndLine": chapter.source_end_line,
            "glossaryPath": "glossary/terms.csv",
            "glossarySha256": glossary_sha256,
            "styleProfilePath": "metadata/style_profile.md",
            "styleProfileSha256": style_sha256,
            "options": {"digestMode": false}
        });
        let task_json = serde_json::to_string_pretty(&task).map_err(|err| err.to_string())? + "\n";
        let task_path = tasks_dir.join(format!("{}.json", chapter.id));
        fs::write(&task_path, &task_json).map_err(|err| err.to_string())?;
        artifacts.push(BookPipelineArtifact {
            kind: "translation_task_manifest".into(),
            path: display_path(&task_path),
            sha256: Some(sha256_str(&task_json)),
            zotero_key: None,
            producer_stage: Some("prepare".into()),
            ..BookPipelineArtifact::default()
        });
    }

    let mut input_hashes = BTreeMap::new();
    input_hashes.insert("sourceMapSha256".into(), source_map_sha256);
    input_hashes.insert("glossarySha256".into(), glossary_sha256);
    input_hashes.insert("styleProfileSha256".into(), style_sha256);
    input_hashes.insert("taskPolicyVersion".into(), TASK_POLICY_VERSION.into());
    Ok(StageRunOutput {
        artifacts,
        artifact_kinds: vec![
            "glossary",
            "style_profile",
            "chapter_control",
            "translation_task_manifest",
        ],
        input_hashes,
        log_summary: vec![format!(
            "Prepared {} translation task manifest(s)",
            document.chapters.len()
        )],
        unit_summary: None,
        error: None,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranslationEngineReport {
    schema: String,
    summary: TranslationEngineReportSummary,
    units: Vec<TranslationEngineUnitReport>,
}

#[derive(Deserialize)]
struct TranslationEngineReportSummary {
    total: u32,
    completed: u32,
    failed: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranslationEngineUnitReport {
    unit_id: String,
    status: String,
    #[serde(default)]
    artifact: Option<TranslationEngineArtifactReport>,
    // Terms the engine demanded of the model and did not find in the output. A
    // warning, never a failure: Chinese compounding makes false positives likely
    // enough that rejecting a chapter over one would cost more than it saves.
    // Optional so a report from an older engine still parses.
    #[serde(default)]
    glossary_violations: Vec<TranslationEngineGlossaryViolation>,
}

#[derive(Deserialize)]
struct TranslationEngineGlossaryViolation {
    source: String,
    translation: String,
}

#[derive(Deserialize)]
struct TranslationEngineArtifactReport {
    kind: String,
    path: String,
    sha256: String,
    complete: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookPipelineTranslationSampleReport {
    schema: String,
    samples: Vec<BookPipelineTranslationSample>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookPipelineTranslationSample {
    chunk_ref: String,
    source_excerpt: String,
    translated_excerpt: String,
    degradation: String,
}

#[derive(Clone)]
struct TranslationTaskUnit {
    unit_id: String,
    task_manifest_path: String,
    sha256: String,
    source_chapter_path: String,
    source_chapter_sha256: String,
}

struct ValidatedTranslationReport {
    summary: TranslationEngineReportSummary,
    artifacts: Vec<BookPipelineArtifact>,
    failed_unit_ids: BTreeSet<String>,
    // "source -> required translation", deduplicated across units and sorted, so
    // one term missed throughout a book reads as one problem to fix rather than
    // one per chapter.
    glossary_violations: BTreeSet<String>,
}

pub(crate) fn translation_engine_repo_root() -> Result<PathBuf, String> {
    let repo_root = local_reading_repo_root()?;
    let package_manifest = repo_root
        .join("packages")
        .join("translation-engine")
        .join("pyproject.toml");
    if !package_manifest.is_file() {
        return Err(format!(
            "Translation engine package not found at {}",
            display_path(&package_manifest)
        ));
    }
    Ok(repo_root)
}

/// Write the user's chosen model into the manifest when this run's slot is the
/// one they selected in Settings. Absent a selection (or for a different slot),
/// the manifest carries no model and the engine keeps the registry default. The
/// sample and the full run call this identically, so a preview uses the same
/// model as the run it previews.
fn apply_active_model_to_manifest(
    manifest: &mut serde_json::Value,
    profile_id: &str,
    config_id: &str,
) {
    if let Some(active) = crate::read_active_model() {
        if active.profile_id == profile_id
            && active.config_id == config_id
            && !active.model.trim().is_empty()
        {
            manifest["model"] = serde_json::Value::String(active.model);
        }
    }
}

/// Inject the Keychain-stored API key for this slot into the engine subprocess,
/// under the slot's key_env. A no-op when no key is stored, so translation still
/// falls back to a key in the repository-root .env. The engine's credential path
/// is unchanged either way — it reads an environment variable.
fn inject_model_credential(command: &mut RunnerCommand, profile_id: &str, config_id: &str) {
    let Ok(repo_root) = translation_engine_repo_root() else {
        return;
    };
    if let Some((key_env, value)) =
        crate::model_settings::resolve_credential_env(&repo_root, profile_id, config_id)
    {
        command.env.push((key_env, value));
    }
}

fn build_translation_engine_command(
    child: &BookPipelineChildJob,
    manifest_path: &Path,
) -> Result<RunnerCommand, String> {
    let repo_root = translation_engine_repo_root()?;
    let project_root = project_root_from_child(child)?;
    let attempts = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "translate")
        .map(|stage| stage.attempt)
        .unwrap_or(0);
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: TRANSLATION_ENGINE_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args: vec![
            "run".into(),
            "--package".into(),
            "translation-engine".into(),
            "translation-engine".into(),
            "--manifest".into(),
            display_path(manifest_path),
        ],
        env: Vec::new(),
        cwd: Some(repo_root),
        output_dir: project_root,
        attempts,
        accepted_exit_codes: vec![0, 1],
    })
}

fn build_translation_sample_command(
    child: &BookPipelineChildJob,
    manifest_path: &Path,
) -> Result<RunnerCommand, String> {
    let repo_root = translation_engine_repo_root()?;
    let project_root = project_root_from_child(child)?;
    let attempts = child
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "translation_sample_report")
        .count() as u32;
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: TRANSLATION_ENGINE_SAMPLE_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args: vec![
            "run".into(),
            "--package".into(),
            "translation-engine".into(),
            "translation-engine-sample".into(),
            "--manifest".into(),
            display_path(manifest_path),
        ],
        env: Vec::new(),
        cwd: Some(repo_root),
        output_dir: project_root,
        attempts,
        accepted_exit_codes: vec![0],
    })
}

fn translation_task_units(
    child: &BookPipelineChildJob,
    project_root: &Path,
) -> Result<Vec<TranslationTaskUnit>, String> {
    let mut seen = BTreeSet::new();
    let mut units = Vec::new();
    for artifact in child
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "translation_task_manifest")
    {
        let path = PathBuf::from(&artifact.path);
        let relative = path.strip_prefix(project_root).map_err(|_| {
            format!(
                "Translation task manifest is outside the project root: {}",
                artifact.path
            )
        })?;
        let task_json = fs::read_to_string(&path).map_err(|err| {
            format!(
                "Failed to read translation task manifest {}: {err}",
                artifact.path
            )
        })?;
        let task: serde_json::Value =
            serde_json::from_str(&task_json).map_err(|err| err.to_string())?;
        let unit_id = task
            .get("chapterId")
            .and_then(serde_json::Value::as_str)
            .filter(|unit_id| !unit_id.is_empty())
            .ok_or_else(|| {
                format!(
                    "Translation task manifest {} has no chapterId",
                    artifact.path
                )
            })?
            .to_string();
        let source_chapter_path = task
            .get("sourceChapterPath")
            .and_then(serde_json::Value::as_str)
            .filter(|path| !path.is_empty() && !Path::new(path).is_absolute())
            .ok_or_else(|| {
                format!(
                    "Translation task manifest {} has no relative sourceChapterPath",
                    artifact.path
                )
            })?
            .to_string();
        let source_chapter_sha256 = task
            .get("sourceChapterSha256")
            .and_then(serde_json::Value::as_str)
            .filter(|sha256| !sha256.is_empty())
            .ok_or_else(|| {
                format!(
                    "Translation task manifest {} has no sourceChapterSha256",
                    artifact.path
                )
            })?
            .to_string();
        if !seen.insert(unit_id.clone()) {
            return Err(format!("Duplicate translation unit ID: {unit_id}"));
        }
        let expected_sha256 = artifact
            .sha256
            .clone()
            .ok_or_else(|| format!("Translation task manifest {} has no SHA-256", artifact.path))?;
        if sha256_file(&path)? != expected_sha256 {
            return Err(format!(
                "Translation task manifest changed after approval: {}",
                artifact.path
            ));
        }
        units.push(TranslationTaskUnit {
            unit_id,
            task_manifest_path: display_path(relative),
            sha256: expected_sha256,
            source_chapter_path,
            source_chapter_sha256,
        });
    }
    units.sort_by(|left, right| left.unit_id.cmp(&right.unit_id));
    if units.is_empty() {
        return Err("No prepared translation task manifests are available.".into());
    }
    Ok(units)
}

fn validate_translation_sample_report(
    report: &BookPipelineTranslationSampleReport,
    units: &[TranslationTaskUnit],
) -> Result<(), String> {
    if report.schema != TRANSLATION_ENGINE_SAMPLE_REPORT_SCHEMA {
        return Err("Translation sample report has an unsupported schema.".into());
    }
    let expected_count = units.len().saturating_sub(2).min(TRANSLATION_SAMPLE_COUNT);
    if report.samples.len() != expected_count {
        return Err("Translation sample report has an unexpected sample count.".into());
    }
    let internal_positions = units
        .iter()
        .enumerate()
        .skip(1)
        .take(units.len().saturating_sub(2))
        .map(|(index, unit)| (unit.unit_id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut previous_position = None;
    for sample in &report.samples {
        let position = internal_positions
            .get(sample.chunk_ref.as_str())
            .copied()
            .ok_or_else(|| {
                "Translation sample report referenced a non-internal task.".to_string()
            })?;
        if !seen.insert(sample.chunk_ref.as_str())
            || previous_position.is_some_and(|previous| position <= previous)
        {
            return Err("Translation sample report contains duplicate or unordered tasks.".into());
        }
        if !matches!(sample.degradation.as_str(), "none" | "aligned" | "source") {
            return Err("Translation sample report has an invalid degradation marker.".into());
        }
        previous_position = Some(position);
    }
    Ok(())
}

fn run_translation_sample_with_executor(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    provider_profile_id: &str,
    provider_config_id: &str,
    apply_to_job: bool,
    executor: &dyn RunnerCommandExecutor,
) -> Result<BookPipelineJob, String> {
    let provider_profile_id = provider_profile_id.trim();
    let provider_config_id = provider_config_id.trim();
    if provider_profile_id.is_empty() || provider_config_id.is_empty() {
        return Err("Translation sample requires provider profile and config IDs.".into());
    }

    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
    let job = &state.jobs[job_index];
    if job.translation_mode != TRANSLATION_MODE_FAST {
        return Err("Programmatic translation samples require fast mode.".into());
    }
    let child = &job.children[child_index];
    let gate_is_ready = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .is_some_and(|stage| stage.status == STATUS_READY && stage.approval_id.is_none());
    let translation_not_started = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "translate")
        .is_some_and(|stage| stage.status == STATUS_PENDING);
    if !gate_is_ready || !translation_not_started {
        return Err("Translation samples run only at the ready pre-translation gate.".into());
    }

    let project_root = project_root_from_child(child)?;
    let units = translation_task_units(child, &project_root)?;
    // textCleanup and customInstructions are read from the same places the full
    // run reads them, so the preview cannot show a translation the real run
    // would not produce. See the run manifest assembly further down.
    let mut manifest = serde_json::json!({
        "schema": TRANSLATION_ENGINE_SAMPLE_SCHEMA,
        "projectRoot": display_path(&project_root),
        "sourceMapPath": "metadata/source_map.json",
        "sourceLanguage": "auto",
        "targetLanguage": "zh-Hans",
        "providerProfileId": provider_profile_id,
        "providerConfigId": provider_config_id,
        "sampleCount": TRANSLATION_SAMPLE_COUNT,
        "characterBudget": TRANSLATION_SAMPLE_CHARACTER_BUDGET,
        "textCleanup": job.text_cleanup,
        "placeholderRetries": TRANSLATION_ENGINE_PLACEHOLDER_RETRIES,
        "units": units
            .iter()
            .map(|unit| serde_json::json!({"taskManifestPath": unit.task_manifest_path.as_str()}))
            .collect::<Vec<_>>(),
    });
    if let Some(custom_instructions) = &child.custom_instructions {
        manifest["customInstructions"] = serde_json::to_value(custom_instructions)
            .map_err(|err| format!("Could not serialize custom instructions: {err}"))?;
    }
    apply_active_model_to_manifest(&mut manifest, provider_profile_id, provider_config_id);
    let sample_dir = project_root.join("qa").join("sample-compare");
    fs::create_dir_all(&sample_dir).map_err(|err| err.to_string())?;
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())? + "\n";
    static SAMPLE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let manifest_path = sample_dir.join(format!(
        "manifest-{}-{}.json",
        std::process::id(),
        SAMPLE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&manifest_path, manifest_json).map_err(|err| err.to_string())?;

    let mut command = build_translation_sample_command(child, &manifest_path)?;
    inject_model_credential(&mut command, provider_profile_id, provider_config_id);
    let command_result = match executor.execute(&command) {
        Ok(result) => {
            fs::remove_file(&manifest_path).map_err(|err| {
                format!("Failed to remove the temporary translation sample manifest: {err}")
            })?;
            result
        }
        Err(err) => {
            let _ = fs::remove_file(&manifest_path);
            return Err(err);
        }
    };
    let report: BookPipelineTranslationSampleReport = serde_json::from_str(&command_result.stdout)
        .map_err(|err| format!("Translation engine returned invalid sample JSON: {err}"))?;
    validate_translation_sample_report(&report, &units)?;
    let report_json = serde_json::to_string_pretty(&report).map_err(|err| err.to_string())? + "\n";
    let report_sha256 = sha256_str(&report_json);
    let report_path = sample_dir.join(format!("report-{report_sha256}.json"));
    let report_was_present = report_path.is_file();

    let job = &mut state.jobs[job_index];
    let previous_report_path = job.children[child_index]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "translation_sample_report")
        .map(|artifact| PathBuf::from(&artifact.path));
    // Only when the caller asked. A sample run used to adopt its provider
    // unconditionally, so trying a model out silently redirected the whole book
    // and there was no way back to the one the job was queued with.
    if apply_to_job {
        job.translation_profile_id = provider_profile_id.into();
        job.translation_config_id = provider_config_id.into();
    }
    replace_stage_artifacts(
        &mut job.children[child_index].artifacts,
        &["translation_sample_report"],
        vec![BookPipelineArtifact {
            kind: "translation_sample_report".into(),
            path: display_path(&report_path),
            sha256: Some(report_sha256.clone()),
            producer_stage: Some("approve_translation".into()),
            ..BookPipelineArtifact::default()
        }],
    );
    let _ = ready_translation_approval_gate(job, child_index);
    let Some((current_request, current_input_hashes)) = job
        .children
        .get(child_index)
        .and_then(|child| translation_approval_binding(job, child))
    else {
        return Err("Translation sample could not rebuild the approval binding.".into());
    };
    let gate_matches = job.children[child_index]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .is_some_and(|stage| {
            stage.status == STATUS_READY
                && stage.input_hashes == current_input_hashes
                && stage.approval_request.as_ref().is_some_and(|request| {
                    approval_request_binding_matches(request, &current_request)
                })
        });
    if !gate_matches {
        return Err("Translation sample changed without a current approval binding.".into());
    }
    fs::write(&report_path, &report_json).map_err(|err| err.to_string())?;
    job.current_step = "Reviewed preflight translation samples".into();
    job.log_summary
        .extend(redact_log_lines(&command_result.log_summary));
    job.log_summary.push(format!(
        "Generated {} preflight translation sample(s) for profile {} and config {}",
        report.samples.len(),
        provider_profile_id,
        provider_config_id
    ));
    job.log_summary = trim_log_summary(&job.log_summary);
    job.updated_at = now_label();
    derive_job(job);
    let result = job.clone();
    if let Err(err) = store.save(&state) {
        if !report_was_present {
            let _ = fs::remove_file(&report_path);
        }
        return Err(err);
    }
    if let Some(previous_path) = previous_report_path.filter(|previous_path| {
        previous_path != &report_path && previous_path.parent() == Some(sample_dir.as_path())
    }) {
        let _ = fs::remove_file(previous_path);
    }
    Ok(result)
}

fn set_translation_provider_in_store(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    provider_profile_id: &str,
    provider_config_id: &str,
) -> Result<BookPipelineJob, String> {
    let provider_profile_id = provider_profile_id.trim();
    let provider_config_id = provider_config_id.trim();
    if provider_profile_id.is_empty() || provider_config_id.is_empty() {
        return Err("Setting the translation provider requires profile and config IDs.".into());
    }

    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
    let job = &mut state.jobs[job_index];
    if job.translation_mode != TRANSLATION_MODE_FAST {
        return Err("Only a fast-mode job runs through a provider slot.".into());
    }
    if job.translation_profile_id == provider_profile_id
        && job.translation_config_id == provider_config_id
    {
        return Ok(job.clone());
    }
    // Whether an approval is currently bound decides how strict this has to be:
    // a bound gate must be rebindable, because the provider is part of what was
    // approved. A gate that has not been reached yet has nothing to invalidate.
    let had_binding = job.children[child_index]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .is_some_and(|stage| stage.approval_request.is_some());
    job.translation_profile_id = provider_profile_id.into();
    job.translation_config_id = provider_config_id.into();
    // The provider is inside the binding hash, so this re-readies the gate and
    // drops the old approval. It refuses while a later stage is live — the one
    // case where redirecting the book mid-flight would be unsafe.
    if !ready_translation_approval_gate(job, child_index) && had_binding {
        return Err(
            "The translation provider cannot change while a later stage is running.".into(),
        );
    }
    job.current_step =
        format!("Translation provider set to {provider_profile_id} / {provider_config_id}");
    job.log_summary.push(job.current_step.clone());
    job.log_summary = trim_log_summary(&job.log_summary);
    job.updated_at = now_label();
    derive_job(job);
    let result = job.clone();
    store.save(&state)?;
    Ok(result)
}

fn read_translation_sample_report(
    job: &BookPipelineJob,
    child_id: &str,
) -> Result<BookPipelineTranslationSampleReport, String> {
    let child = job
        .children
        .iter()
        .find(|child| child.id == child_id)
        .ok_or_else(|| "Book Pipeline child job not found.".to_string())?;
    let artifact = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "translation_sample_report")
        .ok_or_else(|| "Translation sample report is not registered.".to_string())?;
    let project_root = project_root_from_child(child)?;
    let canonical_project_root = fs::canonicalize(&project_root)
        .map_err(|_| "Translation sample project root is missing.".to_string())?;
    let path = fs::canonicalize(&artifact.path)
        .map_err(|_| "Translation sample report file is missing.".to_string())?;
    if !path.starts_with(&canonical_project_root) {
        return Err("Translation sample report is outside the project root.".into());
    }
    let expected_sha256 = artifact
        .sha256
        .as_deref()
        .ok_or_else(|| "Translation sample report has no SHA-256.".to_string())?;
    if sha256_file(&path)? != expected_sha256 {
        return Err("Translation sample report changed after registration.".into());
    }
    let report: BookPipelineTranslationSampleReport =
        serde_json::from_str(&fs::read_to_string(path).map_err(|err| err.to_string())?)
            .map_err(|err| format!("Translation sample report is invalid: {err}"))?;
    let units = translation_task_units(child, &project_root)?;
    validate_translation_sample_report(&report, &units)?;
    Ok(report)
}

fn failed_translation_unit_ids(child: &BookPipelineChildJob) -> BTreeSet<String> {
    child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "translate")
        .into_iter()
        .flat_map(|stage| stage.input_hashes.keys())
        .filter_map(|key| key.strip_prefix("failedUnit:"))
        .map(str::to_string)
        .collect()
}

fn translation_approval_input_hashes(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
) -> Result<BTreeMap<String, String>, String> {
    let gate = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .ok_or_else(|| "Translation approval gate is missing.".to_string())?;
    let approval_id = gate
        .approval_id
        .as_deref()
        .filter(|_| gate.status == STATUS_COMPLETED)
        .ok_or_else(|| "Translation approval is not completed.".to_string())?;
    let request = gate
        .approval_request
        .as_ref()
        .ok_or_else(|| "Translation approval request is missing.".to_string())?;
    let approval = job
        .approval_references
        .iter()
        .find(|approval| approval.approval_id == approval_id)
        .ok_or_else(|| "Translation approval reference is missing.".to_string())?;
    let Some((current_request, current_input_hashes)) = translation_approval_binding(job, child)
    else {
        return Err("Translation approval binding is no longer valid.".into());
    };
    if approval.decision != "approved"
        || approval.gate_id != request.gate_id
        || approval.child_job_id != child.id
        || approval.stage_id != "approve_translation"
        || approval.bound_artifact_hashes != request.bound_artifact_hashes
        || request.gate_id != current_request.gate_id
        || request.translation_mode != current_request.translation_mode
        || request.second_pass_enabled != current_request.second_pass_enabled
        || request.text_cleanup != current_request.text_cleanup
        || request.digest_mode != current_request.digest_mode
        || request.output_formats != current_request.output_formats
        || request.provider_profile_id != current_request.provider_profile_id
        || request.agent_profile_id != current_request.agent_profile_id
        || request.config_id != current_request.config_id
        || request.skill_ids != current_request.skill_ids
        || request.qa_policy != current_request.qa_policy
        || request.sample_evidence != current_request.sample_evidence
        || request.bound_artifact_hashes != current_request.bound_artifact_hashes
        || gate.input_hashes != current_input_hashes
    {
        return Err("Translation approval binding changed before execution.".into());
    }
    Ok(current_input_hashes)
}

fn validate_translation_report_artifact(
    project_root: &Path,
    report: &TranslationEngineArtifactReport,
) -> Result<BookPipelineArtifact, String> {
    let relative = Path::new(&report.path);
    if relative.is_absolute() {
        return Err("Translation engine reported an absolute artifact path.".into());
    }
    let path = project_root.join(relative);
    let canonical_root = fs::canonicalize(project_root).map_err(|err| err.to_string())?;
    let canonical_path = fs::canonicalize(&path).map_err(|err| {
        format!(
            "Translation engine artifact is missing at {}: {err}",
            display_path(&path)
        )
    })?;
    if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_file() {
        return Err("Translation engine artifact is outside the project root.".into());
    }
    let actual_sha256 = sha256_file(&canonical_path)?;
    if actual_sha256 != report.sha256 {
        return Err(format!(
            "Translation engine artifact SHA-256 mismatch at {}",
            display_path(&path)
        ));
    }
    Ok(BookPipelineArtifact {
        kind: report.kind.clone(),
        path: display_path(&path),
        sha256: Some(actual_sha256),
        zotero_key: None,
        producer_stage: Some("translate".into()),
        ..BookPipelineArtifact::default()
    })
}

fn validate_translation_engine_report(
    report: TranslationEngineReport,
    requested_units: &[TranslationTaskUnit],
    project_root: &Path,
) -> Result<ValidatedTranslationReport, String> {
    if report.schema != TRANSLATION_ENGINE_REPORT_SCHEMA {
        return Err("Translation engine returned an unsupported report schema.".into());
    }
    let requested_ids = requested_units
        .iter()
        .map(|unit| unit.unit_id.clone())
        .collect::<BTreeSet<_>>();
    let mut reported_ids = BTreeSet::new();
    let mut artifacts = Vec::new();
    let mut failed_unit_ids = BTreeSet::new();
    let mut glossary_violations = BTreeSet::new();
    let mut completed = 0_u32;
    let mut failed = 0_u32;
    for unit in &report.units {
        if !requested_ids.contains(&unit.unit_id) || !reported_ids.insert(unit.unit_id.clone()) {
            return Err(format!(
                "Translation engine returned an unexpected unit: {}",
                unit.unit_id
            ));
        }
        for violation in &unit.glossary_violations {
            glossary_violations
                .insert(format!("{} -> {}", violation.source, violation.translation));
        }
        match unit.status.as_str() {
            "completed" => {
                completed = completed.saturating_add(1);
                let artifact = unit.artifact.as_ref().ok_or_else(|| {
                    format!(
                        "Completed translation unit {} has no artifact",
                        unit.unit_id
                    )
                })?;
                let expected_path = format!("chapters/translated/{}.md", unit.unit_id);
                if artifact.kind != "chapter_translation"
                    || artifact.path != expected_path
                    || !artifact.complete
                {
                    return Err(format!(
                        "Completed translation unit {} has an invalid artifact",
                        unit.unit_id
                    ));
                }
                artifacts.push(validate_translation_report_artifact(
                    project_root,
                    artifact,
                )?);
            }
            "failed" => {
                failed = failed.saturating_add(1);
                failed_unit_ids.insert(unit.unit_id.clone());
                if let Some(artifact) = &unit.artifact {
                    let expected_path =
                        format!("chapters/translated/.partial/{}.degraded.md", unit.unit_id);
                    if artifact.kind != "chapter_translation_degraded"
                        || artifact.path != expected_path
                        || artifact.complete
                    {
                        return Err(format!(
                            "Failed translation unit {} has an invalid artifact",
                            unit.unit_id
                        ));
                    }
                    artifacts.push(validate_translation_report_artifact(
                        project_root,
                        artifact,
                    )?);
                }
            }
            _ => {
                return Err(format!(
                    "Translation engine returned an invalid status for {}",
                    unit.unit_id
                ));
            }
        }
    }
    if reported_ids != requested_ids
        || report.summary.total != report.units.len() as u32
        || report.summary.completed != completed
        || report.summary.failed != failed
        || completed.saturating_add(failed) != report.summary.total
    {
        return Err("Translation engine report summary does not match its units.".into());
    }
    Ok(ValidatedTranslationReport {
        summary: report.summary,
        artifacts,
        failed_unit_ids,
        glossary_violations,
    })
}

fn run_translate_stage(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
    executor: &dyn RunnerCommandExecutor,
) -> Result<StageRunOutput, String> {
    if job.translation_mode != TRANSLATION_MODE_FAST {
        return Err("The translation engine runner only supports fast mode in this slice.".into());
    }
    let project_root = project_root_from_child(child)?;
    let mut input_hashes = translation_approval_input_hashes(job, child)?;
    let all_units = translation_task_units(child, &project_root)?;
    let retry_unit_ids = failed_translation_unit_ids(child);
    let requested_units = if retry_unit_ids.is_empty() {
        all_units.clone()
    } else {
        let units = all_units
            .iter()
            .filter(|unit| retry_unit_ids.contains(&unit.unit_id))
            .cloned()
            .collect::<Vec<_>>();
        if units.len() != retry_unit_ids.len() {
            return Err("A failed translation unit no longer has a task manifest.".into());
        }
        units
    };

    let mut manifest = serde_json::json!({
        "schema": TRANSLATION_ENGINE_RUN_SCHEMA,
        "projectRoot": display_path(&project_root),
        "sourceMapPath": "metadata/source_map.json",
        "sourceLanguage": "auto",
        "targetLanguage": "zh-Hans",
        "providerProfileId": job.translation_profile_id.as_str(),
        "providerConfigId": job.translation_config_id.as_str(),
        "secondPassEnabled": job.second_pass_enabled,
        "textCleanup": job.text_cleanup,
        "translationPolicyVersion": TRANSLATION_POLICY_VERSION,
        "maxTokens": TRANSLATION_ENGINE_MAX_TOKENS,
        "placeholderRetries": TRANSLATION_ENGINE_PLACEHOLDER_RETRIES,
        "units": requested_units
            .iter()
            .map(|unit| serde_json::json!({"taskManifestPath": unit.task_manifest_path.as_str()}))
            .collect::<Vec<_>>(),
    });
    if let Some(custom_instructions) = &child.custom_instructions {
        manifest["customInstructions"] = serde_json::to_value(custom_instructions)
            .map_err(|err| format!("Could not serialize custom instructions: {err}"))?;
    }
    apply_active_model_to_manifest(
        &mut manifest,
        &job.translation_profile_id,
        &job.translation_config_id,
    );
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())? + "\n";
    let manifest_path = project_root.join("qa").join("tasks").join("run.json");
    fs::write(&manifest_path, &manifest_json).map_err(|err| err.to_string())?;
    let manifest_sha256 = sha256_str(&manifest_json);
    input_hashes.insert(
        "translationPolicyVersion".into(),
        TRANSLATION_POLICY_VERSION.into(),
    );
    input_hashes.insert(
        "maxTokens".into(),
        TRANSLATION_ENGINE_MAX_TOKENS.to_string(),
    );
    input_hashes.insert(
        "placeholderRetries".into(),
        TRANSLATION_ENGINE_PLACEHOLDER_RETRIES.to_string(),
    );
    input_hashes.insert("runManifestSha256".into(), manifest_sha256.clone());
    for unit in &requested_units {
        input_hashes.insert(
            format!("translationTaskManifest:{}", unit.unit_id),
            unit.sha256.clone(),
        );
    }

    let mut command = build_translation_engine_command(child, &manifest_path)?;
    inject_model_credential(
        &mut command,
        &job.translation_profile_id,
        &job.translation_config_id,
    );
    let command_result = executor.execute(&command)?;
    let report: TranslationEngineReport = serde_json::from_str(&command_result.stdout)
        .map_err(|err| format!("Translation engine returned invalid report JSON: {err}"))?;
    let validated = validate_translation_engine_report(report, &requested_units, &project_root)?;
    for unit_id in &validated.failed_unit_ids {
        let unit = all_units
            .iter()
            .find(|unit| &unit.unit_id == unit_id)
            .ok_or_else(|| format!("Translation report referenced unknown unit {unit_id}"))?;
        input_hashes.insert(format!("failedUnit:{unit_id}"), unit.sha256.clone());
    }

    let previous_summary = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "translate")
        .and_then(|stage| stage.unit_summary.clone());
    let unit_summary = if retry_unit_ids.is_empty() {
        BookPipelineUnitSummary {
            total: validated.summary.total,
            completed: validated.summary.completed,
            failed: validated.summary.failed,
            ..BookPipelineUnitSummary::default()
        }
    } else {
        let previous = previous_summary.ok_or_else(|| {
            "Failed translation units are present without a prior unit summary.".to_string()
        })?;
        if previous.total != all_units.len() as u32
            || previous.failed != requested_units.len() as u32
        {
            return Err("Translation retry scope does not match the prior failed units.".into());
        }
        let completed = previous
            .completed
            .checked_add(validated.summary.completed)
            .ok_or_else(|| "Translation unit summary overflowed.".to_string())?;
        if completed.saturating_add(validated.summary.failed) != previous.total {
            return Err("Translation retry summary does not reconcile.".into());
        }
        BookPipelineUnitSummary {
            total: previous.total,
            completed,
            failed: validated.summary.failed,
            ..BookPipelineUnitSummary::default()
        }
    };

    let mut artifacts = vec![BookPipelineArtifact {
        kind: "translation_run_manifest".into(),
        path: display_path(&manifest_path),
        sha256: Some(manifest_sha256),
        zotero_key: None,
        producer_stage: Some("translate".into()),
        ..BookPipelineArtifact::default()
    }];
    artifacts.extend(validated.artifacts);
    let mut log_summary = vec![format!(
        "Translation engine command prepared for {} unit(s)",
        requested_units.len()
    )];
    log_summary.extend(redact_log_lines(&command_result.log_summary));
    log_summary.extend(parse_allowlisted_worker_markers(
        &command_result.stderr,
        &[command.output_dir.as_path()],
    ));
    log_summary.push(format!(
        "Translation engine reported {} completed and {} failed unit(s)",
        validated.summary.completed, validated.summary.failed
    ));
    // Without this the check would compute a result the runner parses and then
    // discards -- the report is read from stdout and never written anywhere the
    // reader can open. A log line is the smallest thing that makes drift real;
    // a richer presentation is #98's follow-up, not a reason to surface nothing.
    if !validated.glossary_violations.is_empty() {
        log_summary.push(format!(
            "Glossary check: {} required term(s) not found in the translation: {}. \
             The chapters are complete and unmodified; review these and re-run if needed.",
            validated.glossary_violations.len(),
            validated
                .glossary_violations
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let error = (validated.summary.failed > 0).then(|| {
        format!(
            "Translation engine reported {} failed unit(s).",
            validated.summary.failed
        )
    });
    Ok(StageRunOutput {
        artifacts,
        artifact_kinds: vec![
            "translation_run_manifest",
            "chapter_translation",
            "chapter_translation_degraded",
        ],
        input_hashes,
        log_summary,
        unit_summary: Some(unit_summary),
        error,
    })
}

fn run_expert_translate_stage(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
) -> Result<StageRunOutcome, String> {
    if job.translation_mode != TRANSLATION_MODE_EXPERT {
        return Err("Expert translation handoff requires expert mode.".into());
    }
    let project_root = project_root_from_child(child)?;
    let mut input_hashes = translation_approval_input_hashes(job, child)?;
    let units = translation_task_units(child, &project_root)?;
    let handoff = serde_json::json!({
        "schema": TRANSLATION_HANDOFF_SCHEMA,
        "agentProfileId": job.translation_profile_id.as_str(),
        "skillIds": job.translation_skill_ids.as_slice(),
        "units": units
            .iter()
            .map(|unit| serde_json::json!({
                "unitId": unit.unit_id.as_str(),
                "taskManifestPath": unit.task_manifest_path.as_str(),
                "taskManifestSha256": unit.sha256.as_str(),
                "sourceChapterPath": unit.source_chapter_path.as_str(),
                "sourceChapterSha256": unit.source_chapter_sha256.as_str(),
                "outputPath": format!("chapters/translated/{}.md", unit.unit_id),
            }))
            .collect::<Vec<_>>(),
    });
    let handoff_json =
        serde_json::to_string_pretty(&handoff).map_err(|err| err.to_string())? + "\n";
    let handoff_dir = project_root.join("qa").join("handoffs");
    fs::create_dir_all(&handoff_dir).map_err(|err| err.to_string())?;
    let handoff_path = handoff_dir.join("translate.json");
    fs::write(&handoff_path, &handoff_json).map_err(|err| err.to_string())?;
    let handoff_sha256 = sha256_str(&handoff_json);
    let handoff_was_issued = child.artifacts.iter().any(|artifact| {
        artifact.kind == "translation_handoff"
            && artifact.path == display_path(&handoff_path)
            && artifact.sha256.as_deref() == Some(handoff_sha256.as_str())
    });
    input_hashes.insert("translationHandoffSha256".into(), handoff_sha256.clone());

    let mut artifacts = vec![BookPipelineArtifact {
        kind: "translation_handoff".into(),
        path: display_path(&handoff_path),
        sha256: Some(handoff_sha256),
        zotero_key: None,
        producer_stage: Some("translate".into()),
        ..BookPipelineArtifact::default()
    }];
    let mut completed = 0_u32;
    let mut blocked = 0_u32;
    for unit in &units {
        input_hashes.insert(
            format!("translationTaskManifest:{}", unit.unit_id),
            unit.sha256.clone(),
        );
        input_hashes.insert(
            format!("sourceChapter:{}", unit.unit_id),
            unit.source_chapter_sha256.clone(),
        );
        if !handoff_was_issued {
            blocked = blocked.saturating_add(1);
            continue;
        }
        let path = project_root
            .join("chapters")
            .join("translated")
            .join(format!("{}.md", unit.unit_id));
        let Ok(text) = fs::read_to_string(&path) else {
            blocked = blocked.saturating_add(1);
            continue;
        };
        if text.trim().is_empty() {
            blocked = blocked.saturating_add(1);
            continue;
        }
        completed = completed.saturating_add(1);
        artifacts.push(BookPipelineArtifact {
            kind: "chapter_translation".into(),
            path: display_path(&path),
            sha256: Some(sha256_str(&text)),
            zotero_key: None,
            producer_stage: Some("translate".into()),
            ..BookPipelineArtifact::default()
        });
    }
    let output = StageRunOutput {
        artifacts,
        artifact_kinds: vec!["translation_handoff", "chapter_translation"],
        input_hashes,
        log_summary: vec![if blocked == 0 {
            format!("Accepted {completed} expert translation unit(s)")
        } else {
            format!(
                "Expert translation handoff waiting for {blocked} of {} unit(s)",
                units.len()
            )
        }],
        unit_summary: Some(BookPipelineUnitSummary {
            total: units.len() as u32,
            blocked,
            completed,
            ..BookPipelineUnitSummary::default()
        }),
        error: None,
    };
    if blocked == 0 {
        Ok(StageRunOutcome::Finished(output))
    } else {
        Ok(StageRunOutcome::WaitingForAgent {
            output,
            agent_profile_id: job.translation_profile_id.clone(),
        })
    }
}

#[derive(Clone)]
struct ExpertQaUnit {
    unit_id: String,
    source_text: String,
    translation_text: String,
    translation_path: PathBuf,
    translation_sha256: String,
    control_path: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExpertReviewState {
    Pending,
    Pass,
    Defect,
}

#[derive(Clone)]
struct AutomatedQaChecks {
    placeholder_integrity: bool,
    structure: bool,
    terminology_consistency: bool,
    completeness: bool,
}

impl AutomatedQaChecks {
    fn passed(&self) -> bool {
        self.placeholder_integrity
            && self.structure
            && self.terminology_consistency
            && self.completeness
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ExpertQaHandoff {
    schema: String,
    policy_version: String,
    qa_policy: String,
    agent_profile_id: String,
    skill_ids: Vec<String>,
    base_unit_ids: Vec<String>,
    expansion_unit_ids: Vec<String>,
    translation_hashes: BTreeMap<String, String>,
    translation_paths: BTreeMap<String, String>,
    chapter_control_paths: BTreeMap<String, String>,
}

fn qa_policy(job: &BookPipelineJob) -> Result<&'static str, String> {
    match job.translation_mode.as_str() {
        TRANSLATION_MODE_FAST => Ok(TRANSLATION_MODE_FAST),
        TRANSLATION_MODE_EXPERT => Ok(TRANSLATION_MODE_EXPERT),
        _ => Err("Expert QA requires a supported translation mode.".into()),
    }
}

fn expert_qa_agent_contract(job: &BookPipelineJob) -> (String, Vec<String>) {
    if job.translation_mode == TRANSLATION_MODE_EXPERT {
        (
            job.translation_profile_id.clone(),
            job.translation_skill_ids.clone(),
        )
    } else {
        // In fast mode the job profile belongs to the provider, not an agent;
        // keep the external QA handoff explicit without widening enqueue APIs.
        (
            EXPERT_QA_AGENT_PROFILE_ID.into(),
            vec![EXPERT_QA_SKILL_ID.into()],
        )
    }
}

fn expert_qa_units(
    child: &BookPipelineChildJob,
    project_root: &Path,
) -> Result<Vec<ExpertQaUnit>, String> {
    let tasks = translation_task_units(child, project_root)?;
    let mut units = Vec::with_capacity(tasks.len());
    for task in tasks {
        let unit_id = task.unit_id.clone();
        let source_path = project_root.join(&task.source_chapter_path);
        let source_text = fs::read_to_string(&source_path).map_err(|err| {
            format!(
                "Failed to read source chapter {}: {err}",
                display_path(&source_path)
            )
        })?;
        if sha256_str(&source_text) != task.source_chapter_sha256 {
            return Err(format!("Source chapter changed after prepare: {unit_id}"));
        }
        let translation_path = project_root
            .join("chapters")
            .join("translated")
            .join(format!("{unit_id}.md"));
        let translation_text = fs::read_to_string(&translation_path).map_err(|err| {
            format!(
                "Failed to read translated chapter {}: {err}",
                display_path(&translation_path)
            )
        })?;
        if translation_text.trim().is_empty() {
            return Err(format!("Translated chapter {unit_id} is empty."));
        }
        let expected_path = display_path(&translation_path);
        if !child.artifacts.iter().any(|artifact| {
            artifact.kind == "chapter_translation" && artifact.path == expected_path
        }) {
            return Err(format!(
                "Translated chapter {unit_id} is not registered as an artifact."
            ));
        }
        units.push(ExpertQaUnit {
            unit_id: unit_id.clone(),
            source_text,
            translation_sha256: sha256_str(&translation_text),
            translation_text,
            translation_path,
            control_path: project_root
                .join("qa")
                .join("chapter_controls")
                .join(format!("{unit_id}.json")),
        });
    }
    Ok(units)
}

fn placeholder_tokens(text: &str) -> BTreeMap<String, u32> {
    let mut tokens = BTreeMap::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        match (start, character) {
            (None, '{') => start = Some(index),
            (Some(open), '}') => {
                let token = &text[open..index + character.len_utf8()];
                if token.len() > 2 && !token.chars().any(char::is_whitespace) {
                    *tokens.entry(token.to_string()).or_insert(0) += 1;
                }
                start = None;
            }
            _ => {}
        }
    }
    tokens
}

fn markdown_heading_shape(text: &str) -> Vec<usize> {
    let mut in_fence = false;
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
                return None;
            }
            (!in_fence).then(|| atx_heading_level(line)).flatten()
        })
        .collect()
}

fn markdown_content_block_count(text: &str) -> usize {
    let mut count = 0;
    let mut in_block = false;
    for line in text.lines() {
        let content = !line.trim().is_empty() && atx_heading_level(line).is_none();
        if content && !in_block {
            count += 1;
        }
        in_block = content;
    }
    count
}

fn parse_csv_row(line: &str) -> Result<Vec<String>, String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut characters = line.chars().peekable();
    let mut quoted = false;
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                current.push('"');
                characters.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(current);
                current = String::new();
            }
            _ => current.push(character),
        }
    }
    if quoted {
        return Err("Glossary CSV has an unterminated quoted field.".into());
    }
    fields.push(current);
    Ok(fields)
}

fn glossary_terms(project_root: &Path) -> Result<Vec<(String, String)>, String> {
    let path = project_root.join("glossary").join("terms.csv");
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read glossary {}: {err}", display_path(&path)))?;
    let mut terms = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        let fields = parse_csv_row(line)?;
        if fields.len() < 2 {
            return Err(format!(
                "Glossary CSV row {} has fewer than two fields.",
                index + 1
            ));
        }
        let source = fields[0].trim();
        let translation = fields[1].trim();
        if !source.is_empty() && !translation.is_empty() {
            terms.push((source.to_string(), translation.to_string()));
        }
    }
    Ok(terms)
}

fn automated_qa_checks(unit: &ExpertQaUnit, terms: &[(String, String)]) -> AutomatedQaChecks {
    AutomatedQaChecks {
        placeholder_integrity: placeholder_tokens(&unit.source_text)
            == placeholder_tokens(&unit.translation_text),
        structure: markdown_heading_shape(&unit.source_text)
            == markdown_heading_shape(&unit.translation_text),
        terminology_consistency: terms.iter().all(|(source, translation)| {
            !unit.source_text.contains(source) || unit.translation_text.contains(translation)
        }),
        completeness: !unit.translation_text.trim().is_empty()
            && markdown_content_block_count(&unit.source_text)
                == markdown_content_block_count(&unit.translation_text),
    }
}

fn read_chapter_control(unit: &ExpertQaUnit) -> Result<serde_json::Value, String> {
    let text = fs::read_to_string(&unit.control_path).map_err(|err| {
        format!(
            "Failed to read chapter control {}: {err}",
            display_path(&unit.control_path)
        )
    })?;
    let control: serde_json::Value = serde_json::from_str(&text).map_err(|err| err.to_string())?;
    if control.get("schema").and_then(serde_json::Value::as_str) != Some(CHAPTER_CONTROL_SCHEMA)
        || control.get("chapterId").and_then(serde_json::Value::as_str)
            != Some(unit.unit_id.as_str())
    {
        return Err(format!(
            "Chapter control contract mismatch for {}.",
            unit.unit_id
        ));
    }
    Ok(control)
}

fn write_chapter_control(unit: &ExpertQaUnit, control: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(control).map_err(|err| err.to_string())? + "\n";
    fs::write(&unit.control_path, text).map_err(|err| err.to_string())
}

fn qa_status(passed: bool) -> &'static str {
    if passed {
        "pass"
    } else {
        "failed"
    }
}

fn update_automated_control(
    control: &mut serde_json::Value,
    unit: &ExpertQaUnit,
    policy: &str,
    checks: &AutomatedQaChecks,
    attempt: u32,
) {
    let previous_status = control
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let previous_translation_sha256 = control
        .get("translationSha256")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let translation_changed = previous_translation_sha256 != unit.translation_sha256;
    control["qaPolicy"] = serde_json::json!(policy);
    control["qaPolicyVersion"] = serde_json::json!(EXPERT_QA_POLICY_VERSION);
    control["translationSha256"] = serde_json::json!(unit.translation_sha256);
    control["automationAttempt"] = serde_json::json!(attempt);
    control["automatedChecks"] = serde_json::json!({
        "placeholderIntegrity": qa_status(checks.placeholder_integrity),
        "structure": qa_status(checks.structure),
        "terminologyConsistency": qa_status(checks.terminology_consistency),
        "completeness": qa_status(checks.completeness),
    });
    control["checks"]["targetOnlyReading"] = serde_json::json!(qa_status(checks.passed()));
    control["checks"]["closure"] = serde_json::json!("pending");
    control["closureEvidence"] = serde_json::json!({"status": "pending"});
    if checks.passed() {
        control["status"] = serde_json::json!("pending");
        if previous_status == "failed" && translation_changed {
            control["fixAttempt"] = serde_json::json!(attempt);
        }
    } else {
        control["status"] = serde_json::json!("failed");
        control["automationFailureAttempt"] = serde_json::json!(attempt);
    }
    if translation_changed {
        control["expertReview"] = serde_json::json!({
            "required": true,
            "status": "pending",
        });
        control["checks"]["sourceFidelity"] = serde_json::json!("pending");
        control["checks"]["polysemyBackCheck"] = serde_json::json!("pending");
        control["checks"]["proseRebuild"] = serde_json::json!("pending");
    }
}

fn initial_fast_qa_sample(child: &BookPipelineChildJob, units: &[ExpertQaUnit]) -> Vec<String> {
    let Some(first) = units.first() else {
        return Vec::new();
    };
    let mut selected = vec![first.unit_id.clone()];
    let mut candidates = units
        .iter()
        .skip(1)
        .map(|unit| {
            (
                sha256_str(&format!(
                    "{}:{}:{}",
                    child.id, unit.unit_id, unit.translation_sha256
                )),
                unit.unit_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    candidates.sort();
    selected.extend(
        candidates
            .into_iter()
            .take(EXPERT_QA_FAST_RANDOM_SAMPLE_SIZE)
            .map(|(_, unit_id)| unit_id),
    );
    selected
}

fn valid_unit_id_set(units: &[ExpertQaUnit]) -> BTreeSet<String> {
    units.iter().map(|unit| unit.unit_id.clone()).collect()
}

fn load_or_create_qa_handoff(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
    project_root: &Path,
    units: &[ExpertQaUnit],
    policy: &str,
) -> Result<(ExpertQaHandoff, PathBuf), String> {
    let (agent_profile_id, skill_ids) = expert_qa_agent_contract(job);
    let handoff_path = project_root
        .join("qa")
        .join("handoffs")
        .join("expert_qa.json");
    let valid_ids = valid_unit_id_set(units);
    let existing = fs::read_to_string(&handoff_path)
        .ok()
        .and_then(|text| serde_json::from_str::<ExpertQaHandoff>(&text).ok())
        .filter(|handoff| {
            handoff.schema == EXPERT_QA_HANDOFF_SCHEMA
                && handoff.policy_version == EXPERT_QA_POLICY_VERSION
                && handoff.qa_policy == policy
                && handoff.agent_profile_id == agent_profile_id
                && handoff.skill_ids == skill_ids
                && handoff
                    .base_unit_ids
                    .iter()
                    .chain(handoff.expansion_unit_ids.iter())
                    .all(|unit_id| valid_ids.contains(unit_id))
        });
    let base_unit_ids = if policy == TRANSLATION_MODE_EXPERT {
        units.iter().map(|unit| unit.unit_id.clone()).collect()
    } else if let Some(existing) = &existing {
        existing.base_unit_ids.clone()
    } else {
        initial_fast_qa_sample(child, units)
    };
    let expansion_unit_ids = existing
        .map(|handoff| handoff.expansion_unit_ids)
        .unwrap_or_default();
    Ok((
        ExpertQaHandoff {
            schema: EXPERT_QA_HANDOFF_SCHEMA.into(),
            policy_version: EXPERT_QA_POLICY_VERSION.into(),
            qa_policy: policy.into(),
            agent_profile_id,
            skill_ids,
            base_unit_ids,
            expansion_unit_ids,
            translation_hashes: units
                .iter()
                .map(|unit| (unit.unit_id.clone(), unit.translation_sha256.clone()))
                .collect(),
            translation_paths: units
                .iter()
                .map(|unit| {
                    (
                        unit.unit_id.clone(),
                        format!("chapters/translated/{}.md", unit.unit_id),
                    )
                })
                .collect(),
            chapter_control_paths: units
                .iter()
                .map(|unit| {
                    (
                        unit.unit_id.clone(),
                        format!("qa/chapter_controls/{}.json", unit.unit_id),
                    )
                })
                .collect(),
        },
        handoff_path,
    ))
}

fn expert_review_state(
    control: &serde_json::Value,
    unit: &ExpertQaUnit,
    skill_ids: &[String],
) -> ExpertReviewState {
    let Some(review) = control.get("expertReview") else {
        return ExpertReviewState::Pending;
    };
    let reviewed_sha256 = review
        .get("translationSha256")
        .and_then(serde_json::Value::as_str);
    let reviewed_skills = review
        .get("skillIds")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
    if reviewed_sha256 != Some(unit.translation_sha256.as_str())
        || reviewed_skills.as_deref() != Some(skill_ids)
    {
        return ExpertReviewState::Pending;
    }
    let unresolved = review.get("unresolved");
    let has_unresolved = [
        "fidelity",
        "terminology",
        "note",
        "traceability",
        "polysemy",
    ]
    .iter()
    .any(|key| {
        unresolved
            .and_then(|value| value.get(*key))
            .and_then(serde_json::Value::as_u64)
            != Some(0)
    });
    match review.get("status").and_then(serde_json::Value::as_str) {
        Some("pass")
            if !has_unresolved
                && control
                    .get("unresolvedPolysemy")
                    .and_then(serde_json::Value::as_u64)
                    == Some(0) =>
        {
            ExpertReviewState::Pass
        }
        Some("failed") | Some("pass") => ExpertReviewState::Defect,
        _ => ExpertReviewState::Pending,
    }
}

fn handoff_sample_ids(handoff: &ExpertQaHandoff) -> Vec<String> {
    handoff
        .base_unit_ids
        .iter()
        .chain(handoff.expansion_unit_ids.iter())
        .cloned()
        .collect()
}

fn next_expansion_unit(handoff: &ExpertQaHandoff, units: &[ExpertQaUnit]) -> Option<String> {
    let sampled = handoff_sample_ids(handoff)
        .into_iter()
        .collect::<BTreeSet<_>>();
    units
        .iter()
        .find(|unit| !sampled.contains(&unit.unit_id))
        .map(|unit| unit.unit_id.clone())
}

fn write_qa_handoff(
    handoff: &ExpertQaHandoff,
    path: &Path,
) -> Result<(BookPipelineArtifact, String), String> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| "Expert QA handoff has no parent directory.".to_string())?,
    )
    .map_err(|err| err.to_string())?;
    let text = serde_json::to_string_pretty(handoff).map_err(|err| err.to_string())? + "\n";
    fs::write(path, &text).map_err(|err| err.to_string())?;
    let sha256 = sha256_str(&text);
    Ok((
        BookPipelineArtifact {
            kind: "expert_qa_handoff".into(),
            path: display_path(path),
            sha256: Some(sha256.clone()),
            zotero_key: None,
            producer_stage: Some("expert_qa".into()),
            ..BookPipelineArtifact::default()
        },
        sha256,
    ))
}

fn qa_retry_unit_ids(child: &BookPipelineChildJob) -> BTreeSet<String> {
    child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "expert_qa")
        .into_iter()
        .flat_map(|stage| stage.input_hashes.keys())
        .filter_map(|key| {
            key.strip_prefix("failedUnit:")
                .or_else(|| key.strip_prefix("invalidatedUnit:"))
        })
        .map(str::to_string)
        .collect()
}

fn expert_qa_attempt(child: &BookPipelineChildJob) -> u32 {
    child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "expert_qa")
        .map(|stage| stage.attempt)
        .unwrap_or(0)
}

fn chapter_control_artifacts(units: &[ExpertQaUnit]) -> Result<Vec<BookPipelineArtifact>, String> {
    units
        .iter()
        .map(|unit| {
            Ok(BookPipelineArtifact {
                kind: "chapter_control".into(),
                path: display_path(&unit.control_path),
                sha256: Some(sha256_file(&unit.control_path)?),
                zotero_key: None,
                producer_stage: Some("expert_qa".into()),
                ..BookPipelineArtifact::default()
            })
        })
        .collect()
}

fn current_translation_artifacts(units: &[ExpertQaUnit]) -> Vec<BookPipelineArtifact> {
    units
        .iter()
        .map(|unit| BookPipelineArtifact {
            kind: "chapter_translation".into(),
            path: display_path(&unit.translation_path),
            sha256: Some(unit.translation_sha256.clone()),
            zotero_key: None,
            producer_stage: Some("translate".into()),
            ..BookPipelineArtifact::default()
        })
        .collect()
}

fn run_expert_qa_stage(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
) -> Result<StageRunOutcome, String> {
    let policy = qa_policy(job)?;
    let project_root = project_root_from_child(child)?;
    let units = expert_qa_units(child, &project_root)?;
    if units.is_empty() {
        return Err("Expert QA has no translated chapter units.".into());
    }
    let terms = glossary_terms(&project_root)?;
    let retry_ids = qa_retry_unit_ids(child);
    let requested_ids = if retry_ids.is_empty() {
        units
            .iter()
            .map(|unit| unit.unit_id.clone())
            .collect::<BTreeSet<_>>()
    } else {
        let available = valid_unit_id_set(&units);
        if !retry_ids.is_subset(&available) {
            return Err("A failed or invalidated QA unit no longer exists.".into());
        }
        retry_ids
    };
    let attempt = expert_qa_attempt(child);
    let mut controls = BTreeMap::new();
    let mut input_hashes = BTreeMap::new();
    input_hashes.insert("qaPolicy".into(), policy.into());
    input_hashes.insert("qaPolicyVersion".into(), EXPERT_QA_POLICY_VERSION.into());
    let mut automated_failures = BTreeSet::new();
    for unit in &units {
        let mut control = read_chapter_control(unit)?;
        input_hashes.insert(
            format!("translation:{}", unit.unit_id),
            unit.translation_sha256.clone(),
        );
        if requested_ids.contains(&unit.unit_id) {
            input_hashes.insert(
                format!("requestedUnit:{}", unit.unit_id),
                unit.translation_sha256.clone(),
            );
            let checks = automated_qa_checks(unit, &terms);
            update_automated_control(&mut control, unit, policy, &checks, attempt);
            if !checks.passed() {
                automated_failures.insert(unit.unit_id.clone());
                input_hashes.insert(
                    format!("failedUnit:{}", unit.unit_id),
                    unit.translation_sha256.clone(),
                );
            }
            write_chapter_control(unit, &control)?;
        }
        controls.insert(unit.unit_id.clone(), control);
    }

    if !automated_failures.is_empty() {
        let handoff_path = project_root
            .join("qa")
            .join("handoffs")
            .join("expert_qa.json");
        if handoff_path.is_file() {
            fs::remove_file(&handoff_path).map_err(|err| err.to_string())?;
        }
        let mut artifacts = current_translation_artifacts(&units);
        artifacts.extend(chapter_control_artifacts(&units)?);
        return Ok(StageRunOutcome::Finished(StageRunOutput {
            artifacts,
            artifact_kinds: vec!["chapter_control", "expert_qa_handoff"],
            input_hashes,
            log_summary: vec![format!(
                "Automated QA failed for {} of {} unit(s)",
                automated_failures.len(),
                units.len()
            )],
            unit_summary: Some(BookPipelineUnitSummary {
                total: units.len() as u32,
                pending: units.len().saturating_sub(automated_failures.len()) as u32,
                failed: automated_failures.len() as u32,
                ..BookPipelineUnitSummary::default()
            }),
            error: Some(format!(
                "Automated QA failed for {} unit(s).",
                automated_failures.len()
            )),
        }));
    }

    let (mut handoff, handoff_path) =
        load_or_create_qa_handoff(job, child, &project_root, &units, policy)?;
    let proposed_handoff_json =
        serde_json::to_string_pretty(&handoff).map_err(|err| err.to_string())? + "\n";
    let proposed_handoff_sha256 = sha256_str(&proposed_handoff_json);
    let handoff_was_issued = child.artifacts.iter().any(|artifact| {
        artifact.kind == "expert_qa_handoff"
            && artifact.path == display_path(&handoff_path)
            && artifact.sha256.as_deref() == Some(proposed_handoff_sha256.as_str())
    });
    let mut sampled_ids = handoff_sample_ids(&handoff);
    for unit in &units {
        let sampled = sampled_ids.contains(&unit.unit_id);
        let control = controls
            .get_mut(&unit.unit_id)
            .ok_or_else(|| format!("Missing in-memory control for {}", unit.unit_id))?;
        control["expertSampled"] = serde_json::json!(sampled);
        if sampled {
            let review_matches_translation = control
                .get("expertReview")
                .and_then(|review| review.get("translationSha256"))
                .and_then(serde_json::Value::as_str)
                == Some(unit.translation_sha256.as_str());
            if !handoff_was_issued || !review_matches_translation {
                control["expertReview"] = serde_json::json!({
                    "required": true,
                    "status": "pending",
                });
            }
        } else {
            control["expertReview"] = serde_json::json!({
                "required": false,
                "status": "not_sampled",
            });
        }
        write_chapter_control(unit, control)?;
    }

    let review_skill_ids = handoff.skill_ids.clone();
    let state_for = |unit_id: &str,
                     controls: &BTreeMap<String, serde_json::Value>|
     -> Result<ExpertReviewState, String> {
        let unit = units
            .iter()
            .find(|unit| unit.unit_id == unit_id)
            .ok_or_else(|| format!("Unknown QA sample unit {unit_id}"))?;
        let control = controls
            .get(unit_id)
            .ok_or_else(|| format!("Missing QA sample control {unit_id}"))?;
        Ok(expert_review_state(control, unit, &review_skill_ids))
    };
    if policy == TRANSLATION_MODE_FAST {
        let should_expand = if let Some(last_expansion) = handoff.expansion_unit_ids.last() {
            state_for(last_expansion, &controls)? == ExpertReviewState::Defect
        } else {
            handoff.base_unit_ids.iter().any(|unit_id| {
                state_for(unit_id, &controls).ok() == Some(ExpertReviewState::Defect)
            })
        };
        if should_expand {
            if let Some(unit_id) = next_expansion_unit(&handoff, &units) {
                handoff.expansion_unit_ids.push(unit_id.clone());
                sampled_ids.push(unit_id.clone());
                let unit = units
                    .iter()
                    .find(|unit| unit.unit_id == unit_id)
                    .ok_or_else(|| format!("Unknown expansion unit {unit_id}"))?;
                let control = controls
                    .get_mut(&unit_id)
                    .ok_or_else(|| format!("Missing expansion control {unit_id}"))?;
                control["expertSampled"] = serde_json::json!(true);
                control["expertReview"] = serde_json::json!({
                    "required": true,
                    "status": "pending",
                });
                write_chapter_control(unit, control)?;
            }
        }
    }

    let (handoff_artifact, handoff_sha256) = write_qa_handoff(&handoff, &handoff_path)?;
    input_hashes.insert("expertQaHandoffSha256".into(), handoff_sha256.clone());
    sampled_ids = handoff_sample_ids(&handoff);
    let mut pending_reviews = BTreeSet::new();
    let mut defect_reviews = BTreeSet::new();
    for unit_id in &sampled_ids {
        match state_for(unit_id, &controls)? {
            ExpertReviewState::Pending => {
                pending_reviews.insert(unit_id.clone());
            }
            ExpertReviewState::Defect => {
                defect_reviews.insert(unit_id.clone());
                let unit = units
                    .iter()
                    .find(|unit| &unit.unit_id == unit_id)
                    .ok_or_else(|| format!("Unknown defect unit {unit_id}"))?;
                input_hashes.insert(
                    format!("failedUnit:{unit_id}"),
                    unit.translation_sha256.clone(),
                );
            }
            ExpertReviewState::Pass => {}
        }
    }
    for unit in &units {
        let control = controls
            .get(&unit.unit_id)
            .ok_or_else(|| format!("Missing unresolved control for {}", unit.unit_id))?;
        if control_has_unresolved_findings(control) {
            defect_reviews.insert(unit.unit_id.clone());
            input_hashes.insert(
                format!("failedUnit:{}", unit.unit_id),
                unit.translation_sha256.clone(),
            );
        }
    }

    let mut same_attempt_fix = false;
    if pending_reviews.is_empty() && defect_reviews.is_empty() {
        for unit in &units {
            let sampled = sampled_ids.contains(&unit.unit_id);
            let control = controls
                .get_mut(&unit.unit_id)
                .ok_or_else(|| format!("Missing final control for {}", unit.unit_id))?;
            if control
                .get("fixAttempt")
                .and_then(serde_json::Value::as_u64)
                == Some(attempt as u64)
            {
                same_attempt_fix = true;
                continue;
            }
            let expert_status = if sampled { "pass" } else { "policy_pass" };
            control["checks"]["sourceFidelity"] = serde_json::json!(expert_status);
            control["checks"]["polysemyBackCheck"] = serde_json::json!(expert_status);
            control["checks"]["proseRebuild"] = serde_json::json!(expert_status);
            control["checks"]["closure"] = serde_json::json!("pass");
            control["status"] = serde_json::json!("pass");
            control["unresolvedPolysemy"] = serde_json::json!(0);
            control["unresolvedFindings"] = serde_json::json!({
                "fidelity": 0,
                "terminology": 0,
                "note": 0,
                "traceability": 0,
                "polysemy": 0,
            });
            control["closureEvidence"] = serde_json::json!({
                "status": "pass",
                "qaPolicy": policy,
                "translationSha256": unit.translation_sha256.as_str(),
                "expertQaHandoffSha256": handoff_sha256.as_str(),
                "passAttempt": attempt,
            });
            write_chapter_control(unit, control)?;
        }
    }

    let mut artifacts = current_translation_artifacts(&units);
    artifacts.extend(chapter_control_artifacts(&units)?);
    artifacts.push(handoff_artifact);
    for unit in &units {
        input_hashes.insert(
            format!("chapterControl:{}", unit.unit_id),
            sha256_file(&unit.control_path)?,
        );
    }
    let waiting = !pending_reviews.is_empty() || !defect_reviews.is_empty() || same_attempt_fix;
    let output = StageRunOutput {
        artifacts,
        artifact_kinds: vec!["chapter_control", "expert_qa_handoff"],
        input_hashes,
        log_summary: vec![if waiting {
            format!(
                "Expert QA handoff waiting: {} pending, {} defect unit(s)",
                pending_reviews.len(),
                defect_reviews.len()
            )
        } else {
            format!(
                "Expert QA closed {} unit(s) under {policy} policy",
                units.len()
            )
        }],
        unit_summary: Some(if waiting {
            BookPipelineUnitSummary {
                total: units.len() as u32,
                blocked: pending_reviews
                    .len()
                    .saturating_add(defect_reviews.len())
                    .max(if same_attempt_fix { 1 } else { 0 }) as u32,
                failed: defect_reviews.len() as u32,
                ..BookPipelineUnitSummary::default()
            }
        } else {
            BookPipelineUnitSummary {
                total: units.len() as u32,
                completed: units.len() as u32,
                ..BookPipelineUnitSummary::default()
            }
        }),
        error: None,
    };
    if waiting {
        Ok(StageRunOutcome::WaitingForAgent {
            output,
            agent_profile_id: handoff.agent_profile_id,
        })
    } else {
        Ok(StageRunOutcome::Finished(output))
    }
}

fn translation_approval_binding(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
) -> Option<(BookPipelineApprovalRequest, BTreeMap<String, String>)> {
    if !["split", "prepare"].iter().all(|stage_id| {
        child
            .stages
            .iter()
            .find(|stage| stage.stage_id == *stage_id)
            .is_some_and(|stage| stage.status == STATUS_COMPLETED)
    }) || job.translation_profile_id.trim().is_empty()
        || job.translation_config_id.trim().is_empty()
    {
        return None;
    }
    if job.translation_mode == TRANSLATION_MODE_FAST && !job.translation_skill_ids.is_empty() {
        return None;
    }
    if job.translation_mode == TRANSLATION_MODE_EXPERT && job.translation_skill_ids.is_empty() {
        return None;
    }

    let split = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "split")?;
    let mut bound_artifact_hashes = BTreeMap::new();
    bound_artifact_hashes.insert(
        "source_markdown".into(),
        split.input_hashes.get("sourceMarkdownSha256")?.clone(),
    );
    let source_map = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "source_map")?;
    bound_artifact_hashes.insert("source_map".into(), source_map.sha256.clone()?);

    let mut task_count = 0;
    for artifact in child
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "translation_task_manifest")
    {
        let file_name = Path::new(&artifact.path).file_name()?.to_str()?;
        bound_artifact_hashes.insert(
            format!("translation_task_manifest:{file_name}"),
            artifact.sha256.clone()?,
        );
        task_count += 1;
    }
    if task_count == 0 {
        return None;
    }

    let provider_profile_id =
        (job.translation_mode == TRANSLATION_MODE_FAST).then(|| job.translation_profile_id.clone());
    let agent_profile_id = (job.translation_mode == TRANSLATION_MODE_EXPERT)
        .then(|| job.translation_profile_id.clone());
    let skill_ids = if job.translation_mode == TRANSLATION_MODE_EXPERT {
        job.translation_skill_ids.clone()
    } else {
        Vec::new()
    };
    let second_pass_enabled =
        job.translation_mode == TRANSLATION_MODE_FAST && job.second_pass_enabled;
    let mut sample_evidence = BTreeMap::new();
    if let Some(report_sha256) = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "translation_sample_report")
        .and_then(|artifact| artifact.sha256.clone())
    {
        sample_evidence.insert("translation_sample_report".into(), report_sha256);
    }
    let text_cleanup = job.translation_mode == TRANSLATION_MODE_FAST && job.text_cleanup;
    let custom_instructions_sha256 = child
        .custom_instructions
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok())
        .map(|value| sha256_str(&value));
    let mut binding = serde_json::json!({
        "translationMode": job.translation_mode.as_str(),
        "secondPassEnabled": second_pass_enabled,
        "textCleanup": text_cleanup,
        "digestMode": job.digest_mode,
        "outputFormats": job.output_formats.as_slice(),
        "providerProfileId": provider_profile_id.as_deref(),
        "agentProfileId": agent_profile_id.as_deref(),
        "configId": job.translation_config_id.as_str(),
        "skillIds": skill_ids.as_slice(),
        "sampleEvidence": &sample_evidence,
        "boundArtifactHashes": &bound_artifact_hashes,
    });
    if let Some(custom_instructions_sha256) = &custom_instructions_sha256 {
        binding["customInstructionsSha256"] =
            serde_json::Value::String(custom_instructions_sha256.clone());
    }
    let binding_json = serde_json::to_string(&binding).ok()?;
    let mut input_hashes = bound_artifact_hashes.clone();
    if let Some(custom_instructions_sha256) = custom_instructions_sha256 {
        input_hashes.insert(
            "customInstructionsSha256".into(),
            custom_instructions_sha256,
        );
    }
    input_hashes.insert("approvalBindingSha256".into(), sha256_str(&binding_json));
    Some((
        BookPipelineApprovalRequest {
            gate_id: "translation_disclosure".into(),
            translation_mode: job.translation_mode.clone(),
            second_pass_enabled,
            text_cleanup,
            digest_mode: job.digest_mode,
            output_formats: job.output_formats.clone(),
            provider_profile_id,
            agent_profile_id,
            config_id: job.translation_config_id.clone(),
            skill_ids,
            qa_policy: None,
            sample_evidence,
            bound_artifact_hashes,
            requested_at: now_label(),
        },
        input_hashes,
    ))
}

fn ready_translation_approval_gate(job: &mut BookPipelineJob, child_index: usize) -> bool {
    let Some((approval_request, input_hashes)) = job
        .children
        .get(child_index)
        .and_then(|child| translation_approval_binding(job, child))
    else {
        return false;
    };
    let child = &job.children[child_index];
    let Some(gate) = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
    else {
        return false;
    };
    let binding_changed = gate.input_hashes != input_hashes;
    let request_missing = gate.approval_request.is_none();
    let should_ready = gate.status == STATUS_PENDING
        || (binding_changed
            && matches!(
                gate.status.as_str(),
                STATUS_READY | STATUS_WAITING_FOR_APPROVAL | STATUS_COMPLETED
            ))
        || (gate.status == STATUS_READY && request_missing);
    if !should_ready {
        return false;
    }

    let gate_order = ordered_stage_index("approve_translation").unwrap_or(0);
    let later_stage_is_active = child.stages.iter().any(|stage| {
        ordered_stage_index(&stage.stage_id).is_some_and(|order| order > gate_order)
            && (matches!(
                stage.status.as_str(),
                STATUS_RUNNING | STATUS_WAITING_FOR_APPROVAL
            ) || is_agent_handoff_waiting(stage))
    });
    if binding_changed && later_stage_is_active {
        return false;
    }

    let child_id = child.id.clone();
    if binding_changed {
        let digest_mode = job.digest_mode;
        let approval_binding_sha256 = input_hashes.get("approvalBindingSha256").cloned();
        invalidate_completed_downstream(&mut job.children[child_index], "prepare");
        for stage in &mut job.children[child_index].stages {
            if ordered_stage_index(&stage.stage_id).is_some_and(|order| order > gate_order)
                && matches!(
                    stage.status.as_str(),
                    STATUS_READY | STATUS_FAILED | STATUS_BLOCKED
                )
            {
                stage.status = STATUS_PENDING.into();
                stage.input_hashes.clear();
                stage.error = None;
                stage.finished_at = None;
                stage.approval_id = None;
                stage.approval_request = None;
                stage.execution_owner = None;
                // Carry the new approval binding while downstream is pending
                // so the persisted invalidation transition stays auditable.
                if let Some(approval_binding_sha256) = &approval_binding_sha256 {
                    stage.input_hashes.insert(
                        "translationApprovalBindingSha256".into(),
                        approval_binding_sha256.clone(),
                    );
                }
            }
        }
        if let Some(stage) = stage_mut(&mut job.children[child_index], "build_digest") {
            stage.status = if digest_mode {
                STATUS_PENDING.into()
            } else {
                STATUS_SKIPPED.into()
            };
            stage.input_hashes.clear();
            stage.error = None;
            stage.finished_at = None;
            stage.execution_owner = None;
        }
        job.approval_references.retain(|approval| {
            approval.child_job_id != child_id
                || !matches!(
                    approval.stage_id.as_str(),
                    "approve_translation" | "approve_promotion"
                )
        });
    }
    if let Some(gate) = stage_mut(&mut job.children[child_index], "approve_translation") {
        gate.status = STATUS_READY.into();
        gate.error = None;
        gate.input_hashes = input_hashes;
        gate.approval_request = Some(approval_request);
        gate.approval_id = None;
        gate.finished_at = None;
        gate.execution_owner = None;
    }
    true
}

fn should_auto_approve_translation(job: &BookPipelineJob, child_index: usize) -> bool {
    job.kind != "collection"
        && job.source.kind == "fake"
        && job.translation_mode == TRANSLATION_MODE_FAST
        && job.translation_profile_id == "fake-provider-profile"
        && job.children.get(child_index).is_some_and(|child| {
            child
                .stages
                .iter()
                .find(|stage| stage.stage_id == "approve_translation")
                .is_some_and(|stage| stage.status == STATUS_READY)
        })
}

fn approve_translation_gate(job: &mut BookPipelineJob, child_index: usize) -> bool {
    let Some((current_request, current_input_hashes)) = job
        .children
        .get(child_index)
        .and_then(|child| translation_approval_binding(job, child))
    else {
        return false;
    };
    let Some(child) = job.children.get(child_index) else {
        return false;
    };
    let Some(gate) = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
    else {
        return false;
    };
    let Some(request) = gate.approval_request.as_ref() else {
        return false;
    };
    if gate.status != STATUS_READY
        || gate.input_hashes != current_input_hashes
        || request.gate_id != current_request.gate_id
        || request.translation_mode != current_request.translation_mode
        || request.second_pass_enabled != current_request.second_pass_enabled
        || request.text_cleanup != current_request.text_cleanup
        || request.digest_mode != current_request.digest_mode
        || request.output_formats != current_request.output_formats
        || request.provider_profile_id != current_request.provider_profile_id
        || request.agent_profile_id != current_request.agent_profile_id
        || request.config_id != current_request.config_id
        || request.skill_ids != current_request.skill_ids
        || request.qa_policy != current_request.qa_policy
        || request.sample_evidence != current_request.sample_evidence
        || request.bound_artifact_hashes != current_request.bound_artifact_hashes
    {
        return false;
    }

    let approval_id = new_approval_id();
    let child_id = child.id.clone();
    let gate_id = request.gate_id.clone();
    let bound_artifact_hashes = request.bound_artifact_hashes.clone();
    job.approval_references.retain(|approval| {
        approval.child_job_id != child_id || approval.stage_id != "approve_translation"
    });
    job.approval_references.push(BookPipelineApprovalReference {
        approval_id: approval_id.clone(),
        gate_id,
        child_job_id: child_id,
        stage_id: "approve_translation".into(),
        decision: "approved".into(),
        bound_artifact_hashes,
    });

    let child = &mut job.children[child_index];
    if let Some(gate) = stage_mut(child, "approve_translation") {
        gate.status = STATUS_COMPLETED.into();
        gate.attempt = gate.attempt.saturating_add(1);
        gate.error = None;
        gate.approval_id = Some(approval_id);
        gate.finished_at = Some(now_label());
        gate.execution_owner = None;
    }
    if stage_mut(child, "translate").is_some_and(|stage| stage.status == STATUS_PENDING) {
        set_stage_status(child, "translate", STATUS_READY, None);
    }
    true
}

fn control_unresolved_count(control: &serde_json::Value, key: &str) -> Option<u64> {
    control
        .get("unresolvedFindings")
        .and_then(|findings| findings.get(key))
        .and_then(serde_json::Value::as_u64)
}

fn control_has_unresolved_findings(control: &serde_json::Value) -> bool {
    control
        .get("unresolvedPolysemy")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        > 0
        || [
            "fidelity",
            "terminology",
            "note",
            "traceability",
            "polysemy",
        ]
        .iter()
        .any(|key| control_unresolved_count(control, key).unwrap_or(0) > 0)
}

fn chapter_control_is_promotion_ready(
    control: &serde_json::Value,
    unit: &ExpertQaUnit,
    policy: &str,
    sampled: bool,
) -> bool {
    let checks = control.get("checks");
    let expert_status = if sampled { "pass" } else { "policy_pass" };
    let automated = control.get("automatedChecks");
    let closure = control.get("closureEvidence");
    let pass_attempt = closure
        .and_then(|value| value.get("passAttempt"))
        .and_then(serde_json::Value::as_u64);
    let fix_attempt = control
        .get("fixAttempt")
        .and_then(serde_json::Value::as_u64);
    control.get("schema").and_then(serde_json::Value::as_str) == Some(CHAPTER_CONTROL_SCHEMA)
        && control.get("chapterId").and_then(serde_json::Value::as_str)
            == Some(unit.unit_id.as_str())
        && control.get("qaPolicy").and_then(serde_json::Value::as_str) == Some(policy)
        && control.get("status").and_then(serde_json::Value::as_str) == Some("pass")
        && control
            .get("translationSha256")
            .and_then(serde_json::Value::as_str)
            == Some(unit.translation_sha256.as_str())
        && checks
            .and_then(|value| value.get("targetOnlyReading"))
            .and_then(serde_json::Value::as_str)
            == Some("pass")
        && checks
            .and_then(|value| value.get("sourceFidelity"))
            .and_then(serde_json::Value::as_str)
            == Some(expert_status)
        && checks
            .and_then(|value| value.get("polysemyBackCheck"))
            .and_then(serde_json::Value::as_str)
            == Some(expert_status)
        && checks
            .and_then(|value| value.get("proseRebuild"))
            .and_then(serde_json::Value::as_str)
            == Some(expert_status)
        && checks
            .and_then(|value| value.get("closure"))
            .and_then(serde_json::Value::as_str)
            == Some("pass")
        && [
            "placeholderIntegrity",
            "structure",
            "terminologyConsistency",
            "completeness",
        ]
        .iter()
        .all(|key| {
            automated
                .and_then(|value| value.get(*key))
                .and_then(serde_json::Value::as_str)
                == Some("pass")
        })
        && closure
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str)
            == Some("pass")
        && closure
            .and_then(|value| value.get("qaPolicy"))
            .and_then(serde_json::Value::as_str)
            == Some(policy)
        && closure
            .and_then(|value| value.get("translationSha256"))
            .and_then(serde_json::Value::as_str)
            == Some(unit.translation_sha256.as_str())
        && pass_attempt.is_some()
        && fix_attempt != pass_attempt
        && control
            .get("unresolvedPolysemy")
            .and_then(serde_json::Value::as_u64)
            == Some(0)
        && [
            "fidelity",
            "terminology",
            "note",
            "traceability",
            "polysemy",
        ]
        .iter()
        .all(|key| control_unresolved_count(control, key) == Some(0))
        && control
            .get("expertSampled")
            .and_then(serde_json::Value::as_bool)
            == Some(sampled)
}

fn promotion_approval_binding(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
) -> Option<(BookPipelineApprovalRequest, BTreeMap<String, String>)> {
    if !["translate", "expert_qa"].iter().all(|stage_id| {
        child
            .stages
            .iter()
            .find(|stage| stage.stage_id == *stage_id)
            .is_some_and(|stage| stage.status == STATUS_COMPLETED)
    }) {
        return None;
    }
    let policy = qa_policy(job).ok()?;
    let project_root = project_root_from_child(child).ok()?;
    let units = expert_qa_units(child, &project_root).ok()?;
    let handoff_artifact = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "expert_qa_handoff")?;
    let handoff_text = fs::read_to_string(&handoff_artifact.path).ok()?;
    let handoff: ExpertQaHandoff = serde_json::from_str(&handoff_text).ok()?;
    let handoff_sha256 = sha256_str(&handoff_text);
    if handoff_artifact.sha256.as_deref() != Some(handoff_sha256.as_str())
        || handoff.qa_policy != policy
    {
        return None;
    }
    let sampled_ids = handoff_sample_ids(&handoff)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut bound_artifact_hashes = BTreeMap::new();
    let mut sample_evidence = BTreeMap::new();
    bound_artifact_hashes.insert("qa_policy".into(), sha256_str(policy));
    bound_artifact_hashes.insert("expert_qa_handoff".into(), handoff_sha256.clone());
    sample_evidence.insert("expert_qa_handoff".into(), handoff_sha256);
    for unit in &units {
        let translation_artifact = child.artifacts.iter().find(|artifact| {
            artifact.kind == "chapter_translation"
                && artifact.path == display_path(&unit.translation_path)
        })?;
        if translation_artifact.sha256.as_deref() != Some(unit.translation_sha256.as_str()) {
            return None;
        }
        let control_text = fs::read_to_string(&unit.control_path).ok()?;
        let control_sha256 = sha256_str(&control_text);
        let control_artifact = child.artifacts.iter().find(|artifact| {
            artifact.kind == "chapter_control" && artifact.path == display_path(&unit.control_path)
        })?;
        if control_artifact.sha256.as_deref() != Some(control_sha256.as_str()) {
            return None;
        }
        let control: serde_json::Value = serde_json::from_str(&control_text).ok()?;
        let sampled = sampled_ids.contains(&unit.unit_id);
        if !chapter_control_is_promotion_ready(&control, unit, policy, sampled) {
            return None;
        }
        bound_artifact_hashes.insert(
            format!("chapter_translation:{}", unit.unit_id),
            unit.translation_sha256.clone(),
        );
        bound_artifact_hashes.insert(
            format!("chapter_control:{}", unit.unit_id),
            control_sha256.clone(),
        );
        if sampled {
            sample_evidence.insert(format!("chapter_control:{}", unit.unit_id), control_sha256);
        }
    }
    let (agent_profile_id, skill_ids) = expert_qa_agent_contract(job);
    let binding_json = serde_json::to_string(&serde_json::json!({
        "translationMode": job.translation_mode.as_str(),
        "qaPolicy": policy,
        "agentProfileId": agent_profile_id.as_str(),
        "configId": job.translation_config_id.as_str(),
        "skillIds": skill_ids.as_slice(),
        "sampleEvidence": &sample_evidence,
        "boundArtifactHashes": &bound_artifact_hashes,
    }))
    .ok()?;
    let mut input_hashes = bound_artifact_hashes.clone();
    input_hashes.insert("approvalBindingSha256".into(), sha256_str(&binding_json));
    Some((
        BookPipelineApprovalRequest {
            gate_id: "promotion".into(),
            translation_mode: job.translation_mode.clone(),
            second_pass_enabled: false,
            text_cleanup: false,
            digest_mode: false,
            output_formats: default_output_formats(),
            provider_profile_id: None,
            agent_profile_id: Some(agent_profile_id),
            config_id: job.translation_config_id.clone(),
            skill_ids,
            qa_policy: Some(policy.into()),
            sample_evidence,
            bound_artifact_hashes,
            requested_at: now_label(),
        },
        input_hashes,
    ))
}

fn ready_promotion_approval_gate(job: &mut BookPipelineJob, child_index: usize) -> bool {
    let Some((approval_request, input_hashes)) = job
        .children
        .get(child_index)
        .and_then(|child| promotion_approval_binding(job, child))
    else {
        return false;
    };
    let child = &job.children[child_index];
    let Some(gate) = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_promotion")
    else {
        return false;
    };
    let binding_changed = gate.input_hashes != input_hashes;
    let should_ready = gate.status == STATUS_PENDING
        || (binding_changed
            && matches!(
                gate.status.as_str(),
                STATUS_READY | STATUS_WAITING_FOR_APPROVAL | STATUS_COMPLETED
            ))
        || (gate.status == STATUS_READY && gate.approval_request.is_none());
    if !should_ready {
        return false;
    }
    let gate_order = ordered_stage_index("approve_promotion").unwrap_or(0);
    let later_stage_is_active = child.stages.iter().any(|stage| {
        ordered_stage_index(&stage.stage_id).is_some_and(|order| order > gate_order)
            && !matches!(stage.status.as_str(), STATUS_PENDING | STATUS_SKIPPED)
    });
    if binding_changed && later_stage_is_active {
        return false;
    }
    let child_id = child.id.clone();
    if binding_changed {
        invalidate_completed_downstream(&mut job.children[child_index], "expert_qa");
        job.approval_references.retain(|approval| {
            approval.child_job_id != child_id || approval.stage_id != "approve_promotion"
        });
    }
    if let Some(gate) = stage_mut(&mut job.children[child_index], "approve_promotion") {
        gate.status = STATUS_READY.into();
        gate.error = None;
        gate.input_hashes = input_hashes;
        gate.approval_request = Some(approval_request);
        gate.approval_id = None;
        gate.finished_at = None;
        gate.execution_owner = None;
    }
    true
}

fn approval_request_binding_matches(
    left: &BookPipelineApprovalRequest,
    right: &BookPipelineApprovalRequest,
) -> bool {
    left.gate_id == right.gate_id
        && left.translation_mode == right.translation_mode
        && left.second_pass_enabled == right.second_pass_enabled
        && left.text_cleanup == right.text_cleanup
        && left.digest_mode == right.digest_mode
        && left.output_formats == right.output_formats
        && left.provider_profile_id == right.provider_profile_id
        && left.agent_profile_id == right.agent_profile_id
        && left.config_id == right.config_id
        && left.skill_ids == right.skill_ids
        && left.qa_policy == right.qa_policy
        && left.sample_evidence == right.sample_evidence
        && left.bound_artifact_hashes == right.bound_artifact_hashes
}

fn approve_promotion_gate(job: &mut BookPipelineJob, child_index: usize) -> bool {
    let Some((current_request, current_input_hashes)) = job
        .children
        .get(child_index)
        .and_then(|child| promotion_approval_binding(job, child))
    else {
        return false;
    };
    let Some(child) = job.children.get(child_index) else {
        return false;
    };
    let Some(gate) = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_promotion")
    else {
        return false;
    };
    let Some(request) = gate.approval_request.as_ref() else {
        return false;
    };
    if gate.status != STATUS_READY
        || gate.input_hashes != current_input_hashes
        || !approval_request_binding_matches(request, &current_request)
    {
        return false;
    }

    let approval_id = new_approval_id();
    let child_id = child.id.clone();
    let gate_id = request.gate_id.clone();
    let bound_artifact_hashes = request.bound_artifact_hashes.clone();
    job.approval_references.retain(|approval| {
        approval.child_job_id != child_id || approval.stage_id != "approve_promotion"
    });
    job.approval_references.push(BookPipelineApprovalReference {
        approval_id: approval_id.clone(),
        gate_id,
        child_job_id: child_id,
        stage_id: "approve_promotion".into(),
        decision: "approved".into(),
        bound_artifact_hashes,
    });

    let child = &mut job.children[child_index];
    if let Some(gate) = stage_mut(child, "approve_promotion") {
        gate.status = STATUS_COMPLETED.into();
        gate.attempt = gate.attempt.saturating_add(1);
        gate.error = None;
        gate.approval_id = Some(approval_id);
        gate.finished_at = Some(now_label());
        gate.execution_owner = None;
    }
    true
}

fn promotion_approval_input_hashes(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
) -> Result<
    (
        String,
        BookPipelineApprovalRequest,
        BTreeMap<String, String>,
    ),
    String,
> {
    let gate = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_promotion")
        .ok_or_else(|| "Promotion approval gate is missing.".to_string())?;
    let approval_id = gate
        .approval_id
        .as_deref()
        .filter(|_| gate.status == STATUS_COMPLETED)
        .ok_or_else(|| "Promotion approval is not completed.".to_string())?;
    let request = gate
        .approval_request
        .as_ref()
        .ok_or_else(|| "Promotion approval request is missing.".to_string())?;
    let approval = job
        .approval_references
        .iter()
        .find(|approval| approval.approval_id == approval_id)
        .ok_or_else(|| "Promotion approval reference is missing.".to_string())?;
    let Some((current_request, current_input_hashes)) = promotion_approval_binding(job, child)
    else {
        return Err("Promotion approval binding is no longer valid.".into());
    };
    if approval.decision != "approved"
        || approval.gate_id != request.gate_id
        || approval.child_job_id != child.id
        || approval.stage_id != "approve_promotion"
        || approval.bound_artifact_hashes != request.bound_artifact_hashes
        || !approval_request_binding_matches(request, &current_request)
        || gate.input_hashes != current_input_hashes
    {
        return Err("Promotion approval binding changed before execution.".into());
    }
    Ok((
        approval_id.to_string(),
        request.clone(),
        current_input_hashes,
    ))
}

fn run_promote_stage(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
) -> Result<StageRunOutput, String> {
    let project_root = project_root_from_child(child)?;
    let (approval_id, request, mut input_hashes) = promotion_approval_input_hashes(job, child)?;
    let units = expert_qa_units(child, &project_root)?;
    let approved_unit_ids = request
        .bound_artifact_hashes
        .keys()
        .filter_map(|key| key.strip_prefix("chapter_translation:"))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    if approved_unit_ids.is_empty() {
        return Err("Promotion approval contains no approved chapter units.".into());
    }

    let mut approved_units = Vec::new();
    for unit_id in &approved_unit_ids {
        let unit = units
            .iter()
            .find(|unit| &unit.unit_id == unit_id)
            .ok_or_else(|| format!("Approved promotion unit no longer exists: {unit_id}"))?;
        let approved_translation_sha256 = request
            .bound_artifact_hashes
            .get(&format!("chapter_translation:{unit_id}"))
            .ok_or_else(|| format!("Promotion approval has no translation hash for {unit_id}"))?;
        if &sha256_file(&unit.translation_path)? != approved_translation_sha256 {
            return Err(format!(
                "Translated chapter changed after promotion approval: {unit_id}"
            ));
        }
        let control_sha256 = sha256_file(&unit.control_path)?;
        if request
            .bound_artifact_hashes
            .get(&format!("chapter_control:{unit_id}"))
            != Some(&control_sha256)
        {
            return Err(format!(
                "Chapter control changed after promotion approval: {unit_id}"
            ));
        }
        approved_units.push((unit, control_sha256));
    }

    let final_dir = project_root.join("chapters").join("final");
    fs::create_dir_all(&final_dir).map_err(|err| err.to_string())?;
    for entry in fs::read_dir(&final_dir).map_err(|err| err.to_string())? {
        let path = entry.map_err(|err| err.to_string())?.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            fs::remove_file(path).map_err(|err| err.to_string())?;
        }
    }

    let mut artifacts = Vec::new();
    let mut manifest_units = Vec::new();
    for (unit, control_sha256) in approved_units {
        let final_path = final_dir.join(format!("{}.md", unit.unit_id));
        fs::copy(&unit.translation_path, &final_path).map_err(|err| err.to_string())?;
        let final_sha256 = sha256_file(&final_path)?;
        if final_sha256 != unit.translation_sha256 {
            return Err(format!(
                "Promoted chapter hash does not match approved translation: {}",
                unit.unit_id
            ));
        }
        artifacts.push(BookPipelineArtifact {
            kind: "chapter_final".into(),
            path: display_path(&final_path),
            sha256: Some(final_sha256.clone()),
            zotero_key: None,
            producer_stage: Some("promote".into()),
            ..BookPipelineArtifact::default()
        });
        manifest_units.push(serde_json::json!({
            "unitId": unit.unit_id.as_str(),
            "translationSha256": unit.translation_sha256.as_str(),
            "chapterControlSha256": control_sha256,
            "finalPath": format!("chapters/final/{}.md", unit.unit_id),
            "finalSha256": final_sha256,
        }));
    }

    let approval_binding_sha256 = input_hashes
        .get("approvalBindingSha256")
        .cloned()
        .ok_or_else(|| "Promotion approval has no binding hash.".to_string())?;
    input_hashes.insert("approvalIdSha256".into(), sha256_str(&approval_id));
    let manifest = serde_json::json!({
        "schema": PROMOTION_MANIFEST_SCHEMA,
        "approvalId": approval_id,
        "approvalBindingSha256": approval_binding_sha256,
        "units": manifest_units,
    });
    let manifest_text =
        serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())? + "\n";
    let manifest_path = project_root.join("qa").join("promotion_manifest.json");
    fs::write(&manifest_path, &manifest_text).map_err(|err| err.to_string())?;
    artifacts.push(BookPipelineArtifact {
        kind: "promotion_manifest".into(),
        path: display_path(&manifest_path),
        sha256: Some(sha256_str(&manifest_text)),
        zotero_key: None,
        producer_stage: Some("promote".into()),
        ..BookPipelineArtifact::default()
    });

    Ok(StageRunOutput {
        artifacts,
        artifact_kinds: vec!["chapter_final", "promotion_manifest"],
        input_hashes,
        log_summary: vec![format!(
            "Promoted {} approved chapter unit(s)",
            approved_unit_ids.len()
        )],
        unit_summary: None,
        error: None,
    })
}

fn prepare_reading_builder(project_root: &Path) -> Result<PathBuf, String> {
    let source_dir = local_reading_repo_root()?
        .join("template")
        .join("epub_pipeline")
        .join("common")
        .join("scripts");
    let target_dir = project_root.join("scripts");
    fs::create_dir_all(&target_dir).map_err(|err| err.to_string())?;
    for file_name in ["build_epub.js", "run_python.js"] {
        let source = source_dir.join(file_name);
        if !source.is_file() {
            return Err(format!(
                "Reading builder dependency is missing at {}",
                display_path(&source)
            ));
        }
        fs::copy(&source, target_dir.join(file_name)).map_err(|err| err.to_string())?;
    }
    Ok(target_dir.join("build_epub.js"))
}

fn build_reading_command(
    child: &BookPipelineChildJob,
    script_path: &Path,
) -> Result<RunnerCommand, String> {
    let project_root = project_root_from_child(child)?;
    let attempts = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "build_reading")
        .map(|stage| stage.attempt)
        .unwrap_or(0);
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: READING_BUILD_COMMAND_LABEL.into(),
        program: PathBuf::from("node"),
        args: vec![display_path(script_path)],
        env: Vec::new(),
        cwd: Some(project_root.clone()),
        output_dir: project_root,
        attempts,
        accepted_exit_codes: vec![0],
    })
}

fn prepare_bilingual_builder(project_root: &Path) -> Result<PathBuf, String> {
    let source = local_reading_repo_root()?
        .join("tools")
        .join("bibliosmith-launcher")
        .join("source")
        .join("scripts")
        .join("build_bilingual_epub.py");
    if !source.is_file() {
        return Err(format!(
            "Bilingual builder is missing at {}",
            display_path(&source)
        ));
    }
    let target_dir = project_root.join("scripts");
    fs::create_dir_all(&target_dir).map_err(|err| err.to_string())?;
    let target = target_dir.join("build_bilingual_epub.py");
    fs::copy(&source, &target).map_err(|err| err.to_string())?;
    Ok(target)
}

fn build_bilingual_command(
    child: &BookPipelineChildJob,
    script_path: &Path,
) -> Result<RunnerCommand, String> {
    let project_root = project_root_from_child(child)?;
    let attempts = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "build_reading")
        .map(|stage| stage.attempt)
        .unwrap_or(0);
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: BILINGUAL_BUILD_COMMAND_LABEL.into(),
        program: PathBuf::from("python3"),
        args: vec![
            display_path(script_path),
            "--book-root".into(),
            display_path(&project_root),
        ],
        env: Vec::new(),
        cwd: Some(project_root.clone()),
        output_dir: project_root,
        attempts,
        accepted_exit_codes: vec![0],
    })
}

fn required_stage_artifact(
    kind: &str,
    path: &Path,
    producer_stage: &str,
) -> Result<BookPipelineArtifact, String> {
    let metadata = fs::metadata(path).map_err(|err| {
        format!(
            "Required {kind} output is missing at {}: {err}",
            display_path(path)
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(format!(
            "Required {kind} output is empty at {}",
            display_path(path)
        ));
    }
    Ok(BookPipelineArtifact {
        kind: kind.into(),
        path: display_path(path),
        sha256: Some(sha256_file(path)?),
        zotero_key: None,
        producer_stage: Some(producer_stage.into()),
        ..BookPipelineArtifact::default()
    })
}

fn run_build_reading_stage(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
    executor: &dyn RunnerCommandExecutor,
) -> Result<StageRunOutput, String> {
    let project_root = project_root_from_child(child)?;
    let (_, _, mut input_hashes) = promotion_approval_input_hashes(job, child)?;
    input_hashes.insert(
        "outputFormatsSha256".into(),
        sha256_str(&serde_json::to_string(&job.output_formats).map_err(|err| err.to_string())?),
    );
    let final_dir = project_root.join("chapters").join("final");
    let mut final_artifacts = child
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind == "chapter_final"
                && artifact.producer_stage.as_deref() == Some("promote")
        })
        .collect::<Vec<_>>();
    final_artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    if final_artifacts.is_empty() {
        return Err("Reading build has no promoted final chapters.".into());
    }

    let mut reading_markdown = String::new();
    for artifact in final_artifacts {
        let path = PathBuf::from(&artifact.path);
        if path.parent() != Some(final_dir.as_path()) {
            return Err(format!(
                "Final chapter artifact is outside chapters/final: {}",
                artifact.path
            ));
        }
        let expected_sha256 = artifact
            .sha256
            .as_ref()
            .ok_or_else(|| format!("Final chapter has no SHA-256: {}", artifact.path))?;
        let actual_sha256 = sha256_file(&path)?;
        if &actual_sha256 != expected_sha256 {
            return Err(format!(
                "Final chapter changed before build: {}",
                artifact.path
            ));
        }
        let unit_id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("Invalid final chapter path: {}", artifact.path))?;
        input_hashes.insert(format!("chapterFinal:{unit_id}"), actual_sha256);
        let text = fs::read_to_string(&path).map_err(|err| err.to_string())?;
        if text.trim().is_empty() {
            return Err(format!("Final chapter is empty: {}", artifact.path));
        }
        if !reading_markdown.is_empty() && !reading_markdown.ends_with("\n\n") {
            if !reading_markdown.ends_with('\n') {
                reading_markdown.push('\n');
            }
            reading_markdown.push('\n');
        }
        reading_markdown.push_str(&text);
        if !reading_markdown.ends_with('\n') {
            reading_markdown.push('\n');
        }
    }

    let wants_markdown = output_format_enabled(job, OUTPUT_FORMAT_MD);
    let wants_html = output_format_enabled(job, OUTPUT_FORMAT_HTML);
    let wants_epub = output_format_enabled(job, OUTPUT_FORMAT_EPUB);
    let wants_bilingual = output_format_enabled(job, OUTPUT_FORMAT_BILINGUAL);
    let reading_dir = project_root.join("output").join("reading");
    let markdown_path = reading_dir.join("book.md");
    let mut artifacts = Vec::new();
    let mut command_results = Vec::new();
    if wants_markdown {
        fs::create_dir_all(&reading_dir).map_err(|err| err.to_string())?;
        fs::write(&markdown_path, &reading_markdown).map_err(|err| err.to_string())?;
        artifacts.push(required_stage_artifact(
            "reading_markdown",
            &markdown_path,
            "build_reading",
        )?);
    } else if markdown_path.is_file() {
        fs::remove_file(&markdown_path).map_err(|err| err.to_string())?;
    }

    let standard_epub_path = project_root.join("output").join("book.epub");
    if wants_html || wants_epub {
        let script_path = prepare_reading_builder(&project_root)?;
        input_hashes.insert("buildScriptSha256".into(), sha256_file(&script_path)?);
        command_results.push(executor.execute(&build_reading_command(child, &script_path)?)?);

        if wants_html {
            let html_dir = project_root.join("output").join("epub_work").join("EPUB");
            let mut html_paths = fs::read_dir(&html_dir)
                .map_err(|err| {
                    format!(
                        "Reading builder produced no semantic HTML directory at {}: {err}",
                        display_path(&html_dir)
                    )
                })?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path.file_name().and_then(|name| name.to_str()) != Some("nav.xhtml")
                        && path
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("xhtml"))
                })
                .collect::<Vec<_>>();
            html_paths.sort();
            if html_paths.is_empty() {
                return Err("Reading builder produced no semantic HTML chapters.".into());
            }
            for path in html_paths {
                artifacts.push(required_stage_artifact(
                    "reading_html",
                    &path,
                    "build_reading",
                )?);
            }
        }
        if wants_epub {
            artifacts.push(required_stage_artifact(
                "reading_epub",
                &standard_epub_path,
                "build_reading",
            )?);
        } else if standard_epub_path.is_file() {
            fs::remove_file(&standard_epub_path).map_err(|err| err.to_string())?;
        }
    } else {
        if standard_epub_path.is_file() {
            fs::remove_file(&standard_epub_path).map_err(|err| err.to_string())?;
        }
        let work_dir = project_root.join("output").join("epub_work");
        if work_dir.is_dir() {
            fs::remove_dir_all(work_dir).map_err(|err| err.to_string())?;
        }
    }

    let bilingual_epub_path = project_root.join("output").join("book_bilingual.epub");
    if wants_bilingual {
        let source_map = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "source_map")
            .ok_or_else(|| "Bilingual build requires the source_map artifact.".to_string())?;
        let source_map_path = PathBuf::from(&source_map.path);
        let source_map_sha256 = sha256_file(&source_map_path)?;
        if source_map.sha256.as_ref() != Some(&source_map_sha256) {
            return Err("Source map changed before bilingual build.".into());
        }
        input_hashes.insert("bilingualSourceMapSha256".into(), source_map_sha256);
        let mut source_chapters = child
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "chapter_source")
            .collect::<Vec<_>>();
        source_chapters.sort_by(|left, right| left.path.cmp(&right.path));
        if source_chapters.is_empty() {
            return Err("Bilingual build requires source chapter artifacts.".into());
        }
        for artifact in source_chapters {
            let path = PathBuf::from(&artifact.path);
            let actual_sha256 = sha256_file(&path)?;
            if artifact.sha256.as_ref() != Some(&actual_sha256) {
                return Err(format!(
                    "Source chapter changed before bilingual build: {}",
                    artifact.path
                ));
            }
            let unit_id = path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("Invalid source chapter path: {}", artifact.path))?;
            input_hashes.insert(format!("chapterSource:{unit_id}"), actual_sha256);
        }
        let bilingual_script = prepare_bilingual_builder(&project_root)?;
        input_hashes.insert(
            "bilingualBuildScriptSha256".into(),
            sha256_file(&bilingual_script)?,
        );
        command_results
            .push(executor.execute(&build_bilingual_command(child, &bilingual_script)?)?);
        artifacts.push(required_stage_artifact(
            "reading_bilingual_epub",
            &bilingual_epub_path,
            "build_reading",
        )?);
    } else {
        if bilingual_epub_path.is_file() {
            fs::remove_file(&bilingual_epub_path).map_err(|err| err.to_string())?;
        }
        let work_dir = project_root.join("output").join("bilingual_epub_work");
        if work_dir.is_dir() {
            fs::remove_dir_all(work_dir).map_err(|err| err.to_string())?;
        }
    }

    let html_count = artifacts
        .iter()
        .filter(|artifact| artifact.kind == "reading_html")
        .count();
    let mut log_summary = if job.output_formats == default_output_formats() {
        vec![format!(
            "Built reading Markdown, {html_count} semantic HTML file(s), and EPUB"
        )]
    } else {
        vec![format!(
            "Built selected reading format(s): {}",
            job.output_formats.join(", ")
        )]
    };
    for command_result in command_results {
        log_summary.extend(redact_log_lines(&command_result.log_summary));
        log_summary.extend(parse_allowlisted_worker_markers(
            &command_result.stderr,
            &[project_root.as_path()],
        ));
        if wants_bilingual {
            log_summary.extend(parse_allowlisted_worker_markers(
                &command_result.stdout,
                &[project_root.as_path()],
            ));
        }
    }
    Ok(StageRunOutput {
        artifacts,
        artifact_kinds: vec![
            "reading_markdown",
            "reading_html",
            "reading_epub",
            "reading_bilingual_epub",
        ],
        input_hashes,
        log_summary,
        unit_summary: None,
        error: None,
    })
}

#[derive(Deserialize, Default)]
struct EpubCheckReport {
    checker: EpubCheckSummary,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct EpubCheckSummary {
    #[serde(default)]
    n_fatal: u32,
    #[serde(default)]
    n_error: u32,
    #[serde(default)]
    n_warning: u32,
}

struct EpubCheckStageResult {
    report_artifact: BookPipelineArtifact,
    summary: EpubCheckSummary,
    log_summary: Vec<String>,
    error: Option<String>,
}

fn book_pipeline_epubcheck_jar_path() -> Result<PathBuf, String> {
    let books_dir = local_reading_repo_root()?.join("books");
    let epubchecker_dir = books_dir.join("node_modules").join("epubchecker");
    let installed_package = epubchecker_dir.join("package.json");
    let version = if installed_package.is_file() {
        let json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&installed_package).map_err(|err| err.to_string())?,
        )
        .map_err(|err| err.to_string())?;
        json.get("epubcheckVersion")
            .and_then(serde_json::Value::as_str)
            .filter(|version| !version.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| "epubchecker package.json has no epubcheckVersion.".to_string())?
    } else {
        let lock_path = books_dir.join("package-lock.json");
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&lock_path).map_err(|err| err.to_string())?)
                .map_err(|err| err.to_string())?;
        json.pointer("/packages/node_modules~1epubchecker/version")
            .and_then(serde_json::Value::as_str)
            .filter(|version| !version.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| "books/package-lock.json has no epubchecker version.".to_string())?
    };
    Ok(epubchecker_dir
        .join("vendors")
        .join(format!("epubcheck-{version}"))
        .join("epubcheck.jar"))
}

fn build_epubcheck_command(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
    epub_path: &Path,
    report_path: &Path,
    stage_id: &str,
) -> Result<RunnerCommand, String> {
    let project_root = project_root_from_child(child)?;
    let jar_path = book_pipeline_epubcheck_jar_path()?;
    let fake_fixture = job.kind != "collection" && job.source.kind == "fake";
    if !fake_fixture && !jar_path.is_file() {
        return Err(format!(
            "EPUBCheck vendor jar is missing at {}",
            display_path(&jar_path)
        ));
    }
    let attempts = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == stage_id)
        .map(|stage| stage.attempt)
        .unwrap_or(0);
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: EPUBCHECK_COMMAND_LABEL.into(),
        program: PathBuf::from("java"),
        args: vec![
            "-jar".into(),
            display_path(&jar_path),
            display_path(epub_path),
            "--json".into(),
            display_path(report_path),
            "-q".into(),
        ],
        env: Vec::new(),
        cwd: Some(project_root.clone()),
        output_dir: project_root,
        attempts,
        accepted_exit_codes: vec![0, 1],
    })
}

fn run_epubcheck(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
    executor: &dyn RunnerCommandExecutor,
    epub_path: &Path,
    report_path: &Path,
    stage_id: &str,
    artifact_kind: &str,
) -> Result<EpubCheckStageResult, String> {
    if report_path.is_file() {
        fs::remove_file(report_path).map_err(|err| err.to_string())?;
    }
    let command = build_epubcheck_command(job, child, epub_path, report_path, stage_id)?;
    let command_result = executor.execute(&command)?;
    let report_artifact = required_stage_artifact(artifact_kind, report_path, stage_id)?;
    let report: EpubCheckReport =
        serde_json::from_str(&fs::read_to_string(report_path).map_err(|err| err.to_string())?)
            .map_err(|err| format!("EPUBCheck returned an invalid JSON report: {err}"))?;
    let passed = report.checker.n_fatal == 0 && report.checker.n_error == 0;
    let mut log_summary = vec![format!(
        "EPUBCheck reported fatal={}, error={}, warning={}",
        report.checker.n_fatal, report.checker.n_error, report.checker.n_warning
    )];
    log_summary.extend(redact_log_lines(&command_result.log_summary));
    log_summary.extend(parse_allowlisted_worker_markers(
        &command_result.stderr,
        &[command.output_dir.as_path()],
    ));
    let error = (!passed).then(|| {
        format!(
            "EPUBCheck reported {} fatal finding(s) and {} error(s).",
            report.checker.n_fatal, report.checker.n_error
        )
    });
    Ok(EpubCheckStageResult {
        report_artifact,
        summary: report.checker,
        log_summary,
        error,
    })
}

fn validated_reading_artifacts(
    child: &BookPipelineChildJob,
    kind: &str,
) -> Result<Vec<(PathBuf, String)>, String> {
    let mut artifacts = child
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind == kind && artifact.producer_stage.as_deref() == Some("build_reading")
        })
        .map(|artifact| {
            let path = PathBuf::from(&artifact.path);
            let expected_sha256 = artifact
                .sha256
                .as_ref()
                .ok_or_else(|| format!("{kind} artifact has no SHA-256: {}", artifact.path))?;
            let actual_sha256 = sha256_file(&path)?;
            if &actual_sha256 != expected_sha256 {
                return Err(format!(
                    "{kind} artifact changed before validation: {}",
                    artifact.path
                ));
            }
            Ok((path, actual_sha256))
        })
        .collect::<Result<Vec<_>, String>>()?;
    artifacts.sort_by(|left, right| left.0.cmp(&right.0));
    if artifacts.is_empty() {
        return Err(format!("Required {kind} artifact is missing."));
    }
    Ok(artifacts)
}

/// One line per recorded reader, inside the generated section so the report tells
/// the same story the job state does. A stale record is shown as stale rather
/// than hidden: knowing someone checked an older build is worth more than a
/// silent gap, which is what hand-writing these lines used to leave behind.
fn reading_validation_reader_lines(evidence: &[BookPipelineReaderEvidence]) -> Vec<String> {
    if evidence.is_empty() {
        return vec!["- reader verification: not recorded".into()];
    }
    evidence
        .iter()
        .map(|record| {
            let staleness = if record.stale {
                " (stale: the artifact changed since)"
            } else {
                ""
            };
            format!(
                "- reader verification: {} {} on {} — {} [sha256 {}]{staleness}",
                record.reader,
                record.reader_version,
                record.artifact_kind,
                record.conclusion,
                record.artifact_sha256,
            )
        })
        .collect()
}

fn write_reading_validation_status(
    project_root: &Path,
    summary: &EpubCheckSummary,
    passed: bool,
    reader_evidence: &[BookPipelineReaderEvidence],
) -> Result<PathBuf, String> {
    let path = project_root.join("qa").join("status.md");
    let existing = fs::read_to_string(&path).unwrap_or_else(|_| "# QA Status\n".into());
    let reading_status = if passed { "passed" } else { "failed" };
    let mut lines = Vec::new();
    let mut in_previous_section = false;
    let mut replaced_reading_status = false;
    for line in existing.lines() {
        if line == READING_VALIDATION_STATUS_START {
            in_previous_section = true;
            continue;
        }
        if in_previous_section {
            if line == READING_VALIDATION_STATUS_END {
                in_previous_section = false;
            }
            continue;
        }
        if line.starts_with("- reading output:") {
            lines.push(format!("- reading output: {reading_status}"));
            replaced_reading_status = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !replaced_reading_status {
        lines.push(format!("- reading output: {reading_status}"));
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let residual_risk = if !passed {
        "none accepted; EPUBCheck fatal/error findings remain blocking".to_string()
    } else if summary.n_warning == 0 {
        "none".to_string()
    } else {
        format!(
            "{} EPUBCheck warning(s), accepted for local reading output",
            summary.n_warning
        )
    };
    lines.extend([
        String::new(),
        READING_VALIDATION_STATUS_START.into(),
        format!(
            "- EPUBCheck: fatal={}, error={}, warning={}",
            summary.n_fatal, summary.n_error, summary.n_warning
        ),
    ]);
    lines.extend(reading_validation_reader_lines(reader_evidence));
    lines.extend([
        format!("- accepted residual risks: {residual_risk}"),
        READING_VALIDATION_STATUS_END.into(),
    ]);
    fs::write(&path, lines.join("\n") + "\n").map_err(|err| err.to_string())?;
    Ok(path)
}

fn run_validate_reading_stage(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
    executor: &dyn RunnerCommandExecutor,
) -> Result<StageRunOutput, String> {
    let project_root = project_root_from_child(child)?;
    let mut input_hashes = BTreeMap::new();
    if output_format_enabled(job, OUTPUT_FORMAT_MD) {
        let markdown = validated_reading_artifacts(child, "reading_markdown")?;
        if markdown.len() != 1 {
            return Err("Reading validation requires exactly one Markdown artifact.".into());
        }
        input_hashes.insert("readingMarkdownSha256".into(), markdown[0].1.clone());
    }
    if output_format_enabled(job, OUTPUT_FORMAT_HTML) {
        let html = validated_reading_artifacts(child, "reading_html")?;
        for (index, (_, sha256)) in html.iter().enumerate() {
            input_hashes.insert(format!("readingHtml:{index:03}"), sha256.clone());
        }
    }

    let mut artifacts = Vec::new();
    let mut aggregate = EpubCheckSummary::default();
    let mut log_summary = Vec::new();
    let mut checked_epubs = 0;
    for (format, artifact_kind, input_key, report_name, report_kind) in [
        (
            OUTPUT_FORMAT_EPUB,
            "reading_epub",
            "readingEpubSha256",
            "epubcheck.json",
            "epubcheck_report",
        ),
        (
            OUTPUT_FORMAT_BILINGUAL,
            "reading_bilingual_epub",
            "readingBilingualEpubSha256",
            "epubcheck_bilingual.json",
            "bilingual_epubcheck_report",
        ),
    ] {
        if !output_format_enabled(job, format) {
            continue;
        }
        let epub = validated_reading_artifacts(child, artifact_kind)?;
        if epub.len() != 1 {
            return Err(format!(
                "Reading validation requires exactly one {artifact_kind} artifact."
            ));
        }
        input_hashes.insert(input_key.into(), epub[0].1.clone());
        let report_path = project_root.join("output").join(report_name);
        let epubcheck = run_epubcheck(
            job,
            child,
            executor,
            &epub[0].0,
            &report_path,
            "validate_reading",
            report_kind,
        )?;
        aggregate.n_fatal = aggregate.n_fatal.saturating_add(epubcheck.summary.n_fatal);
        aggregate.n_error = aggregate.n_error.saturating_add(epubcheck.summary.n_error);
        aggregate.n_warning = aggregate
            .n_warning
            .saturating_add(epubcheck.summary.n_warning);
        artifacts.push(epubcheck.report_artifact);
        log_summary.extend(epubcheck.log_summary);
        checked_epubs += 1;
    }
    if checked_epubs == 0 {
        log_summary
            .push("No EPUB format selected; Markdown/HTML artifact validation passed".into());
    }
    let passed = aggregate.n_fatal == 0 && aggregate.n_error == 0;
    let status_path =
        write_reading_validation_status(&project_root, &aggregate, passed, &child.reader_evidence)?;
    let status_artifact = required_stage_artifact("qa_status", &status_path, "validate_reading")?;
    artifacts.push(status_artifact);
    let error = (!passed).then(|| {
        format!(
            "EPUBCheck reported {} fatal finding(s) and {} error(s).",
            aggregate.n_fatal, aggregate.n_error
        )
    });
    Ok(StageRunOutput {
        artifacts,
        artifact_kinds: vec![
            "epubcheck_report",
            "bilingual_epubcheck_report",
            "qa_status",
        ],
        input_hashes,
        log_summary,
        unit_summary: None,
        error,
    })
}

fn validated_stage_artifact(
    child: &BookPipelineChildJob,
    kind: &str,
    producer_stage: &str,
) -> Result<(PathBuf, String), String> {
    let mut artifacts = child.artifacts.iter().filter(|artifact| {
        artifact.kind == kind && artifact.producer_stage.as_deref() == Some(producer_stage)
    });
    let artifact = artifacts
        .next()
        .ok_or_else(|| format!("Required {kind} artifact is missing."))?;
    if artifacts.next().is_some() {
        return Err(format!("Expected exactly one {kind} artifact."));
    }
    let expected_sha256 = artifact
        .sha256
        .as_ref()
        .ok_or_else(|| format!("{kind} artifact has no SHA-256: {}", artifact.path))?;
    let path = PathBuf::from(&artifact.path);
    let actual_sha256 = sha256_file(&path)?;
    if &actual_sha256 != expected_sha256 {
        return Err(format!(
            "{kind} artifact changed before Digest build: {}",
            artifact.path
        ));
    }
    Ok((path, actual_sha256))
}

fn simple_book_metadata_value(text: &str, keys: &[&str]) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if !keys.iter().any(|candidate| key.trim() == *candidate) {
            return None;
        }
        let value = value.trim();
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        (!value.trim().is_empty()).then(|| value.trim().to_string())
    })
}

fn digest_project_metadata(
    job: &BookPipelineJob,
    project_root: &Path,
) -> Result<(String, String, String, Option<String>), String> {
    let manifest_path = project_root.join("metadata").join("source_manifest.json");
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|err| {
        format!(
            "Digest project metadata is missing at {}: {err}",
            display_path(&manifest_path)
        )
    })?;
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_text).map_err(|err| err.to_string())?;
    let manifest_language = manifest
        .get("target_language")
        .and_then(serde_json::Value::as_str)
        .filter(|language| !language.trim().is_empty())
        .ok_or_else(|| "Digest project metadata has no target_language.".to_string())?;
    let book_metadata_path = project_root.join("metadata").join("book.yaml");
    let book_metadata = fs::read_to_string(&book_metadata_path).ok();
    let title = book_metadata
        .as_deref()
        .and_then(|text| simple_book_metadata_value(text, &["title", "title_zh", "title_zh_hans"]))
        .unwrap_or_else(|| source_title(&job.source));
    let language = book_metadata
        .as_deref()
        .and_then(|text| simple_book_metadata_value(text, &["language"]))
        .unwrap_or_else(|| manifest_language.to_string());
    Ok((
        title,
        language,
        sha256_str(&manifest_text),
        book_metadata.as_deref().map(sha256_str),
    ))
}

fn build_digest_command(child: &BookPipelineChildJob) -> Result<RunnerCommand, String> {
    let project_root = project_root_from_child(child)?;
    let attempts = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "build_digest")
        .map(|stage| stage.attempt)
        .unwrap_or(0);
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: DIGEST_BUILD_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args: vec![
            "run".into(),
            "--package".into(),
            "digest".into(),
            "python".into(),
            "-m".into(),
            "digest.bibliosmith_digest".into(),
            "--book-root".into(),
            display_path(&project_root),
        ],
        env: Vec::new(),
        cwd: Some(local_reading_repo_root()?),
        output_dir: project_root,
        attempts,
        accepted_exit_codes: vec![0],
    })
}

fn run_build_digest_stage(
    job: &BookPipelineJob,
    child: &BookPipelineChildJob,
    executor: &dyn RunnerCommandExecutor,
) -> Result<StageRunOutput, String> {
    if !job.digest_mode {
        return Err("Digest stage cannot run when digestMode is disabled.".into());
    }
    if !child
        .stages
        .iter()
        .any(|stage| stage.stage_id == "validate_reading" && stage.status == STATUS_COMPLETED)
    {
        return Err("Digest build requires completed reading validation.".into());
    }
    let project_root = project_root_from_child(child)?;
    let (source_epub, source_epub_sha256) =
        validated_stage_artifact(child, "reading_epub", "build_reading")?;
    let (_, epubcheck_report_sha256) =
        validated_stage_artifact(child, "epubcheck_report", "validate_reading")?;
    let expected_source_epub = project_root.join("output").join("book.epub");
    let output_epub = project_root.join("output").join("book_digest.epub");
    if source_epub != expected_source_epub {
        return Err(format!(
            "Digest source EPUB must be output/book.epub, found {}",
            display_path(&source_epub)
        ));
    }
    if source_epub == output_epub {
        return Err("Digest output EPUB must not overwrite the source EPUB.".into());
    }

    let (title, language, source_manifest_sha256, book_metadata_sha256) =
        digest_project_metadata(job, &project_root)?;
    let config_path = project_root.join("digest.config.json");
    let mut config = if config_path.is_file() {
        serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(&config_path).map_err(|err| err.to_string())?,
        )
        .map_err(|err| format!("Existing digest.config.json is invalid: {err}"))?
    } else {
        serde_json::json!({})
    };
    let config_object = config
        .as_object_mut()
        .ok_or_else(|| "Existing digest.config.json must contain a JSON object.".to_string())?;
    for (key, value) in [
        ("enabled", serde_json::json!(true)),
        ("merge_into_epub", serde_json::json!(true)),
        ("source_epub", serde_json::json!("output/book.epub")),
        ("output_epub", serde_json::json!("output/book_digest.epub")),
        ("title", serde_json::json!(title)),
        ("language", serde_json::json!(language)),
    ] {
        config_object.insert(key.into(), value);
    }
    let config_text = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())? + "\n";
    fs::write(&config_path, &config_text).map_err(|err| err.to_string())?;

    let outputs = [
        output_epub.clone(),
        project_root.join("output/digest/digest.xhtml"),
        project_root.join("output/digest/knowledge_map.svg"),
        project_root.join("qa/digest/digest_review_checklist.md"),
        project_root.join("qa/digest/digest_report.json"),
    ];
    for path in &outputs {
        if path.is_file() {
            fs::remove_file(path).map_err(|err| err.to_string())?;
        }
    }

    let command_result = executor.execute(&build_digest_command(child)?)?;
    let report_text = fs::read_to_string(&outputs[4])
        .map_err(|err| format!("Digest report is missing or unreadable: {err}"))?;
    let report: serde_json::Value = serde_json::from_str(&report_text)
        .map_err(|err| format!("Digest returned an invalid JSON report: {err}"))?;
    if report.get("status").and_then(serde_json::Value::as_str) != Some("PASS") {
        return Err("Digest report status is not PASS.".into());
    }
    if report.get("merged").and_then(serde_json::Value::as_bool) != Some(true)
        || report
            .get("source_epub")
            .and_then(serde_json::Value::as_str)
            != Some("output/book.epub")
        || report
            .get("output_epub")
            .and_then(serde_json::Value::as_str)
            != Some("output/book_digest.epub")
    {
        return Err("Digest report does not confirm the requested EPUB merge.".into());
    }

    let epubcheck_report_path = project_root.join("output").join("digest_epubcheck.json");
    let epubcheck = run_epubcheck(
        job,
        child,
        executor,
        &output_epub,
        &epubcheck_report_path,
        "build_digest",
        "digest_epubcheck_report",
    )?;

    let mut artifacts = [
        ("digest_epub", outputs[0].as_path()),
        ("digest_xhtml", outputs[1].as_path()),
        ("digest_knowledge_map", outputs[2].as_path()),
        ("digest_review_checklist", outputs[3].as_path()),
        ("digest_report", outputs[4].as_path()),
    ]
    .into_iter()
    .map(|(kind, path)| required_stage_artifact(kind, path, "build_digest"))
    .collect::<Result<Vec<_>, _>>()?;
    artifacts.push(epubcheck.report_artifact);
    let mut input_hashes = BTreeMap::new();
    input_hashes.insert("readingEpubSha256".into(), source_epub_sha256);
    input_hashes.insert("epubcheckReportSha256".into(), epubcheck_report_sha256);
    input_hashes.insert("sourceManifestSha256".into(), source_manifest_sha256);
    if let Some(book_metadata_sha256) = book_metadata_sha256 {
        input_hashes.insert("bookMetadataSha256".into(), book_metadata_sha256);
    }
    input_hashes.insert("digestConfigSha256".into(), sha256_str(&config_text));
    let mut log_summary = vec!["Built merged Digest EPUB with PASS report".into()];
    log_summary.extend(redact_log_lines(&command_result.log_summary));
    log_summary.extend(parse_allowlisted_worker_markers(
        &command_result.stderr,
        &[project_root.as_path()],
    ));
    log_summary.extend(epubcheck.log_summary);

    Ok(StageRunOutput {
        artifacts,
        artifact_kinds: vec![
            "digest_epub",
            "digest_xhtml",
            "digest_knowledge_map",
            "digest_review_checklist",
            "digest_report",
            "digest_epubcheck_report",
        ],
        input_hashes,
        log_summary,
        unit_summary: None,
        error: epubcheck.error,
    })
}

fn current_file_sha256(path: &Path) -> String {
    sha256_file(path).unwrap_or_else(|_| "missing".into())
}

fn invalidate_stale_expert_qa(
    job: &mut BookPipelineJob,
    child_index: usize,
) -> Result<bool, String> {
    let child = job
        .children
        .get(child_index)
        .ok_or_else(|| "Book Pipeline child not found.".to_string())?;
    let Some(qa_stage) = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "expert_qa")
        .filter(|stage| stage.status == STATUS_COMPLETED)
    else {
        return Ok(false);
    };
    let project_root = project_root_from_child(child)?;
    let units = translation_task_units(child, &project_root)?;
    let mut invalidated = BTreeMap::new();
    let handoff_sha256 = current_file_sha256(
        &project_root
            .join("qa")
            .join("handoffs")
            .join("expert_qa.json"),
    );
    let handoff_stale = qa_stage.input_hashes.get("expertQaHandoffSha256") != Some(&handoff_sha256);
    for unit in &units {
        let translation_path = project_root
            .join("chapters")
            .join("translated")
            .join(format!("{}.md", unit.unit_id));
        let control_path = project_root
            .join("qa")
            .join("chapter_controls")
            .join(format!("{}.json", unit.unit_id));
        let translation_sha256 = current_file_sha256(&translation_path);
        let control_sha256 = current_file_sha256(&control_path);
        if handoff_stale
            || qa_stage
                .input_hashes
                .get(&format!("translation:{}", unit.unit_id))
                != Some(&translation_sha256)
            || qa_stage
                .input_hashes
                .get(&format!("chapterControl:{}", unit.unit_id))
                != Some(&control_sha256)
        {
            invalidated.insert(unit.unit_id.clone(), translation_sha256);
        }
    }
    if invalidated.is_empty() {
        return Ok(false);
    }
    let child_id = child.id.clone();
    let total = units.len() as u32;
    let invalidated_count = invalidated.len() as u32;
    let child = &mut job.children[child_index];
    invalidate_completed_downstream(child, "expert_qa");
    if let Some(stage) = stage_mut(child, "expert_qa") {
        stage.status = STATUS_READY.into();
        stage.error = None;
        stage.finished_at = None;
        stage.execution_owner = None;
        stage.input_hashes = invalidated
            .into_iter()
            .map(|(unit_id, sha256)| (format!("invalidatedUnit:{unit_id}"), sha256))
            .collect();
        stage.unit_summary = Some(BookPipelineUnitSummary {
            total,
            ready: invalidated_count,
            completed: total.saturating_sub(invalidated_count),
            ..BookPipelineUnitSummary::default()
        });
    }
    job.approval_references.retain(|approval| {
        approval.child_job_id != child_id || approval.stage_id != "approve_promotion"
    });
    Ok(true)
}

fn deterministic_stage_to_run(child: &BookPipelineChildJob) -> Option<&'static str> {
    for stage in &child.stages {
        match stage.status.as_str() {
            STATUS_COMPLETED | STATUS_SKIPPED => continue,
            status => {
                return match stage.stage_id.as_str() {
                    "split" if matches!(status, STATUS_READY | STATUS_FAILED) => Some("split"),
                    "prepare"
                        if matches!(status, STATUS_READY | STATUS_FAILED | STATUS_PENDING) =>
                    {
                        Some("prepare")
                    }
                    "translate"
                        if matches!(status, STATUS_READY | STATUS_FAILED)
                            || (status == STATUS_BLOCKED && is_agent_handoff_waiting(stage)) =>
                    {
                        Some("translate")
                    }
                    "expert_qa"
                        if matches!(status, STATUS_PENDING | STATUS_READY | STATUS_FAILED)
                            || (status == STATUS_BLOCKED && is_agent_handoff_waiting(stage)) =>
                    {
                        Some("expert_qa")
                    }
                    "promote"
                        if matches!(status, STATUS_PENDING | STATUS_READY | STATUS_FAILED) =>
                    {
                        Some("promote")
                    }
                    "build_reading"
                        if matches!(status, STATUS_PENDING | STATUS_READY | STATUS_FAILED) =>
                    {
                        Some("build_reading")
                    }
                    "validate_reading"
                        if matches!(status, STATUS_PENDING | STATUS_READY | STATUS_FAILED) =>
                    {
                        Some("validate_reading")
                    }
                    "build_digest"
                        if matches!(status, STATUS_PENDING | STATUS_READY | STATUS_FAILED) =>
                    {
                        Some("build_digest")
                    }
                    _ => None,
                };
            }
        }
    }
    None
}

enum SplitFreshnessAction {
    Block,
    InvalidateForRerun,
    InvalidateDownstreamAndRerun,
}

struct SplitFreshnessChange {
    action: SplitFreshnessAction,
    new_source_hash: String,
    stop_after: bool,
}

/// Detect whether a completed (or already blocked) split still matches the
/// current source Markdown. A changed fingerprint invalidates downstream work:
/// with no committed downstream stage the split re-runs automatically, with a
/// completed downstream stage it becomes `split_blocked` until the caller
/// approves invalidation.
fn evaluate_split_freshness(
    child: &BookPipelineChildJob,
    invalidate_downstream: bool,
) -> Result<Option<SplitFreshnessChange>, String> {
    let Some(split) = child.stages.iter().find(|stage| stage.stage_id == "split") else {
        return Ok(None);
    };
    if !matches!(split.status.as_str(), STATUS_COMPLETED | STATUS_BLOCKED) {
        return Ok(None);
    }
    let source_md = project_root_from_child(child)?
        .join("source")
        .join("source.md");
    if !source_md.is_file() {
        return Ok(None);
    }
    let current = sha256_file(&source_md)?;

    let split_order = ordered_stage_index("split").unwrap_or(0);
    let downstream = || {
        child.stages.iter().filter(|stage| {
            ordered_stage_index(&stage.stage_id).is_some_and(|order| order > split_order)
        })
    };
    let downstream_active = downstream().any(|stage| {
        let invalidatable_translation_gate = stage.stage_id == "approve_translation"
            && matches!(
                stage.status.as_str(),
                STATUS_READY | STATUS_WAITING_FOR_APPROVAL
            );
        !invalidatable_translation_gate
            && matches!(
                stage.status.as_str(),
                STATUS_READY
                    | STATUS_RUNNING
                    | STATUS_WAITING_FOR_APPROVAL
                    | STATUS_FAILED
                    | STATUS_BLOCKED
            )
    });
    if downstream_active {
        return Ok(None);
    }
    let downstream_committed = downstream().any(|stage| stage.status == STATUS_COMPLETED);

    if split.status == STATUS_BLOCKED {
        return Ok(invalidate_downstream.then_some(SplitFreshnessChange {
            action: SplitFreshnessAction::InvalidateDownstreamAndRerun,
            new_source_hash: current,
            stop_after: false,
        }));
    }
    if split.input_hashes.get("sourceMarkdownSha256") == Some(&current) {
        return Ok(None);
    }
    let action = if invalidate_downstream {
        SplitFreshnessAction::InvalidateDownstreamAndRerun
    } else if downstream_committed {
        SplitFreshnessAction::Block
    } else {
        SplitFreshnessAction::InvalidateForRerun
    };
    let stop_after = matches!(action, SplitFreshnessAction::Block);
    Ok(Some(SplitFreshnessChange {
        action,
        new_source_hash: current,
        stop_after,
    }))
}

fn invalidate_completed_downstream(child: &mut BookPipelineChildJob, after_stage: &str) {
    let after_order = ordered_stage_index(after_stage).unwrap_or(0);
    for stage in &mut child.stages {
        let is_downstream =
            ordered_stage_index(&stage.stage_id).is_some_and(|order| order > after_order);
        let invalidatable_approval_gate = stage.stage_id.starts_with("approve_")
            && matches!(
                stage.status.as_str(),
                STATUS_READY | STATUS_WAITING_FOR_APPROVAL
            );
        if is_downstream && (stage.status == STATUS_COMPLETED || invalidatable_approval_gate) {
            stage.input_hashes.clear();
            stage.status = STATUS_PENDING.into();
            stage.error = None;
            stage.finished_at = None;
            stage.approval_id = None;
            stage.approval_request = None;
            stage.execution_owner = None;
        }
    }
}

fn apply_split_freshness(child: &mut BookPipelineChildJob, change: &SplitFreshnessChange) {
    // Blocking and explicit invalidation both roll back completed downstream
    // stages so the re-blocked/re-readied split stays a valid ordered state; the
    // rollback is a no-op when nothing downstream was committed. Generated files
    // stay on disk and are overwritten only when split actually re-runs.
    invalidate_completed_downstream(child, "split");
    if let Some(split) = stage_mut(child, "split") {
        split.input_hashes.insert(
            "sourceMarkdownSha256".into(),
            change.new_source_hash.clone(),
        );
    }
    match change.action {
        SplitFreshnessAction::Block => {
            set_stage_status(
                child,
                "split",
                STATUS_BLOCKED,
                Some(SPLIT_BLOCKED_ERROR.into()),
            );
            child.last_error = Some(SPLIT_BLOCKED_ERROR.into());
        }
        SplitFreshnessAction::InvalidateForRerun
        | SplitFreshnessAction::InvalidateDownstreamAndRerun => {
            set_stage_status(child, "split", STATUS_READY, None);
            child.last_error = None;
        }
    }
}

fn find_job_index(state: &BookPipelineState, job_id: &str) -> Result<usize, String> {
    state
        .jobs
        .iter()
        .position(|job| job.id == job_id)
        .ok_or_else(|| "Book Pipeline job not found.".to_string())
}

fn locate_child_index(job: &BookPipelineJob, child_id: Option<&str>) -> Result<usize, String> {
    match child_id {
        Some(child_id) => job
            .children
            .iter()
            .position(|child| child.id == child_id)
            .ok_or_else(|| "Book Pipeline child not found.".to_string()),
        None => job
            .children
            .iter()
            .position(|child| deterministic_stage_to_run(child).is_some())
            .or_else(|| (!job.children.is_empty()).then_some(0))
            .ok_or_else(|| "This job has no child to advance.".to_string()),
    }
}

/// Advance a child through the deterministic post-handoff stages. Real jobs
/// stop at either hash-bound human approval gate. The narrowly scoped fake
/// provider fixture may pre-approve translation disclosure, but promotion is
/// always left ready for an explicit second approval.
fn advance_job(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    invalidate_downstream: bool,
) -> Result<BookPipelineJob, String> {
    advance_job_with_executor(
        store,
        job_id,
        child_id,
        invalidate_downstream,
        &SystemCommandExecutor,
    )
}

fn advance_job_with_executor(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    invalidate_downstream: bool,
    executor: &dyn RunnerCommandExecutor,
) -> Result<BookPipelineJob, String> {
    advance_job_stage(
        store,
        job_id,
        child_id,
        invalidate_downstream,
        executor,
        false,
    )
}

/// `retrying_stage` re-enters for an automatic retry, which must re-run *only*
/// the stage that failed. The freshness and gate phases below are the work of
/// starting an advance, not of retrying one: Phase 0 rolls completed downstream
/// stages back to pending, so running it again per retry would re-open the
/// stage that had just succeeded and the retry would never converge.
fn advance_job_stage(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    invalidate_downstream: bool,
    executor: &dyn RunnerCommandExecutor,
    retrying_stage: bool,
) -> Result<BookPipelineJob, String> {
    // Phase 0: re-evaluate a completed/blocked split against the current source.
    if !retrying_stage {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
        if let Some(change) = evaluate_split_freshness(
            &state.jobs[job_index].children[child_index],
            invalidate_downstream,
        )? {
            let stop_after = change.stop_after;
            apply_split_freshness(&mut state.jobs[job_index].children[child_index], &change);
            state.jobs[job_index].updated_at = now_label();
            derive_job(&mut state.jobs[job_index]);
            let job = state.jobs[job_index].clone();
            store.save(&state)?;
            if stop_after {
                return Ok(job);
            }
        }
    }

    // Phase 0.25: any translation/control hash drift after a completed QA pass
    // reopens only the affected QA units and invalidates promotion approval.
    if !retrying_stage {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
        if invalidate_stale_expert_qa(&mut state.jobs[job_index], child_index)? {
            state.jobs[job_index].current_step = "Expert QA binding invalidated".into();
            state.jobs[job_index]
                .log_summary
                .push("Expert QA binding invalidated by artifact hash change".into());
            state.jobs[job_index].log_summary =
                trim_log_summary(&state.jobs[job_index].log_summary);
            state.jobs[job_index].updated_at = now_label();
            derive_job(&mut state.jobs[job_index]);
            store.save(&state)?;
        }
    }

    // Phase 0.5: ready a prepared gate, including refreshing a stale binding
    // after a mode/profile/config change. Only the explicit fake fixture scope
    // can pre-approve the request.
    if !retrying_stage {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
        if ready_translation_approval_gate(&mut state.jobs[job_index], child_index) {
            state.jobs[job_index].current_step = "Translation approval review ready".into();
            state.jobs[job_index]
                .log_summary
                .push("Translation approval review ready".into());
            state.jobs[job_index].log_summary =
                trim_log_summary(&state.jobs[job_index].log_summary);
            state.jobs[job_index].updated_at = now_label();
            derive_job(&mut state.jobs[job_index]);
            let job = state.jobs[job_index].clone();
            store.save(&state)?;
            if should_auto_approve_translation(&job, child_index) {
                return advance_job_with_executor(
                    store,
                    job_id,
                    child_id,
                    invalidate_downstream,
                    executor,
                );
            }
            return Ok(job);
        }
        if should_auto_approve_translation(&state.jobs[job_index], child_index)
            && approve_translation_gate(&mut state.jobs[job_index], child_index)
        {
            state.jobs[job_index].current_step = "Fake translation approval recorded".into();
            state.jobs[job_index]
                .log_summary
                .push("Fake translation approval recorded".into());
            state.jobs[job_index].log_summary =
                trim_log_summary(&state.jobs[job_index].log_summary);
            state.jobs[job_index].updated_at = now_label();
            derive_job(&mut state.jobs[job_index]);
            store.save(&state)?;
        }
    }

    // Phase 0.75: the second human gate is readied only after exact translated
    // artifacts and PASS chapter-control hashes satisfy the selected QA policy.
    if !retrying_stage {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
        if ready_promotion_approval_gate(&mut state.jobs[job_index], child_index) {
            state.jobs[job_index].current_step = "Promotion approval review ready".into();
            state.jobs[job_index]
                .log_summary
                .push("Promotion approval review ready".into());
            state.jobs[job_index].log_summary =
                trim_log_summary(&state.jobs[job_index].log_summary);
            state.jobs[job_index].updated_at = now_label();
            derive_job(&mut state.jobs[job_index]);
            let job = state.jobs[job_index].clone();
            store.save(&state)?;
            return Ok(job);
        }
    }

    // Phase 1: mark the next deterministic stage running (persisted checkpoint).
    let (stage_id, running_job, running_child) = {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
        let Some(stage_id) =
            deterministic_stage_to_run(&state.jobs[job_index].children[child_index])
        else {
            return Ok(state.jobs[job_index].clone());
        };
        let execution_owner = store.execution_owner()?;
        {
            let child = &mut state.jobs[job_index].children[child_index];
            start_stage(child, stage_id, execution_owner);
            child.attempts = child.attempts.saturating_add(1);
        }
        state.jobs[job_index].current_step = format!("Running {stage_id} stage");
        state.jobs[job_index].last_error = None;
        state.jobs[job_index].updated_at = now_label();
        derive_job(&mut state.jobs[job_index]);
        let running_job = state.jobs[job_index].clone();
        let running_child = state.jobs[job_index].children[child_index].clone();
        store.save(&state)?;
        (stage_id, running_job, running_child)
    };

    // Phase 2: run the deterministic work off the persisted running snapshot.
    let result: Result<StageRunOutcome, String> = match stage_id {
        "split" => run_split_stage(&running_child).map(StageRunOutcome::finished),
        "prepare" => run_prepare_stage(&running_child).map(StageRunOutcome::finished),
        "translate" if running_job.translation_mode == TRANSLATION_MODE_EXPERT => {
            run_expert_translate_stage(&running_job, &running_child)
        }
        "translate" => run_translate_stage(&running_job, &running_child, executor)
            .map(StageRunOutcome::finished),
        "expert_qa" => run_expert_qa_stage(&running_job, &running_child),
        "promote" => run_promote_stage(&running_job, &running_child).map(StageRunOutcome::finished),
        "build_reading" => run_build_reading_stage(&running_job, &running_child, executor)
            .map(StageRunOutcome::finished),
        "validate_reading" => run_validate_reading_stage(&running_job, &running_child, executor)
            .map(StageRunOutcome::finished),
        "build_digest" => run_build_digest_stage(&running_job, &running_child, executor)
            .map(StageRunOutcome::finished),
        other => Err(format!("Stage {other} is not runnable in this slice.")),
    };

    // Phase 3: record completion or a redacted failure against the exact child
    // Phase 1 started, not a re-resolved heuristic pick.
    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], Some(&running_child.id))?;
    match result {
        Ok(outcome) => {
            let (output, waiting_agent_profile) = match outcome {
                StageRunOutcome::Finished(output) => (output, None),
                StageRunOutcome::WaitingForAgent {
                    output,
                    agent_profile_id,
                } => (output, Some(agent_profile_id)),
            };
            let StageRunOutput {
                artifacts,
                artifact_kinds,
                input_hashes,
                log_summary,
                unit_summary,
                error,
            } = output;
            let stage_error = error.as_deref().map(redact_runner_message);
            {
                let child = &mut state.jobs[job_index].children[child_index];
                if let Some(stage) = stage_mut(child, stage_id) {
                    stage.input_hashes = input_hashes;
                    stage.unit_summary = unit_summary;
                }
                if stage_id == "translate" {
                    replace_translation_stage_artifacts(&mut child.artifacts, artifacts);
                    let artifact_ids = child
                        .artifacts
                        .iter()
                        .filter(|artifact| artifact.producer_stage.as_deref() == Some("translate"))
                        .map(|artifact| artifact.path.clone())
                        .collect();
                    if let Some(stage) = stage_mut(child, stage_id) {
                        stage.artifact_ids = artifact_ids;
                    }
                } else if stage_id == "expert_qa" {
                    replace_expert_qa_stage_artifacts(
                        &mut child.artifacts,
                        &artifact_kinds,
                        artifacts,
                    );
                    let artifact_ids = child
                        .artifacts
                        .iter()
                        .filter(|artifact| artifact.producer_stage.as_deref() == Some("expert_qa"))
                        .map(|artifact| artifact.path.clone())
                        .collect();
                    if let Some(stage) = stage_mut(child, stage_id) {
                        stage.artifact_ids = artifact_ids;
                    }
                } else {
                    replace_stage_artifacts(&mut child.artifacts, &artifact_kinds, artifacts);
                    if matches!(
                        stage_id,
                        "promote" | "build_reading" | "validate_reading" | "build_digest"
                    ) {
                        let artifact_ids = child
                            .artifacts
                            .iter()
                            .filter(|artifact| artifact.producer_stage.as_deref() == Some(stage_id))
                            .map(|artifact| artifact.path.clone())
                            .collect();
                        if let Some(stage) = stage_mut(child, stage_id) {
                            stage.artifact_ids = artifact_ids;
                        }
                    }
                }
                if let Some(agent_profile_id) = &waiting_agent_profile {
                    set_agent_handoff_waiting(child, stage_id, agent_profile_id);
                    child.last_error = None;
                } else if let Some(error) = &stage_error {
                    set_stage_status(child, stage_id, STATUS_FAILED, Some(error.clone()));
                    child.last_error = Some(error.clone());
                } else {
                    set_stage_status(child, stage_id, STATUS_COMPLETED, None);
                    child.last_error = None;
                }
            }
            state.jobs[job_index].log_summary.extend(log_summary);
            if waiting_agent_profile.is_some() {
                state.jobs[job_index].current_step =
                    format!("Waiting for agent handoff at {stage_id}");
                state.jobs[job_index].last_error = None;
            } else if let Some(error) = stage_error {
                state.jobs[job_index].current_step = if stage_id == "validate_reading" {
                    "validation_failed".into()
                } else {
                    format!("{stage_id} stage failed")
                };
                state.jobs[job_index].last_error = Some(error);
            } else {
                state.jobs[job_index].current_step = format!("Completed {stage_id} stage");
                state.jobs[job_index].last_error = None;
            }
        }
        Err(error) => {
            let redacted = redact_runner_message(&error);
            set_stage_status(
                &mut state.jobs[job_index].children[child_index],
                stage_id,
                STATUS_FAILED,
                Some(redacted.clone()),
            );
            state.jobs[job_index].children[child_index].last_error = Some(redacted.clone());
            state.jobs[job_index].current_step = if stage_id == "validate_reading" {
                "validation_failed".into()
            } else {
                format!("{stage_id} stage failed")
            };
            state.jobs[job_index].last_error = Some(redacted);
        }
    }
    let translation_gate_readied =
        ready_translation_approval_gate(&mut state.jobs[job_index], child_index);
    let promotion_gate_readied =
        ready_promotion_approval_gate(&mut state.jobs[job_index], child_index);
    if promotion_gate_readied {
        state.jobs[job_index].current_step = "Promotion approval review ready".into();
        state.jobs[job_index]
            .log_summary
            .push("Promotion approval review ready".into());
    } else if translation_gate_readied {
        state.jobs[job_index].current_step = "Translation approval review ready".into();
        state.jobs[job_index]
            .log_summary
            .push("Translation approval review ready".into());
    }
    state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    // `derive_job` is what classifies the failure, so the retry decision comes
    // after it and the progress recomputed from it goes out in the same save:
    // the countdown a poll reads is written *before* the wait, never after.
    let retry_after_seconds =
        schedule_stage_retry(&mut state.jobs[job_index].children[child_index], stage_id);
    if let Some(seconds) = retry_after_seconds {
        let attempts_remaining = stage_ref(&state.jobs[job_index].children[child_index], stage_id)
            .map(stage_attempts_remaining)
            .unwrap_or_default();
        state.jobs[job_index].current_step = format!(
            "Retrying {stage_id} stage in {seconds}s ({attempts_remaining} attempt(s) left)"
        );
        state.jobs[job_index].log_summary.push(format!(
            "Automatic {stage_id} retry scheduled in {seconds}s"
        ));
        state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    } else if let Some(reason) = stage_ref(&state.jobs[job_index].children[child_index], stage_id)
        .and_then(|stage| stage.give_up_reason.clone())
    {
        state.jobs[job_index]
            .log_summary
            .push(format!("Stopped retrying {stage_id}: {reason}"));
        state.jobs[job_index].log_summary = trim_log_summary(&state.jobs[job_index].log_summary);
    }
    state.jobs[job_index].progress = derive_job_progress(&state.jobs[job_index]);
    let job = state.jobs[job_index].clone();
    store.save(&state)?;
    if let Some(seconds) = retry_after_seconds {
        // The failure, the countdown and the remaining budget are all persisted
        // above, so this wait is observable rather than a silent stall.
        thread::sleep(Duration::from_secs(seconds.into()));
        return advance_job_stage(
            store,
            job_id,
            Some(&running_child.id),
            invalidate_downstream,
            executor,
            true,
        );
    }
    if translation_gate_readied && should_auto_approve_translation(&job, child_index) {
        return advance_job_with_executor(store, job_id, child_id, invalidate_downstream, executor);
    }
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_tail_surfaces_the_last_lines_of_a_python_traceback() {
        let stderr = "Traceback (most recent call last):\n  File \"x.py\", line 1\nRuntimeError: GOOGLE_API_KEY or GEMINI_API_KEY not set.\n";
        assert_eq!(
            stderr_tail(stderr),
            "Traceback (most recent call last): | File \"x.py\", line 1 | RuntimeError: GOOGLE_API_KEY or GEMINI_API_KEY not set."
        );
    }

    #[test]
    fn stderr_tail_ignores_blank_lines_and_caps_at_three() {
        let stderr = "\n  \nfirst\nsecond\nthird\nfourth\n";
        assert_eq!(stderr_tail(stderr), "second | third | fourth");
    }

    #[test]
    fn stderr_tail_is_empty_for_empty_stderr() {
        assert_eq!(stderr_tail(""), "");
    }

    #[test]
    fn stderr_tail_redacts_an_auth_header_but_not_a_missing_key_message() {
        let stderr = "connecting...\nAuthorization: Bearer sk-abc123\nRuntimeError: GEMINI_API_KEY not set.\n";
        let tail = stderr_tail(stderr);
        assert!(!tail.contains("sk-abc123"), "leaked a secret: {tail}");
        assert!(
            tail.contains("GEMINI_API_KEY not set"),
            "over-redacted a message that names no secret: {tail}"
        );
    }

    #[test]
    fn stderr_tail_redacts_a_key_assignment() {
        let stderr = "DASHSCOPE_API_KEY=sk-abc123\n";
        let tail = stderr_tail(stderr);
        assert!(!tail.contains("sk-abc123"), "leaked a secret: {tail}");
    }

    fn executable_fixture(dir: &Path, name: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        path
    }

    #[test]
    fn program_search_dirs_keep_inherited_path_ahead_of_the_desktop_fallbacks() {
        let inherited = env::join_paths(["/usr/bin", "/bin"].map(PathBuf::from)).unwrap();
        let dirs = program_search_dirs_from(
            Some(inherited.as_os_str()),
            vec![PathBuf::from("/opt/homebrew/bin")],
        );
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
                PathBuf::from("/opt/homebrew/bin"),
            ]
        );
    }

    #[test]
    fn program_search_dirs_drop_duplicates_and_empty_entries() {
        let inherited = env::join_paths(["/usr/bin", "", "/opt/homebrew/bin"].map(PathBuf::from))
            .unwrap_or_else(|_| OsString::from("/usr/bin::/opt/homebrew/bin"));
        let dirs = program_search_dirs_from(
            Some(inherited.as_os_str()),
            vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/bin")],
        );
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/bin"),
            ]
        );
    }

    #[test]
    fn program_search_dirs_survive_a_desktop_launch_without_path() {
        let dirs = program_search_dirs_from(None, vec![PathBuf::from("/opt/homebrew/bin")]);
        assert_eq!(dirs, vec![PathBuf::from("/opt/homebrew/bin")]);
    }

    // The launchd default a Finder-launched .app inherits holds no uv and no
    // node, so a bare name has to resolve out of the fallback roots instead.
    #[test]
    fn runner_program_resolves_a_bare_name_a_desktop_path_cannot_reach() {
        let root = temp_root("program-lookup");
        let desktop_path = root.join("usr-bin");
        let homebrew = root.join("homebrew-bin");
        fs::create_dir_all(&desktop_path).unwrap();
        let uv = executable_fixture(&homebrew, "uv");

        let dirs = program_search_dirs_from(
            Some(env::join_paths([&desktop_path]).unwrap().as_os_str()),
            vec![homebrew.clone()],
        );
        assert_eq!(
            resolve_runner_program_in(&dirs, Path::new("uv")),
            uv,
            "a bare uv should resolve out of the fallback roots"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn runner_program_keeps_an_explicit_path_untouched() {
        let root = temp_root("program-explicit");
        let bin = root.join("bin");
        let shadow = executable_fixture(&bin, "python3");
        let explicit = PathBuf::from("/opt/homebrew/bin/python3.11");

        let dirs = vec![bin.clone()];
        assert_eq!(
            resolve_runner_program_in(&dirs, &explicit),
            explicit,
            "an explicit interpreter choice must not be re-resolved"
        );
        assert_ne!(resolve_runner_program_in(&dirs, &explicit), shadow);

        fs::remove_dir_all(&root).ok();
    }

    // Falling back to the bare name keeps the spawn error naming the tool the
    // caller asked for rather than an invented path.
    #[test]
    fn runner_program_falls_back_to_the_bare_name_when_nothing_resolves() {
        let root = temp_root("program-missing");
        fs::create_dir_all(&root).unwrap();
        assert_eq!(
            resolve_runner_program_in(&[root.clone()], Path::new("uv")),
            PathBuf::from("uv")
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn runner_child_path_carries_every_search_dir() {
        let value = runner_path_env_value().expect("search dirs should join into a PATH");
        let carried = env::split_paths(&value).collect::<Vec<_>>();
        assert_eq!(carried, program_search_dirs());
    }

    fn nvm_fixture(root: &Path, versions: &[&str], default_alias: Option<&str>) -> PathBuf {
        let nvm_root = root.join(".nvm");
        for version in versions {
            fs::create_dir_all(
                nvm_root
                    .join("versions")
                    .join("node")
                    .join(version)
                    .join("bin"),
            )
            .unwrap();
        }
        if let Some(alias) = default_alias {
            let alias_dir = nvm_root.join("alias");
            fs::create_dir_all(&alias_dir).unwrap();
            fs::write(alias_dir.join("default"), format!("{alias}\n")).unwrap();
        }
        nvm_root
    }

    // A machine whose only node came from nvm has it under a versioned directory
    // no constant can spell, so `build_reading` used to fail to spawn there.
    #[test]
    fn nvm_bin_dirs_lead_with_the_default_alias() {
        let root = temp_root("nvm-default-alias");
        let nvm_root = nvm_fixture(&root, &["v18.20.4", "v22.11.0", "v24.17.0"], Some("22"));

        let dirs = nvm_bin_dirs(&nvm_root);

        assert_eq!(
            dirs.first(),
            Some(&nvm_root.join("versions/node/v22.11.0/bin")),
            "the default alias should win over the newest install"
        );
        assert_eq!(dirs.len(), 3, "the other installs stay as fallbacks");
        fs::remove_dir_all(&root).ok();
    }

    // This machine's own `~/.nvm/alias/default` says `22` while only v24 is
    // installed; naming nothing installed must not leave the search without node.
    #[test]
    fn nvm_bin_dirs_fall_back_to_the_newest_install() {
        let root = temp_root("nvm-stale-alias");
        let nvm_root = nvm_fixture(&root, &["v9.11.2", "v24.17.0"], Some("22"));

        assert_eq!(
            nvm_bin_dirs(&nvm_root),
            vec![
                nvm_root.join("versions/node/v24.17.0/bin"),
                nvm_root.join("versions/node/v9.11.2/bin"),
            ],
            "v24 outranks v9 numerically, not as a string"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nvm_bin_dirs_are_empty_without_nvm() {
        let root = temp_root("nvm-absent");
        assert!(nvm_bin_dirs(&root.join(".nvm")).is_empty());
    }

    // Every process command the pipeline builds must be a bare name the resolver
    // handles or an already-resolved absolute path; a relative multi-component
    // program would silently depend on the child's cwd.
    #[test]
    fn pipeline_process_programs_are_resolvable() {
        for program in ["uv", "node", "java", "python3"] {
            let resolved = resolve_runner_program(Path::new(program));
            assert!(
                resolved.is_absolute() || resolved == PathBuf::from(program),
                "{program} resolved to an unusable relative path: {}",
                display_path(&resolved)
            );
        }
    }

    struct ArtifactFixtureRunner;

    impl PipelineRunner for ArtifactFixtureRunner {
        fn run(&self, _job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerOutput, String> {
            fs::create_dir_all(output_dir).unwrap();
            fs::write(output_dir.join("book.md"), "# Markdown\n").unwrap();
            fs::write(output_dir.join("book.html"), "<h1>HTML</h1>\n").unwrap();
            fs::write(output_dir.join("book.epub"), "epub bytes").unwrap();
            Ok(RunnerOutput {
                log_summary: vec!["fixture runner completed".into()],
                artifacts: scan_artifacts(output_dir)?,
                collection_items: Vec::new(),
                output_dir: Some(output_dir.to_path_buf()),
                current_step: None,
            })
        }
    }

    struct ConversionFailingRunner;

    impl PipelineRunner for ConversionFailingRunner {
        fn run(&self, _job: &BookPipelineJob, _output_dir: &Path) -> Result<RunnerOutput, String> {
            Err("Fake conversion backend failed".into())
        }
    }

    struct MissingMarkdownRunner;

    impl PipelineRunner for MissingMarkdownRunner {
        fn run(&self, _job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerOutput, String> {
            fs::create_dir_all(output_dir).unwrap();
            Ok(RunnerOutput {
                log_summary: vec!["fixture extraction returned no Markdown".into()],
                artifacts: Vec::new(),
                collection_items: Vec::new(),
                output_dir: Some(output_dir.to_path_buf()),
                current_step: None,
            })
        }
    }

    #[derive(Default)]
    struct RecordingNotificationSink {
        events: Mutex<Vec<BookPipelineTerminalEvent>>,
    }

    impl BookPipelineNotificationSink for RecordingNotificationSink {
        fn deliver(&self, event: &BookPipelineTerminalEvent) -> Result<(), String> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    struct FakeTranslationHandoffRunner;

    impl TranslationHandoffRunner for FakeTranslationHandoffRunner {
        fn handoff(
            &self,
            job: &BookPipelineJob,
            artifact_path: Option<&str>,
            repo_root: &Path,
        ) -> Result<TranslationHandoffOutput, String> {
            let markdown = selected_markdown_artifact(job, artifact_path)?;
            let project_root = repo_root
                .join("books")
                .join("local")
                .join("zh-Hans")
                .join("001_fake_handoff");
            let source_path = project_root.join("source").join("source.md");
            let manifest_path = project_root.join("metadata").join("source_manifest.json");
            fs::create_dir_all(source_path.parent().unwrap()).unwrap();
            fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
            fs::copy(&markdown.path, &source_path).unwrap();
            fs::write(&manifest_path, "{\"schema\":\"fake-source-manifest-v1\"}\n").unwrap();
            Ok(TranslationHandoffOutput {
                log_summary: vec!["Fake translation handoff ready".into()],
                artifacts: vec![
                    BookPipelineArtifact {
                        kind: "translation_source".into(),
                        path: display_path(&source_path),
                        sha256: Some(sha256_file(&source_path).unwrap()),
                        zotero_key: markdown.zotero_key.clone(),
                        producer_stage: Some("handoff".into()),
                        ..BookPipelineArtifact::default()
                    },
                    BookPipelineArtifact {
                        kind: "source_manifest".into(),
                        path: display_path(&manifest_path),
                        sha256: Some(sha256_file(&manifest_path).unwrap()),
                        zotero_key: markdown.zotero_key.clone(),
                        producer_stage: Some("handoff".into()),
                        ..BookPipelineArtifact::default()
                    },
                ],
            })
        }

        fn handoff_attachment(
            &self,
            _job: &BookPipelineJob,
            child: &BookPipelineChildJob,
            artifact_path: &str,
            repo_root: &Path,
        ) -> Result<TranslationHandoffOutput, String> {
            assert_eq!(child.source.kind, "zotero_attachment");
            assert!(child
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == "markdown" && artifact.path == artifact_path));
            let attachment_key = child.source.selector.as_deref().unwrap();
            let project_root = repo_root
                .join("books")
                .join("local")
                .join("zh-Hans")
                .join(format!("001_fake_handoff_{attachment_key}"));
            let source_path = project_root.join("source").join("source.md");
            let manifest_path = project_root.join("metadata").join("source_manifest.json");
            fs::create_dir_all(source_path.parent().unwrap()).unwrap();
            fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
            fs::copy(artifact_path, &source_path).unwrap();
            fs::write(&manifest_path, "{\"schema\":\"fake-source-manifest-v1\"}\n").unwrap();
            let markdown_key = child
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == "markdown" && artifact.path == artifact_path)
                .and_then(|artifact| artifact.zotero_key.clone());
            Ok(TranslationHandoffOutput {
                log_summary: vec!["Fake attachment translation handoff ready".into()],
                artifacts: vec![
                    BookPipelineArtifact {
                        kind: "translation_source".into(),
                        path: display_path(&source_path),
                        sha256: Some(sha256_file(&source_path).unwrap()),
                        zotero_key: markdown_key.clone(),
                        producer_stage: Some("handoff".into()),
                        ..BookPipelineArtifact::default()
                    },
                    BookPipelineArtifact {
                        kind: "source_manifest".into(),
                        path: display_path(&manifest_path),
                        sha256: Some(sha256_file(&manifest_path).unwrap()),
                        zotero_key: markdown_key,
                        producer_stage: Some("handoff".into()),
                        ..BookPipelineArtifact::default()
                    },
                ],
            })
        }
    }

    struct FailingTranslationHandoffRunner;

    impl TranslationHandoffRunner for FailingTranslationHandoffRunner {
        fn handoff(
            &self,
            _job: &BookPipelineJob,
            _artifact_path: Option<&str>,
            _repo_root: &Path,
        ) -> Result<TranslationHandoffOutput, String> {
            Err("Fake translation handoff failed".into())
        }
    }

    struct SecretFailingExecutor;

    impl RunnerCommandExecutor for SecretFailingExecutor {
        fn execute(&self, _command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            Err("ZOTERO_API_KEY=supersecret token=abc Authorization: bearer nope".into())
        }
    }

    struct SecretLoggingExecutor;

    impl RunnerCommandExecutor for SecretLoggingExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            fs::create_dir_all(&command.output_dir).unwrap();
            fs::write(command.output_dir.join("book.md"), "# Markdown\n").unwrap();
            Ok(RunnerCommandResult {
                stdout: "token=abc".into(),
                stderr: "Authorization: bearer nope".into(),
                log_summary: vec![
                    "ZOTERO_API_KEY=supersecret".into(),
                    ".env content was not read".into(),
                ],
            })
        }
    }

    struct LocalPdfFixtureExecutor;

    impl RunnerCommandExecutor for LocalPdfFixtureExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.kind, RunnerCommandKind::Process);
            assert_eq!(command.label, "local PDF conversion wrapper");
            assert!(has_arg_pair(
                &command.args,
                "--output-dir",
                &display_path(&command.output_dir)
            ));
            let book_dir = command.output_dir.join("Sample Book");
            fs::create_dir_all(&book_dir).unwrap();
            fs::write(book_dir.join("sample.md"), "# Markdown\n").unwrap();
            fs::write(book_dir.join("sample.html"), "<h1>HTML</h1>\n").unwrap();
            fs::write(book_dir.join("sample.epub"), "epub bytes").unwrap();
            fs::write(book_dir.join("_state.json"), "{\"status\":\"done\"}\n").unwrap();
            fs::write(book_dir.join("pages.jsonl"), "{\"page\":1}\n").unwrap();
            fs::write(book_dir.join("search.index"), "term -> page 1\n").unwrap();
            Ok(RunnerCommandResult {
                stdout: "DONE: sample.pdf -> sample.html".into(),
                stderr: String::new(),
                log_summary: vec!["Local PDF fixture wrapper completed".into()],
            })
        }
    }

    struct LocalPdfFailingExecutor;

    impl RunnerCommandExecutor for LocalPdfFailingExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.kind, RunnerCommandKind::Process);
            Err("Local PDF fixture wrapper failed".into())
        }
    }

    struct ZoteroDiscoveryExecutor;

    impl RunnerCommandExecutor for ZoteroDiscoveryExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.kind, RunnerCommandKind::Process);
            assert_eq!(command.label, "Zotero discovery dry-run");
            assert!(command.args.iter().any(|arg| arg == "--dry-run"));
            assert!(has_arg_pair(&command.args, "--limit", "5"));
            assert!(has_arg_pair(&command.args, "--parent-item-type", "book"));
            Ok(RunnerCommandResult {
                stdout: String::new(),
                stderr: [
                    "12:00:00 INFO PLAN DIRECT1 route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=2000 title=Born Digital Book",
                    "12:00:01 INFO PLAN SCAN1 route=paddle-ocr pages=240 selected=240 parent_type=book sampled_chars=0 title=Scanned Book",
                ]
                .join("\n"),
                log_summary: vec!["Zotero dry-run completed".into()],
            })
        }
    }

    struct ZoteroDiscoverySecretFailingExecutor;

    impl RunnerCommandExecutor for ZoteroDiscoverySecretFailingExecutor {
        fn execute(&self, _command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            Err("Zotero discovery failed with ZOTERO_API_KEY=secret".into())
        }
    }

    struct ZoteroRoutePreviewExecutor;

    impl RunnerCommandExecutor for ZoteroRoutePreviewExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.label, "Zotero discovery dry-run");
            assert!(command.args.iter().any(|arg| arg == "--dry-run"));
            Ok(RunnerCommandResult {
                stdout: String::new(),
                stderr: [
                    "12:00:00 INFO PLAN DIRECT route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=2000 title=Direct Text",
                    "12:00:01 INFO PLAN SCAN route=paddle-ocr pages=240 selected=240 parent_type=book sampled_chars=0 title=Scanned PDF",
                    "12:00:02 INFO PLAN MINERU route=mineru pages=32 selected=32 parent_type=journalArticle sampled_chars=0 title=MinerU Candidate",
                    "12:00:03 INFO PLAN DIRTY route=needs-mineru pages=12 selected=12 parent_type=book sampled_chars=600 title=Dirty Text Layer",
                    "12:00:04 INFO SKIP completed DONE Already Converted",
                ]
                .join("\n"),
                log_summary: vec!["Zotero route dry-run completed".into()],
            })
        }
    }

    struct ZoteroFingerprintPreviewExecutor;

    impl RunnerCommandExecutor for ZoteroFingerprintPreviewExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.label, "Zotero discovery dry-run");
            Ok(RunnerCommandResult {
                stdout: String::new(),
                stderr: [
                    "12:00:00 INFO SKIP completed CURRENT Current Title source_md5=aaa111 output_path=/tmp/current.md zotero_attachment_key=MDOLD",
                    "12:00:01 INFO REBUILD completed MISSING because uploaded Zotero attachment is missing",
                    "12:00:02 INFO PLAN MISSING route=pdf-text pages=10 selected=10 parent_type=book sampled_chars=1200 title=Missing Upload source_md5=aaa111",
                    "12:00:03 INFO PLAN CHANGED route=pdf-text pages=12 selected=12 parent_type=book sampled_chars=1400 title=Changed Source source_md5=bbb222",
                    "12:00:04 INFO PLAN DIRTY route=needs-mineru pages=8 selected=8 parent_type=book sampled_chars=500 title=Dirty Blocked source_md5=ccc333",
                ]
                .join("\n"),
                log_summary: vec!["Zotero fingerprint dry-run completed".into()],
            })
        }
    }

    fn fixture_item_index_profile_result() -> RunnerCommandResult {
        RunnerCommandResult {
            stdout: serde_json::json!({
                "embeddingProfileId": "fixture-embedding:3",
            })
            .to_string(),
            stderr: String::new(),
            log_summary: vec!["Zotero item index profile fixture completed".into()],
        }
    }

    struct ZoteroConversionExecutor;

    impl RunnerCommandExecutor for ZoteroConversionExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.kind, RunnerCommandKind::Process);
            match command.label.as_str() {
                ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
                "Zotero conversion worker" => {
                    assert!(has_arg_pair(&command.args, "--attachment-key", "DIRECT"));
                    assert!(command.args.iter().any(|arg| arg == "--force-text"));
                    assert!(has_env_pair(
                        &command.env,
                        "OCR_OUTPUT_ROOT",
                        &display_path(&command.output_dir)
                    ));
                    let staging = command
                        .output_dir
                        .join(".state")
                        .join("staging")
                        .join("DIRECT");
                    fs::create_dir_all(&staging).unwrap();
                    fs::write(
                        staging.join("direct.md"),
                        "---\nparent_item_key: \"PARENT123\"\n---\n\n# Direct Markdown\n",
                    )
                    .unwrap();
                    fs::write(staging.join("direct.jsonl"), "{\"page\":1}\n").unwrap();
                    Ok(RunnerCommandResult {
                        stdout: "Uploaded direct.md to Zotero attachment MDKEY123 status=completed"
                            .into(),
                        stderr: String::new(),
                        log_summary: vec!["Zotero conversion fixture completed".into()],
                    })
                }
                "Zotero item-scoped full-text index" => {
                    let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                    Ok(RunnerCommandResult {
                        stdout: serde_json::json!({
                            "parentItemKey": "PARENT123",
                            "sourceSha256": markdown_sha256,
                            "chunkCount": 1,
                            "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                            "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                            "embeddingProfileId": "fixture-embedding:3",
                            "completedAt": "2026-07-15T12:00:00Z",
                            "reused": false,
                        })
                        .to_string(),
                        stderr: String::new(),
                        log_summary: vec!["Zotero item index fixture completed".into()],
                    })
                }
                other => panic!("unexpected command: {other}"),
            }
        }
    }

    struct ZoteroExtractIndexExecutor {
        command_labels: Mutex<Vec<String>>,
        fail_index_once: Mutex<bool>,
        omit_markdown_attachment_key: bool,
    }

    impl ZoteroExtractIndexExecutor {
        fn succeeding() -> Self {
            Self {
                command_labels: Mutex::new(Vec::new()),
                fail_index_once: Mutex::new(false),
                omit_markdown_attachment_key: false,
            }
        }

        fn failing_index_once() -> Self {
            Self {
                command_labels: Mutex::new(Vec::new()),
                fail_index_once: Mutex::new(true),
                omit_markdown_attachment_key: false,
            }
        }

        fn missing_markdown_attachment_key() -> Self {
            Self {
                command_labels: Mutex::new(Vec::new()),
                fail_index_once: Mutex::new(false),
                omit_markdown_attachment_key: true,
            }
        }

        fn command_labels(&self) -> Vec<String> {
            self.command_labels.lock().unwrap().clone()
        }
    }

    impl RunnerCommandExecutor for ZoteroExtractIndexExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            self.command_labels
                .lock()
                .unwrap()
                .push(command.label.clone());
            match command.label.as_str() {
                ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
                "Zotero conversion worker" => {
                    let staging = command
                        .output_dir
                        .join(".state")
                        .join("staging")
                        .join("DIRECT");
                    fs::create_dir_all(&staging).unwrap();
                    fs::write(
                        staging.join("direct.md"),
                        "---\nparent_item_key: \"PARENT123\"\nsource_pdf_key: \"DIRECT\"\n---\n\n# Direct Markdown\n",
                    )
                    .unwrap();
                    let stdout = if self.omit_markdown_attachment_key {
                        "Completed direct.md without upload evidence status=completed"
                    } else {
                        "Uploaded direct.md to Zotero attachment MDKEY123 status=completed"
                    };
                    Ok(RunnerCommandResult {
                        stdout: stdout.into(),
                        stderr: String::new(),
                        log_summary: vec!["Zotero conversion fixture completed".into()],
                    })
                }
                "Zotero item-scoped full-text index" => {
                    let mut fail_index_once = self.fail_index_once.lock().unwrap();
                    if *fail_index_once {
                        *fail_index_once = false;
                        return Err("fixture index backend unavailable".into());
                    }
                    assert!(has_arg_pair(
                        &command.args,
                        "--parent-item-key",
                        "PARENT123"
                    ));
                    assert!(has_arg_pair(
                        &command.args,
                        "--chunk-contract-version",
                        "zfulltext-chunk-v1"
                    ));
                    assert!(has_arg_pair(
                        &command.args,
                        "--embedding-profile-id",
                        "fixture-embedding:3"
                    ));
                    let markdown_path = command_arg_value(command, "--markdown").unwrap();
                    let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                    assert!(Path::new(markdown_path).is_file());
                    assert_eq!(
                        sha256_file(Path::new(markdown_path)).unwrap(),
                        markdown_sha256
                    );
                    Ok(RunnerCommandResult {
                        stdout: serde_json::json!({
                            "parentItemKey": "PARENT123",
                            "sourceSha256": markdown_sha256,
                            "chunkCount": 1,
                            "indexContractVersion": "zfulltext-item-index-v1",
                            "chunkContractVersion": "zfulltext-chunk-v1",
                            "embeddingProfileId": "fixture-embedding:3",
                            "completedAt": "2026-07-15T12:00:00Z",
                            "reused": false,
                        })
                        .to_string(),
                        stderr: String::new(),
                        log_summary: vec!["Zotero item index fixture completed".into()],
                    })
                }
                other => panic!("unexpected command: {other}"),
            }
        }
    }

    struct ZoteroConversionFailingExecutor;

    impl RunnerCommandExecutor for ZoteroConversionFailingExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.label, "Zotero conversion worker");
            Err("Zotero conversion fixture failed: diagnosis preserved".into())
        }
    }

    struct MineruFixtureExecutor;

    impl RunnerCommandExecutor for MineruFixtureExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.kind, RunnerCommandKind::Process);
            match command.label.as_str() {
                ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
                "MinerU extraction wrapper" => {
                    assert!(has_arg_pair(&command.args, "--attachment-key", "MINERU"));
                    assert!(has_arg_pair(
                        &command.args,
                        "--output-dir",
                        &display_path(&command.output_dir)
                    ));
                    fs::create_dir_all(&command.output_dir).unwrap();
                    fs::write(
                        command.output_dir.join("mineru.md"),
                        "---\nparent_item_key: \"PARENTMINERU\"\n---\n\n# MinerU Markdown\n",
                    )
                    .unwrap();
                    fs::write(
                        command.output_dir.join("mineru.json"),
                        "{\"engine\":\"mineru\"}\n",
                    )
                    .unwrap();
                    Ok(RunnerCommandResult {
                        stdout: "MinerU completed without token details".into(),
                        stderr: String::new(),
                        log_summary: vec!["MinerU fixture completed".into()],
                    })
                }
                "Zotero item-scoped full-text index" => {
                    let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                    Ok(RunnerCommandResult {
                        stdout: serde_json::json!({
                            "parentItemKey": "PARENTMINERU",
                            "sourceSha256": markdown_sha256,
                            "chunkCount": 1,
                            "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                            "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                            "embeddingProfileId": "fixture-embedding:3",
                            "completedAt": "2026-07-15T12:00:00Z",
                            "reused": false,
                        })
                        .to_string(),
                        stderr: String::new(),
                        log_summary: vec!["Zotero item index fixture completed".into()],
                    })
                }
                other => panic!("unexpected command: {other}"),
            }
        }
    }

    struct ExternalAdapterFixtureExecutor;

    impl RunnerCommandExecutor for ExternalAdapterFixtureExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.kind, RunnerCommandKind::Process);
            assert_eq!(command.label, "external Book Pipeline adapter");
            assert!(has_arg_pair(
                &command.args,
                "--output-dir",
                &display_path(&command.output_dir)
            ));
            fs::create_dir_all(&command.output_dir).unwrap();
            fs::write(
                command.output_dir.join("adapter.md"),
                "# Adapter Markdown\n",
            )
            .unwrap();
            fs::write(
                command.output_dir.join("adapter.html"),
                "<h1>Adapter</h1>\n",
            )
            .unwrap();
            Ok(RunnerCommandResult {
                stdout: "adapter completed".into(),
                stderr: String::new(),
                log_summary: vec!["External adapter fixture completed".into()],
            })
        }
    }

    struct TranslationEngineFixtureExecutor {
        fail_once: Mutex<Option<String>>,
        requested_units: Mutex<Vec<Vec<String>>>,
        expected_second_pass_enabled: bool,
        expected_text_cleanup: bool,
        expected_custom_instructions: Option<BookPipelineCustomInstructions>,
        merge_translation_paragraphs: bool,
        // Emitted on every completed unit, so a test can drive the runner's
        // handling of a glossary warning without a real model to disobey.
        glossary_violations: Vec<(String, String)>,
    }

    impl TranslationEngineFixtureExecutor {
        fn succeeding() -> Self {
            Self {
                fail_once: Mutex::new(None),
                requested_units: Mutex::new(Vec::new()),
                expected_second_pass_enabled: false,
                expected_text_cleanup: false,
                expected_custom_instructions: None,
                merge_translation_paragraphs: false,
                glossary_violations: Vec::new(),
            }
        }

        fn with_second_pass_enabled() -> Self {
            Self {
                fail_once: Mutex::new(None),
                requested_units: Mutex::new(Vec::new()),
                expected_second_pass_enabled: true,
                expected_text_cleanup: false,
                expected_custom_instructions: None,
                merge_translation_paragraphs: false,
                glossary_violations: Vec::new(),
            }
        }

        fn with_text_cleanup() -> Self {
            Self {
                fail_once: Mutex::new(None),
                requested_units: Mutex::new(Vec::new()),
                expected_second_pass_enabled: false,
                expected_text_cleanup: true,
                expected_custom_instructions: None,
                merge_translation_paragraphs: false,
                glossary_violations: Vec::new(),
            }
        }

        fn with_custom_instructions(custom_instructions: BookPipelineCustomInstructions) -> Self {
            Self {
                fail_once: Mutex::new(None),
                requested_units: Mutex::new(Vec::new()),
                expected_second_pass_enabled: true,
                expected_text_cleanup: false,
                expected_custom_instructions: Some(custom_instructions),
                merge_translation_paragraphs: false,
                glossary_violations: Vec::new(),
            }
        }

        fn failing_once(unit_id: &str) -> Self {
            Self {
                fail_once: Mutex::new(Some(unit_id.into())),
                requested_units: Mutex::new(Vec::new()),
                expected_second_pass_enabled: false,
                expected_text_cleanup: false,
                expected_custom_instructions: None,
                merge_translation_paragraphs: false,
                glossary_violations: Vec::new(),
            }
        }

        fn with_paragraph_mismatch() -> Self {
            Self {
                fail_once: Mutex::new(None),
                requested_units: Mutex::new(Vec::new()),
                expected_second_pass_enabled: false,
                expected_text_cleanup: false,
                expected_custom_instructions: None,
                merge_translation_paragraphs: true,
                glossary_violations: Vec::new(),
            }
        }

        fn reporting_glossary_violations(violations: &[(&str, &str)]) -> Self {
            Self {
                glossary_violations: violations
                    .iter()
                    .map(|(source, translation)| {
                        ((*source).to_string(), (*translation).to_string())
                    })
                    .collect(),
                ..Self::succeeding()
            }
        }

        fn requested_units(&self) -> Vec<Vec<String>> {
            self.requested_units.lock().unwrap().clone()
        }
    }

    impl RunnerCommandExecutor for TranslationEngineFixtureExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.kind, RunnerCommandKind::Process);
            assert_eq!(command.label, TRANSLATION_ENGINE_COMMAND_LABEL);
            assert_eq!(command.program, PathBuf::from("uv"));
            let repo_root = local_reading_repo_root().unwrap();
            assert_eq!(command.cwd.as_deref(), Some(repo_root.as_path()));
            assert_eq!(command.accepted_exit_codes, vec![0, 1]);
            let manifest_path = PathBuf::from(&command.args[5]);
            assert_eq!(
                command.args,
                vec![
                    "run".to_string(),
                    "--package".to_string(),
                    "translation-engine".to_string(),
                    "translation-engine".to_string(),
                    "--manifest".to_string(),
                    display_path(&manifest_path),
                ]
            );
            let manifest: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
            assert_eq!(manifest["schema"], TRANSLATION_ENGINE_RUN_SCHEMA);
            assert_eq!(manifest["projectRoot"], display_path(&command.output_dir));
            assert_eq!(manifest["sourceMapPath"], "metadata/source_map.json");
            assert_eq!(manifest["sourceLanguage"], "auto");
            assert_eq!(manifest["targetLanguage"], "zh-Hans");
            assert_eq!(manifest["providerProfileId"], "fake-provider-profile");
            assert_eq!(manifest["providerConfigId"], "fake-provider-config");
            match &self.expected_custom_instructions {
                Some(custom_instructions) => assert_eq!(
                    manifest["customInstructions"],
                    serde_json::to_value(custom_instructions).unwrap()
                ),
                None => assert!(manifest.get("customInstructions").is_none()),
            }
            assert_eq!(
                manifest["secondPassEnabled"],
                self.expected_second_pass_enabled
            );
            assert_eq!(manifest["textCleanup"], self.expected_text_cleanup);
            assert_eq!(
                manifest["translationPolicyVersion"],
                TRANSLATION_POLICY_VERSION
            );
            assert_eq!(manifest["maxTokens"], TRANSLATION_ENGINE_MAX_TOKENS);
            assert_eq!(
                manifest["placeholderRetries"],
                TRANSLATION_ENGINE_PLACEHOLDER_RETRIES
            );

            let mut requested = Vec::new();
            for unit in manifest["units"].as_array().unwrap() {
                let task_path = command
                    .output_dir
                    .join(unit["taskManifestPath"].as_str().unwrap());
                let task: serde_json::Value =
                    serde_json::from_str(&fs::read_to_string(task_path).unwrap()).unwrap();
                requested.push((
                    task["chapterId"].as_str().unwrap().to_string(),
                    task["sourceChapterPath"].as_str().unwrap().to_string(),
                ));
            }
            self.requested_units.lock().unwrap().push(
                requested
                    .iter()
                    .map(|(unit_id, _)| unit_id.clone())
                    .collect(),
            );
            let fail_unit = self.fail_once.lock().unwrap().take();
            let mut reports = Vec::new();
            for (unit_id, source_chapter_path) in requested {
                let failed = fail_unit.as_deref() == Some(unit_id.as_str());
                let relative = if failed {
                    format!("chapters/translated/.partial/{unit_id}.degraded.md")
                } else {
                    format!("chapters/translated/{unit_id}.md")
                };
                let path = command.output_dir.join(&relative);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                let source =
                    fs::read_to_string(command.output_dir.join(source_chapter_path)).unwrap();
                let mut translation = fixture_translation(&source, &unit_id);
                if self.merge_translation_paragraphs {
                    translation = translation.replace("\n\n", "\n");
                }
                fs::write(&path, translation).unwrap();
                if !failed {
                    let _ = fs::remove_file(
                        command
                            .output_dir
                            .join("chapters")
                            .join("translated")
                            .join(".partial")
                            .join(format!("{unit_id}.degraded.md")),
                    );
                }
                let artifact = serde_json::json!({
                    "kind": if failed {
                        "chapter_translation_degraded"
                    } else {
                        "chapter_translation"
                    },
                    "path": relative,
                    "sha256": sha256_file(&path).unwrap(),
                    "complete": !failed,
                });
                reports.push(if failed {
                    serde_json::json!({
                        "unitId": unit_id,
                        "status": "failed",
                        "artifact": artifact,
                        "error": {"code": "translation_incomplete", "retryable": true},
                    })
                } else {
                    let mut completed = serde_json::json!({
                        "unitId": unit_id,
                        "status": "completed",
                        "artifact": artifact,
                    });
                    if !self.glossary_violations.is_empty() {
                        completed["glossaryViolations"] = serde_json::Value::Array(
                            self.glossary_violations
                                .iter()
                                .map(|(source, translation)| {
                                    serde_json::json!({
                                        "source": source,
                                        "translation": translation,
                                    })
                                })
                                .collect(),
                        );
                    }
                    completed
                });
            }
            let failed = reports
                .iter()
                .filter(|report| report["status"] == "failed")
                .count();
            let total = reports.len();
            let report = serde_json::json!({
                "schema": TRANSLATION_ENGINE_REPORT_SCHEMA,
                "summary": {
                    "total": total,
                    "completed": total - failed,
                    "failed": failed,
                },
                "units": reports,
            });
            Ok(RunnerCommandResult {
                stdout: serde_json::to_string(&report).unwrap(),
                stderr: String::new(),
                log_summary: vec!["Translation engine fixture completed".into()],
            })
        }
    }

    #[derive(Default)]
    struct TranslationSampleFixtureExecutor {
        requests: Mutex<Vec<(String, String)>>,
        // Recorded so a test can assert the sample manifest carries the same
        // translation settings as the full run; without them the preview shows a
        // translation the real run would not produce.
        prompt_inputs: Mutex<Vec<(serde_json::Value, serde_json::Value)>>,
    }

    impl TranslationSampleFixtureExecutor {
        fn requests(&self) -> Vec<(String, String)> {
            self.requests.lock().unwrap().clone()
        }

        fn prompt_inputs(&self) -> Vec<(serde_json::Value, serde_json::Value)> {
            self.prompt_inputs.lock().unwrap().clone()
        }
    }

    impl RunnerCommandExecutor for TranslationSampleFixtureExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.kind, RunnerCommandKind::Process);
            assert_eq!(command.label, TRANSLATION_ENGINE_SAMPLE_COMMAND_LABEL);
            assert_eq!(command.program, PathBuf::from("uv"));
            assert_eq!(command.accepted_exit_codes, vec![0]);
            let manifest_path = PathBuf::from(&command.args[5]);
            assert_eq!(
                command.args,
                vec![
                    "run".to_string(),
                    "--package".to_string(),
                    "translation-engine".to_string(),
                    "translation-engine-sample".to_string(),
                    "--manifest".to_string(),
                    display_path(&manifest_path),
                ]
            );
            let manifest: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(manifest_path).unwrap()).unwrap();
            assert_eq!(manifest["schema"], TRANSLATION_ENGINE_SAMPLE_SCHEMA);
            assert_eq!(manifest["sampleCount"], TRANSLATION_SAMPLE_COUNT);
            assert_eq!(
                manifest["characterBudget"],
                TRANSLATION_SAMPLE_CHARACTER_BUDGET
            );
            let profile = manifest["providerProfileId"].as_str().unwrap().to_string();
            let config = manifest["providerConfigId"].as_str().unwrap().to_string();
            self.requests
                .lock()
                .unwrap()
                .push((profile, config.clone()));
            self.prompt_inputs.lock().unwrap().push((
                manifest["textCleanup"].clone(),
                manifest
                    .get("customInstructions")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            ));

            let units = manifest["units"].as_array().unwrap();
            assert_eq!(units.len(), 5);
            let mut chapter_ids = Vec::new();
            for unit in units {
                let task_path = command
                    .output_dir
                    .join(unit["taskManifestPath"].as_str().unwrap());
                let task: serde_json::Value =
                    serde_json::from_str(&fs::read_to_string(task_path).unwrap()).unwrap();
                chapter_ids.push(task["chapterId"].as_str().unwrap().to_string());
            }
            let samples = chapter_ids[1..4]
                .iter()
                .map(|chapter_id| {
                    serde_json::json!({
                        "chunkRef": chapter_id,
                        "sourceExcerpt": format!("Source {chapter_id}."),
                        "translatedExcerpt": format!("{config}: Translated {chapter_id}."),
                        "degradation": if chapter_id == "chapter_003" { "aligned" } else { "none" },
                    })
                })
                .collect::<Vec<_>>();
            Ok(RunnerCommandResult {
                stdout: serde_json::to_string(&serde_json::json!({
                    "schema": TRANSLATION_ENGINE_SAMPLE_REPORT_SCHEMA,
                    "samples": samples,
                }))
                .unwrap(),
                stderr: String::new(),
                log_summary: vec!["Translation sample fixture completed".into()],
            })
        }
    }

    struct ReadingPipelineFixtureExecutor {
        translation: TranslationEngineFixtureExecutor,
        reading_epubcheck_passes: bool,
        digest_epubcheck_passes: bool,
        digest_enabled: bool,
        bilingual_fallback: bool,
        command_labels: Mutex<Vec<String>>,
    }

    impl ReadingPipelineFixtureExecutor {
        fn passing() -> Self {
            Self {
                translation: TranslationEngineFixtureExecutor::succeeding(),
                reading_epubcheck_passes: true,
                digest_epubcheck_passes: true,
                digest_enabled: false,
                bilingual_fallback: false,
                command_labels: Mutex::new(Vec::new()),
            }
        }

        fn passing_with_digest() -> Self {
            Self {
                translation: TranslationEngineFixtureExecutor::succeeding(),
                reading_epubcheck_passes: true,
                digest_epubcheck_passes: true,
                digest_enabled: true,
                bilingual_fallback: false,
                command_labels: Mutex::new(Vec::new()),
            }
        }

        fn failing_epubcheck() -> Self {
            Self {
                translation: TranslationEngineFixtureExecutor::succeeding(),
                reading_epubcheck_passes: false,
                digest_epubcheck_passes: true,
                digest_enabled: false,
                bilingual_fallback: false,
                command_labels: Mutex::new(Vec::new()),
            }
        }

        fn passing_with_bilingual_fallback() -> Self {
            Self {
                translation: TranslationEngineFixtureExecutor::with_paragraph_mismatch(),
                reading_epubcheck_passes: true,
                digest_epubcheck_passes: true,
                digest_enabled: false,
                bilingual_fallback: true,
                command_labels: Mutex::new(Vec::new()),
            }
        }

        fn failing_digest_epubcheck() -> Self {
            Self {
                translation: TranslationEngineFixtureExecutor::succeeding(),
                reading_epubcheck_passes: true,
                digest_epubcheck_passes: false,
                digest_enabled: true,
                bilingual_fallback: false,
                command_labels: Mutex::new(Vec::new()),
            }
        }

        fn command_labels(&self) -> Vec<String> {
            self.command_labels.lock().unwrap().clone()
        }
    }

    impl RunnerCommandExecutor for ReadingPipelineFixtureExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            self.command_labels
                .lock()
                .unwrap()
                .push(command.label.clone());
            match command.label.as_str() {
                TRANSLATION_ENGINE_COMMAND_LABEL => self.translation.execute(command),
                READING_BUILD_COMMAND_LABEL => {
                    assert_eq!(command.kind, RunnerCommandKind::Process);
                    assert_eq!(command.program, PathBuf::from("node"));
                    assert_eq!(command.cwd.as_deref(), Some(command.output_dir.as_path()));
                    assert_eq!(command.accepted_exit_codes, vec![0]);
                    assert_eq!(command.args.len(), 1);
                    assert_eq!(
                        Path::new(&command.args[0])
                            .file_name()
                            .and_then(|name| name.to_str()),
                        Some("build_epub.js")
                    );
                    assert!(command.output_dir.join("output/reading/book.md").is_file());
                    let final_dir = command.output_dir.join("chapters/final");
                    let mut final_paths = fs::read_dir(&final_dir)
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .filter(|path| {
                            path.extension().and_then(|value| value.to_str()) == Some("md")
                        })
                        .collect::<Vec<_>>();
                    final_paths.sort();
                    assert!(!final_paths.is_empty());
                    let html_dir = command.output_dir.join("output/epub_work/EPUB");
                    fs::create_dir_all(&html_dir).unwrap();
                    for path in final_paths {
                        let unit_id = path.file_stem().unwrap().to_string_lossy();
                        fs::write(
                            html_dir.join(format!("{unit_id}.xhtml")),
                            format!("<html><body><p>{unit_id}</p></body></html>\n"),
                        )
                        .unwrap();
                    }
                    fs::write(command.output_dir.join("output/book.epub"), "canned epub").unwrap();
                    Ok(RunnerCommandResult {
                        stdout: "wrote output/book.epub".into(),
                        stderr: String::new(),
                        log_summary: vec!["Reading builder fixture completed".into()],
                    })
                }
                BILINGUAL_BUILD_COMMAND_LABEL => {
                    assert_eq!(command.kind, RunnerCommandKind::Process);
                    assert_eq!(command.program, PathBuf::from("python3"));
                    assert_eq!(command.cwd.as_deref(), Some(command.output_dir.as_path()));
                    assert_eq!(command.accepted_exit_codes, vec![0]);
                    assert_eq!(command.args.len(), 3);
                    assert_eq!(
                        Path::new(&command.args[0])
                            .file_name()
                            .and_then(|name| name.to_str()),
                        Some("build_bilingual_epub.py")
                    );
                    assert_eq!(command.args[1], "--book-root");
                    assert_eq!(command.args[2], display_path(&command.output_dir));
                    assert!(command
                        .output_dir
                        .join("metadata/source_map.json")
                        .is_file());
                    assert!(command
                        .output_dir
                        .join("chapters/src/chapter_001.md")
                        .is_file());
                    assert!(command
                        .output_dir
                        .join("chapters/final/chapter_001.md")
                        .is_file());
                    let paragraph_count = |path: &Path| {
                        fs::read_to_string(path)
                            .unwrap()
                            .split("\n\n")
                            .filter(|paragraph| !paragraph.trim().is_empty())
                            .count()
                    };
                    let source_count =
                        paragraph_count(&command.output_dir.join("chapters/src/chapter_001.md"));
                    let target_count =
                        paragraph_count(&command.output_dir.join("chapters/final/chapter_001.md"));
                    let alignment = if self.bilingual_fallback {
                        assert_ne!(source_count, target_count);
                        "chapter-fallback"
                    } else {
                        assert_eq!(source_count, target_count);
                        "paragraph"
                    };
                    fs::write(
                        command.output_dir.join("output/book_bilingual.epub"),
                        "canned bilingual epub",
                    )
                    .unwrap();
                    Ok(RunnerCommandResult {
                        stdout: format!(
                            "chapter_001: alignment={alignment} source_paragraphs={source_count} target_paragraphs={target_count}\nwrote output/book_bilingual.epub"
                        ),
                        stderr: String::new(),
                        log_summary: vec!["Bilingual builder fixture completed".into()],
                    })
                }
                EPUBCHECK_COMMAND_LABEL => {
                    assert_eq!(command.kind, RunnerCommandKind::Process);
                    assert_eq!(command.program, PathBuf::from("java"));
                    assert_eq!(command.cwd.as_deref(), Some(command.output_dir.as_path()));
                    assert_eq!(command.accepted_exit_codes, vec![0, 1]);
                    assert_eq!(command.args[0], "-jar");
                    assert!(command.args[1].ends_with("epubcheck.jar"));
                    assert!(Path::new(&command.args[2]).is_file());
                    assert_eq!(command.args[3], "--json");
                    assert_eq!(command.args[5], "-q");
                    let epub_path = PathBuf::from(&command.args[2]);
                    let digest_epubcheck = epub_path.file_name().and_then(|name| name.to_str())
                        == Some("book_digest.epub");
                    let epubcheck_passes = if digest_epubcheck {
                        self.digest_epubcheck_passes
                    } else {
                        self.reading_epubcheck_passes
                    };
                    let report_path = PathBuf::from(&command.args[4]);
                    fs::write(
                        &report_path,
                        serde_json::to_string_pretty(&serde_json::json!({
                            "checker": {
                                "nFatal": 0,
                                "nError": if epubcheck_passes { 0 } else { 1 },
                                "nWarning": if epubcheck_passes { 1 } else { 0 },
                            }
                        }))
                        .unwrap()
                            + "\n",
                    )
                    .unwrap();
                    Ok(RunnerCommandResult {
                        stdout: if epubcheck_passes {
                            "epubcheck: fatal=0, error=0, warning=1".into()
                        } else {
                            "epubcheck: fatal=0, error=1, warning=0".into()
                        },
                        stderr: String::new(),
                        log_summary: vec!["EPUBCheck fixture completed".into()],
                    })
                }
                DIGEST_BUILD_COMMAND_LABEL if self.digest_enabled => {
                    assert_eq!(command.kind, RunnerCommandKind::Process);
                    assert_eq!(command.program, PathBuf::from("uv"));
                    assert_eq!(
                        command.cwd.as_deref(),
                        Some(local_reading_repo_root().unwrap().as_path())
                    );
                    assert_eq!(command.accepted_exit_codes, vec![0]);
                    assert_eq!(
                        command.args,
                        vec![
                            "run".to_string(),
                            "--package".to_string(),
                            "digest".to_string(),
                            "python".to_string(),
                            "-m".to_string(),
                            "digest.bibliosmith_digest".to_string(),
                            "--book-root".to_string(),
                            display_path(&command.output_dir),
                        ]
                    );
                    let config: serde_json::Value = serde_json::from_str(
                        &fs::read_to_string(command.output_dir.join("digest.config.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(config["enabled"], true);
                    assert_eq!(config["merge_into_epub"], true);
                    assert_eq!(config["source_epub"], "output/book.epub");
                    assert_eq!(config["output_epub"], "output/book_digest.epub");
                    assert_eq!(config["title"], "Digest Fixture Title");
                    assert_eq!(config["language"], "zh-CN");
                    fs::create_dir_all(command.output_dir.join("output/digest")).unwrap();
                    fs::create_dir_all(command.output_dir.join("qa/digest")).unwrap();
                    fs::write(
                        command.output_dir.join("output/book_digest.epub"),
                        "canned digest epub",
                    )
                    .unwrap();
                    fs::write(
                        command.output_dir.join("output/digest/digest.xhtml"),
                        "<html><body>Digest</body></html>\n",
                    )
                    .unwrap();
                    fs::write(
                        command.output_dir.join("output/digest/knowledge_map.svg"),
                        "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>\n",
                    )
                    .unwrap();
                    fs::write(
                        command
                            .output_dir
                            .join("qa/digest/digest_review_checklist.md"),
                        "# Digest Review\n",
                    )
                    .unwrap();
                    fs::write(
                        command.output_dir.join("qa/digest/digest_report.json"),
                        serde_json::to_string_pretty(&serde_json::json!({
                            "status": "PASS",
                            "merged": true,
                            "source_epub": "output/book.epub",
                            "output_epub": "output/book_digest.epub",
                        }))
                        .unwrap()
                            + "\n",
                    )
                    .unwrap();
                    Ok(RunnerCommandResult {
                        stdout: "digest status=PASS".into(),
                        stderr: String::new(),
                        log_summary: vec!["Digest fixture completed".into()],
                    })
                }
                other => panic!("unexpected reading pipeline command {other}"),
            }
        }
    }

    struct ZoteroBatchFixtureExecutor;

    impl RunnerCommandExecutor for ZoteroBatchFixtureExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            if command.label == ITEM_INDEX_PROFILE_COMMAND_LABEL {
                return Ok(fixture_item_index_profile_result());
            }
            if command.label == ITEM_INDEX_COMMAND_LABEL {
                let parent_item_key = command_arg_value(command, "--parent-item-key").unwrap();
                let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                return Ok(RunnerCommandResult {
                    stdout: serde_json::json!({
                        "parentItemKey": parent_item_key,
                        "sourceSha256": markdown_sha256,
                        "chunkCount": 1,
                        "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                        "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                        "embeddingProfileId": "fixture-embedding:3",
                        "completedAt": "2026-07-15T12:00:00Z",
                        "reused": false,
                    })
                    .to_string(),
                    stderr: String::new(),
                    log_summary: vec![format!("Indexed {parent_item_key}")],
                });
            }
            let key = command_arg_value(command, "--attachment-key").unwrap();
            match key {
                "DIRECT" => {
                    assert_eq!(command.label, "Zotero conversion worker");
                    assert!(command.args.iter().any(|arg| arg == "--force-text"));
                }
                "SCAN" => {
                    assert_eq!(command.label, "Zotero conversion worker");
                    assert!(command.args.iter().any(|arg| arg == "--force-ocr"));
                }
                "MINERU" => {
                    assert_eq!(command.label, "MinerU extraction wrapper");
                }
                other => panic!("unexpected batch key {other}"),
            }
            fs::create_dir_all(&command.output_dir).unwrap();
            fs::write(
                command.output_dir.join(format!("{key}.md")),
                format!("---\nparent_item_key: \"{key}PARENT\"\n---\n\n# {key}\n"),
            )
            .unwrap();
            fs::write(
                command.output_dir.join(format!("{key}.json")),
                format!("{{\"key\":\"{key}\"}}\n"),
            )
            .unwrap();
            Ok(RunnerCommandResult {
                stdout: format!("Uploaded {key}.md to Zotero attachment {key}MD"),
                stderr: String::new(),
                log_summary: vec![format!("Batch fixture completed {key}")],
            })
        }
    }

    struct RetryCollectionExecutor {
        fail_once: std::sync::Mutex<bool>,
    }

    impl RunnerCommandExecutor for RetryCollectionExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            if command.label == ITEM_INDEX_PROFILE_COMMAND_LABEL {
                return Ok(fixture_item_index_profile_result());
            }
            if command.label == ITEM_INDEX_COMMAND_LABEL {
                let parent_item_key = command_arg_value(command, "--parent-item-key").unwrap();
                let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                return Ok(RunnerCommandResult {
                    stdout: serde_json::json!({
                        "parentItemKey": parent_item_key,
                        "sourceSha256": markdown_sha256,
                        "chunkCount": 1,
                        "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                        "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                        "embeddingProfileId": "fixture-embedding:3",
                        "completedAt": "2026-07-15T12:00:00Z",
                        "reused": false,
                    })
                    .to_string(),
                    stderr: String::new(),
                    log_summary: vec![format!("Indexed {parent_item_key}")],
                });
            }
            let key = command_arg_value(command, "--attachment-key").unwrap();
            if key == "FAIL" {
                let mut fail_once = self.fail_once.lock().unwrap();
                if *fail_once {
                    *fail_once = false;
                    return Err("item diagnosis: first attempt failed".into());
                }
            }
            fs::create_dir_all(&command.output_dir).unwrap();
            fs::write(
                command.output_dir.join(format!("{key}.md")),
                format!("---\nparent_item_key: \"{key}PARENT\"\n---\n\n# {key}\n"),
            )
            .unwrap();
            Ok(RunnerCommandResult {
                stdout: format!("Uploaded {key}.md to Zotero attachment {key}MD"),
                stderr: String::new(),
                log_summary: vec![format!("Retry fixture completed {key}")],
            })
        }
    }

    struct PanicExecutor;

    impl RunnerCommandExecutor for PanicExecutor {
        fn execute(&self, _command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            panic!("fake Zotero discovery should not execute a command")
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("book-pipeline-{name}-{suffix}"))
    }

    fn fake_source(behavior: Option<&str>) -> BookPipelineSource {
        BookPipelineSource {
            kind: "fake".into(),
            title: Some("Fake source".into()),
            path: None,
            selector: None,
            runner_behavior: behavior.map(str::to_string),
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        }
    }

    fn local_pdf_source(input: &Path) -> BookPipelineSource {
        BookPipelineSource {
            kind: "local_pdf_folder".into(),
            title: Some("PDF folder".into()),
            path: Some(display_path(input)),
            selector: None,
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        }
    }

    fn fake_wrapper_root(root: &Path) -> PathBuf {
        let wrapper_root = root.join("packages").join("ocr");
        let wrapper_script = wrapper_root
            .join("scripts")
            .join("pdf_to_html_paddleocr.py");
        fs::create_dir_all(wrapper_script.parent().unwrap()).unwrap();
        fs::write(&wrapper_script, "print('fixture')\n").unwrap();
        wrapper_root
    }

    fn fake_zotero_worker_root(root: &Path) -> PathBuf {
        let worker_root = root.join("packages").join("ocr");
        let worker_script = worker_root.join("scripts").join("zotero_llm_worker.py");
        fs::create_dir_all(worker_script.parent().unwrap()).unwrap();
        fs::write(&worker_script, "print('fixture')\n").unwrap();
        worker_root
    }

    fn fake_full_worker_root(root: &Path) -> PathBuf {
        let worker_root = fake_zotero_worker_root(root);
        fs::write(worker_root.join("mineru.py"), "print('mineru fixture')\n").unwrap();
        worker_root
    }

    fn has_arg_pair(args: &[String], key: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == key && pair[1] == value)
    }

    fn has_env_pair(env: &[(String, String)], key: &str, value: &str) -> bool {
        env.iter()
            .any(|(env_key, env_value)| env_key == key && env_value == value)
    }

    fn fake_direct_zotero_source() -> BookPipelineSource {
        BookPipelineSource {
            kind: "zotero_attachment".into(),
            title: Some("Direct Text".into()),
            path: None,
            selector: Some("DIRECT".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: Some(vec![FakeZoteroItem {
                key: "DIRECT".into(),
                title: "Direct Text".into(),
                attachment_path: Some("zotero://attachment/DIRECT".into()),
                has_text_layer: true,
                dirty_text_layer: false,
                scanned: false,
                already_converted: false,
                prefer_mineru: false,
            }]),
            route_overrides: BTreeMap::new(),
        }
    }

    fn markdown_source(path: &Path) -> BookPipelineSource {
        BookPipelineSource {
            kind: "markdown_source".into(),
            title: Some("Markdown Source".into()),
            path: Some(display_path(path)),
            selector: None,
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        }
    }

    fn fake_mineru_zotero_source() -> BookPipelineSource {
        BookPipelineSource {
            kind: "zotero_attachment".into(),
            title: Some("MinerU Candidate".into()),
            path: None,
            selector: Some("MINERU".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: Some(vec![FakeZoteroItem {
                key: "MINERU".into(),
                title: "MinerU Candidate".into(),
                attachment_path: Some("zotero://attachment/MINERU".into()),
                has_text_layer: false,
                dirty_text_layer: false,
                scanned: true,
                already_converted: false,
                prefer_mineru: true,
            }]),
            route_overrides: BTreeMap::new(),
        }
    }

    fn fake_collection_source() -> BookPipelineSource {
        BookPipelineSource {
            kind: "zotero_collection".into(),
            title: Some("Mixed collection".into()),
            path: None,
            selector: Some("COLLECTION".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: Some(vec![
                FakeZoteroItem {
                    key: "DIRECT".into(),
                    title: "Direct Text".into(),
                    attachment_path: Some("zotero://attachment/DIRECT".into()),
                    has_text_layer: true,
                    dirty_text_layer: false,
                    scanned: false,
                    already_converted: false,
                    prefer_mineru: false,
                },
                FakeZoteroItem {
                    key: "SCAN".into(),
                    title: "Scanned PDF".into(),
                    attachment_path: Some("zotero://attachment/SCAN".into()),
                    has_text_layer: false,
                    dirty_text_layer: false,
                    scanned: true,
                    already_converted: false,
                    prefer_mineru: false,
                },
                FakeZoteroItem {
                    key: "MINERU".into(),
                    title: "MinerU Candidate".into(),
                    attachment_path: Some("zotero://attachment/MINERU".into()),
                    has_text_layer: false,
                    dirty_text_layer: false,
                    scanned: true,
                    already_converted: false,
                    prefer_mineru: true,
                },
                FakeZoteroItem {
                    key: "DIRTY".into(),
                    title: "Dirty Text Layer".into(),
                    attachment_path: Some("zotero://attachment/DIRTY".into()),
                    has_text_layer: true,
                    dirty_text_layer: true,
                    scanned: false,
                    already_converted: false,
                    prefer_mineru: false,
                },
                FakeZoteroItem {
                    key: "DONE".into(),
                    title: "Already Done".into(),
                    attachment_path: Some("zotero://attachment/DONE".into()),
                    has_text_layer: true,
                    dirty_text_layer: false,
                    scanned: false,
                    already_converted: true,
                    prefer_mineru: false,
                },
            ]),
            route_overrides: BTreeMap::new(),
        }
    }

    struct MemoryStateStore {
        state: Mutex<BookPipelineState>,
        output_root: PathBuf,
        save_count: Mutex<u32>,
        reject_save: bool,
    }

    impl MemoryStateStore {
        fn new(root: &Path) -> Self {
            Self {
                state: Mutex::new(BookPipelineState::default()),
                output_root: root.join("memory-output"),
                save_count: Mutex::new(0),
                reject_save: false,
            }
        }

        fn rejecting(root: &Path) -> Self {
            Self {
                reject_save: true,
                ..Self::new(root)
            }
        }
    }

    impl BookPipelineStateStore for MemoryStateStore {
        fn load(&self) -> Result<BookPipelineState, String> {
            Ok(self.state.lock().unwrap().clone())
        }

        fn save(&self, state: &BookPipelineState) -> Result<(), String> {
            if self.reject_save {
                return Err("fixture atomic save rejected".into());
            }
            let mut next = state.clone();
            next.revision = next.revision.saturating_add(1);
            *self.state.lock().unwrap() = next;
            *self.save_count.lock().unwrap() += 1;
            Ok(())
        }

        fn job_output_dir(&self, job_id: &str) -> PathBuf {
            self.output_root.join(job_id)
        }

        fn execution_owner(&self) -> Result<&str, String> {
            Ok("memory-state-owner")
        }
    }

    struct CollectionSnapshotExecutor {
        payload: String,
        calls: Mutex<u32>,
    }

    impl CollectionSnapshotExecutor {
        fn new(version: u64, include_new_member: bool) -> Self {
            let mut members = vec![
                serde_json::json!({
                    "parentItemKey": "PARENT1",
                    "parentItemType": "book",
                    "parentItemVersion": 7,
                    "parentDateModified": "2026-07-15 10:00:00",
                    "title": "Eligible PDF",
                    "attachmentKey": "PDFOK",
                    "attachmentVersion": 21,
                    "attachmentDateModified": "2026-07-15 11:00:00",
                    "contentType": "application/pdf",
                    "linkMode": 0,
                    "storagePath": "storage:ok.pdf",
                    "attachmentPath": "/private/zotero/PDFOK/ok.pdf",
                    "pathExists": true,
                    "fileSize": 1234,
                    "fileMtimeNs": 111,
                    "eligibility": "eligible_pdf",
                    "reason": null,
                }),
                serde_json::json!({
                    "parentItemKey": "PARENT2",
                    "parentItemType": "book",
                    "parentItemVersion": 8,
                    "parentDateModified": "2026-07-15 10:01:00",
                    "title": "Missing PDF",
                    "attachmentKey": "PDFMISSING",
                    "attachmentVersion": 22,
                    "attachmentDateModified": "2026-07-15 11:01:00",
                    "contentType": "application/pdf",
                    "linkMode": 0,
                    "storagePath": "storage:missing.pdf",
                    "attachmentPath": "/private/zotero/PDFMISSING/missing.pdf",
                    "pathExists": false,
                    "fileSize": null,
                    "fileMtimeNs": null,
                    "eligibility": "missing_file",
                    "reason": "PDF attachment file is missing."
                }),
                serde_json::json!({
                    "parentItemKey": "PARENT3",
                    "parentItemType": "book",
                    "parentItemVersion": 9,
                    "parentDateModified": "2026-07-15 10:02:00",
                    "title": "Unsupported attachment",
                    "attachmentKey": "TEXT1",
                    "attachmentVersion": 23,
                    "attachmentDateModified": "2026-07-15 11:02:00",
                    "contentType": "text/plain",
                    "linkMode": 0,
                    "storagePath": "storage:notes.txt",
                    "attachmentPath": "/private/zotero/TEXT1/notes.txt",
                    "pathExists": true,
                    "fileSize": 10,
                    "fileMtimeNs": 222,
                    "eligibility": "unsupported_content_type",
                    "reason": "Unsupported attachment content type: text/plain."
                }),
                serde_json::json!({
                    "parentItemKey": "PARENT4",
                    "parentItemType": "book",
                    "parentItemVersion": 10,
                    "parentDateModified": "2026-07-15 10:03:00",
                    "title": "No attachment",
                    "attachmentKey": null,
                    "attachmentVersion": null,
                    "attachmentDateModified": null,
                    "contentType": null,
                    "linkMode": null,
                    "storagePath": null,
                    "attachmentPath": null,
                    "pathExists": false,
                    "fileSize": null,
                    "fileMtimeNs": null,
                    "eligibility": "no_attachment",
                    "reason": "Collection member has no file attachment."
                }),
            ];
            if include_new_member {
                members.push(serde_json::json!({
                    "parentItemKey": "PARENT5",
                    "parentItemType": "journalArticle",
                    "parentItemVersion": 11,
                    "parentDateModified": "2026-07-15 10:04:00",
                    "title": "New PDF",
                    "attachmentKey": "PDFNEW",
                    "attachmentVersion": 24,
                    "attachmentDateModified": "2026-07-15 11:03:00",
                    "contentType": "application/pdf",
                    "linkMode": 0,
                    "storagePath": "storage:new.pdf",
                    "attachmentPath": "/private/zotero/PDFNEW/new.pdf",
                    "pathExists": true,
                    "fileSize": 5678,
                    "fileMtimeNs": 333,
                    "eligibility": "eligible_pdf",
                    "reason": null,
                }));
            }
            for member in &mut members {
                member["collectionKey"] = serde_json::json!("COLL1");
            }
            Self {
                payload: serde_json::json!({
                    "schemaVersion": "zotero-collection-snapshot-v1",
                    "collection": {
                        "key": "COLL1",
                        "name": "Direct collection",
                        "version": version,
                    },
                    "members": members,
                })
                .to_string(),
                calls: Mutex::new(0),
            }
        }

        fn without_eligible_pdf() -> Self {
            let mut executor = Self::new(11, false);
            let mut payload: serde_json::Value = serde_json::from_str(&executor.payload).unwrap();
            payload["members"]
                .as_array_mut()
                .unwrap()
                .retain(|member| member["eligibility"] != "eligible_pdf");
            executor.payload = payload.to_string();
            executor
        }

        fn with_first_member_value(mut self, field: &str, value: serde_json::Value) -> Self {
            let mut payload: serde_json::Value = serde_json::from_str(&self.payload).unwrap();
            payload["members"][0][field] = value;
            self.payload = payload.to_string();
            self
        }

        fn mixed_routes(root: &Path) -> Self {
            let specs = [
                ("DIRECT", "PARENT1", "Direct PDF", "direct.pdf"),
                ("SCAN", "PARENT2", "Scanned PDF", "scan.pdf"),
                ("MINERU", "PARENT3", "MinerU PDF", "mineru.pdf"),
                ("DONE", "PARENT4", "Already completed PDF", "done.pdf"),
                ("FAIL", "PARENT5", "Failing PDF", "fail.pdf"),
                (
                    "NOPADDLE",
                    "PARENT6",
                    "Paddle credential blocked PDF",
                    "no-paddle.pdf",
                ),
            ];
            let mut members = specs
                .iter()
                .enumerate()
                .map(|(index, (attachment_key, parent_key, title, filename))| {
                    let path = root.join("zotero").join(attachment_key).join(filename);
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(
                        &path,
                        format!("%PDF mixed collection fixture {attachment_key}\n"),
                    )
                    .unwrap();
                    let metadata = fs::metadata(&path).unwrap();
                    serde_json::json!({
                        "collectionKey": "COLL1",
                        "parentItemKey": parent_key,
                        "parentItemType": "book",
                        "parentItemVersion": 20 + index,
                        "parentDateModified": "2026-07-15 10:00:00",
                        "title": title,
                        "attachmentKey": attachment_key,
                        "attachmentVersion": 40 + index,
                        "attachmentDateModified": "2026-07-15 11:00:00",
                        "contentType": "application/pdf",
                        "linkMode": 0,
                        "storagePath": format!("storage:{filename}"),
                        "attachmentPath": display_path(&path),
                        "pathExists": true,
                        "fileSize": metadata.len(),
                        "fileMtimeNs": file_mtime_ns(&metadata).unwrap(),
                        "eligibility": "eligible_pdf",
                        "reason": null,
                    })
                })
                .collect::<Vec<_>>();
            members.push(serde_json::json!({
                "collectionKey": "COLL1",
                "parentItemKey": "PARENT7",
                "parentItemType": "book",
                "parentItemVersion": 26,
                "parentDateModified": "2026-07-15 10:00:00",
                "title": "Missing PDF",
                "attachmentKey": "MISSING",
                "attachmentVersion": 46,
                "attachmentDateModified": "2026-07-15 11:00:00",
                "contentType": "application/pdf",
                "linkMode": 0,
                "storagePath": "storage:missing.pdf",
                "attachmentPath": display_path(&root.join("zotero/MISSING/missing.pdf")),
                "pathExists": false,
                "fileSize": null,
                "fileMtimeNs": null,
                "eligibility": "missing_file",
                "reason": "PDF attachment file is missing.",
            }));
            Self {
                payload: serde_json::json!({
                    "schemaVersion": ZOTERO_COLLECTION_SNAPSHOT_SCHEMA,
                    "collection": {
                        "key": "COLL1",
                        "name": "Mixed durable collection",
                        "version": 31,
                    },
                    "members": members,
                })
                .to_string(),
                calls: Mutex::new(0),
            }
        }

        fn recovery_routes(root: &Path) -> Self {
            let mut executor = Self::mixed_routes(root);
            let mut payload: serde_json::Value = serde_json::from_str(&executor.payload).unwrap();
            payload["members"].as_array_mut().unwrap().retain(|member| {
                member["attachmentKey"]
                    .as_str()
                    .is_some_and(|key| matches!(key, "DIRECT" | "SCAN" | "MINERU" | "DONE"))
            });
            executor.payload = payload.to_string();
            executor
        }
    }

    impl RunnerCommandExecutor for CollectionSnapshotExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.label, "Zotero collection snapshot");
            assert!(has_arg_pair(&command.args, "collection-snapshot", "COLL1"));
            *self.calls.lock().unwrap() += 1;
            Ok(RunnerCommandResult {
                stdout: self.payload.clone(),
                stderr: String::new(),
                log_summary: vec!["fixture private payload must not be persisted".into()],
            })
        }
    }

    fn collection_snapshot_executor_for_pdf(root: &Path) -> (CollectionSnapshotExecutor, PathBuf) {
        let pdf = root.join("zotero").join("PDFOK").join("ok.pdf");
        fs::create_dir_all(pdf.parent().unwrap()).unwrap();
        fs::write(&pdf, b"%PDF durable collection fixture\n").unwrap();
        let metadata = fs::metadata(&pdf).unwrap();
        let mtime_ns = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let executor = CollectionSnapshotExecutor::new(11, false)
            .with_first_member_value("attachmentPath", serde_json::json!(display_path(&pdf)))
            .with_first_member_value("fileSize", serde_json::json!(metadata.len()))
            .with_first_member_value("fileMtimeNs", serde_json::json!(mtime_ns));
        (executor, pdf)
    }

    struct DurableCollectionChildExecutor {
        labels: Mutex<Vec<String>>,
        fail_index: bool,
    }

    impl DurableCollectionChildExecutor {
        fn new() -> Self {
            Self {
                labels: Mutex::new(Vec::new()),
                fail_index: false,
            }
        }

        fn failing_index() -> Self {
            Self {
                labels: Mutex::new(Vec::new()),
                fail_index: true,
            }
        }

        fn labels(&self) -> Vec<String> {
            self.labels.lock().unwrap().clone()
        }
    }

    impl RunnerCommandExecutor for DurableCollectionChildExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            self.labels.lock().unwrap().push(command.label.clone());
            match command.label.as_str() {
                "Zotero discovery dry-run" => {
                    assert!(has_arg_pair(&command.args, "--attachment-key", "PDFOK"));
                    Ok(RunnerCommandResult {
                        stdout: "12:00:00 INFO PLAN PDFOK route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=2000 title=Eligible PDF".into(),
                        stderr: String::new(),
                        log_summary: vec!["Single attachment route selected".into()],
                    })
                }
                ZOTERO_CONVERSION_COMMAND_LABEL => {
                    assert!(has_arg_pair(&command.args, "--attachment-key", "PDFOK"));
                    assert!(command.args.iter().any(|arg| arg == "--force-text"));
                    fs::create_dir_all(&command.output_dir).unwrap();
                    fs::write(
                        command.output_dir.join("PDFOK.md"),
                        "---\nparent_item_key: \"PARENT1\"\n---\n\n# Extracted\n",
                    )
                    .unwrap();
                    Ok(RunnerCommandResult {
                        stdout: "Uploaded PDFOK.md to Zotero attachment MARKDOWN1".into(),
                        stderr: String::new(),
                        log_summary: vec!["Single attachment extraction completed".into()],
                    })
                }
                ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
                ITEM_INDEX_COMMAND_LABEL => {
                    if self.fail_index {
                        return Err("Fixture item index unavailable".into());
                    }
                    assert_eq!(
                        command_arg_value(command, "--parent-item-key"),
                        Some("PARENT1")
                    );
                    let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                    Ok(RunnerCommandResult {
                        stdout: serde_json::json!({
                            "parentItemKey": "PARENT1",
                            "sourceSha256": markdown_sha256,
                            "chunkCount": 1,
                            "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                            "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                            "embeddingProfileId": "fixture-embedding:3",
                            "completedAt": "2026-07-15T12:00:00Z",
                            "reused": false,
                        })
                        .to_string(),
                        stderr: String::new(),
                        log_summary: vec!["Single attachment index completed".into()],
                    })
                }
                other => panic!("unexpected durable collection child command {other}"),
            }
        }
    }

    struct MixedDurableCollectionExecutor {
        calls: Mutex<Vec<String>>,
        completed_markdown: PathBuf,
        completed_source_sha256: String,
        fail_index_once_parent: Mutex<Option<String>>,
        indexed_markdown_sha256: Mutex<BTreeMap<String, String>>,
    }

    impl MixedDurableCollectionExecutor {
        fn new(root: &Path) -> Self {
            let completed_markdown = root.join("already-completed").join("DONE.md");
            fs::create_dir_all(completed_markdown.parent().unwrap()).unwrap();
            fs::write(
                &completed_markdown,
                "---\nparent_item_key: \"PARENT4\"\n---\n\n# Reused\n",
            )
            .unwrap();
            Self {
                calls: Mutex::new(Vec::new()),
                completed_markdown,
                completed_source_sha256: sha256_file(
                    &root.join("zotero").join("DONE").join("done.pdf"),
                )
                .unwrap(),
                fail_index_once_parent: Mutex::new(None),
                indexed_markdown_sha256: Mutex::new(BTreeMap::new()),
            }
        }

        fn failing_index_once(root: &Path, parent_item_key: &str) -> Self {
            let executor = Self::new(root);
            *executor.fail_index_once_parent.lock().unwrap() = Some(parent_item_key.into());
            executor
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn indexed_sha256(&self, parent_item_key: &str) -> Option<String> {
            self.indexed_markdown_sha256
                .lock()
                .unwrap()
                .get(parent_item_key)
                .cloned()
        }
    }

    impl RunnerCommandExecutor for MixedDurableCollectionExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            match command.label.as_str() {
                "Zotero discovery dry-run" => {
                    let key = command_arg_value(command, "--attachment-key")
                        .expect("durable routing must target one frozen attachment");
                    assert!(command.args.iter().any(|arg| arg == "--pipeline-route"));
                    self.calls.lock().unwrap().push(format!("route:{key}"));
                    let stdout = match key {
                        "DIRECT" => "12:00:00 INFO PLAN DIRECT route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=2000 title=Direct PDF".into(),
                        "SCAN" => "12:00:01 INFO PLAN SCAN route=paddle-ocr pages=20 selected=20 parent_type=book sampled_chars=0 title=Scanned PDF".into(),
                        "MINERU" => "12:00:02 INFO PLAN MINERU route=mineru pages=12 selected=12 parent_type=book sampled_chars=0 title=MinerU PDF".into(),
                        "DONE" => format!(
                            "12:00:03 INFO SKIP completed DONE Already completed PDF\n12:00:03 INFO BOOK_PIPELINE_ATTACHMENT_EVIDENCE {}",
                            serde_json::json!({
                                "schemaVersion": "zotero-worker-attachment-evidence-v1",
                                "extractionContractVersion": ZOTERO_WORKER_EXTRACTION_CONTRACT_VERSION,
                                "status": "already_completed",
                                "route": "pdf-text",
                                "pdfAttachmentKey": "DONE",
                                "parentItemKey": "PARENT4",
                                "sourceSha256": self.completed_source_sha256.clone(),
                                "markdownPath": display_path(&self.completed_markdown),
                                "markdownSha256": sha256_file(&self.completed_markdown).unwrap(),
                                "markdownAttachmentKey": "MARKDONE",
                            })
                        ),
                        "FAIL" => "12:00:04 INFO PLAN FAIL route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=1800 title=Failing PDF".into(),
                        "NOPADDLE" => "12:00:05 INFO PLAN NOPADDLE route=missing-paddleocr-token pages=10 selected=10 parent_type=book sampled_chars=0 title=Paddle credential blocked PDF".into(),
                        other => panic!("unexpected mixed route attachment {other}"),
                    };
                    Ok(RunnerCommandResult {
                        stdout,
                        stderr: String::new(),
                        log_summary: vec!["Per-attachment route fixture completed".into()],
                    })
                }
                ZOTERO_CONVERSION_COMMAND_LABEL => {
                    let key = command_arg_value(command, "--attachment-key")
                        .expect("mixed extraction must target one frozen attachment");
                    self.calls.lock().unwrap().push(format!("extract:{key}"));
                    assert!(command.args.iter().any(|arg| arg == "--preserve-source"));
                    match key {
                        "DIRECT" => assert!(command.args.iter().any(|arg| arg == "--force-text")),
                        "SCAN" => assert!(command.args.iter().any(|arg| arg == "--force-ocr")),
                        "MINERU" => {
                            assert!(command.args.iter().any(|arg| arg == "--force-mineru"))
                        }
                        "FAIL" => return Err("fixture attachment extraction failed".into()),
                        other => panic!("unexpected mixed extraction attachment {other}"),
                    }
                    let parent = match key {
                        "DIRECT" => "PARENT1",
                        "SCAN" => "PARENT2",
                        "MINERU" => "PARENT3",
                        _ => unreachable!(),
                    };
                    fs::create_dir_all(&command.output_dir).unwrap();
                    fs::write(
                        command.output_dir.join(format!("{key}.md")),
                        format!("---\nparent_item_key: \"{parent}\"\n---\n\n# {key}\n"),
                    )
                    .unwrap();
                    Ok(RunnerCommandResult {
                        stdout: format!("Uploaded {key}.md to Zotero attachment MARK{key}"),
                        stderr: String::new(),
                        log_summary: vec!["Single attachment extraction completed".into()],
                    })
                }
                ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
                ITEM_INDEX_COMMAND_LABEL => {
                    let parent = command_arg_value(command, "--parent-item-key").unwrap();
                    self.calls.lock().unwrap().push(format!("index:{parent}"));
                    let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                    let mut fail_once = self.fail_index_once_parent.lock().unwrap();
                    if fail_once.as_deref() == Some(parent) {
                        fail_once.take();
                        return Err("fixture item index interrupted once".into());
                    }
                    drop(fail_once);
                    self.indexed_markdown_sha256
                        .lock()
                        .unwrap()
                        .insert(parent.to_string(), markdown_sha256.to_string());
                    Ok(RunnerCommandResult {
                        stdout: serde_json::json!({
                            "parentItemKey": parent,
                            "sourceSha256": markdown_sha256,
                            "chunkCount": 1,
                            "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                            "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                            "embeddingProfileId": "fixture-embedding:3",
                            "completedAt": "2026-07-15T12:00:00Z",
                            "reused": false,
                        })
                        .to_string(),
                        stderr: String::new(),
                        log_summary: vec!["Single attachment index completed".into()],
                    })
                }
                other => panic!("unexpected mixed collection command {other}"),
            }
        }
    }

    struct PanicPipelineRunner;

    impl PipelineRunner for PanicPipelineRunner {
        fn run(&self, _job: &BookPipelineJob, _output_dir: &Path) -> Result<RunnerOutput, String> {
            panic!("durable collection execution must not invoke the batch runner")
        }

        fn route_attachment(
            &self,
            _job: &BookPipelineJob,
            child: &BookPipelineChildJob,
            _output_dir: &Path,
        ) -> Result<AttachmentRouteOutput, String> {
            Ok(AttachmentRouteOutput {
                route: BookPipelineRouteItem {
                    id: child.source.selector.clone().unwrap(),
                    title: child.source.title.clone().unwrap(),
                    source_kind: "zotero_attachment".into(),
                    source_ref: child.source.path.clone().unwrap(),
                    route_kind: "direct_text".into(),
                    can_run: true,
                    blocked_reason: None,
                    summary: "Fixture direct attachment route".into(),
                    route_override: None,
                },
                log_summary: vec!["Fixture attachment route selected".into()],
                reused_artifact: None,
            })
        }
    }

    fn real_collection_source() -> BookPipelineSource {
        BookPipelineSource {
            kind: "zotero_collection".into(),
            title: Some("Selected collection".into()),
            path: None,
            selector: Some("COLL1".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        }
    }

    fn fast_translation_intent() -> BookPipelineTranslationIntent {
        BookPipelineTranslationIntent {
            translation_mode: TRANSLATION_MODE_FAST.into(),
            profile_id: "fixture-profile".into(),
            config_id: "fixture-config".into(),
            skill_ids: Vec::new(),
            second_pass_enabled: false,
            text_cleanup: false,
            digest_mode: false,
            output_formats: default_output_formats(),
        }
    }

    fn cleanup_fixture_job(
        root: &Path,
        store: &BookPipelineStore,
        zotero_key: Option<&str>,
    ) -> BookPipelineJob {
        let output_dir = root.join("cleanup-output");
        fs::create_dir_all(&output_dir).unwrap();
        let markdown = output_dir.join("book.md");
        fs::write(&markdown, "# Clean Markdown\n").unwrap();
        let source_pdf = root.join("source.pdf");
        fs::write(&source_pdf, "%PDF fixture").unwrap();
        let mut source = fake_direct_zotero_source();
        source.path = Some(display_path(&source_pdf));
        let job = queue_job(
            store,
            source,
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let mut state = store.load().unwrap();
        let stored = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap();
        stored.status = STATUS_COMPLETED.into();
        stored.current_step = "Completed".into();
        stored.output_dir = Some(display_path(&output_dir));
        stored.artifacts = vec![
            BookPipelineArtifact {
                kind: "output_dir".into(),
                path: display_path(&output_dir),
                sha256: None,
                zotero_key: None,
                producer_stage: None,
                ..BookPipelineArtifact::default()
            },
            BookPipelineArtifact {
                kind: "markdown".into(),
                path: display_path(&markdown),
                sha256: Some(sha256_file(&markdown).unwrap()),
                zotero_key: zotero_key.map(str::to_string),
                producer_stage: None,
                ..BookPipelineArtifact::default()
            },
        ];
        let job = stored.clone();
        store.save(&state).unwrap();
        job
    }

    #[test]
    fn delete_job_removes_a_queued_job_and_persists() {
        let root = temp_root("delete-queued-job");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let state = delete_job(&store, &job.id, true).unwrap();

        assert!(state.jobs.is_empty());
        assert!(store.load().unwrap().jobs.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_job_requires_explicit_approval_and_a_known_job() {
        let root = temp_root("delete-job-guards");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let refused = delete_job(&store, &job.id, false).unwrap_err();
        assert!(refused.contains("Explicit approval"), "got: {refused}");
        assert_eq!(store.load().unwrap().jobs.len(), 1);

        let missing = delete_job(&store, "job-nonexistent", true).unwrap_err();
        assert!(missing.contains("not found"), "got: {missing}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_job_with_a_running_stage_counts_as_actively_running() {
        let root = temp_root("delete-running-guard");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        assert!(!job_is_actively_running(&job));
        let mut running = job;
        start_stage(&mut running.children[0], "extract", "test-owner");
        assert!(job_is_actively_running(&running));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn book_ocr_conversion_root_prefers_monorepo_ocr_package() {
        let root = book_ocr_conversion_root();

        assert!(
            root.ends_with(Path::new("packages").join("ocr")),
            "expected monorepo OCR package root, got {}",
            display_path(&root)
        );
    }

    #[test]
    fn legacy_jobs_migrate_to_versioned_parent_child_stage_state() {
        let root = temp_root("legacy-state-migration");
        let store = BookPipelineStore::for_test(&root);
        fs::create_dir_all(store.state_path.parent().unwrap()).unwrap();
        let legacy_job = |id: &str, status: &str| {
            serde_json::json!({
                "id": id,
                "mode": "convert_then_translate",
                "source": {
                    "kind": "zotero_attachment",
                    "title": "Fabricated source",
                    "path": "zotero://attachment/FAKEPDF",
                    "selector": "FAKEPDF",
                    "runnerBehavior": null,
                    "translationStrategy": "reflection",
                    "adapterCommand": null,
                    "fakeZoteroItems": null
                },
                "route": [{
                    "id": "FAKEPDF",
                    "title": "Fabricated source",
                    "sourceKind": "zotero_attachment",
                    "sourceRef": "zotero://attachment/FAKEPDF",
                    "routeKind": "direct_text",
                    "canRun": true,
                    "blockedReason": null,
                    "summary": "Fabricated route"
                }],
                "status": status,
                "currentStep": "Legacy state",
                "lastError": "preserved diagnosis",
                "logSummary": ["preserved log"],
                "artifacts": [{
                    "kind": "markdown",
                    "path": "/tmp/fabricated.md",
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "zoteroKey": "FAKEMD"
                }],
                "collectionItems": [{
                    "id": "FAKEPDF",
                    "title": "Fabricated source",
                    "routeKind": "direct_text",
                    "status": "completed",
                    "lastError": null,
                    "artifacts": [],
                    "attempts": 2
                }],
                "outputDir": "/tmp/fabricated-output",
                "attempts": 2,
                "createdAt": "2026-07-10T09:00:00+08:00",
                "updatedAt": "2026-07-10T09:05:00+08:00"
            })
        };
        let legacy = serde_json::json!({
            "jobs": [
                legacy_job("legacy-routed", STATUS_ROUTED),
                legacy_job("legacy-handoff", STATUS_HANDOFF_RUNNING),
                legacy_job("legacy-ready", STATUS_TRANSLATION_READY)
            ]
        });
        fs::write(
            &store.state_path,
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let state = store.load().unwrap();

        assert_eq!(state.schema_version, STATE_SCHEMA_VERSION);
        assert_eq!(state.revision, 1);
        let routed = state
            .jobs
            .iter()
            .find(|job| job.id == "legacy-routed")
            .unwrap();
        assert_eq!(routed.schema_version, JOB_SCHEMA_VERSION);
        assert_eq!(routed.translation_mode, TRANSLATION_MODE_FAST);
        assert!(!routed.second_pass_enabled);
        assert!(!routed.text_cleanup);
        assert_eq!(
            routed.source.translation_strategy.as_deref(),
            Some("reflection")
        );
        assert_eq!(routed.status, STATUS_READY);
        assert_eq!(routed.attempts, 2);
        assert_eq!(routed.last_error.as_deref(), Some("preserved diagnosis"));
        assert_eq!(routed.collection_items.len(), 1);
        assert_eq!(routed.artifacts.len(), 1);
        assert_eq!(routed.children.len(), 1);
        assert_eq!(routed.children[0].current_stage_id, "extract");
        assert_eq!(routed.children[0].status, STATUS_READY);

        let handoff = state
            .jobs
            .iter()
            .find(|job| job.id == "legacy-handoff")
            .unwrap();
        assert_eq!(handoff.current_stage_id, "handoff");
        assert_eq!(handoff.status, STATUS_RUNNING);
        assert_eq!(handoff.children[0].status, STATUS_RUNNING);
        assert_eq!(
            child_stage_status(handoff, "index"),
            STATUS_SKIPPED,
            "legacy downstream progress must not fabricate completed index evidence"
        );
        assert!(handoff.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "handoff")
            .unwrap()
            .execution_owner
            .is_some());

        let translation_ready = state
            .jobs
            .iter()
            .find(|job| job.id == "legacy-ready")
            .unwrap();
        assert_eq!(translation_ready.current_stage_id, "split");
        assert_eq!(translation_ready.status, STATUS_READY);
        assert_eq!(translation_ready.children[0].status, STATUS_READY);
        assert_eq!(
            translation_ready.children[0]
                .stages
                .iter()
                .find(|stage| stage.stage_id == "handoff")
                .unwrap()
                .status,
            STATUS_COMPLETED
        );

        let persisted: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
        assert_eq!(persisted["schemaVersion"], STATE_SCHEMA_VERSION);
        assert_eq!(persisted["revision"], 1);
        assert!(persisted["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|job| job["schemaVersion"] == JOB_SCHEMA_VERSION));

        let recovered_after_migration = store.load().unwrap();
        let handoff = recovered_after_migration
            .jobs
            .iter()
            .find(|job| job.id == "legacy-handoff")
            .unwrap();
        assert_eq!(handoff.current_stage_id, "handoff");
        assert_eq!(handoff.status, STATUS_FAILED);
        assert!(handoff.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "handoff")
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("interrupted"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn queued_translation_modes_and_binding_identity_survive_persistence() {
        let root = temp_root("translation-mode-persistence");
        let store = BookPipelineStore::for_test(&root);
        let fast = queue_job_with_translation_intent(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelineTranslationIntent {
                translation_mode: TRANSLATION_MODE_FAST.into(),
                profile_id: "fake-provider-profile".into(),
                config_id: "fake-provider-config".into(),
                skill_ids: Vec::new(),
                second_pass_enabled: true,
                text_cleanup: true,
                digest_mode: false,
                output_formats: default_output_formats(),
            },
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let expert = queue_job_with_translation_intent(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelineTranslationIntent {
                translation_mode: TRANSLATION_MODE_EXPERT.into(),
                profile_id: "fake-agent-profile".into(),
                config_id: "fake-agent-config".into(),
                skill_ids: vec!["expert-translation-quality".into()],
                second_pass_enabled: false,
                text_cleanup: false,
                digest_mode: false,
                output_formats: default_output_formats(),
            },
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let state = store.load().unwrap();
        let persisted_fast = state.jobs.iter().find(|job| job.id == fast.id).unwrap();
        assert_eq!(persisted_fast.translation_mode, TRANSLATION_MODE_FAST);
        assert_eq!(
            persisted_fast.translation_profile_id,
            "fake-provider-profile"
        );
        assert_eq!(persisted_fast.translation_config_id, "fake-provider-config");
        assert!(persisted_fast.translation_skill_ids.is_empty());
        assert!(persisted_fast.second_pass_enabled);
        assert!(persisted_fast.text_cleanup);
        let persisted_expert = state.jobs.iter().find(|job| job.id == expert.id).unwrap();
        assert_eq!(persisted_expert.translation_mode, TRANSLATION_MODE_EXPERT);
        assert_eq!(
            persisted_expert.translation_profile_id,
            "fake-agent-profile"
        );
        assert_eq!(persisted_expert.translation_config_id, "fake-agent-config");
        assert_eq!(
            persisted_expert.translation_skill_ids,
            vec!["expert-translation-quality"]
        );
        assert!(!persisted_expert.second_pass_enabled);
        assert!(!persisted_expert.text_cleanup);

        let persisted_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
        assert!(persisted_json["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|job| matches!(job["translationMode"].as_str(), Some("fast" | "expert"))));
        assert!(persisted_json["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["translationMode"] == "fast" && job["secondPassEnabled"] == true));
        assert!(persisted_json["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["translationMode"] == "fast" && job["textCleanup"] == true));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn queued_digest_mode_is_book_level_and_survives_persistence() {
        let root = temp_root("digest-mode-persistence");
        let store = BookPipelineStore::for_test(&root);
        let fast_intent: BookPipelineTranslationIntent =
            serde_json::from_value(serde_json::json!({
                "translationMode": TRANSLATION_MODE_FAST,
                "profileId": "fake-provider-profile",
                "configId": "fake-provider-config",
                "skillIds": [],
                "secondPassEnabled": false,
                "digestMode": true,
            }))
            .unwrap();
        let expert_intent: BookPipelineTranslationIntent =
            serde_json::from_value(serde_json::json!({
                "translationMode": TRANSLATION_MODE_EXPERT,
                "profileId": "fake-agent-profile",
                "configId": "fake-agent-config",
                "skillIds": ["expert-translation-quality"],
                "secondPassEnabled": false,
                "digestMode": true,
            }))
            .unwrap();
        let fast = queue_job_with_translation_intent(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_intent,
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let expert = queue_job_with_translation_intent(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            expert_intent,
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let persisted: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
        for job_id in [fast.id, expert.id] {
            let job = persisted["jobs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|job| job["id"] == job_id)
                .unwrap();
            assert_eq!(job["digestMode"], true);
            assert_eq!(
                job["outputFormats"],
                serde_json::json!(["md", "html", "epub"])
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn queued_output_formats_are_ordered_deduplicated_and_persisted() {
        let root = temp_root("output-formats-persistence");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job_with_translation_intent(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelineTranslationIntent {
                translation_mode: TRANSLATION_MODE_FAST.into(),
                profile_id: "fake-provider-profile".into(),
                config_id: "fake-provider-config".into(),
                skill_ids: Vec::new(),
                second_pass_enabled: false,
                text_cleanup: false,
                digest_mode: false,
                output_formats: vec![
                    "bilingual".into(),
                    "epub".into(),
                    "BILINGUAL".into(),
                    "md".into(),
                ],
            },
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        assert_eq!(job.output_formats, vec!["bilingual", "epub", "md"]);
        let persisted: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
        let stored = persisted["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == job.id)
            .unwrap();
        assert_eq!(
            stored["outputFormats"],
            serde_json::json!(["bilingual", "epub", "md"])
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn expert_translation_intent_rejects_fast_only_second_pass() {
        let error = validate_translation_intent(&BookPipelineTranslationIntent {
            translation_mode: TRANSLATION_MODE_EXPERT.into(),
            profile_id: "fake-agent-profile".into(),
            config_id: "fake-agent-config".into(),
            skill_ids: vec!["expert-translation-quality".into()],
            second_pass_enabled: true,
            text_cleanup: false,
            digest_mode: false,
            output_formats: default_output_formats(),
        })
        .unwrap_err();

        assert!(error.contains("only available in fast translation mode"));
    }

    #[test]
    fn expert_translation_intent_rejects_fast_only_text_cleanup() {
        let error = validate_translation_intent(&BookPipelineTranslationIntent {
            translation_mode: TRANSLATION_MODE_EXPERT.into(),
            profile_id: "fake-agent-profile".into(),
            config_id: "fake-agent-config".into(),
            skill_ids: vec!["expert-translation-quality".into()],
            second_pass_enabled: false,
            text_cleanup: true,
            digest_mode: false,
            output_formats: default_output_formats(),
        })
        .unwrap_err();

        assert!(error.contains("only available in fast translation mode"));
    }

    #[test]
    fn legacy_collection_migration_preserves_route_union_and_parent_handoff_state() {
        let root = temp_root("legacy-collection-state-migration");
        let store = BookPipelineStore::for_test(&root);
        fs::create_dir_all(store.state_path.parent().unwrap()).unwrap();
        let legacy = serde_json::json!({
            "jobs": [{
                "id": "legacy-collection-handoff",
                "mode": MODE_CONVERT_THEN_TRANSLATE,
                "source": {
                    "kind": "zotero_collection",
                    "title": "Fabricated collection",
                    "path": null,
                    "selector": "FAKECOLL",
                    "runnerBehavior": null,
                    "translationStrategy": null,
                    "adapterCommand": null,
                    "fakeZoteroItems": null
                },
                "route": [{
                    "id": "ROUTEONLY",
                    "title": "Route-only attachment",
                    "sourceKind": "zotero_attachment",
                    "sourceRef": "zotero://attachment/ROUTEONLY",
                    "routeKind": "direct_text",
                    "canRun": true,
                    "blockedReason": null,
                    "summary": "Fabricated route"
                }],
                "status": STATUS_HANDOFF_RUNNING,
                "currentStep": "Legacy collection handoff",
                "lastError": null,
                "logSummary": [],
                "artifacts": [],
                "collectionItems": [{
                    "id": "RESULTONLY",
                    "title": "Result-only attachment",
                    "routeKind": "direct_text",
                    "status": STATUS_COMPLETED,
                    "lastError": null,
                    "artifacts": [],
                    "attempts": 3
                }],
                "outputDir": null,
                "attempts": 3,
                "createdAt": "2026-07-10T09:00:00+08:00",
                "updatedAt": "2026-07-10T09:05:00+08:00"
            }]
        });
        fs::write(
            &store.state_path,
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let state = store.load().unwrap();
        let job = &state.jobs[0];

        assert_eq!(job.status, STATUS_RUNNING);
        assert_eq!(job.children.len(), 2);
        assert_eq!(
            job.children
                .iter()
                .map(|child| child.source.selector.as_deref().unwrap())
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["RESULTONLY", "ROUTEONLY"])
        );
        let handoff_child = job
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("RESULTONLY"))
            .unwrap();
        assert_eq!(handoff_child.current_stage_id, "handoff");
        assert_eq!(handoff_child.status, STATUS_RUNNING);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn collection_parent_derives_partial_status_from_durable_children() {
        let root = temp_root("collection-status-aggregation");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_collection_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
        )
        .unwrap();

        assert_eq!(job.kind, "collection");
        assert_eq!(job.children.len(), 5);
        assert_eq!(
            job.membership.as_ref().unwrap().child_job_ids.len(),
            job.children.len()
        );

        let mut state = store.load().unwrap();
        let stored = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap();
        for child in &mut stored.children {
            if matches!(child.source.selector.as_deref(), Some("DIRECT" | "SCAN")) {
                start_stage(child, "extract", store.execution_owner().unwrap());
            }
        }
        store.save(&state).unwrap();

        let mut state = store.load().unwrap();
        let stored = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap();
        for child in &mut stored.children {
            let status = match child.source.selector.as_deref().unwrap() {
                "DIRECT" => STATUS_COMPLETED,
                "SCAN" => STATUS_FAILED,
                "MINERU" | "DIRTY" => STATUS_BLOCKED,
                "DONE" => STATUS_SKIPPED,
                other => panic!("unexpected fixture child {other}"),
            };
            child.stages[0].status = if child.source.selector.as_deref() == Some("DIRTY") {
                STATUS_BLOCKED.into()
            } else if status == STATUS_SKIPPED {
                STATUS_SKIPPED.into()
            } else {
                STATUS_COMPLETED.into()
            };
            if child.source.selector.as_deref() == Some("DIRTY") {
                set_stage_status(child, "extract", STATUS_PENDING, None);
            } else {
                set_stage_status(child, "extract", status, None);
            }
            if status == STATUS_COMPLETED {
                set_stage_status(child, "index", STATUS_SKIPPED, None);
            }
        }
        store.save(&state).unwrap();

        let recovered = store.load().unwrap();
        let parent = recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap();
        assert_eq!(parent.status, STATUS_PARTIAL);
        assert_eq!(parent.summary.total, 5);
        assert_eq!(parent.summary.completed, 1);
        assert_eq!(parent.summary.failed, 1);
        assert_eq!(parent.summary.blocked, 2);
        assert_eq!(parent.summary.skipped, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn versioned_stage_units_and_approvals_survive_restart() {
        let root = temp_root("versioned-restart-recovery");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let mut state = store.load().unwrap();
        let stored = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap();
        let child = stored.children.first_mut().unwrap();
        let translate = child
            .stages
            .iter_mut()
            .find(|stage| stage.stage_id == "translate")
            .unwrap();
        translate.unit_summary = Some(BookPipelineUnitSummary {
            total: 2,
            completed: 1,
            failed: 1,
            ..BookPipelineUnitSummary::default()
        });
        stored.approval_references = vec![BookPipelineApprovalReference {
            approval_id: "approval-fake-1".into(),
            gate_id: "translation_disclosure".into(),
            child_job_id: child.id.clone(),
            stage_id: "approve_translation".into(),
            decision: "approved".into(),
            bound_artifact_hashes: std::collections::BTreeMap::from([(
                "task-manifest".into(),
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            )]),
        }];
        store.save(&state).unwrap();

        let restarted_store = BookPipelineStore::for_test(&root);
        let recovered = restarted_store.load().unwrap();
        let recovered_job = recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap();
        let recovered_child = recovered_job.children.first().unwrap();
        assert_eq!(
            recovered_child
                .stages
                .iter()
                .map(|stage| stage.stage_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "route",
                "extract",
                "index",
                "handoff",
                "split",
                "prepare",
                "approve_translation",
                "translate",
                "expert_qa",
                "approve_promotion",
                "promote",
                "build_reading",
                "validate_reading",
                "build_digest"
            ]
        );
        assert_eq!(
            recovered_child
                .stages
                .iter()
                .find(|stage| stage.stage_id == "build_digest")
                .unwrap()
                .status,
            STATUS_SKIPPED
        );
        let unit_summary = recovered_child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "translate")
            .unwrap()
            .unit_summary
            .as_ref()
            .unwrap();
        assert_eq!(unit_summary.total, 2);
        assert_eq!(unit_summary.completed, 1);
        assert_eq!(unit_summary.failed, 1);
        assert_eq!(recovered_job.approval_references.len(), 1);
        assert!(recovered_child
            .stages
            .iter()
            .all(|stage| !stage.contract_version.is_empty()));
        assert_eq!(
            recovered_job.approval_references[0]
                .bound_artifact_hashes
                .get("task-manifest")
                .map(String::as_str),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn foreign_running_stage_recovers_as_retryable_failure_after_restart() {
        let root = temp_root("interrupted-stage-recovery");
        let store = BookPipelineStore::for_test_with_owner(&root, "worker-before-restart");
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        store.execution_owner().unwrap();
        let mut state = store.load().unwrap();
        let extract = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter_mut()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();
        extract.status = STATUS_RUNNING.into();
        extract.execution_owner = Some("worker-before-restart".into());
        store.save(&state).unwrap();
        let running_revision = store.load().unwrap().revision;
        drop(store);

        let restarted = BookPipelineStore::for_test_with_owner(&root, "worker-after-restart")
            .load()
            .unwrap();
        let recovered = restarted
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap();
        let extract = recovered.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();

        assert_eq!(extract.status, STATUS_FAILED);
        assert!(extract.error.as_deref().unwrap().contains("interrupted"));
        assert_eq!(extract.execution_owner, None);
        assert_eq!(restarted.revision, running_revision + 1);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(any(unix, target_os = "windows"))]
    #[test]
    fn live_foreign_execution_owner_is_not_mistaken_for_restart() {
        let root = temp_root("live-foreign-stage-owner");
        #[cfg(unix)]
        let mut live_process = Command::new("sh")
            .args(["-c", "while :; do sleep 1; done"])
            .spawn()
            .unwrap();
        #[cfg(target_os = "windows")]
        let mut live_process = Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
            .spawn()
            .unwrap();
        let writer_owner = format!("process-{}-writer", live_process.id());
        let reader_owner = new_execution_owner();
        let writer = BookPipelineStore::for_test_with_owner(&root, &writer_owner);
        let job = queue_job(
            &writer,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        writer.execution_owner().unwrap();
        let mut state = writer.load().unwrap();
        let extract = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter_mut()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();
        extract.status = STATUS_RUNNING.into();
        extract.execution_owner = Some(writer_owner.clone());
        writer.save(&state).unwrap();
        let running_revision = writer.load().unwrap().revision;

        let observed = BookPipelineStore::for_test_with_owner(&root, &reader_owner)
            .load()
            .unwrap();
        let extract = observed
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();

        assert_eq!(extract.status, STATUS_RUNNING);
        assert_eq!(
            extract.execution_owner.as_deref(),
            Some(writer_owner.as_str())
        );
        assert_eq!(observed.revision, running_revision);
        live_process.kill().unwrap();
        live_process.wait().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(any(unix, target_os = "windows"))]
    #[test]
    fn live_unrelated_pid_without_matching_lease_is_interrupted() {
        let root = temp_root("live-unrelated-pid-stage-owner");
        #[cfg(unix)]
        let mut live_process = Command::new("sh")
            .args(["-c", "while :; do sleep 1; done"])
            .spawn()
            .unwrap();
        #[cfg(target_os = "windows")]
        let mut live_process = Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
            .spawn()
            .unwrap();
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let mut state = store.load().unwrap();
        let extract = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter_mut()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();
        extract.status = STATUS_RUNNING.into();
        extract.execution_owner = Some(format!("process-{}-stale", live_process.id()));
        store.write_state_unlocked(&state).unwrap();

        let recovered = store.load().unwrap();
        let extract = recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();

        assert_eq!(extract.status, STATUS_FAILED);
        live_process.kill().unwrap();
        live_process.wait().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reused_current_pid_does_not_keep_stale_owner_alive() {
        let root = temp_root("reused-current-pid-stage-owner");
        let stale_owner = format!("process-{}-stale-owner", std::process::id());
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let mut state = store.load().unwrap();
        let extract = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter_mut()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();
        extract.status = STATUS_RUNNING.into();
        extract.execution_owner = Some(stale_owner);
        store.write_state_unlocked(&state).unwrap();

        let recovered = store.load().unwrap();
        let extract = recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();

        assert_eq!(extract.status, STATUS_FAILED);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ownerless_running_stage_recovers_as_interrupted() {
        let root = temp_root("ownerless-stage-recovery");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let mut state = store.load().unwrap();
        let extract = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter_mut()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();
        extract.status = STATUS_RUNNING.into();
        extract.execution_owner = None;
        store.write_state_unlocked(&state).unwrap();

        let recovered = store.load().unwrap();
        let extract = recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();

        assert_eq!(extract.status, STATUS_FAILED);
        assert!(extract.error.as_deref().unwrap().contains("interrupted"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn frozen_collection_membership_rejects_child_drift() {
        let root = temp_root("frozen-collection-membership");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_collection_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
        )
        .unwrap();
        let mut state = store.load().unwrap();
        state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children
            .pop();

        let error = store.save(&state).unwrap_err();

        assert!(error.contains("frozen membership"));
        assert_eq!(
            store
                .load()
                .unwrap()
                .jobs
                .iter()
                .find(|stored| stored.id == job.id)
                .unwrap()
                .children
                .len(),
            job.children.len()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn real_collection_discovery_atomically_freezes_durable_attachment_children() {
        let root = temp_root("collection-snapshot-freeze");
        let store = MemoryStateStore::new(&root);
        let executor = CollectionSnapshotExecutor::new(11, false);

        let job = queue_job_with_translation_intent_and_executor(
            &store,
            &executor,
            real_collection_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        assert_eq!(*executor.calls.lock().unwrap(), 1);
        assert_eq!(*store.save_count.lock().unwrap(), 1);
        assert_eq!(job.kind, "collection");
        assert!(job.collection_items.is_empty());
        let membership = job.membership.as_ref().unwrap();
        assert_eq!(membership.collection_key, "COLL1");
        assert_eq!(membership.revision, 1);
        assert_eq!(membership.snapshot_sha256.len(), 64);
        assert_eq!(membership.child_job_ids.len(), 3);
        assert_eq!(job.children.len(), 3);
        assert!(job
            .children
            .iter()
            .all(|child| child.id.contains("-r1-") && child.parent_job_id == job.id));
        let eligible = job
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
            .unwrap();
        assert_eq!(stage_ref(eligible, "route").unwrap().status, STATUS_READY);
        assert_eq!(
            stage_ref(eligible, "extract").unwrap().status,
            STATUS_PENDING
        );
        let identity = eligible.source_identity.as_ref().unwrap();
        assert_eq!(identity.collection_key, "COLL1");
        assert_eq!(identity.parent_item_key, "PARENT1");
        assert_eq!(identity.pdf_attachment_key, "PDFOK");
        assert_eq!(identity.content_type, "application/pdf");
        assert_eq!(identity.file_size, Some(1234));
        assert_eq!(identity.file_mtime_ns, Some(111));
        let missing = job
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFMISSING"))
            .unwrap();
        assert_eq!(stage_ref(missing, "route").unwrap().status, STATUS_BLOCKED);
        assert!(stage_ref(missing, "route")
            .unwrap()
            .error
            .as_deref()
            .is_some_and(|error| error.contains("missing")));
        assert!(job.artifacts.iter().any(|artifact| {
            artifact.kind == "collection_manifest"
                && artifact.sha256.is_some()
                && artifact.producer.stage_id == "discover"
        }));
        let persisted = store.load().unwrap();
        assert_eq!(persisted.jobs.len(), 1);
        assert_eq!(persisted.jobs[0].membership, job.membership);
        assert_eq!(persisted.jobs[0].children.len(), 3);
        assert!(!serde_json::to_string(&persisted)
            .unwrap()
            .contains("fixture private payload"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn identical_collection_snapshot_is_idempotent_and_changed_snapshot_revises() {
        let root = temp_root("collection-snapshot-idempotency");
        let store = MemoryStateStore::new(&root);
        let first_executor = CollectionSnapshotExecutor::new(11, false);
        let first = queue_job_with_translation_intent_and_executor(
            &store,
            &first_executor,
            real_collection_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let same_executor = CollectionSnapshotExecutor::new(11, false);

        let same = queue_job_with_translation_intent_and_executor(
            &store,
            &same_executor,
            real_collection_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        assert_eq!(same.id, first.id);
        assert_eq!(*store.save_count.lock().unwrap(), 1);
        assert_eq!(store.load().unwrap().jobs.len(), 1);

        let changed_executor = CollectionSnapshotExecutor::new(12, true);
        let changed = queue_job_with_translation_intent_and_executor(
            &store,
            &changed_executor,
            real_collection_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        assert_ne!(changed.id, first.id);
        assert_eq!(changed.membership.as_ref().unwrap().revision, 2);
        assert_eq!(changed.children.len(), 4);
        assert_eq!(store.load().unwrap().jobs.len(), 2);
        assert_eq!(*store.save_count.lock().unwrap(), 2);
        let old = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == first.id)
            .unwrap();
        assert_eq!(old.membership.as_ref().unwrap().revision, 1);
        assert_eq!(old.children.len(), 3);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejected_state_save_publishes_neither_membership_nor_children() {
        let root = temp_root("collection-snapshot-atomic-rejection");
        let store = MemoryStateStore::rejecting(&root);
        let executor = CollectionSnapshotExecutor::new(11, false);

        let error = queue_job_with_translation_intent_and_executor(
            &store,
            &executor,
            real_collection_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap_err();

        assert!(error.contains("atomic save rejected"));
        assert!(store.load().unwrap().jobs.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn collection_snapshot_without_eligible_pdf_is_durably_explainable() {
        let root = temp_root("collection-snapshot-no-eligible-pdf");
        let store = MemoryStateStore::new(&root);
        let executor = CollectionSnapshotExecutor::without_eligible_pdf();

        let job = queue_job_with_translation_intent_and_executor(
            &store,
            &executor,
            real_collection_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        assert_eq!(job.status, STATUS_BLOCKED);
        assert_eq!(job.current_stage_id, "discover");
        assert_eq!(job.last_error.as_deref(), Some("no_eligible_pdf"));
        assert_eq!(job.children.len(), 2);
        assert!(job
            .children
            .iter()
            .all(|child| stage_ref(child, "route").unwrap().status == STATUS_BLOCKED));
        assert_eq!(store.load().unwrap().jobs.len(), 1);
        assert_eq!(*store.save_count.lock().unwrap(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn collection_snapshot_rejects_inconsistent_eligible_file_evidence() {
        for (case, executor) in [
            (
                "missing-path",
                CollectionSnapshotExecutor::new(11, false)
                    .with_first_member_value("pathExists", serde_json::json!(false)),
            ),
            (
                "non-pdf-content",
                CollectionSnapshotExecutor::new(11, false)
                    .with_first_member_value("contentType", serde_json::json!("text/plain")),
            ),
            (
                "wrong-link-mode",
                CollectionSnapshotExecutor::new(11, false)
                    .with_first_member_value("linkMode", serde_json::json!(2)),
            ),
            (
                "mismatched-resolved-path",
                CollectionSnapshotExecutor::new(11, false).with_first_member_value(
                    "attachmentPath",
                    serde_json::json!("/private/zotero/OTHER/secret.pdf"),
                ),
            ),
        ] {
            let root = temp_root(&format!("collection-snapshot-invalid-{case}"));
            let store = MemoryStateStore::new(&root);

            let error = queue_job_with_translation_intent_and_executor(
                &store,
                &executor,
                real_collection_source(),
                "conversion_only".into(),
                fast_translation_intent(),
                BookPipelinePreviewConfig::default(),
            )
            .unwrap_err();

            assert!(error.contains("inconsistent attachment evidence"));
            assert!(store.load().unwrap().jobs.is_empty());
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn public_collection_discovery_and_preview_use_readonly_snapshot_contract() {
        let discovery_executor = CollectionSnapshotExecutor::new(11, false);

        let discovery =
            discover_zotero_sources(&discovery_executor, &real_collection_source(), 20).unwrap();

        assert_eq!(*discovery_executor.calls.lock().unwrap(), 1);
        assert_eq!(discovery.sources.len(), 1);
        assert_eq!(discovery.sources[0].kind, "zotero_collection");
        assert_eq!(discovery.sources[0].selector.as_deref(), Some("COLL1"));
        assert_eq!(
            discovery.sources[0].title.as_deref(),
            Some("Direct collection")
        );
        assert!(discovery.sources[0].fake_zotero_items.is_none());
        assert!(discovery
            .log_summary
            .iter()
            .any(|line| line.contains("members=4 attachments=3 eligible=1")));

        let preview_executor = CollectionSnapshotExecutor::new(11, false);
        let preview = preview_book_pipeline_route_with_executor(
            &preview_executor,
            &real_collection_source(),
            "conversion_only",
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        assert_eq!(*preview_executor.calls.lock().unwrap(), 1);
        assert_eq!(preview.len(), 3);
        assert!(preview.iter().any(|route| {
            route.id == "PDFOK" && route.route_kind == "pending_route" && route.can_run
        }));
        assert!(preview.iter().any(|route| {
            route.id == "PDFMISSING" && route.route_kind == "missing_file" && !route.can_run
        }));
    }

    #[test]
    fn durable_collection_run_claims_route_without_invoking_batch_runner() {
        let root = temp_root("collection-durable-route-claim");
        let store = MemoryStateStore::new(&root);
        let (executor, _) = collection_snapshot_executor_for_pdf(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &executor,
            real_collection_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let routed = run_job(&store, &PanicPipelineRunner, &queued.id).unwrap();

        assert_eq!(routed.id, queued.id);
        assert!(routed
            .children
            .iter()
            .any(|child| stage_ref(child, "route").unwrap().status == STATUS_COMPLETED));
        assert!(routed
            .children
            .iter()
            .filter(|child| stage_ref(child, "route").unwrap().status == STATUS_COMPLETED)
            .all(|child| stage_ref(child, "extract").unwrap().status == STATUS_READY));
        assert_eq!(*store.save_count.lock().unwrap(), 3);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_collection_child_runs_route_extract_index_and_handoff_in_order() {
        let root = temp_root("durable-collection-child-chain");
        let worker_root = fake_zotero_worker_root(&root);
        let repo_root = root.join("repo");
        let store = BookPipelineStore::for_test(&root);
        let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let child_id = queued
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
            .unwrap()
            .id
            .clone();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            DurableCollectionChildExecutor::new(),
            worker_root,
        );

        let routed = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();

        assert_eq!(routed.children.len(), queued.children.len());
        assert!(routed.collection_items.is_empty());
        let child = routed
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_COMPLETED);
        assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_READY);
        assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_PENDING);
        assert_eq!(runner.executor.labels(), vec!["Zotero discovery dry-run"]);
        assert_eq!(
            stage_ref(
                store
                    .load()
                    .unwrap()
                    .jobs
                    .iter()
                    .find(|job| job.id == queued.id)
                    .unwrap()
                    .children
                    .iter()
                    .find(|child| child.id == child_id)
                    .unwrap(),
                "extract",
            )
            .unwrap()
            .status,
            STATUS_READY
        );

        let extracted = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
        let child = extracted
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        assert_eq!(
            stage_ref(child, "extract").unwrap().status,
            STATUS_COMPLETED
        );
        assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_READY);
        assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_PENDING);
        let markdown = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .unwrap();
        assert_eq!(
            markdown.source_refs.collection_key.as_deref(),
            Some("COLL1")
        );
        assert_eq!(
            markdown.source_refs.parent_item_key.as_deref(),
            Some("PARENT1")
        );
        assert_eq!(
            markdown.source_refs.pdf_attachment_key.as_deref(),
            Some("PDFOK")
        );
        assert_eq!(
            markdown.source_refs.markdown_attachment_key.as_deref(),
            Some("MARKDOWN1")
        );
        assert_eq!(
            runner.executor.labels(),
            vec!["Zotero discovery dry-run", ZOTERO_CONVERSION_COMMAND_LABEL]
        );
        assert_eq!(
            stage_ref(
                store
                    .load()
                    .unwrap()
                    .jobs
                    .iter()
                    .find(|job| job.id == queued.id)
                    .unwrap()
                    .children
                    .iter()
                    .find(|child| child.id == child_id)
                    .unwrap(),
                "index",
            )
            .unwrap()
            .status,
            STATUS_READY
        );

        let indexed = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
        let child = indexed
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_COMPLETED);
        assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_READY);
        assert!(stage_ref(child, "index").unwrap().index_evidence.is_some());
        assert_eq!(
            runner.executor.labels(),
            vec![
                "Zotero discovery dry-run",
                ZOTERO_CONVERSION_COMMAND_LABEL,
                ITEM_INDEX_PROFILE_COMMAND_LABEL,
                ITEM_INDEX_COMMAND_LABEL,
            ]
        );
        assert_eq!(
            stage_ref(
                store
                    .load()
                    .unwrap()
                    .jobs
                    .iter()
                    .find(|job| job.id == queued.id)
                    .unwrap()
                    .children
                    .iter()
                    .find(|child| child.id == child_id)
                    .unwrap(),
                "handoff",
            )
            .unwrap()
            .status,
            STATUS_READY
        );

        let handed_off = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
        let child = handed_off
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        assert_eq!(
            stage_ref(child, "handoff").unwrap().status,
            STATUS_COMPLETED
        );
        assert_eq!(stage_ref(child, "split").unwrap().status, STATUS_READY);
        assert!(child
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "translation_source"));
        assert_eq!(handed_off.children.len(), queued.children.len());
        assert!(handed_off.collection_items.is_empty());
        assert_eq!(store.load().unwrap().jobs.len(), 1);
        assert_eq!(
            runner.executor.labels(),
            vec![
                "Zotero discovery dry-run",
                ZOTERO_CONVERSION_COMMAND_LABEL,
                ITEM_INDEX_PROFILE_COMMAND_LABEL,
                ITEM_INDEX_COMMAND_LABEL,
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_collection_index_failure_preserves_extract_and_blocks_handoff() {
        let root = temp_root("durable-collection-child-index-failure");
        let worker_root = fake_zotero_worker_root(&root);
        let repo_root = root.join("repo");
        let store = BookPipelineStore::for_test(&root);
        let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let child_id = queued
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
            .unwrap()
            .id
            .clone();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            DurableCollectionChildExecutor::failing_index(),
            worker_root,
        );

        run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
        run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
        let failed = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();

        let child = failed
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        assert_eq!(
            stage_ref(child, "extract").unwrap().status,
            STATUS_COMPLETED
        );
        assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_FAILED);
        assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_PENDING);
        assert!(child
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown"));
        assert!(!child
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "translation_source"));
        assert_eq!(failed.children.len(), queued.children.len());
        assert!(failed.collection_items.is_empty());
        let labels_before_repeat = runner.executor.labels();

        let repeated = retry_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();

        let labels_after_retry = runner.executor.labels();
        assert_eq!(labels_after_retry.len(), labels_before_repeat.len() + 1);
        assert_eq!(labels_after_retry.last().unwrap(), ITEM_INDEX_COMMAND_LABEL);
        assert_eq!(repeated.children.len(), queued.children.len());
        let child = repeated
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        assert_eq!(stage_ref(child, "extract").unwrap().attempt, 1);
        assert_eq!(stage_ref(child, "index").unwrap().attempt, 2);
        assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_PENDING);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_collection_missing_extract_artifact_fails_index_without_reextracting() {
        let root = temp_root("durable-collection-missing-extract-artifact");
        let store = BookPipelineStore::for_test(&root);
        let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            DurableCollectionChildExecutor::new(),
            fake_zotero_worker_root(&root),
        );

        run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&root.join("repo")),
        )
        .unwrap();
        let extracted = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&root.join("repo")),
        )
        .unwrap();
        let child = extracted
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
            .unwrap();
        let markdown_path = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .unwrap()
            .path
            .clone();
        fs::remove_file(markdown_path).unwrap();
        let labels_before_index = runner.executor.labels();

        let failed = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&root.join("repo")),
        )
        .unwrap();

        assert_eq!(runner.executor.labels(), labels_before_index);
        let child = failed
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
            .unwrap();
        assert_eq!(
            stage_ref(child, "extract").unwrap().status,
            STATUS_COMPLETED
        );
        assert_eq!(stage_ref(child, "extract").unwrap().attempt, 1);
        assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_FAILED);
        assert_eq!(stage_ref(child, "index").unwrap().attempt, 1);
        assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_PENDING);
        assert!(child
            .last_error
            .as_deref()
            .is_some_and(|error| !error.trim().is_empty()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_collection_blocks_when_frozen_pdf_changes_before_route() {
        let root = temp_root("durable-collection-source-drift");
        let store = BookPipelineStore::for_test(&root);
        let (snapshot_executor, pdf) = collection_snapshot_executor_for_pdf(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        fs::write(
            &pdf,
            b"%PDF replaced after discovery with different bytes\n",
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            DurableCollectionChildExecutor::new(),
            fake_zotero_worker_root(&root),
        );

        let blocked = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&root.join("repo")),
        )
        .unwrap();

        let child = blocked
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
            .unwrap();
        assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_BLOCKED);
        assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_PENDING);
        assert!(child
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("frozen source")));
        assert!(runner.executor.labels().is_empty());
        assert_eq!(blocked.children.len(), queued.children.len());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_collection_blocks_handoff_when_indexed_markdown_changes() {
        let root = temp_root("durable-collection-markdown-drift");
        let worker_root = fake_zotero_worker_root(&root);
        let repo_root = root.join("repo");
        let store = BookPipelineStore::for_test(&root);
        let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            DurableCollectionChildExecutor::new(),
            worker_root,
        );
        run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
        run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
        let indexed = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
        let child = indexed
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
            .unwrap();
        let markdown_path = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .unwrap()
            .path
            .clone();
        fs::OpenOptions::new()
            .append(true)
            .open(&markdown_path)
            .unwrap()
            .write_all(b"\nchanged after index\n")
            .unwrap();

        let blocked = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();

        let child = blocked
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
            .unwrap();
        assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_COMPLETED);
        assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_BLOCKED);
        assert!(child
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("indexed Markdown")));
        assert!(!child
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "translation_source"));
        assert!(!repo_root.join("books").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_collection_mineru_route_uses_single_attachment_worker_adapter() {
        let root = temp_root("durable-collection-mineru-boundary");
        let store = BookPipelineStore::for_test(&root);
        let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let mut child = queued
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
            .unwrap()
            .clone();
        child.route[0].route_kind = "mineru".into();
        child.route[0].can_run = true;
        child.route[0].blocked_reason = None;

        let command = build_zotero_child_conversion_command_for_root(
            &child,
            &root.join("output"),
            &fake_full_worker_root(&root),
        )
        .unwrap();

        assert_eq!(command.label, ZOTERO_CONVERSION_COMMAND_LABEL);
        assert!(has_arg_pair(&command.args, "--attachment-key", "PDFOK"));
        assert!(command.args.iter().any(|arg| arg == "--force-mineru"));
        assert!(command.args.iter().any(|arg| arg == "--preserve-source"));
        assert!(!command.args.iter().any(|arg| arg.ends_with("mineru.py")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn completed_worker_evidence_rejects_another_extraction_contract() {
        let root = temp_root("completed-worker-contract-mismatch");
        let store = BookPipelineStore::for_test(&root);
        let snapshot_executor = CollectionSnapshotExecutor::mixed_routes(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let child = queued
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("DONE"))
            .unwrap();
        let markdown = root.join("legacy-completed.md");
        fs::write(
            &markdown,
            "---\nparent_item_key: \"PARENT4\"\n---\n\n# Legacy\n",
        )
        .unwrap();
        let payload = format!(
            "BOOK_PIPELINE_ATTACHMENT_EVIDENCE {}",
            serde_json::json!({
                "schemaVersion": ZOTERO_WORKER_ATTACHMENT_EVIDENCE_SCHEMA,
                "extractionContractVersion": "zotero-worker-extraction-legacy",
                "status": "already_completed",
                "route": "pdf-text",
                "pdfAttachmentKey": "DONE",
                "parentItemKey": "PARENT4",
                "sourceSha256": sha256_file(&root.join("zotero/DONE/done.pdf")).unwrap(),
                "markdownPath": display_path(&markdown),
                "markdownSha256": sha256_file(&markdown).unwrap(),
                "markdownAttachmentKey": "MARKDONE",
            })
        );
        let evidence = parse_zotero_worker_attachment_evidence(&payload, "DONE")
            .unwrap()
            .unwrap();

        let error = reused_markdown_artifact_from_evidence(child, &evidence).unwrap_err();

        assert!(error.contains("extraction contract"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_mixed_collection_routes_and_isolates_attachment_outcomes() {
        let root = temp_root("durable-mixed-collection");
        let store = BookPipelineStore::for_test(&root);
        let snapshot_executor = CollectionSnapshotExecutor::mixed_routes(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            MixedDurableCollectionExecutor::new(&root),
            fake_zotero_worker_root(&root),
        );
        let repo_root = root.join("repo");

        for _ in 0..32 {
            let current = store
                .load()
                .unwrap()
                .jobs
                .into_iter()
                .find(|job| job.id == queued.id)
                .unwrap();
            if durable_collection_stage_to_run(&current).is_none() {
                break;
            }
            run_job_with_handoff(
                &store,
                &runner,
                &FakeTranslationHandoffRunner,
                &queued.id,
                Some(&repo_root),
            )
            .unwrap();
        }

        let finished = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == queued.id)
            .unwrap();
        assert!(durable_collection_stage_to_run(&finished).is_none());
        assert!(finished.collection_items.is_empty());
        assert_eq!(finished.children.len(), 7);
        for (key, expected_route) in [
            ("DIRECT", "direct_text"),
            ("SCAN", "remote_paddleocr"),
            ("MINERU", "mineru"),
        ] {
            let child = finished
                .children
                .iter()
                .find(|child| child.source.selector.as_deref() == Some(key))
                .unwrap();
            assert_eq!(child.route[0].route_kind, expected_route);
            assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_COMPLETED);
            assert_eq!(
                stage_ref(child, "extract").unwrap().status,
                STATUS_COMPLETED
            );
            assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_COMPLETED);
            assert_eq!(
                stage_ref(child, "handoff").unwrap().status,
                STATUS_COMPLETED
            );
        }
        let reused = finished
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("DONE"))
            .unwrap();
        assert_eq!(reused.route[0].route_kind, "already_converted");
        assert_eq!(
            stage_ref(reused, "extract").unwrap().status,
            STATUS_COMPLETED
        );
        assert_eq!(stage_ref(reused, "index").unwrap().status, STATUS_COMPLETED);
        assert_eq!(
            stage_ref(reused, "handoff").unwrap().status,
            STATUS_COMPLETED
        );
        assert!(reused.artifacts.iter().any(|artifact| {
            artifact.kind == "markdown" && artifact.zotero_key.as_deref() == Some("MARKDONE")
        }));

        let failed = finished
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("FAIL"))
            .unwrap();
        assert_eq!(stage_ref(failed, "route").unwrap().status, STATUS_COMPLETED);
        assert_eq!(stage_ref(failed, "extract").unwrap().status, STATUS_FAILED);
        assert_eq!(stage_ref(failed, "index").unwrap().status, STATUS_PENDING);
        assert_eq!(stage_ref(failed, "handoff").unwrap().status, STATUS_PENDING);
        let blocked = finished
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("MISSING"))
            .unwrap();
        assert_eq!(stage_ref(blocked, "route").unwrap().status, STATUS_BLOCKED);
        assert_eq!(
            stage_ref(blocked, "extract").unwrap().status,
            STATUS_PENDING
        );
        let missing_credentials = finished
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("NOPADDLE"))
            .unwrap();
        assert_eq!(
            stage_ref(missing_credentials, "route").unwrap().status,
            STATUS_BLOCKED
        );
        assert!(missing_credentials
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("PaddleOCR is unavailable")));
        assert_eq!(
            stage_ref(missing_credentials, "extract").unwrap().status,
            STATUS_PENDING
        );

        assert_eq!(finished.summary.total, 7);
        assert_eq!(finished.summary.ready, 4);
        assert_eq!(finished.summary.failed, 1);
        assert_eq!(finished.summary.blocked, 2);
        let calls = runner.executor.calls();
        for key in ["DIRECT", "SCAN", "MINERU", "DONE", "FAIL", "NOPADDLE"] {
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| call.as_str() == format!("route:{key}"))
                    .count(),
                1
            );
        }
        assert!(!calls.iter().any(|call| call == "extract:DONE"));
        assert!(!calls.iter().any(|call| call == "extract:NOPADDLE"));
        assert!(calls.iter().any(|call| call == "index:PARENT4"));
        assert_eq!(
            finished.membership.as_ref().unwrap().snapshot_sha256,
            queued.membership.as_ref().unwrap().snapshot_sha256
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_collection_restart_retries_only_interrupted_child_stages() {
        let root = temp_root("durable-collection-targeted-recovery");
        let store = BookPipelineStore::for_test_with_owner(&root, "worker-before-restart");
        let snapshot_executor = CollectionSnapshotExecutor::recovery_routes(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let frozen_membership = queued.membership.clone().unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            MixedDurableCollectionExecutor::new(&root),
            fake_zotero_worker_root(&root),
        );
        let repo_root = root.join("repo");
        let run_next = |store: &BookPipelineStore| {
            run_job_with_handoff(
                store,
                &runner,
                &FakeTranslationHandoffRunner,
                &queued.id,
                Some(&repo_root),
            )
            .unwrap()
        };
        let mark_interrupted = |store: &BookPipelineStore, key: &str, stage_id: &str| {
            let mut state = store.load().unwrap();
            let job = state
                .jobs
                .iter_mut()
                .find(|job| job.id == queued.id)
                .unwrap();
            let child_index = job
                .children
                .iter()
                .position(|child| child.source.selector.as_deref() == Some(key))
                .unwrap();
            let index_input_hashes = (stage_id == "index").then(|| {
                let child = &job.children[child_index];
                let markdown = child
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == "markdown")
                    .unwrap();
                runner
                    .index_input(job, child, markdown, &store.job_output_dir(&queued.id))
                    .unwrap()
                    .hashes()
            });
            let child = &mut job.children[child_index];
            if let Some(input_hashes) = index_input_hashes {
                stage_mut(child, "index").unwrap().input_hashes = input_hashes;
            }
            start_stage(child, stage_id, store.execution_owner().unwrap());
            child.attempts = child.attempts.saturating_add(1);
            derive_job(job);
            store.save(&state).unwrap();
        };

        for _ in 0..4 {
            run_next(&store);
        }
        for _ in 0..2 {
            run_next(&store);
        }
        mark_interrupted(&store, "SCAN", "index");
        run_next(&store);
        mark_interrupted(&store, "MINERU", "extract");
        for _ in 0..3 {
            run_next(&store);
        }
        mark_interrupted(&store, "DONE", "handoff");

        let before_restart = store.load().unwrap();
        let job_before_restart = before_restart
            .jobs
            .iter()
            .find(|job| job.id == queued.id)
            .unwrap();
        assert_eq!(
            job_before_restart.membership.as_ref(),
            Some(&frozen_membership)
        );
        assert_eq!(
            stage_ref(
                job_before_restart
                    .children
                    .iter()
                    .find(|child| child.source.selector.as_deref() == Some("DIRECT"))
                    .unwrap(),
                "handoff",
            )
            .unwrap()
            .status,
            STATUS_COMPLETED
        );
        drop(store);

        let restarted = BookPipelineStore::for_test_with_owner(&root, "worker-after-restart");
        let recovered = restarted.load().unwrap();
        let recovered_revision = recovered.revision;
        let recovered_job = recovered
            .jobs
            .iter()
            .find(|job| job.id == queued.id)
            .unwrap();
        for (key, stage_id) in [
            ("SCAN", "index"),
            ("MINERU", "extract"),
            ("DONE", "handoff"),
        ] {
            let stage = stage_ref(
                recovered_job
                    .children
                    .iter()
                    .find(|child| child.source.selector.as_deref() == Some(key))
                    .unwrap(),
                stage_id,
            )
            .unwrap();
            assert_eq!(stage.status, STATUS_FAILED);
            assert_eq!(stage.attempt, 1);
            assert_eq!(stage.safe_error.as_ref().unwrap().code, "interrupted");
        }
        assert_eq!(restarted.load().unwrap().revision, recovered_revision);

        for _ in 0..6 {
            retry_job_with_handoff(
                &restarted,
                &runner,
                &FakeTranslationHandoffRunner,
                &queued.id,
                Some(&repo_root),
            )
            .unwrap();
        }

        let finished = restarted
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == queued.id)
            .unwrap();
        assert_eq!(finished.membership.as_ref(), Some(&frozen_membership));
        assert!(finished.collection_items.is_empty());
        for key in ["DIRECT", "SCAN", "MINERU", "DONE"] {
            let child = finished
                .children
                .iter()
                .find(|child| child.source.selector.as_deref() == Some(key))
                .unwrap();
            assert_eq!(
                stage_ref(child, "handoff").unwrap().status,
                STATUS_COMPLETED
            );
        }
        let stage_attempt = |key: &str, stage_id: &str| {
            stage_ref(
                finished
                    .children
                    .iter()
                    .find(|child| child.source.selector.as_deref() == Some(key))
                    .unwrap(),
                stage_id,
            )
            .unwrap()
            .attempt
        };
        assert_eq!(stage_attempt("DIRECT", "handoff"), 1);
        assert_eq!(stage_attempt("SCAN", "extract"), 1);
        assert_eq!(stage_attempt("SCAN", "index"), 2);
        assert_eq!(stage_attempt("MINERU", "extract"), 2);
        assert_eq!(stage_attempt("MINERU", "index"), 1);
        assert_eq!(stage_attempt("DONE", "extract"), 1);
        assert_eq!(stage_attempt("DONE", "handoff"), 2);
        let calls = runner.executor.calls();
        assert_eq!(
            calls
                .iter()
                .filter(|call| *call == "extract:DIRECT")
                .count(),
            1
        );
        assert_eq!(
            calls.iter().filter(|call| *call == "extract:SCAN").count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| *call == "extract:MINERU")
                .count(),
            1
        );
        assert!(!calls.iter().any(|call| call == "extract:DONE"));
        assert_eq!(*snapshot_executor.calls.lock().unwrap(), 1);

        let revision_before_repeat = restarted.load().unwrap().revision;
        let calls_before_repeat = runner.executor.calls();
        let repeated = retry_job_with_handoff(
            &restarted,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
        assert_eq!(restarted.load().unwrap().revision, revision_before_repeat);
        assert_eq!(runner.executor.calls(), calls_before_repeat);
        assert_eq!(repeated.membership.as_ref(), Some(&frozen_membership));
        assert!(repeated.children.iter().all(|child| {
            stage_ref(child, "handoff").is_some_and(|stage| stage.status == STATUS_COMPLETED)
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn durable_collection_user_run_and_retry_complete_the_searchable_end_to_end_chain() {
        let root = temp_root("durable-collection-end-to-end");
        let store = BookPipelineStore::for_test_with_owner(&root, "worker-before-restart");
        let snapshot_executor = CollectionSnapshotExecutor::recovery_routes(&root);
        let queued = queue_job_with_translation_intent_and_executor(
            &store,
            &snapshot_executor,
            real_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let frozen_membership = queued.membership.clone().unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            MixedDurableCollectionExecutor::failing_index_once(&root, "PARENT2"),
            fake_zotero_worker_root(&root),
        );
        let repo_root = root.join("repo");

        let first_run = run_job_to_quiescence_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();

        assert_eq!(*snapshot_executor.calls.lock().unwrap(), 1);
        assert_eq!(first_run.membership.as_ref(), Some(&frozen_membership));
        assert!(first_run.collection_items.is_empty());
        assert_eq!(first_run.summary.total, 4);
        assert_eq!(first_run.summary.failed, 1);
        for key in ["DIRECT", "MINERU", "DONE"] {
            let child = first_run
                .children
                .iter()
                .find(|child| child.source.selector.as_deref() == Some(key))
                .unwrap();
            assert_eq!(
                stage_ref(child, "handoff").unwrap().status,
                STATUS_COMPLETED
            );
        }
        let scan = first_run
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some("SCAN"))
            .unwrap();
        assert_eq!(stage_ref(scan, "extract").unwrap().status, STATUS_COMPLETED);
        assert_eq!(stage_ref(scan, "index").unwrap().status, STATUS_FAILED);
        assert_eq!(stage_ref(scan, "handoff").unwrap().status, STATUS_PENDING);
        assert_eq!(runner.executor.indexed_sha256("PARENT2"), None);
        drop(store);

        let restarted = BookPipelineStore::for_test_with_owner(&root, "worker-after-restart");
        let completed = retry_job_to_quiescence_with_handoff(
            &restarted,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();

        assert_eq!(completed.membership.as_ref(), Some(&frozen_membership));
        assert!(completed.collection_items.is_empty());
        assert_eq!(completed.summary.total, 4);
        assert_eq!(completed.summary.failed, 0);
        assert_eq!(completed.summary.blocked, 0);
        for (key, parent) in [
            ("DIRECT", "PARENT1"),
            ("SCAN", "PARENT2"),
            ("MINERU", "PARENT3"),
            ("DONE", "PARENT4"),
        ] {
            let child = completed
                .children
                .iter()
                .find(|child| child.source.selector.as_deref() == Some(key))
                .unwrap();
            assert_eq!(
                stage_ref(child, "handoff").unwrap().status,
                STATUS_COMPLETED
            );
            let evidence = stage_ref(child, "index")
                .unwrap()
                .index_evidence
                .as_ref()
                .unwrap();
            assert_eq!(
                runner.executor.indexed_sha256(parent).as_deref(),
                Some(evidence.source_sha256.as_str()),
                "fake zfulltext query surface must immediately expose the indexed artifact"
            );
            assert!(child
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == "translation_source"));
        }
        let calls = runner.executor.calls();
        assert_eq!(
            calls.iter().filter(|call| *call == "extract:SCAN").count(),
            1
        );
        assert_eq!(
            calls.iter().filter(|call| *call == "index:PARENT2").count(),
            2
        );
        assert_eq!(*snapshot_executor.calls.lock().unwrap(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn store_rejects_running_stage_with_incomplete_prerequisite() {
        let root = temp_root("stage-prerequisite-validation");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let mut state = store.load().unwrap();
        let child = &mut state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0];
        start_stage(child, "handoff", store.execution_owner().unwrap());

        let error = store.save(&state).unwrap_err();

        assert!(error.contains("incomplete prerequisite"));
        assert!(error.contains("handoff"));
        let _ = fs::remove_dir_all(root);
    }

    // The runner's output lives only in memory until the save below the handoff
    // start. A handoff that could not start used to return through `?` and take
    // the whole conversion with it, leaving the extract stage `running` on disk,
    // so retrying the handoff meant re-running the OCR.
    #[test]
    fn a_handoff_that_cannot_start_keeps_the_extraction_it_just_produced() {
        let root = temp_root("handoff-early-return");
        let repo_root = root.join("repo");
        let store = MemoryStateStore::new(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        // A handoff stage left running is a state `mark_handoff_running` refuses
        // to start from, which is what puts the job on this path.
        let mut state = store.load().unwrap();
        let child = &mut state.jobs[0].children[0];
        ensure_translation_stages(child, false);
        stage_mut(child, "handoff").unwrap().status = STATUS_RUNNING.into();
        store.save(&state).unwrap();

        let finished = run_job_with_handoff(
            &store,
            &ArtifactFixtureRunner,
            &FakeTranslationHandoffRunner,
            &job.id,
            Some(&repo_root),
        )
        .unwrap();

        let stored = store.load().unwrap().jobs[0].clone();
        assert!(
            stored
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == "markdown"),
            "the conversion the runner just produced must survive: {:?}",
            stored.artifacts
        );
        assert!(stored.output_dir.is_some(), "output_dir was dropped");
        assert_eq!(stored.artifacts, finished.artifacts);
        let child = &stored.children[0];
        assert_eq!(
            stage_ref(child, "extract").unwrap().status,
            STATUS_COMPLETED,
            "the extraction must not be left running"
        );
        assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_FAILED);
        assert_eq!(
            stored.last_error.as_deref(),
            Some("No completed extraction is ready for translation handoff.")
        );

        // Retrying the handoff alone has to work from here: this entry point
        // takes no pipeline runner, so it cannot re-run the extraction.
        let handed_off = handoff_job_markdown_with_runner(
            &store,
            &job.id,
            None,
            &repo_root,
            &FakeTranslationHandoffRunner,
        )
        .unwrap();
        let child = &handed_off.children[0];
        assert_eq!(
            stage_ref(child, "extract").unwrap().status,
            STATUS_COMPLETED
        );
        assert_eq!(
            stage_ref(child, "handoff").unwrap().status,
            STATUS_COMPLETED
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn store_rejects_completed_stage_regression_without_invalidation() {
        let root = temp_root("invalid-stage-transition");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let set_extract_status = |state: &mut BookPipelineState, status: &str| {
            let stored = state
                .jobs
                .iter_mut()
                .find(|stored| stored.id == job.id)
                .unwrap();
            if status == STATUS_RUNNING {
                start_stage(
                    &mut stored.children[0],
                    "extract",
                    store.execution_owner().unwrap(),
                );
            } else {
                set_stage_status(&mut stored.children[0], "extract", status, None);
            }
        };

        let mut running = store.load().unwrap();
        set_extract_status(&mut running, STATUS_RUNNING);
        store.save(&running).unwrap();
        let mut completed = store.load().unwrap();
        set_extract_status(&mut completed, STATUS_COMPLETED);
        store.save(&completed).unwrap();
        let mut regressed = store.load().unwrap();
        set_extract_status(&mut regressed, STATUS_RUNNING);

        let error = store.save(&regressed).unwrap_err();

        assert!(error.contains("Invalid Book Pipeline stage transition"));
        assert!(error.contains("extract"));
        assert!(error.contains("completed -> running"));
        let recovered = store.load().unwrap();
        let extract = recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();
        assert_eq!(extract.status, STATUS_COMPLETED);

        let mut invalidated = store.load().unwrap();
        let extract = invalidated
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter_mut()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();
        extract.contract_version = "book-pipeline-job-v3-test".into();
        extract.status = STATUS_READY.into();
        store.save(&invalidated).unwrap();
        let recovered = store.load().unwrap();
        assert_eq!(
            recovered
                .jobs
                .iter()
                .find(|stored| stored.id == job.id)
                .unwrap()
                .children[0]
                .stages
                .iter()
                .find(|stage| stage.stage_id == "extract")
                .unwrap()
                .status,
            STATUS_READY
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_and_blocked_children_do_not_regress_parent_to_pending() {
        let summary = BookPipelineStatusSummary {
            total: 2,
            failed: 1,
            blocked: 1,
            ..BookPipelineStatusSummary::default()
        };

        assert_eq!(aggregate_parent_status(&summary), STATUS_BLOCKED);
    }

    #[test]
    fn public_state_reports_stage_and_unit_progress() {
        let root = temp_root("public-progress-contract");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let queued = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|stored| stored.id == job.id)
            .unwrap();

        assert_eq!(queued.progress.stage_total, 3);
        assert_eq!(queued.progress.stage_completed, 2);
        assert_eq!(queued.progress.percent, 66);
        assert_eq!(queued.progress.active_stage_id, "extract");
        assert!(queued.progress.unit_summary.is_none());
        let serialized = serde_json::to_value(&queued).unwrap();
        assert_eq!(serialized["progress"]["stageTotal"], 3);
        assert_eq!(serialized["progress"]["activeStageId"], "extract");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_webhook_is_safe_deterministic_and_idempotent() {
        let root = temp_root("terminal-webhook-contract");
        let store = BookPipelineStore::for_test(&root);
        let mut source = fake_source(None);
        source.title = Some("private title must not leave the app".into());
        source.path = Some("/private/library/secret.md".into());
        let job = queue_job(
            &store,
            source,
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        run_job(&store, &SystemPipelineRunner, &job.id).unwrap();
        let sink = RecordingNotificationSink::default();

        let first = dispatch_terminal_notification(&store, &sink, &job.id).unwrap();
        let second = dispatch_terminal_notification(&store, &sink, &job.id).unwrap();

        assert_eq!(first.notification_deliveries.len(), 1);
        assert_eq!(second.notification_deliveries.len(), 1);
        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.status, STATUS_COMPLETED);
        assert_eq!(event.progress.percent, 100);
        assert_eq!(event.event_id, first.notification_deliveries[0].event_id);
        let payload = serde_json::to_string(event).unwrap();
        assert!(!payload.contains("private title"));
        assert!(!payload.contains("/private/library"));
        assert!(!payload.contains("lastError"));
        assert!(!payload.contains("logSummary"));
        let _ = fs::remove_dir_all(root);
    }

    // ADR 0002 promises one webhook per terminal outcome. Folding `updated_at`
    // and `attempts` into the event id delivered one per (outcome, timestamp),
    // so a job that reached the same terminal status again — retried and failed
    // again, or simply touched while terminal — notified a second time.
    #[test]
    fn reaching_the_same_terminal_status_again_delivers_one_webhook() {
        let root = temp_root("terminal-webhook-restate");
        // A memory store so the terminal status can be restated directly; the
        // durable store re-derives it from the children on every save.
        let store = MemoryStateStore::new(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
        let sink = RecordingNotificationSink::default();

        dispatch_terminal_notification(&store, &sink, &job.id).unwrap();

        // Reaching the terminal status again moves the clock and the attempt
        // counter; neither may mint a second event.
        let mut state = store.load().unwrap();
        let stored = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap();
        stored.attempts += 1;
        stored.updated_at = "2026-07-26T09:00:00Z".into();
        store.save(&state).unwrap();

        let second = dispatch_terminal_notification(&store, &sink, &job.id).unwrap();

        assert_eq!(
            sink.events.lock().unwrap().len(),
            1,
            "the same terminal outcome must notify once"
        );
        assert_eq!(second.notification_deliveries.len(), 1);

        // A different terminal outcome is still its own event.
        let mut state = store.load().unwrap();
        let stored = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap();
        stored.status = STATUS_FAILED.into();
        store.save(&state).unwrap();

        let failed = dispatch_terminal_notification(&store, &sink, &job.id).unwrap();

        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].status, STATUS_FAILED);
        assert_ne!(events[0].event_id, events[1].event_id);
        assert_eq!(failed.notification_deliveries.len(), 2);
        drop(events);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn webhook_config_reads_only_the_requested_dotenv_value() {
        let raw = "# private config\nOTHER_SETTING=ignored\nexport BOOK_PIPELINE_WEBHOOK_URL='https://localhost/hooks/books'\n";

        assert_eq!(
            dotenv_value(raw, "BOOK_PIPELINE_WEBHOOK_URL").as_deref(),
            Some("https://localhost/hooks/books")
        );
        assert_eq!(dotenv_value(raw, "MISSING"), None);
    }

    #[test]
    fn concurrent_saves_reject_one_stale_revision_and_keep_valid_json() {
        let root = temp_root("concurrent-save-protection");
        let store = std::sync::Arc::new(BookPipelineStore::for_test(&root));
        let job = queue_job(
            store.as_ref(),
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let mut left = store.load().unwrap();
        let mut right = store.load().unwrap();
        let starting_revision = left.revision;
        left.jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .log_summary
            .push("left writer".into());
        right
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .log_summary
            .push("right writer".into());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let left_store = std::sync::Arc::clone(&store);
        let left_barrier = std::sync::Arc::clone(&barrier);
        let left_writer = std::thread::spawn(move || {
            left_barrier.wait();
            left_store.save(&left)
        });
        let right_store = std::sync::Arc::clone(&store);
        let right_barrier = std::sync::Arc::clone(&barrier);
        let right_writer = std::thread::spawn(move || {
            right_barrier.wait();
            right_store.save(&right)
        });

        let results = [left_writer.join().unwrap(), right_writer.join().unwrap()];

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let conflict = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap();
        assert!(conflict.contains("Book Pipeline state changed concurrently"));
        let persisted_text = fs::read_to_string(&store.state_path).unwrap();
        let persisted: BookPipelineState = serde_json::from_str(&persisted_text).unwrap();
        assert_eq!(persisted.revision, starting_revision + 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_job_records_failure_and_retry_success() {
        let root = temp_root("fake-retry");
        let store = BookPipelineStore::for_test(&root);
        let source = fake_source(Some("fail_once"));

        let job = queue_job(
            &store,
            source,
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let failed = run_job(&store, &SystemPipelineRunner, &job.id).unwrap();
        assert_eq!(failed.status, STATUS_FAILED);
        assert!(failed.last_error.is_some());
        assert_eq!(failed.attempts, 1);

        let completed = run_job(&store, &SystemPipelineRunner, &job.id).unwrap();
        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(completed.attempts, 2);
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown" && artifact.sha256.is_some()));

        let recovered = store.load().unwrap();
        assert_eq!(recovered.jobs[0].status, STATUS_COMPLETED);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn completed_job_rejects_duplicate_runner_execution() {
        let root = temp_root("completed-job-no-duplicate-run");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let completed = run_job(&store, &SystemPipelineRunner, &job.id).unwrap();
        assert_eq!(completed.status, STATUS_COMPLETED);

        let error = run_job(&store, &SystemPipelineRunner, &job.id).unwrap_err();

        assert!(error.contains("No eligible extraction stage"));
        let recovered = store.load().unwrap();
        let recovered = recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap();
        assert_eq!(recovered.attempts, 1);
        assert_eq!(recovered.status, STATUS_COMPLETED);
        let _ = fs::remove_dir_all(root);
    }

    fn zotero_route(route_kind: &str) -> BookPipelineRouteItem {
        BookPipelineRouteItem {
            id: "ATTACH1".into(),
            title: "Fixture attachment".into(),
            source_kind: "zotero_attachment".into(),
            source_ref: "ATTACH1".into(),
            route_kind: route_kind.into(),
            can_run: true,
            blocked_reason: None,
            summary: "fixture route".into(),
            route_override: None,
        }
    }

    fn assert_runs_in_the_ocr_workspace(command: &RunnerCommand, script: &str) {
        assert_eq!(
            command.program,
            PathBuf::from("uv"),
            "{}: a bare interpreter only finds PyMuPDF where it happens to be installed globally",
            command.label
        );
        assert_eq!(
            command.args[..4],
            [
                "run".to_string(),
                "--package".to_string(),
                "ocr".to_string(),
                "python".to_string(),
            ],
            "{} must resolve its imports from the workspace venv",
            command.label
        );
        assert!(
            command.args[4].ends_with(script),
            "{} should run {script}, got {}",
            command.label,
            command.args[4]
        );
    }

    // The OCR line was the one pipeline stage still spawning a bare interpreter,
    // so its imports came from whatever the machine happened to have installed.
    #[test]
    fn every_ocr_entry_point_runs_through_the_workspace_venv() {
        let root = temp_root("ocr-workspace-venv");
        let input = root.join("input");
        let output = root.join("output");
        fs::create_dir_all(&input).unwrap();
        let worker_root = fake_full_worker_root(&root);
        fake_wrapper_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            local_pdf_source(&input),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        assert_runs_in_the_ocr_workspace(
            &build_local_pdf_folder_command_for_root(&job, &output, &worker_root).unwrap(),
            "pdf_to_html_paddleocr.py",
        );
        assert_runs_in_the_ocr_workspace(
            &build_zotero_conversion_command_for_source(
                &fake_direct_zotero_source(),
                &zotero_route("direct_text"),
                0,
                &output,
                &worker_root,
            )
            .unwrap(),
            "zotero_llm_worker.py",
        );
        assert_runs_in_the_ocr_workspace(
            &build_zotero_conversion_command_for_source(
                &fake_direct_zotero_source(),
                &zotero_route("mineru"),
                0,
                &output,
                &worker_root,
            )
            .unwrap(),
            "mineru.py",
        );
        assert_runs_in_the_ocr_workspace(
            &build_zotero_discovery_command_for_root(&fake_direct_zotero_source(), 5, &worker_root)
                .unwrap(),
            "zotero_llm_worker.py",
        );

        fs::remove_dir_all(&root).ok();
    }

    // `~/BiblioSmith` is a guess, not a promise. Handing a missing directory to
    // the runner as its cwd only produced an errno about a path the user never
    // picked, so the check has to name the settings that fix it instead.
    #[test]
    fn missing_repo_root_names_the_settings_that_fix_it() {
        let missing = temp_root("repo-root-absent");
        let error = existing_repo_root(missing.clone()).unwrap_err();

        assert!(error.contains(&display_path(&missing)), "{error}");
        assert!(error.contains("设置"), "{error}");
        assert!(error.contains("BIBLIOSMITH_HOME"), "{error}");

        fs::create_dir_all(&missing).unwrap();
        assert_eq!(existing_repo_root(missing.clone()).unwrap(), missing);
        fs::remove_dir_all(&missing).ok();
    }

    #[test]
    fn local_pdf_runner_command_uses_existing_wrapper_contract() {
        let root = temp_root("local-pdf-command");
        let input = root.join("input");
        let output = root.join("output");
        fs::create_dir_all(&input).unwrap();
        let wrapper_root = fake_wrapper_root(&root);
        let wrapper_script = wrapper_root
            .join("scripts")
            .join("pdf_to_html_paddleocr.py");
        let source = local_pdf_source(&input);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            source,
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let command =
            build_local_pdf_folder_command_for_root(&job, &output, &wrapper_root).unwrap();

        assert_eq!(command.kind, RunnerCommandKind::Process);
        assert_eq!(command.label, "local PDF conversion wrapper");
        assert_eq!(command.cwd, Some(wrapper_root));
        assert_eq!(command.output_dir, output);
        assert_eq!(command.program, PathBuf::from("uv"));
        assert_eq!(
            command.args[..5],
            [
                "run".to_string(),
                "--package".to_string(),
                "ocr".to_string(),
                "python".to_string(),
                display_path(&wrapper_script),
            ]
        );
        assert!(has_arg_pair(
            &command.args,
            "--input-dir",
            &display_path(&input)
        ));
        assert!(has_arg_pair(
            &command.args,
            "--output-dir",
            &display_path(&command.output_dir)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_pdf_runner_contract_records_wrapper_artifacts() {
        let root = temp_root("local-pdf-wrapper-artifacts");
        let input = root.join("input");
        fs::create_dir_all(&input).unwrap();
        fs::write(input.join("sample.pdf"), "%PDF fixture").unwrap();
        let wrapper_root = fake_wrapper_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            local_pdf_source(&input),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let completed = run_job(
            &store,
            &CommandPipelineRunner::with_book_ocr_conversion_root(
                LocalPdfFixtureExecutor,
                wrapper_root,
            ),
            &job.id,
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(completed.current_step, "Completed");
        assert!(completed.last_error.is_none());
        assert!(completed.output_dir.is_some());
        for kind in ["markdown", "html", "epub", "metadata", "index"] {
            assert!(completed
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == kind && artifact.sha256.is_some()));
        }
        assert!(completed
            .artifacts
            .iter()
            .all(|artifact| Path::new(&artifact.path).is_file()));
        assert!(completed
            .navigation_targets
            .iter()
            .any(|target| target.kind == "workspace"));
        let log = completed.log_summary.join("\n");
        assert!(log.contains("Runner command prepared: local PDF conversion wrapper"));
        assert!(log.contains("Local PDF fixture wrapper completed"));
        assert!(!log.contains("DONE: sample.pdf -> sample.html"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_pdf_runner_contract_records_wrapper_failure() {
        let root = temp_root("local-pdf-wrapper-failure");
        let input = root.join("input");
        fs::create_dir_all(&input).unwrap();
        fs::write(input.join("sample.pdf"), "%PDF fixture").unwrap();
        let wrapper_root = fake_wrapper_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            local_pdf_source(&input),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let failed = run_job(
            &store,
            &CommandPipelineRunner::with_book_ocr_conversion_root(
                LocalPdfFailingExecutor,
                wrapper_root,
            ),
            &job.id,
        )
        .unwrap();

        assert_eq!(failed.status, STATUS_FAILED);
        assert_eq!(failed.current_step, "Failed");
        assert_eq!(failed.attempts, 1);
        assert_eq!(
            failed.last_error.as_deref(),
            Some("Local PDF fixture wrapper failed")
        );
        assert!(failed
            .log_summary
            .iter()
            .any(|line| line.contains("Runner failed: Local PDF fixture wrapper failed")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn translate_only_markdown_source_creates_local_reading_project() {
        let root = temp_root("translate-only-markdown");
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("tools")).unwrap();
        fs::write(repo.join("AGENTS.md"), "# fixture\n").unwrap();
        fs::write(
            repo.join("tools").join("create_local_book_project.py"),
            "# fixture\n",
        )
        .unwrap();
        let source_path = root.join("source.md");
        fs::write(&source_path, "# Source\n\nText to translate.\n").unwrap();
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            markdown_source(&source_path),
            MODE_TRANSLATE_ONLY.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let completed = run_job_with_handoff(
            &store,
            &SystemPipelineRunner,
            &LocalProjectHandoffRunner,
            &job.id,
            Some(&repo),
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_READY);
        assert_eq!(completed.current_stage_id, "split");
        assert!(completed
            .route
            .iter()
            .any(|item| item.route_kind == "translation_ready" && item.can_run));
        let translation_source = completed
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "translation_source")
            .unwrap();
        assert_eq!(
            fs::read_to_string(&translation_source.path).unwrap(),
            "# Source\n\nText to translate.\n"
        );
        assert!(completed.children[0]
            .local_project_root
            .as_deref()
            .is_some_and(|path| Path::new(path).is_dir()));
        let manifest_path = Path::new(&translation_source.path)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("metadata")
            .join("source_manifest.json");
        let manifest = fs::read_to_string(manifest_path).unwrap();
        assert!(manifest.contains("\"source_sha256\""));
        assert!(manifest.contains("\"extraction_status\": \"cleaned_markdown_ready\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn translate_only_rejects_runtime_staging_markdown_paths() {
        let root = temp_root("translate-only-staging-block");
        let staging = root.join(".state").join("staging").join("source.md");
        fs::create_dir_all(staging.parent().unwrap()).unwrap();
        fs::write(&staging, "# temporary\n").unwrap();

        let route = preview_route(
            &markdown_source(&staging),
            MODE_TRANSLATE_ONLY,
            BookPipelinePreviewConfig::default(),
        );

        assert!(!route[0].can_run);
        assert_eq!(
            route[0].blocked_reason.as_deref(),
            Some("OCR runtime staging paths are rejected by default.")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mineru_runner_command_records_artifacts() {
        let root = temp_root("mineru-wrapper-artifacts");
        let worker_root = fake_full_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_mineru_zotero_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: false,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
        )
        .unwrap();

        let completed = run_job(
            &store,
            &CommandPipelineRunner::with_book_ocr_conversion_root(
                MineruFixtureExecutor,
                worker_root,
            ),
            &job.id,
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_COMPLETED);
        assert!(completed
            .route
            .iter()
            .any(|item| item.route_kind == "mineru" && item.can_run));
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown" && artifact.sha256.is_some()));
        assert!(completed
            .log_summary
            .iter()
            .any(|line| line.contains("Runner command prepared: MinerU extraction wrapper")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn external_adapter_route_normalizes_outputs() {
        let root = temp_root("external-adapter");
        fs::create_dir_all(&root).unwrap();
        let adapter = root.join("adapter.sh");
        fs::write(&adapter, "#!/bin/sh\n").unwrap();
        let input = root.join("input.pdf");
        fs::write(&input, "%PDF fixture").unwrap();
        let source = BookPipelineSource {
            kind: "external_adapter".into(),
            title: Some("External Adapter".into()),
            path: Some(display_path(&input)),
            selector: None,
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: Some(display_path(&adapter)),
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        };
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            source,
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let completed = run_job(
            &store,
            &CommandPipelineRunner::new(ExternalAdapterFixtureExecutor),
            &job.id,
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_COMPLETED);
        assert!(completed
            .route
            .iter()
            .any(|item| item.route_kind == "external_adapter" && item.can_run));
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown"));
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "html"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_collection_runs_direct_ocr_and_mineru_items_independently() {
        let root = temp_root("zotero-collection-mixed");
        let worker_root = fake_full_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_collection_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
        )
        .unwrap();

        let completed = run_job(
            &store,
            &CommandPipelineRunner::with_book_ocr_conversion_root(
                ZoteroBatchFixtureExecutor,
                worker_root,
            ),
            &job.id,
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_PARTIAL);
        assert_eq!(
            completed
                .collection_items
                .iter()
                .filter(|item| item.status == STATUS_COMPLETED)
                .count(),
            3
        );
        assert!(completed.collection_items.iter().any(|item| {
            item.id == "DIRTY" && item.status == STATUS_BLOCKED && item.last_error.is_some()
        }));
        assert!(completed
            .collection_items
            .iter()
            .any(|item| item.id == "DONE" && item.status == "skipped"));
        assert!(completed.current_step.contains("completed=3"));
        assert!(completed.current_step.contains("blocked=1"));
        assert!(completed.current_step.contains("skipped=1"));
        assert!(completed.artifacts.iter().any(|artifact| {
            artifact.kind == "markdown" && artifact.zotero_key.as_deref() == Some("DIRECTMD")
        }));
        assert!(completed.artifacts.iter().any(|artifact| {
            artifact.kind == "markdown" && artifact.zotero_key.as_deref() == Some("SCANMD")
        }));
        assert!(completed.artifacts.iter().any(|artifact| {
            artifact.kind == "markdown" && artifact.zotero_key.as_deref() == Some("MINERUMD")
        }));
        assert!(completed.children.iter().all(|child| {
            stage_ref(child, "extract").unwrap().status != STATUS_COMPLETED
                || stage_ref(child, "index").unwrap().status == STATUS_COMPLETED
        }));
        let manifest = completed
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "collection_manifest")
            .expect("hashed collection summary manifest");
        assert!(manifest.validation.hash_matches);
        assert!(Path::new(&manifest.path).is_file());
        assert_eq!(
            completed
                .open_target
                .as_ref()
                .map(|target| target.action_label.as_str()),
            Some("Inspect partial results")
        );
        let selected = completed.open_target.as_ref().unwrap();
        let target = completed
            .navigation_targets
            .iter()
            .find(|target| target.target_id == selected.target_id)
            .unwrap();
        assert_eq!(
            target.artifact_id.as_deref(),
            Some(manifest.artifact_id.as_str())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_collection_retry_targets_failed_items_only() {
        let root = temp_root("zotero-collection-retry");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let source = BookPipelineSource {
            kind: "zotero_collection".into(),
            title: Some("Retry collection".into()),
            path: None,
            selector: Some("RETRY".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: Some(vec![
                FakeZoteroItem {
                    key: "OK".into(),
                    title: "Already OK".into(),
                    attachment_path: Some("zotero://attachment/OK".into()),
                    has_text_layer: true,
                    dirty_text_layer: false,
                    scanned: false,
                    already_converted: false,
                    prefer_mineru: false,
                },
                FakeZoteroItem {
                    key: "FAIL".into(),
                    title: "Fails Once".into(),
                    attachment_path: Some("zotero://attachment/FAIL".into()),
                    has_text_layer: true,
                    dirty_text_layer: false,
                    scanned: false,
                    already_converted: false,
                    prefer_mineru: false,
                },
            ]),
            route_overrides: BTreeMap::new(),
        };
        let job = queue_job(
            &store,
            source,
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let executor = RetryCollectionExecutor {
            fail_once: std::sync::Mutex::new(true),
        };

        let partial = run_job(
            &store,
            &CommandPipelineRunner::with_book_ocr_conversion_root(executor, worker_root.clone()),
            &job.id,
        )
        .unwrap();

        assert_eq!(partial.status, STATUS_PARTIAL);
        assert!(partial.collection_items.iter().any(|item| {
            item.id == "OK"
                && item.status == STATUS_COMPLETED
                && item
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.kind == "markdown")
        }));
        assert!(partial.collection_items.iter().any(|item| {
            item.id == "FAIL"
                && item.status == STATUS_FAILED
                && item
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("first attempt failed"))
        }));
        let executor = RetryCollectionExecutor {
            fail_once: std::sync::Mutex::new(false),
        };

        let completed = run_job(
            &store,
            &CommandPipelineRunner::with_book_ocr_conversion_root(executor, worker_root),
            &job.id,
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_COMPLETED);
        assert!(completed.collection_items.iter().any(|item| {
            item.id == "OK" && item.status == STATUS_COMPLETED && item.attempts == 1
        }));
        assert!(completed.collection_items.iter().any(|item| {
            item.id == "FAIL" && item.status == STATUS_COMPLETED && item.attempts == 2
        }));
        assert!(completed.current_step.contains("failed=0"));
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown" && artifact.path.contains("OK")));
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown" && artifact.path.contains("FAIL")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reflection_translation_strategy_records_distinct_phase_artifacts() {
        let root = temp_root("reflection-translation");
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("tools")).unwrap();
        fs::write(repo.join("AGENTS.md"), "# fixture\n").unwrap();
        fs::write(
            repo.join("tools").join("create_local_book_project.py"),
            "# fixture\n",
        )
        .unwrap();
        let source_path = root.join("source.md");
        fs::write(
            &source_path,
            "# Source\n\nPrivate text stays out of logs.\n",
        )
        .unwrap();
        let mut source = markdown_source(&source_path);
        source.translation_strategy = Some("reflection".into());
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            source,
            MODE_TRANSLATE_ONLY.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let completed = run_job_with_handoff(
            &store,
            &SystemPipelineRunner,
            &LocalProjectHandoffRunner,
            &job.id,
            Some(&repo),
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_READY);
        assert_eq!(completed.current_stage_id, "split");
        for kind in [
            "translation_draft",
            "translation_reflection",
            "translation_revised",
        ] {
            assert!(completed
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == kind && artifact.sha256.is_some()));
        }
        let logs = completed.log_summary.join("\n");
        assert!(logs.contains("Reflection translation strategy recorded"));
        assert!(!logs.contains("Private text stays out of logs"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_discovery_parses_worker_dry_run_plan_sources() {
        let root = temp_root("zotero-discovery-plan");
        let worker_root = fake_zotero_worker_root(&root);
        let source = BookPipelineSource {
            kind: "zotero_filter".into(),
            title: Some("Book filter".into()),
            path: None,
            selector: Some("parent_item_type=book".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        };

        let result =
            discover_zotero_sources_with_root(&ZoteroDiscoveryExecutor, &source, 5, &worker_root)
                .unwrap();

        assert!(result
            .sources
            .iter()
            .any(|source| source.kind == "zotero_filter"
                && source.selector.as_deref() == Some("parent_item_type=book")));
        let direct = result
            .sources
            .iter()
            .find(|source| source.selector.as_deref() == Some("DIRECT1"))
            .unwrap();
        assert_eq!(direct.kind, "zotero_attachment");
        assert_eq!(direct.title.as_deref(), Some("Born Digital Book"));
        let direct_item = &direct.fake_zotero_items.as_ref().unwrap()[0];
        assert!(direct_item.has_text_layer);
        assert!(!direct_item.scanned);

        let scanned = result
            .sources
            .iter()
            .find(|source| source.selector.as_deref() == Some("SCAN1"))
            .unwrap();
        let scanned_item = &scanned.fake_zotero_items.as_ref().unwrap()[0];
        assert!(!scanned_item.has_text_layer);
        assert!(scanned_item.scanned);
        assert!(result
            .log_summary
            .iter()
            .any(|line| line.contains("Runner command prepared: Zotero discovery dry-run")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_query_filter_reads_the_same_prefixes_as_a_title_search_box() {
        // No prefix at all -- and no other recognised prefix -- means "no
        // search", the same way an empty selector means no filter today.
        assert_eq!(zotero_query_filter(Some("just typed text")), None);
        assert_eq!(zotero_query_filter(None), None);
        for prefix in ["query=", "query:", "q=", "q:", "title=", "title:"] {
            assert_eq!(
                zotero_query_filter(Some(&format!("{prefix}Geschäftsgeheimnisse"))),
                Some("Geschäftsgeheimnisse".to_string()),
                "prefix {prefix} was not recognised",
            );
        }
    }

    struct ZoteroQuerySelectorExecutor;

    impl RunnerCommandExecutor for ZoteroQuerySelectorExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert!(has_arg_pair(
                &command.args,
                "--query",
                "Geschäftsgeheimnisse"
            ));
            assert!(!command.args.iter().any(|arg| arg == "--parent-item-type"));
            Ok(RunnerCommandResult {
                stdout: String::new(),
                stderr: String::new(),
                log_summary: vec!["Zotero dry-run completed".into()],
            })
        }
    }

    #[test]
    fn a_title_search_selector_reaches_the_discovery_command_as_query() {
        let root = temp_root("zotero-discovery-query");
        let worker_root = fake_zotero_worker_root(&root);
        let source = BookPipelineSource {
            kind: "zotero_filter".into(),
            title: Some("Title search".into()),
            path: None,
            selector: Some("query=Geschäftsgeheimnisse".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        };

        discover_zotero_sources_with_root(&ZoteroQuerySelectorExecutor, &source, 5, &worker_root)
            .unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_zotero_discovery_items_do_not_execute_worker() {
        let source = BookPipelineSource {
            kind: "zotero_collection".into(),
            title: Some("Fake collection".into()),
            path: None,
            selector: Some("COLLECTION".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: Some(vec![FakeZoteroItem {
                key: "FAKE1".into(),
                title: "Fake attachment".into(),
                attachment_path: Some("zotero://fake/FAKE1.pdf".into()),
                has_text_layer: true,
                dirty_text_layer: false,
                scanned: false,
                already_converted: false,
                prefer_mineru: false,
            }]),
            route_overrides: BTreeMap::new(),
        };

        let result = discover_zotero_sources(&PanicExecutor, &source, 20).unwrap();

        assert_eq!(result.sources.len(), 1);
        assert_eq!(result.sources[0].kind, "zotero_collection");
        assert_eq!(
            result.sources[0].fake_zotero_items.as_ref().unwrap()[0].key,
            "FAKE1"
        );
    }

    #[test]
    fn zotero_discovery_failure_is_redacted() {
        let root = temp_root("zotero-discovery-redacted");
        let worker_root = fake_zotero_worker_root(&root);
        let source = BookPipelineSource {
            kind: "zotero_attachment".into(),
            title: Some("Single attachment".into()),
            path: None,
            selector: Some("ABC123".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        };

        let error = discover_zotero_sources_with_root(
            &ZoteroDiscoverySecretFailingExecutor,
            &source,
            1,
            &worker_root,
        )
        .unwrap_err();

        assert_eq!(
            error,
            "Sensitive credential or signed-request details were redacted."
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_route_preview_uses_worker_dry_run_policy() {
        let root = temp_root("zotero-route-preview");
        let worker_root = fake_zotero_worker_root(&root);
        let source = BookPipelineSource {
            kind: "zotero_filter".into(),
            title: Some("Preview queue".into()),
            path: None,
            selector: Some("parent_item_type=book".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        };

        let route = preview_zotero_route_from_worker(
            &ZoteroRoutePreviewExecutor,
            &source,
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: false,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
            20,
            &worker_root,
        )
        .unwrap();

        assert!(route
            .iter()
            .any(|item| item.id == "DIRECT" && item.route_kind == "direct_text" && item.can_run));
        assert!(route.iter().any(|item| {
            item.id == "SCAN"
                && item.route_kind == "missing_credentials"
                && !item.can_run
                && item
                    .blocked_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("PaddleOCR"))
        }));
        assert!(route
            .iter()
            .any(|item| item.id == "MINERU" && item.route_kind == "mineru" && item.can_run));
        assert!(route.iter().any(|item| {
            item.id == "DIRTY"
                && item.route_kind == "blocked_dirty_text_layer"
                && !item.can_run
                && item.blocked_reason.is_some()
        }));
        assert!(route.iter().any(|item| {
            item.id == "DONE" && item.route_kind == "already_converted" && !item.can_run
        }));

        let route_with_remote_ocr = preview_zotero_route_from_worker(
            &ZoteroRoutePreviewExecutor,
            &source,
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
            20,
            &worker_root,
        )
        .unwrap();
        assert!(route_with_remote_ocr.iter().any(|item| {
            item.id == "SCAN" && item.route_kind == "remote_paddleocr" && item.can_run
        }));
        assert!(route_with_remote_ocr
            .iter()
            .any(|item| item.id == "DIRECT" && item.can_run));
        let _ = fs::remove_dir_all(root);
    }

    fn zotero_query_filter_source() -> BookPipelineSource {
        BookPipelineSource {
            kind: "zotero_filter".into(),
            title: Some("Title search".into()),
            path: None,
            selector: Some("query=Der wirtschaftliche Wert".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        }
    }

    #[test]
    fn zotero_filter_queue_discovers_real_children_from_worker() {
        let root = temp_root("zotero-filter-queue-discovery");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);

        let job = queue_standard_job_for_root(
            &store,
            &ZoteroRoutePreviewExecutor,
            zotero_query_filter_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: false,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::from([("SCAN".into(), "direct".into())]),
            },
            &worker_root,
        )
        .unwrap();

        assert_eq!(job.kind, "collection");
        assert_eq!(job.status, STATUS_READY);
        let route_ids: Vec<&str> = job.route.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(route_ids, ["DIRECT", "SCAN", "MINERU", "DIRTY", "DONE"]);
        assert!(job.route.iter().all(|item| !item.id.contains("query=")));
        let scan = job.route.iter().find(|item| item.id == "SCAN").unwrap();
        assert_eq!(scan.route_kind, "direct_text");
        assert!(scan.can_run);
        assert_eq!(scan.route_override.as_deref(), Some("direct"));

        assert_eq!(job.children.len(), 5);
        for child in &job.children {
            assert_eq!(child.source.kind, "zotero_attachment");
            let selector = child.source.selector.as_deref().unwrap();
            assert!(route_ids.contains(&selector));
        }
        let _ = fs::remove_dir_all(root);
    }

    struct ZoteroEmptyDiscoveryExecutor;

    impl RunnerCommandExecutor for ZoteroEmptyDiscoveryExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            assert_eq!(command.label, "Zotero discovery dry-run");
            Ok(RunnerCommandResult {
                stdout: String::new(),
                stderr: String::new(),
                log_summary: vec!["Zotero dry-run found nothing".into()],
            })
        }
    }

    #[test]
    fn zotero_filter_queue_with_no_matches_blocks_without_demo_children() {
        let root = temp_root("zotero-filter-queue-no-matches");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);

        let job = queue_standard_job_for_root(
            &store,
            &ZoteroEmptyDiscoveryExecutor,
            zotero_query_filter_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
            &worker_root,
        )
        .unwrap();

        assert_eq!(job.status, STATUS_BLOCKED);
        assert_eq!(job.route.len(), 1);
        assert_eq!(job.route[0].route_kind, "blocked_no_attachment");
        assert!(!job.route[0].can_run);
        assert!(job.route[0]
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("No matching Zotero attachment")));
        assert!(job
            .children
            .iter()
            .all(|child| !child.id.contains("-DIRECT") && child.status != STATUS_READY));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_conversion_records_markdown_artifact_and_upload_key() {
        let root = temp_root("zotero-conversion-success");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_direct_zotero_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        assert_eq!(job.status, STATUS_READY);

        let completed = run_job(
            &store,
            &CommandPipelineRunner::with_book_ocr_conversion_root(
                ZoteroConversionExecutor,
                worker_root,
            ),
            &job.id,
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(completed.current_step, "Completed");
        let markdown = completed
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .unwrap();
        assert!(markdown.sha256.is_some());
        assert_eq!(markdown.zotero_key.as_deref(), Some("MDKEY123"));
        assert_eq!(
            markdown.source_refs.pdf_attachment_key.as_deref(),
            Some("DIRECT")
        );
        assert_eq!(
            markdown.source_refs.markdown_attachment_key.as_deref(),
            Some("MDKEY123")
        );
        assert!(completed
            .log_summary
            .iter()
            .any(|line| line.contains("Zotero Markdown attachment recorded: MDKEY123")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_extraction_runs_item_scoped_index_before_completion() {
        let root = temp_root("zotero-extract-index-success");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_direct_zotero_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroExtractIndexExecutor::succeeding(),
            worker_root,
        );

        let completed = run_job(&store, &runner, &job.id).unwrap();

        assert_eq!(child_stage_status(&completed, "extract"), STATUS_COMPLETED);
        assert_eq!(child_stage_status(&completed, "index"), STATUS_COMPLETED);
        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(
            runner.executor.command_labels(),
            vec![
                "Zotero conversion worker".to_string(),
                "Zotero item index profile".to_string(),
                "Zotero item-scoped full-text index".to_string(),
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_translation_handoff_waits_for_item_scoped_index() {
        let root = temp_root("zotero-extract-index-handoff");
        let worker_root = fake_zotero_worker_root(&root);
        let repo_root = root.join("repo");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_direct_zotero_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroExtractIndexExecutor::succeeding(),
            worker_root,
        );

        let handed_off = run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &job.id,
            Some(&repo_root),
        )
        .unwrap();

        assert_eq!(child_stage_status(&handed_off, "extract"), STATUS_COMPLETED);
        assert_eq!(child_stage_status(&handed_off, "index"), STATUS_COMPLETED);
        assert_eq!(child_stage_status(&handed_off, "handoff"), STATUS_COMPLETED);
        assert_eq!(handed_off.current_stage_id, "split");
        assert!(handed_off
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "translation_source"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn item_scoped_index_persists_safe_evidence_without_chunk_text() {
        let root = temp_root("zotero-index-evidence");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_direct_zotero_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroExtractIndexExecutor::succeeding(),
            worker_root,
        );

        let completed = run_job(&store, &runner, &job.id).unwrap();

        let index_stage = completed.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "index")
            .unwrap();
        let markdown = completed.children[0]
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .unwrap();
        assert_eq!(
            index_stage.input_hashes.get("markdownArtifactId"),
            Some(&markdown.artifact_id)
        );
        let persisted = serde_json::to_value(index_stage).unwrap();
        assert_eq!(persisted["indexEvidence"]["parentItemKey"], "PARENT123");
        assert_eq!(persisted["indexEvidence"]["chunkCount"], 1);
        assert_eq!(
            persisted["indexEvidence"]["indexContractVersion"],
            ITEM_INDEX_CONTRACT_VERSION
        );
        assert_eq!(
            persisted["indexEvidence"]["embeddingProfileId"],
            "fixture-embedding:3"
        );
        assert!(!persisted.to_string().contains("Direct Markdown"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_extraction_without_markdown_attachment_key_does_not_start_index() {
        let root = temp_root("zotero-index-requires-upload-key");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_direct_zotero_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroExtractIndexExecutor::missing_markdown_attachment_key(),
            worker_root,
        );

        let failed = run_job(&store, &runner, &job.id).unwrap();

        assert_eq!(child_stage_status(&failed, "extract"), STATUS_FAILED);
        assert_eq!(child_stage_status(&failed, "index"), STATUS_PENDING);
        assert_eq!(
            runner.executor.command_labels(),
            vec!["Zotero conversion worker".to_string()]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn completed_item_index_requires_matching_persisted_evidence() {
        let root = temp_root("zotero-index-evidence-validation");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_direct_zotero_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroExtractIndexExecutor::succeeding(),
            worker_root,
        );
        run_job(&store, &runner, &job.id).unwrap();
        let mut state = store.load().unwrap();
        stage_mut(&mut state.jobs[0].children[0], "index")
            .unwrap()
            .index_evidence = None;

        let error = validate_state(&state).unwrap_err();

        assert!(error.contains("completed index evidence"));

        let mut state = store.load().unwrap();
        stage_mut(&mut state.jobs[0].children[0], "index")
            .unwrap()
            .index_evidence
            .as_mut()
            .unwrap()
            .parent_item_key = "WRONGPARENT".into();
        let error = validate_state(&state).unwrap_err();
        assert!(error.contains("mismatched completed index evidence"));

        let mut state = store.load().unwrap();
        stage_mut(&mut state.jobs[0].children[0], "index")
            .unwrap()
            .input_hashes
            .insert("markdownArtifactId".into(), "artifact-wrong".into());
        let error = validate_state(&state).unwrap_err();
        assert!(error.contains("mismatched completed index evidence"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_markdown_is_persisted_as_an_index_failure() {
        let root = temp_root("zotero-index-missing-markdown");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_direct_zotero_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let failed = run_job(&store, &MissingMarkdownRunner, &job.id).unwrap();

        assert_eq!(child_stage_status(&failed, "extract"), STATUS_COMPLETED);
        assert_eq!(child_stage_status(&failed, "index"), STATUS_FAILED);
        assert_eq!(stage_ref(&failed.children[0], "index").unwrap().attempt, 1);
        assert!(failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("no Markdown artifact")));
        let recovered = store.load().unwrap();
        assert_eq!(
            child_stage_status(&recovered.jobs[0], "index"),
            STATUS_FAILED
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn item_scoped_index_retry_does_not_rerun_extraction() {
        let root = temp_root("zotero-index-retry");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_direct_zotero_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroExtractIndexExecutor::failing_index_once(),
            worker_root,
        );

        let failed = run_job(&store, &runner, &job.id).unwrap();

        assert_eq!(child_stage_status(&failed, "extract"), STATUS_COMPLETED);
        assert_eq!(child_stage_status(&failed, "index"), STATUS_FAILED);
        let failed_index = stage_ref(&failed.children[0], "index").unwrap();
        let markdown = failed.children[0]
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .unwrap();
        assert_eq!(
            failed_index.input_hashes.get("markdownArtifactId"),
            Some(&markdown.artifact_id)
        );
        assert_eq!(
            failed_index.input_hashes.get("markdownSha256"),
            markdown.sha256.as_ref()
        );
        assert_eq!(
            failed_index.input_hashes.get("embeddingProfileId"),
            Some(&"fixture-embedding:3".to_string())
        );
        assert!(failed.children[0]
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown" && artifact.validation.hash_matches));

        let completed = run_job(&store, &runner, &job.id).unwrap();

        assert_eq!(child_stage_status(&completed, "extract"), STATUS_COMPLETED);
        assert_eq!(child_stage_status(&completed, "index"), STATUS_COMPLETED);
        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(
            runner.executor.command_labels(),
            vec![
                "Zotero conversion worker".to_string(),
                "Zotero item index profile".to_string(),
                "Zotero item-scoped full-text index".to_string(),
                "Zotero item-scoped full-text index".to_string(),
            ]
        );
        assert_eq!(
            stage_ref(&completed.children[0], "extract")
                .unwrap()
                .attempt,
            1
        );
        assert_eq!(
            stage_ref(&completed.children[0], "index").unwrap().attempt,
            2
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_preview_reconciles_completed_and_changed_fingerprints() {
        let root = temp_root("zotero-fingerprint-preview");
        let worker_root = fake_zotero_worker_root(&root);
        let source = BookPipelineSource {
            kind: "zotero_filter".into(),
            title: Some("Fingerprint queue".into()),
            path: None,
            selector: Some("parent_item_type=book".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        };

        let route = preview_zotero_route_from_worker(
            &ZoteroFingerprintPreviewExecutor,
            &source,
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
            20,
            &worker_root,
        )
        .unwrap();

        let current = route.iter().find(|item| item.id == "CURRENT").unwrap();
        assert_eq!(current.route_kind, "already_converted");
        assert!(!current.can_run);
        assert!(current.source_ref.contains("/tmp/current.md"));
        assert!(current.source_ref.contains("source_md5=aaa111"));

        let missing_upload = route.iter().find(|item| item.id == "MISSING").unwrap();
        assert_eq!(missing_upload.route_kind, "direct_text");
        assert!(missing_upload.can_run);
        assert!(missing_upload.source_ref.contains("source_md5=aaa111"));

        let changed = route.iter().find(|item| item.id == "CHANGED").unwrap();
        assert_eq!(changed.route_kind, "direct_text");
        assert!(changed.can_run);
        assert!(changed.source_ref.contains("source_md5=bbb222"));

        let blocked = route.iter().find(|item| item.id == "DIRTY").unwrap();
        assert_eq!(blocked.route_kind, "blocked_dirty_text_layer");
        assert!(!blocked.can_run);
        assert!(blocked.source_ref.contains("source_md5=ccc333"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_conversion_retry_preserves_failure_diagnosis_in_logs() {
        let root = temp_root("zotero-conversion-retry");
        let worker_root = fake_zotero_worker_root(&root);
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_direct_zotero_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let failed = run_job(
            &store,
            &CommandPipelineRunner::with_book_ocr_conversion_root(
                ZoteroConversionFailingExecutor,
                worker_root.clone(),
            ),
            &job.id,
        )
        .unwrap();
        assert_eq!(failed.status, STATUS_FAILED);
        assert_eq!(
            failed.last_error.as_deref(),
            Some("Zotero conversion fixture failed: diagnosis preserved")
        );

        let completed = run_job(
            &store,
            &CommandPipelineRunner::with_book_ocr_conversion_root(
                ZoteroConversionExecutor,
                worker_root,
            ),
            &job.id,
        )
        .unwrap();
        assert_eq!(completed.status, STATUS_COMPLETED);
        assert!(completed.log_summary.iter().any(|line| {
            line.contains("Runner failed: Zotero conversion fixture failed: diagnosis preserved")
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_mixed_blocked_route_keeps_runnable_items_available() {
        let root = temp_root("zotero-mixed-blocked");
        let store = BookPipelineStore::for_test(&root);
        let source = BookPipelineSource {
            kind: "zotero_filter".into(),
            title: Some("Mixed queue".into()),
            path: None,
            selector: Some("parent_item_type=book".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: Some(vec![
                FakeZoteroItem {
                    key: "DIRECT".into(),
                    title: "Direct Text".into(),
                    attachment_path: Some("zotero://attachment/DIRECT".into()),
                    has_text_layer: true,
                    dirty_text_layer: false,
                    scanned: false,
                    already_converted: false,
                    prefer_mineru: false,
                },
                FakeZoteroItem {
                    key: "SCAN".into(),
                    title: "Scanned PDF".into(),
                    attachment_path: Some("zotero://attachment/SCAN".into()),
                    has_text_layer: false,
                    dirty_text_layer: false,
                    scanned: true,
                    already_converted: false,
                    prefer_mineru: false,
                },
            ]),
            route_overrides: BTreeMap::new(),
        };
        let job = queue_job(
            &store,
            source,
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: false,
                has_mineru_credentials: false,
                route_overrides: BTreeMap::new(),
            },
        )
        .unwrap();
        assert_eq!(job.status, STATUS_READY);
        assert!(job
            .route
            .iter()
            .any(|item| item.id == "DIRECT" && item.route_kind == "direct_text" && item.can_run));
        assert!(job.route.iter().any(|item| {
            item.id == "SCAN" && item.route_kind == "missing_credentials" && !item.can_run
        }));
        let _ = fs::remove_dir_all(root);
    }

    fn override_route_source(overrides: BTreeMap<String, String>) -> BookPipelineSource {
        BookPipelineSource {
            kind: "zotero_filter".into(),
            title: Some("Override queue".into()),
            path: None,
            selector: Some("parent_item_type=book".into()),
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: Some(vec![
                FakeZoteroItem {
                    key: "DIRECT".into(),
                    title: "Direct Text".into(),
                    attachment_path: Some("zotero://attachment/DIRECT".into()),
                    has_text_layer: true,
                    dirty_text_layer: false,
                    scanned: false,
                    already_converted: false,
                    prefer_mineru: false,
                },
                FakeZoteroItem {
                    key: "SCAN".into(),
                    title: "Scanned PDF".into(),
                    attachment_path: Some("zotero://attachment/SCAN".into()),
                    has_text_layer: false,
                    dirty_text_layer: false,
                    scanned: true,
                    already_converted: false,
                    prefer_mineru: false,
                },
            ]),
            route_overrides: overrides,
        }
    }

    #[test]
    fn route_override_forces_mineru_over_automatic_direct_text() {
        let root = temp_root("route-override-mineru");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            override_route_source(BTreeMap::new()),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::from([("DIRECT".into(), "mineru".into())]),
            },
        )
        .unwrap();

        let direct = job.route.iter().find(|item| item.id == "DIRECT").unwrap();
        assert_eq!(direct.route_kind, "mineru");
        assert_eq!(direct.route_override.as_deref(), Some("mineru"));
        assert!(direct.can_run);
        // The untouched item keeps its automatic decision and records no override.
        let scan = job.route.iter().find(|item| item.id == "SCAN").unwrap();
        assert!(scan.route_override.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn route_override_keep_marks_item_already_converted_and_not_runnable() {
        let root = temp_root("route-override-keep");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            override_route_source(BTreeMap::new()),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::from([("SCAN".into(), "keep".into())]),
            },
        )
        .unwrap();

        let scan = job.route.iter().find(|item| item.id == "SCAN").unwrap();
        assert_eq!(scan.route_kind, "already_converted");
        assert!(!scan.can_run);
        assert_eq!(scan.route_override.as_deref(), Some("keep"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn route_override_cannot_bypass_missing_credentials() {
        let root = temp_root("route-override-no-bypass");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            override_route_source(BTreeMap::new()),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: false,
                has_mineru_credentials: false,
                route_overrides: BTreeMap::from([("DIRECT".into(), "mineru".into())]),
            },
        )
        .unwrap();

        // Forcing an engine whose credentials are absent must hold the item, not
        // hand it to a runner that cannot possibly succeed.
        let direct = job.route.iter().find(|item| item.id == "DIRECT").unwrap();
        assert_eq!(direct.route_kind, "missing_credentials");
        assert!(!direct.can_run);
        assert!(direct.blocked_reason.is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn route_override_is_persisted_on_the_queued_source() {
        let root = temp_root("route-override-persisted");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            override_route_source(BTreeMap::new()),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::from([("DIRECT".into(), "mineru".into())]),
            },
        )
        .unwrap();

        // Durable Zotero jobs re-derive their route at run time; the override has
        // to live on the stored source or it would be silently reverted there.
        assert_eq!(
            job.source.route_overrides.get("DIRECT").map(String::as_str),
            Some("mineru")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runner_failure_records_redacted_error_for_retry() {
        let root = temp_root("secret-failure");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let failed = run_job(
            &store,
            &CommandPipelineRunner::new(SecretFailingExecutor),
            &job.id,
        )
        .unwrap();

        assert_eq!(failed.status, STATUS_FAILED);
        assert_eq!(failed.current_step, "Failed");
        assert_eq!(failed.attempts, 1);
        assert_eq!(
            failed.last_error.as_deref(),
            Some("Sensitive credential or signed-request details were redacted.")
        );
        let log = failed.log_summary.join("\n");
        assert!(log.contains("Sensitive credential"));
        assert!(!log.contains("supersecret"));
        assert!(!log.contains("abc"));
        assert!(!log.to_ascii_lowercase().contains("bearer"));
        let extract = failed.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .unwrap();
        assert_eq!(
            extract.safe_error.as_ref().map(|error| error.code.as_str()),
            Some("missing_credentials")
        );
        let timestamp = extract.safe_error.as_ref().unwrap().timestamp.clone();
        let recovered = store.load().unwrap();
        let recovered_error = recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .and_then(|stage| stage.safe_error.as_ref())
            .unwrap();
        assert_eq!(recovered_error.timestamp, timestamp);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runner_success_redacts_secret_streams_and_preserves_artifacts() {
        let root = temp_root("secret-success");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let completed = run_job(
            &store,
            &CommandPipelineRunner::new(SecretLoggingExecutor),
            &job.id,
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_COMPLETED);
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown" && artifact.sha256.is_some()));
        let log = completed.log_summary.join("\n");
        assert!(log.contains("Runner command prepared: fake Book Pipeline runner"));
        assert!(log.contains("Sensitive credential"));
        assert!(!log.contains("supersecret"));
        assert!(!log.contains("abc"));
        assert!(!log.to_ascii_lowercase().contains("bearer"));
        // Merely naming .env, with no assignment or value attached, isn't a
        // leak and should stay legible — unlike the ZOTERO_API_KEY=... line
        // above it, which is a real leak and gets redacted.
        assert!(log.contains(".env content was not read"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_pdf_folder_runner_registers_artifact_checksums() {
        let root = temp_root("local-pdf");
        let input = root.join("input");
        fs::create_dir_all(&input).unwrap();
        fs::write(input.join("sample.pdf"), "%PDF fixture").unwrap();
        let store = BookPipelineStore::for_test(&root);
        let source = BookPipelineSource {
            kind: "local_pdf_folder".into(),
            title: Some("PDF folder".into()),
            path: Some(display_path(&input)),
            selector: None,
            runner_behavior: None,
            translation_strategy: None,
            adapter_command: None,
            fake_zotero_items: None,
            route_overrides: BTreeMap::new(),
        };

        let job = queue_job(
            &store,
            source,
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        assert_eq!(job.route.len(), 1);
        assert_eq!(job.route[0].route_kind, "remote_paddleocr");

        let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
        assert_eq!(completed.status, STATUS_COMPLETED);
        for kind in ["markdown", "html", "epub"] {
            assert!(completed
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == kind && artifact.sha256.is_some()));
        }
        assert!(completed.output_dir.is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn markdown_artifact_handoff_creates_translation_ready_project() {
        let root = temp_root("handoff-success");
        let repo_root = root.join("repo");
        fs::create_dir_all(repo_root.join("tools")).unwrap();
        fs::write(repo_root.join("AGENTS.md"), "fixture").unwrap();
        fs::write(
            repo_root.join("tools").join("create_local_book_project.py"),
            "fixture",
        )
        .unwrap();
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
        assert_eq!(completed.status, STATUS_COMPLETED);

        let handed_off = handoff_job_markdown(&store, &job.id, None, &repo_root).unwrap();

        assert_eq!(handed_off.status, STATUS_READY);
        assert_eq!(handed_off.current_stage_id, "split");
        assert_eq!(handed_off.current_step, "Translation handoff ready");
        let project_root = PathBuf::from(
            handed_off.children[0]
                .local_project_root
                .as_deref()
                .expect("registered local project root"),
        );
        assert!(project_root.join("source").join("source.md").is_file());
        assert!(project_root.join("chapters").join("src").is_dir());
        let source_artifact = handed_off
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "translation_source")
            .unwrap();
        assert!(source_artifact.sha256.is_some());
        let manifest =
            fs::read_to_string(project_root.join("metadata").join("source_manifest.json")).unwrap();
        assert!(manifest.contains("cleaned_markdown_ready"));
        assert_eq!(
            fs::read_to_string(project_root.join("source").join("source.md")).unwrap(),
            fs::read_to_string(project_root.join("source").join("original.md")).unwrap()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn markdown_artifact_handoff_rejects_missing_extraction_prerequisite() {
        let root = temp_root("handoff-failure");
        let repo_root = root.join("repo");
        fs::create_dir_all(repo_root.join("tools")).unwrap();
        fs::write(repo_root.join("AGENTS.md"), "fixture").unwrap();
        fs::write(
            repo_root.join("tools").join("create_local_book_project.py"),
            "fixture",
        )
        .unwrap();
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let error = handoff_job_markdown(&store, &job.id, None, &repo_root).unwrap_err();

        assert!(error.contains("No completed extraction"));
        let recovered = store.load().unwrap();
        let stored = recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap();
        assert_eq!(stored.status, STATUS_READY);
        assert_eq!(stored.current_stage_id, "extract");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn convert_then_translate_records_conversion_and_handoff_artifacts() {
        let root = temp_root("convert-then-translate-success");
        let store = BookPipelineStore::for_test(&root);
        let repo_root = root.join("repo");
        let job = queue_job(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        assert!(job
            .route
            .iter()
            .any(|item| item.route_kind == "translation_handoff" && item.can_run));

        let completed = run_job_with_handoff(
            &store,
            &ArtifactFixtureRunner,
            &FakeTranslationHandoffRunner,
            &job.id,
            Some(&repo_root),
        )
        .unwrap();

        assert_eq!(completed.status, STATUS_READY);
        assert_eq!(completed.current_stage_id, "split");
        assert_eq!(completed.current_step, "Translation handoff ready");
        assert!(completed.last_error.is_none());
        for kind in ["markdown", "html", "epub", "translation_source"] {
            assert!(completed
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == kind));
        }
        let log = completed.log_summary.join("\n");
        assert!(log.contains("fixture runner completed"));
        assert!(log.contains("Conversion completed; translation handoff started"));
        assert!(log.contains("Fake translation handoff ready"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn convert_then_translate_conversion_failure_does_not_run_handoff() {
        let root = temp_root("convert-then-translate-conversion-failure");
        let store = BookPipelineStore::for_test(&root);
        let repo_root = root.join("repo");
        let job = queue_job(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let failed = run_job_with_handoff(
            &store,
            &ConversionFailingRunner,
            &FakeTranslationHandoffRunner,
            &job.id,
            Some(&repo_root),
        )
        .unwrap();

        assert_eq!(failed.status, STATUS_FAILED);
        assert_eq!(failed.current_step, "Conversion failed");
        assert_eq!(
            failed.last_error.as_deref(),
            Some("Fake conversion backend failed")
        );
        assert!(failed.artifacts.is_empty());
        assert!(!failed
            .log_summary
            .iter()
            .any(|line| line.contains("translation handoff")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn convert_then_translate_handoff_failure_preserves_conversion_artifacts() {
        let root = temp_root("convert-then-translate-handoff-failure");
        let store = BookPipelineStore::for_test(&root);
        let repo_root = root.join("repo");
        let job = queue_job(
            &store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let failed = run_job_with_handoff(
            &store,
            &ArtifactFixtureRunner,
            &FailingTranslationHandoffRunner,
            &job.id,
            Some(&repo_root),
        )
        .unwrap();

        assert_eq!(failed.status, STATUS_FAILED);
        assert_eq!(failed.current_step, "Translation handoff failed");
        assert_eq!(
            failed.last_error.as_deref(),
            Some("Fake translation handoff failed")
        );
        for kind in ["markdown", "html", "epub"] {
            assert!(failed
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == kind));
        }
        assert!(!failed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "translation_project"));
        let log = failed.log_summary.join("\n");
        assert!(log.contains("fixture runner completed"));
        assert!(log.contains("Translation handoff failed: Fake translation handoff failed"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_preview_blocks_unsafe_candidate_missing_zotero_child() {
        let root = temp_root("cleanup-unsafe");
        let store = BookPipelineStore::for_test(&root);
        let job = cleanup_fixture_job(&root, &store, None);

        let preview = preview_cleanup_candidates(&store).unwrap();

        assert_eq!(preview.candidates.len(), 1);
        let candidate = &preview.candidates[0];
        assert_eq!(candidate.job_id, job.id);
        assert!(!candidate.can_approve);
        assert!(candidate.checks.iter().any(|check| {
            check.kind == "zotero_child_attachment"
                && !check.ok
                && check.detail.contains("Missing Zotero")
        }));
        let error = approve_cleanup_candidate(&store, &candidate.id, true).unwrap_err();
        assert!(error.contains("zotero_child_attachment"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_approval_requires_explicit_safe_candidate() {
        let root = temp_root("cleanup-safe");
        let store = BookPipelineStore::for_test(&root);
        let job = cleanup_fixture_job(&root, &store, Some("MDKEY123"));
        let source_pdf = job.source.path.as_deref().map(PathBuf::from).unwrap();
        let preview = preview_cleanup_candidates(&store).unwrap();
        let candidate = preview.candidates[0].clone();

        assert!(candidate.can_approve);
        assert!(candidate.checks.iter().all(|check| check.ok));
        assert!(approve_cleanup_candidate(&store, &candidate.id, false)
            .unwrap_err()
            .contains("Explicit cleanup approval"));

        let result = approve_cleanup_candidate(&store, &candidate.id, true).unwrap();

        assert!(result.ok);
        assert!(result.message.contains("launcher did not delete"));
        assert!(source_pdf.is_file());
        let state = store.load().unwrap();
        assert!(state.jobs[0]
            .log_summary
            .iter()
            .any(|line| line.contains("Cleanup approval recorded")));
        let _ = fs::remove_dir_all(root);
    }

    fn queued_collection_with_a_held_book(
        store: &BookPipelineStore,
    ) -> (BookPipelineJob, String, String) {
        let job = queue_job(
            store,
            fake_collection_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
        )
        .unwrap();
        let child = job
            .children
            .iter()
            .find(|child| {
                child
                    .route
                    .iter()
                    .any(|item| item.route_kind == "blocked_dirty_text_layer")
            })
            .expect("the fixture collection holds one dirty-text-layer book");
        let route_item_id = child
            .route
            .iter()
            .find(|item| item.route_kind == "blocked_dirty_text_layer")
            .unwrap()
            .id
            .clone();
        (job.clone(), child.id.clone(), route_item_id)
    }

    fn credentialed_config() -> BookPipelinePreviewConfig {
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        }
    }

    // The Overview tab offered the wizard's three choices with every button
    // disabled, so a held book could only be dealt with by deleting it — which
    // for a collection took the whole batch.
    #[test]
    fn a_held_book_can_be_rerouted_in_place() {
        let root = temp_root("route-override-in-place");
        let store = BookPipelineStore::for_test(&root);
        let (job, child_id, route_item_id) = queued_collection_with_a_held_book(&store);

        let rerouted = set_route_override(
            &store,
            &job.id,
            Some(&child_id),
            &route_item_id,
            "paddle",
            &credentialed_config(),
        )
        .unwrap();

        let child = rerouted
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        let item = child
            .route
            .iter()
            .find(|item| item.id == route_item_id)
            .unwrap();
        assert_eq!(item.route_kind, "remote_paddleocr");
        assert!(item.can_run);
        assert_eq!(item.route_override.as_deref(), Some("paddle"));
        assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_READY);
        assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_PENDING);
        assert!(child.last_error.is_none());

        // The decision has to survive a restart, so it lives on the source the
        // runner re-routes from, not only on the route it just recomputed.
        let reloaded = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|stored| stored.id == job.id)
            .unwrap();
        let child = reloaded
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        assert_eq!(
            child.source.route_overrides.get(&route_item_id),
            Some(&"paddle".to_string())
        );
        assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_READY);

        // `auto` drops the override and schedules a fresh automatic routing pass,
        // which is the only thing that can undo a forced route.
        let cleared = set_route_override(
            &store,
            &job.id,
            Some(&child_id),
            &route_item_id,
            "auto",
            &credentialed_config(),
        )
        .unwrap();
        let child = cleared
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        assert!(child.source.route_overrides.is_empty());
        assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_READY);
        assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_PENDING);

        let _ = fs::remove_dir_all(root);
    }

    // Forcing a provider whose credentials are missing must leave the book held,
    // not ready a stage that would only fail at the next spawn.
    #[test]
    fn a_reroute_without_credentials_keeps_the_book_held() {
        let root = temp_root("route-override-no-credentials");
        let store = BookPipelineStore::for_test(&root);
        let (job, child_id, route_item_id) = queued_collection_with_a_held_book(&store);

        let held = set_route_override(
            &store,
            &job.id,
            Some(&child_id),
            &route_item_id,
            "paddle",
            &BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let child = held
            .children
            .iter()
            .find(|child| child.id == child_id)
            .unwrap();
        let item = child
            .route
            .iter()
            .find(|item| item.id == route_item_id)
            .unwrap();
        assert_eq!(item.route_kind, "missing_credentials");
        assert!(!item.can_run);
        assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_BLOCKED);
        assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_PENDING);
        assert!(child.last_error.is_some());

        for (item_id, token) in [
            (route_item_id.as_str(), "definitely-not-a-token"),
            ("no-such-route-item", "paddle"),
        ] {
            assert!(
                set_route_override(
                    &store,
                    &job.id,
                    Some(&child_id),
                    item_id,
                    token,
                    &credentialed_config(),
                )
                .is_err(),
                "{item_id}/{token} should be rejected"
            );
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zotero_preview_blocks_only_items_that_need_credentials_or_manual_review() {
        let source = fake_collection_source();

        let route = preview_route(
            &source,
            "conversion_only",
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: false,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
        );

        assert!(route
            .iter()
            .any(|item| item.route_kind == "direct_text" && item.can_run));
        assert!(route
            .iter()
            .any(|item| item.route_kind == "missing_credentials" && !item.can_run));
        assert!(route
            .iter()
            .any(|item| item.route_kind == "mineru" && item.can_run));
        assert!(route.iter().any(|item| {
            item.route_kind == "blocked_dirty_text_layer"
                && !item.can_run
                && item.blocked_reason.is_some()
        }));
        assert!(route
            .iter()
            .any(|item| item.route_kind == "already_converted" && !item.can_run));

        let route_with_remote_ocr = preview_route(
            &source,
            "conversion_only",
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
        );
        assert!(route_with_remote_ocr
            .iter()
            .any(|item| item.route_kind == "remote_paddleocr" && item.can_run));
    }

    // ---- Staged-gates runner: split + prepare slice (issue #38) ----

    fn handoff_repo_fixture(root: &Path) -> PathBuf {
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("tools")).unwrap();
        fs::write(repo.join("AGENTS.md"), "# fixture\n").unwrap();
        fs::write(
            repo.join("tools").join("create_local_book_project.py"),
            "# fixture\n",
        )
        .unwrap();
        repo
    }

    fn handoff_ready_child_job(
        store: &BookPipelineStore,
        repo: &Path,
        source_path: &Path,
        source_text: &str,
    ) -> String {
        fs::write(source_path, source_text).unwrap();
        let job = queue_job(
            store,
            markdown_source(source_path),
            MODE_TRANSLATE_ONLY.into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let handed_off = run_job_with_handoff(
            store,
            &SystemPipelineRunner,
            &LocalProjectHandoffRunner,
            &job.id,
            Some(repo),
        )
        .unwrap();
        assert_eq!(handed_off.current_stage_id, "split");
        job.id
    }

    fn fake_handoff_ready_job(store: &BookPipelineStore, repo: &Path) -> String {
        fake_handoff_ready_job_with_options(store, repo, false, false)
    }

    fn fake_handoff_ready_job_with_second_pass(
        store: &BookPipelineStore,
        repo: &Path,
        second_pass_enabled: bool,
    ) -> String {
        fake_handoff_ready_job_with_options(store, repo, second_pass_enabled, false)
    }

    fn fake_handoff_ready_job_with_text_cleanup(store: &BookPipelineStore, repo: &Path) -> String {
        let translation_intent = serde_json::from_value(serde_json::json!({
            "translationMode": TRANSLATION_MODE_FAST,
            "profileId": "fake-provider-profile",
            "configId": "fake-provider-config",
            "skillIds": [],
            "secondPassEnabled": false,
            "textCleanup": true,
            "digestMode": false,
            "outputFormats": default_output_formats(),
        }))
        .unwrap();
        let job = queue_job_with_translation_intent(
            store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            translation_intent,
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let handed_off = run_job_with_handoff(
            store,
            &SystemPipelineRunner,
            &LocalProjectHandoffRunner,
            &job.id,
            Some(repo),
        )
        .unwrap();
        assert_eq!(handed_off.current_stage_id, "split");
        job.id
    }

    fn fake_handoff_ready_job_with_digest(store: &BookPipelineStore, repo: &Path) -> String {
        fake_handoff_ready_job_with_options(store, repo, false, true)
    }

    fn fake_handoff_ready_job_with_options(
        store: &BookPipelineStore,
        repo: &Path,
        second_pass_enabled: bool,
        digest_mode: bool,
    ) -> String {
        fake_handoff_ready_job_with_output_formats(
            store,
            repo,
            second_pass_enabled,
            digest_mode,
            default_output_formats(),
        )
    }

    fn fake_handoff_ready_job_with_output_formats(
        store: &BookPipelineStore,
        repo: &Path,
        second_pass_enabled: bool,
        digest_mode: bool,
        output_formats: Vec<String>,
    ) -> String {
        let job = queue_job_with_translation_intent(
            store,
            fake_source(None),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            BookPipelineTranslationIntent {
                translation_mode: TRANSLATION_MODE_FAST.into(),
                profile_id: "fake-provider-profile".into(),
                config_id: "fake-provider-config".into(),
                skill_ids: Vec::new(),
                second_pass_enabled,
                text_cleanup: false,
                digest_mode,
                output_formats,
            },
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let handed_off = run_job_with_handoff(
            store,
            &SystemPipelineRunner,
            &LocalProjectHandoffRunner,
            &job.id,
            Some(repo),
        )
        .unwrap();
        assert_eq!(handed_off.current_stage_id, "split");
        job.id
    }

    fn child_stage_status(job: &BookPipelineJob, stage_id: &str) -> String {
        job.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == stage_id)
            .unwrap_or_else(|| panic!("stage {stage_id} is missing"))
            .status
            .clone()
    }

    fn child_project_root(job: &BookPipelineJob) -> PathBuf {
        PathBuf::from(
            job.children[0]
                .local_project_root
                .as_deref()
                .expect("registered local project root"),
        )
    }

    fn fixture_translation(source: &str, unit_id: &str) -> String {
        let mut translated = source
            .lines()
            .map(|line| {
                if let Some(level) = atx_heading_level(line) {
                    format!("{} Translated {unit_id}", "#".repeat(level))
                } else if line.trim().is_empty() {
                    String::new()
                } else {
                    format!("Translated: {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        translated.push('\n');
        translated
    }

    fn configure_expert_job(store: &BookPipelineStore, job_id: &str) {
        let mut state = store.load().unwrap();
        let job = state.jobs.iter_mut().find(|job| job.id == job_id).unwrap();
        job.translation_mode = TRANSLATION_MODE_EXPERT.into();
        job.translation_profile_id = "fake-agent-profile".into();
        job.translation_config_id = "fake-agent-config".into();
        job.translation_skill_ids = vec![EXPERT_QA_SKILL_ID.into()];
        job.updated_at = now_label();
        derive_job(job);
        store.save(&state).unwrap();
    }

    fn approve_ready_translation_for_test(store: &BookPipelineStore, job_id: &str) {
        let mut state = store.load().unwrap();
        let job_index = state.jobs.iter().position(|job| job.id == job_id).unwrap();
        assert!(approve_translation_gate(&mut state.jobs[job_index], 0));
        state.jobs[job_index].updated_at = now_label();
        derive_job(&mut state.jobs[job_index]);
        store.save(&state).unwrap();
    }

    fn approve_ready_promotion_for_test(store: &BookPipelineStore, job_id: &str) -> String {
        let mut state = store.load().unwrap();
        let job_index = state.jobs.iter().position(|job| job.id == job_id).unwrap();
        assert!(approve_promotion_gate(&mut state.jobs[job_index], 0));
        let approval_id = state.jobs[job_index].children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_promotion")
            .and_then(|stage| stage.approval_id.clone())
            .unwrap();
        state.jobs[job_index].updated_at = now_label();
        derive_job(&mut state.jobs[job_index]);
        store.save(&state).unwrap();
        approval_id
    }

    fn satisfy_translation_handoff(job: &BookPipelineJob) {
        let project_root = child_project_root(job);
        let handoff: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(
                project_root
                    .join("qa")
                    .join("handoffs")
                    .join("translate.json"),
            )
            .unwrap(),
        )
        .unwrap();
        for unit in handoff["units"].as_array().unwrap() {
            let unit_id = unit["unitId"].as_str().unwrap();
            let source =
                fs::read_to_string(project_root.join(unit["sourceChapterPath"].as_str().unwrap()))
                    .unwrap();
            let output_path = project_root.join(unit["outputPath"].as_str().unwrap());
            fs::create_dir_all(output_path.parent().unwrap()).unwrap();
            fs::write(output_path, fixture_translation(&source, unit_id)).unwrap();
        }
    }

    fn qa_handoff(job: &BookPipelineJob) -> ExpertQaHandoff {
        let project_root = child_project_root(job);
        serde_json::from_str(
            &fs::read_to_string(
                project_root
                    .join("qa")
                    .join("handoffs")
                    .join("expert_qa.json"),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn set_expert_review(job: &BookPipelineJob, unit_id: &str, status: &str, unresolved: u64) {
        let project_root = child_project_root(job);
        let handoff = qa_handoff(job);
        let control_path = project_root
            .join("qa")
            .join("chapter_controls")
            .join(format!("{unit_id}.json"));
        let mut control: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&control_path).unwrap()).unwrap();
        let translation_sha256 = handoff.translation_hashes[unit_id].clone();
        let skill_ids = handoff.skill_ids.clone();
        control["unresolvedPolysemy"] = serde_json::json!(unresolved);
        control["expertReview"] = serde_json::json!({
            "required": true,
            "status": status,
            "translationSha256": translation_sha256,
            "skillIds": skill_ids,
            "unresolved": {
                "fidelity": unresolved,
                "terminology": 0,
                "note": 0,
                "traceability": 0,
                "polysemy": unresolved,
            },
        });
        fs::write(
            control_path,
            serde_json::to_string_pretty(&control).unwrap() + "\n",
        )
        .unwrap();
    }

    fn satisfy_qa_handoff(job: &BookPipelineJob) {
        let handoff = qa_handoff(job);
        for unit_id in handoff_sample_ids(&handoff) {
            set_expert_review(job, &unit_id, "pass", 0);
        }
    }

    fn fake_job_waiting_for_expert_qa(
        store: &BookPipelineStore,
        repo: &Path,
        executor: &dyn RunnerCommandExecutor,
    ) -> (String, BookPipelineJob) {
        fake_job_waiting_for_expert_qa_with_digest(store, repo, executor, false)
    }

    fn fake_job_waiting_for_expert_qa_with_digest(
        store: &BookPipelineStore,
        repo: &Path,
        executor: &dyn RunnerCommandExecutor,
        digest_mode: bool,
    ) -> (String, BookPipelineJob) {
        let job_id = if digest_mode {
            fake_handoff_ready_job_with_digest(store, repo)
        } else {
            fake_handoff_ready_job(store, repo)
        };
        advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
        advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
        let waiting = advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
        assert_eq!(child_stage_status(&waiting, "expert_qa"), STATUS_BLOCKED);
        (job_id, waiting)
    }

    fn chapter_control(job: &BookPipelineJob, unit_id: &str) -> serde_json::Value {
        let project_root = child_project_root(job);
        serde_json::from_str(
            &fs::read_to_string(
                project_root
                    .join("qa")
                    .join("chapter_controls")
                    .join(format!("{unit_id}.json")),
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn automated_qa_covers_placeholders_structure_glossary_and_completeness() {
        let unit = |translation_text: &str| ExpertQaUnit {
            unit_id: "chapter_001".into(),
            source_text: "# Heading\n\nUse {name} Foo.\n".into(),
            translation_text: translation_text.into(),
            translation_path: PathBuf::new(),
            translation_sha256: sha256_str(translation_text),
            control_path: PathBuf::new(),
        };
        let terms = vec![("Foo".to_string(), "术语".to_string())];

        assert!(automated_qa_checks(&unit("# 标题\n\n使用 {name} 术语。\n"), &terms).passed());
        assert!(
            !automated_qa_checks(&unit("# 标题\n\n使用术语。\n"), &terms).placeholder_integrity
        );
        assert!(!automated_qa_checks(&unit("## 标题\n\n使用 {name} 术语。\n"), &terms).structure);
        assert!(
            !automated_qa_checks(&unit("# 标题\n\n使用 {name}。\n"), &terms)
                .terminology_consistency
        );
        assert!(!automated_qa_checks(&unit("# 标题\n"), &terms).completeness);
    }

    #[test]
    fn advance_runs_split_and_keeps_prepare_runnable() {
        let root = temp_root("advance-split");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# Chapter One\n\nAlpha paragraph.\n\nBeta paragraph.\n\n# Chapter Two\n\nGamma paragraph.\n",
        );

        let advanced = advance_job(&store, &job_id, None, false).unwrap();

        assert_eq!(child_stage_status(&advanced, "split"), STATUS_COMPLETED);
        // Prepare is left PENDING but runnable; the gate slice owns readying it.
        assert_eq!(child_stage_status(&advanced, "prepare"), STATUS_PENDING);

        let child = &advanced.children[0];
        let source_map = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "source_map")
            .expect("source_map artifact registered");
        assert!(source_map.sha256.is_some());
        assert_eq!(
            child
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == "chapter_source")
                .count(),
            2
        );

        let split = child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "split")
            .unwrap();
        assert!(split.input_hashes.contains_key("sourceMarkdownSha256"));

        let project_root = child_project_root(&advanced);
        let chapter_one = project_root
            .join("chapters")
            .join("src")
            .join("chapter_001.md");
        assert!(chapter_one.is_file());
        assert!(fs::read_to_string(&chapter_one)
            .unwrap()
            .contains("Chapter One"));
        assert!(project_root
            .join("metadata")
            .join("source_map.json")
            .is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn advance_runs_prepare_and_parks_before_translation_gate() {
        let root = temp_root("advance-prepare");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# Alpha\n\nFirst body paragraph.\n\n# Beta\n\nSecond body paragraph.\n",
        );

        advance_job(&store, &job_id, None, false).unwrap();
        let advanced = advance_job(&store, &job_id, None, false).unwrap();

        assert_eq!(child_stage_status(&advanced, "prepare"), STATUS_COMPLETED);
        // The review packet is ready, but the runner never crosses the gate.
        assert_eq!(
            child_stage_status(&advanced, "approve_translation"),
            STATUS_READY
        );
        assert_eq!(advanced.children[0].current_stage_id, "approve_translation");

        let child = &advanced.children[0];
        let approval_request = child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap()
            .approval_request
            .as_ref()
            .expect("translation approval request");
        assert_eq!(approval_request.gate_id, "translation_disclosure");
        assert_eq!(approval_request.translation_mode, TRANSLATION_MODE_FAST);
        assert!(!approval_request.second_pass_enabled);
        assert!(!approval_request.text_cleanup);
        assert_eq!(
            approval_request.provider_profile_id.as_deref(),
            Some("fake-provider-profile")
        );
        assert_eq!(approval_request.agent_profile_id, None);
        assert_eq!(approval_request.config_id, "fake-provider-config");
        assert!(approval_request.skill_ids.is_empty());
        assert!(approval_request
            .bound_artifact_hashes
            .contains_key("source_markdown"));
        assert!(approval_request
            .bound_artifact_hashes
            .keys()
            .any(|key| key.starts_with("translation_task_manifest:")));
        assert_eq!(
            child
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == "translation_task_manifest")
                .count(),
            2
        );
        for kind in ["glossary", "style_profile"] {
            assert!(
                child
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.kind == kind && artifact.sha256.is_some()),
                "missing {kind} artifact"
            );
        }
        assert_eq!(
            child
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == "chapter_control")
                .count(),
            2
        );

        let prepare = child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "prepare")
            .unwrap();
        for key in [
            "sourceMapSha256",
            "glossarySha256",
            "styleProfileSha256",
            "taskPolicyVersion",
        ] {
            assert!(prepare.input_hashes.contains_key(key), "missing key {key}");
        }

        // Task manifests bind hashes and paths but never embed private source text.
        let project_root = child_project_root(&advanced);
        let task_manifest = fs::read_to_string(
            project_root
                .join("qa")
                .join("tasks")
                .join("chapter_001.json"),
        )
        .unwrap();
        assert!(task_manifest.contains("\"chapterId\": \"chapter_001\""));
        assert!(task_manifest.contains("\"sourceChapterSha256\""));
        assert!(!task_manifest.contains("First body paragraph"));
        let persisted = store.load().unwrap();
        let persisted_job = persisted.jobs.iter().find(|job| job.id == job_id).unwrap();
        assert_eq!(
            child_stage_status(persisted_job, "approve_translation"),
            STATUS_READY
        );

        let original_binding = advanced.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap()
            .input_hashes["approvalBindingSha256"]
            .clone();
        let mut toggled = advanced.clone();
        toggled.second_pass_enabled = true;
        assert!(ready_translation_approval_gate(&mut toggled, 0));
        let toggled_gate = toggled.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert!(
            toggled_gate
                .approval_request
                .as_ref()
                .unwrap()
                .second_pass_enabled
        );
        assert_ne!(
            toggled_gate.input_hashes["approvalBindingSha256"],
            original_binding
        );

        let mut cleanup_toggled = advanced.clone();
        cleanup_toggled.text_cleanup = true;
        assert!(ready_translation_approval_gate(&mut cleanup_toggled, 0));
        let cleanup_gate = cleanup_toggled.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert!(cleanup_gate.approval_request.as_ref().unwrap().text_cleanup);
        assert_ne!(
            cleanup_gate.input_hashes["approvalBindingSha256"],
            original_binding
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_gate_approval_requires_explicit_current_binding() {
        let root = temp_root("public-gate-approval");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# Chapter\n\nBody paragraph.\n",
        );
        advance_job(&store, &job_id, None, false).unwrap();
        let ready = advance_job(&store, &job_id, None, false).unwrap();
        let child_id = ready.children[0].id.clone();

        let rejected = approve_job_gate(
            &store,
            &job_id,
            Some(&child_id),
            "approve_translation",
            false,
        )
        .unwrap_err();
        assert!(rejected.contains("Explicit"));

        let approved = approve_job_gate(
            &store,
            &job_id,
            Some(&child_id),
            "approve_translation",
            true,
        )
        .unwrap();
        assert_eq!(
            child_stage_status(&approved, "approve_translation"),
            STATUS_COMPLETED
        );
        assert_eq!(child_stage_status(&approved, "translate"), STATUS_READY);
        assert!(approved.approval_references.iter().any(|approval| {
            approval.child_job_id == child_id
                && approval.stage_id == "approve_translation"
                && approval.decision == "approved"
        }));

        let repeated = approve_job_gate(
            &store,
            &job_id,
            Some(&child_id),
            "approve_translation",
            true,
        )
        .unwrap_err();
        assert!(repeated.contains("not ready"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preflight_sample_manifest_carries_the_run_text_cleanup_and_custom_instructions() {
        // The sample exists so a user can approve the full run on the strength of
        // a few translated passages. It only earns that if it is translated under
        // the same instructions -- previously it carried neither field, so anyone
        // who had set one was judging a preview the real run would not reproduce.
        let root = temp_root("translation-sample-prompt-inputs");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# One\n\nFirst.\n\n# Two\n\nSecond.\n\n# Three\n\nThird.\n\n# Four\n\nFourth.\n\n# Five\n\nFifth.\n",
        );
        advance_job(&store, &job_id, None, false).unwrap();
        let prepared = advance_job(&store, &job_id, None, false).unwrap();
        let child_id = prepared.children[0].id.clone();

        let custom_instructions = BookPipelineCustomInstructions {
            translation: Some("Use restrained literary Chinese.".into()),
            reflection: Some("Critique anachronistic wording.".into()),
        };
        save_book_custom_instructions(
            &store,
            &job_id,
            Some(&child_id),
            custom_instructions.clone(),
        )
        .unwrap();
        let mut state = store.load().unwrap();
        let job = state.jobs.iter_mut().find(|job| job.id == job_id).unwrap();
        job.text_cleanup = true;
        job.updated_at = now_label();
        store.save(&state).unwrap();

        let executor = TranslationSampleFixtureExecutor::default();
        run_translation_sample_with_executor(
            &store,
            &job_id,
            Some(&child_id),
            "fake-provider-profile",
            "sample-config-a",
            false,
            &executor,
        )
        .unwrap();

        let inputs = executor.prompt_inputs();
        assert_eq!(inputs.len(), 1);
        let (text_cleanup, custom) = &inputs[0];
        assert_eq!(text_cleanup, &serde_json::json!(true));
        // The whole object goes over, matching the run manifest. The engine drops
        // the reflection half on the sample path, which runs no reflection pass.
        assert_eq!(
            custom,
            &serde_json::json!({
                "translation": "Use restrained literary Chinese.",
                "reflection": "Critique anachronistic wording.",
            })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preflight_sample_rebinds_the_gate_without_adopting_its_provider() {
        let root = temp_root("translation-preflight-sample");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# One\n\nFirst.\n\n# Two\n\nSecond.\n\n# Three\n\nThird.\n\n# Four\n\nFourth.\n\n# Five\n\nFifth.\n",
        );
        advance_job(&store, &job_id, None, false).unwrap();
        let prepared = advance_job(&store, &job_id, None, false).unwrap();
        let child_id = prepared.children[0].id.clone();
        let original_binding = prepared.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap()
            .input_hashes["approvalBindingSha256"]
            .clone();
        let queued_profile = prepared.translation_profile_id.clone();
        let queued_config = prepared.translation_config_id.clone();
        let executor = TranslationSampleFixtureExecutor::default();

        let first = run_translation_sample_with_executor(
            &store,
            &job_id,
            Some(&child_id),
            "fake-provider-profile",
            "sample-config-a",
            false,
            &executor,
        )
        .unwrap();
        let first_gate = first.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        let first_binding = first_gate.input_hashes["approvalBindingSha256"].clone();
        let first_evidence = first_gate
            .approval_request
            .as_ref()
            .unwrap()
            .sample_evidence["translation_sample_report"]
            .clone();
        assert_ne!(first_binding, original_binding);
        assert_eq!(first_gate.status, STATUS_READY);
        assert_eq!(child_stage_status(&first, "translate"), STATUS_PENDING);
        // Sampling is "try before you decide": it must not adopt the provider it
        // was run with. It used to, so one sample silently redirected the book.
        assert_eq!(first.translation_profile_id, queued_profile);
        assert_eq!(first.translation_config_id, queued_config);
        let first_artifact = first.children[0]
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "translation_sample_report")
            .unwrap();
        assert_eq!(
            first_artifact.sha256.as_deref(),
            Some(first_evidence.as_str())
        );
        assert_eq!(first_artifact.privacy, "private_text");
        let first_report = read_translation_sample_report(&first, &child_id).unwrap();
        assert_eq!(first_report.samples.len(), 3);
        assert_eq!(first_report.samples[1].degradation, "aligned");
        let sample_dir = child_project_root(&first).join("qa").join("sample-compare");
        assert!(Path::new(&first_artifact.path).is_file());
        assert!(fs::read_dir(&sample_dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("manifest-")));
        assert_eq!(fs::read_dir(&sample_dir).unwrap().count(), 1);
        assert!(fs::read_dir(
            child_project_root(&first)
                .join("chapters")
                .join("translated")
        )
        .unwrap()
        .next()
        .is_none());

        let second = run_translation_sample_with_executor(
            &store,
            &job_id,
            Some(&child_id),
            "fake-provider-profile",
            "sample-config-b",
            false,
            &executor,
        )
        .unwrap();
        let second_gate = second.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert_ne!(
            second_gate.input_hashes["approvalBindingSha256"],
            first_binding
        );
        assert_ne!(
            second_gate
                .approval_request
                .as_ref()
                .unwrap()
                .sample_evidence["translation_sample_report"],
            first_evidence
        );
        assert_eq!(
            second.children[0]
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == "translation_sample_report")
                .count(),
            1
        );
        assert_eq!(fs::read_dir(&sample_dir).unwrap().count(), 1);
        assert_eq!(
            executor.requests(),
            vec![
                ("fake-provider-profile".into(), "sample-config-a".into()),
                ("fake-provider-profile".into(), "sample-config-b".into()),
            ]
        );

        let approved = approve_job_gate(
            &store,
            &job_id,
            Some(&child_id),
            "approve_translation",
            true,
        )
        .unwrap();
        assert_eq!(
            child_stage_status(&approved, "approve_translation"),
            STATUS_COMPLETED
        );
        assert_eq!(child_stage_status(&approved, "translate"), STATUS_READY);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn adopting_a_sampled_provider_is_a_separate_action_that_rebinds_the_gate() {
        // The counterpart to the test above: sampling leaves the job alone, so
        // there has to be an explicit way to say "translate the book with this
        // one", and taking it must drop an approval granted against the old
        // provider -- the provider is inside the binding the user approved.
        let root = temp_root("translation-provider-adopt");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# One\n\nFirst.\n\n# Two\n\nSecond.\n",
        );
        advance_job(&store, &job_id, None, false).unwrap();
        let prepared = advance_job(&store, &job_id, None, false).unwrap();
        let child_id = prepared.children[0].id.clone();
        let queued_config = prepared.translation_config_id.clone();
        let original_binding = prepared.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap()
            .input_hashes["approvalBindingSha256"]
            .clone();

        let adopted = set_translation_provider_in_store(
            &store,
            &job_id,
            Some(&child_id),
            "fake-provider-profile",
            "adopted-config",
        )
        .unwrap();
        assert_ne!(queued_config, "adopted-config");
        assert_eq!(adopted.translation_profile_id, "fake-provider-profile");
        assert_eq!(adopted.translation_config_id, "adopted-config");
        let gate = adopted.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert_ne!(gate.input_hashes["approvalBindingSha256"], original_binding);
        assert_eq!(gate.status, STATUS_READY);
        assert_eq!(
            gate.approval_request.as_ref().unwrap().config_id,
            "adopted-config"
        );
        assert_eq!(child_stage_status(&adopted, "translate"), STATUS_PENDING);

        // Setting the same slot again is a no-op rather than a spurious rebind.
        let repeated = set_translation_provider_in_store(
            &store,
            &job_id,
            Some(&child_id),
            "fake-provider-profile",
            "adopted-config",
        )
        .unwrap();
        assert_eq!(
            repeated.children[0]
                .stages
                .iter()
                .find(|stage| stage.stage_id == "approve_translation")
                .unwrap()
                .input_hashes["approvalBindingSha256"],
            gate.input_hashes["approvalBindingSha256"]
        );

        // An approval granted against one provider must not carry over to another.
        let approved = approve_job_gate(
            &store,
            &job_id,
            Some(&child_id),
            "approve_translation",
            true,
        )
        .unwrap();
        assert_eq!(
            child_stage_status(&approved, "approve_translation"),
            STATUS_COMPLETED
        );
        let switched = set_translation_provider_in_store(
            &store,
            &job_id,
            Some(&child_id),
            "fake-provider-profile",
            "another-config",
        )
        .unwrap();
        assert_eq!(
            child_stage_status(&switched, "approve_translation"),
            STATUS_READY
        );
        assert_eq!(child_stage_status(&switched, "translate"), STATUS_PENDING);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn digest_mode_change_reopens_completed_translation_approval() {
        let root = temp_root("digest-mode-approval-recheck");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# Chapter\n\nBody paragraph.\n",
        );
        advance_job(&store, &job_id, None, false).unwrap();
        advance_job(&store, &job_id, None, false).unwrap();
        approve_ready_translation_for_test(&store, &job_id);
        let approved = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        let original_gate = approved.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert_eq!(original_gate.status, STATUS_COMPLETED);
        let original_binding = original_gate.input_hashes["approvalBindingSha256"].clone();
        let original_approval_id = original_gate.approval_id.clone().unwrap();

        let mut toggled = approved;
        toggled.digest_mode = true;
        assert!(ready_translation_approval_gate(&mut toggled, 0));
        let toggled_gate = toggled.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert_eq!(toggled_gate.status, STATUS_READY);
        assert!(toggled_gate.approval_id.is_none());
        assert_eq!(child_stage_status(&toggled, "build_digest"), STATUS_PENDING);
        assert_ne!(
            toggled_gate.input_hashes["approvalBindingSha256"],
            original_binding
        );
        assert_eq!(
            serde_json::to_value(toggled_gate.approval_request.as_ref().unwrap()).unwrap()
                ["digestMode"],
            true
        );
        assert!(!toggled
            .approval_references
            .iter()
            .any(|approval| approval.approval_id == original_approval_id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn output_formats_change_reopens_completed_translation_approval() {
        let root = temp_root("output-formats-approval-recheck");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# Chapter\n\nBody paragraph.\n",
        );
        advance_job(&store, &job_id, None, false).unwrap();
        advance_job(&store, &job_id, None, false).unwrap();
        approve_ready_translation_for_test(&store, &job_id);
        let approved = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        let original_gate = approved.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert_eq!(original_gate.status, STATUS_COMPLETED);
        let original_binding = original_gate.input_hashes["approvalBindingSha256"].clone();
        let original_approval_id = original_gate.approval_id.clone().unwrap();

        let mut toggled = approved;
        toggled.output_formats.push(OUTPUT_FORMAT_BILINGUAL.into());
        assert!(ready_translation_approval_gate(&mut toggled, 0));
        let toggled_gate = toggled.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert_eq!(toggled_gate.status, STATUS_READY);
        assert!(toggled_gate.approval_id.is_none());
        assert_ne!(
            toggled_gate.input_hashes["approvalBindingSha256"],
            original_binding
        );
        assert_eq!(
            toggled_gate
                .approval_request
                .as_ref()
                .unwrap()
                .output_formats,
            vec!["md", "html", "epub", "bilingual"]
        );
        assert!(!toggled
            .approval_references
            .iter()
            .any(|approval| approval.approval_id == original_approval_id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn custom_instructions_change_reopens_completed_translation_approval() {
        let root = temp_root("custom-instructions-approval-recheck");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# Chapter\n\nBody paragraph.\n",
        );
        advance_job(&store, &job_id, None, false).unwrap();
        advance_job(&store, &job_id, None, false).unwrap();
        approve_ready_translation_for_test(&store, &job_id);
        let approved = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        let original_gate = approved.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        let original_binding = original_gate.input_hashes["approvalBindingSha256"].clone();
        let original_approval_id = original_gate.approval_id.clone().unwrap();

        let updated = save_book_custom_instructions(
            &store,
            &job_id,
            None,
            BookPipelineCustomInstructions {
                translation: Some("Use restrained literary Chinese.".into()),
                reflection: None,
            },
        )
        .unwrap();
        let updated_gate = updated.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert_eq!(updated_gate.status, STATUS_READY);
        assert!(updated_gate.approval_id.is_none());
        assert!(updated_gate
            .input_hashes
            .contains_key("customInstructionsSha256"));
        assert_ne!(
            updated_gate.input_hashes["approvalBindingSha256"],
            original_binding
        );
        assert!(!updated
            .approval_references
            .iter()
            .any(|approval| approval.approval_id == original_approval_id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fast_book_passes_enabled_second_pass_to_translation_manifest() {
        let root = temp_root("fake-translate-second-pass");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job_with_second_pass(&store, &repo, true);
        let executor = TranslationEngineFixtureExecutor::with_second_pass_enabled();

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert!(advanced.second_pass_enabled);
        assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
        assert_eq!(
            executor.requested_units(),
            vec![vec!["chapter_001".to_string()]]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fast_book_passes_enabled_text_cleanup_to_translation_manifest() {
        let root = temp_root("fake-translate-text-cleanup");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job_with_text_cleanup(&store, &repo);
        let executor = TranslationEngineFixtureExecutor::with_text_cleanup();

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert!(advanced.text_cleanup);
        assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
        assert_eq!(
            executor.requested_units(),
            vec![vec!["chapter_001".to_string()]]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn book_custom_instructions_persist_bind_approval_and_flow_to_run_manifest() {
        let root = temp_root("book-custom-instructions");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job_with_second_pass(&store, &repo, true);
        let custom_instructions = BookPipelineCustomInstructions {
            translation: Some("Use restrained literary Chinese.".into()),
            reflection: Some("Critique anachronistic wording.".into()),
        };

        let saved =
            save_book_custom_instructions(&store, &job_id, None, custom_instructions.clone())
                .unwrap();

        assert_eq!(
            saved.children[0].custom_instructions.as_ref(),
            Some(&custom_instructions)
        );
        let persisted: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
        assert_eq!(
            persisted["jobs"][0]["children"][0]["customInstructions"],
            serde_json::json!({
                "translation": "Use restrained literary Chinese.",
                "reflection": "Critique anachronistic wording.",
            })
        );

        let executor =
            TranslationEngineFixtureExecutor::with_custom_instructions(custom_instructions.clone());
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let gate = advanced.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        assert!(gate.input_hashes.contains_key("customInstructionsSha256"));
        assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(child_project_root(&advanced).join("qa/tasks/run.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            manifest["customInstructions"],
            serde_json::to_value(custom_instructions).unwrap()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn custom_instructions_are_per_book_and_reject_overlong_text() {
        let root = temp_root("per-book-custom-instructions");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job_with_translation_intent(
            &store,
            fake_collection_source(),
            MODE_CONVERT_THEN_TRANSLATE.into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let selected_child_id = job.children[0].id.clone();
        let custom_instructions = BookPipelineCustomInstructions {
            translation: Some("Keep this book's dry humor.".into()),
            reflection: None,
        };

        let saved = save_book_custom_instructions(
            &store,
            &job.id,
            Some(&selected_child_id),
            custom_instructions.clone(),
        )
        .unwrap();

        assert_eq!(
            saved.children[0].custom_instructions.as_ref(),
            Some(&custom_instructions)
        );
        assert!(saved.children[1..]
            .iter()
            .all(|child| child.custom_instructions.is_none()));

        let error = save_book_custom_instructions(
            &store,
            &job.id,
            Some(&selected_child_id),
            BookPipelineCustomInstructions {
                translation: Some("x".repeat(2001)),
                reflection: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("custom_instructions_too_long"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn glossary_violations_reach_the_log_without_failing_the_translation() {
        // The engine report is parsed from stdout and never written anywhere the
        // reader can open, so a metric the runner does not surface is a metric
        // nobody will ever see. This pins the surfacing, and pins that it stays
        // a warning: the chapters are complete and the stage still passes.
        let root = temp_root("glossary-violation-log");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job(&store, &repo);
        let executor = TranslationEngineFixtureExecutor::reporting_glossary_violations(&[
            ("Fan", "风扇"),
            ("Secret", "秘密"),
        ]);

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
        let translate_stage = advanced.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "translate")
            .unwrap();
        assert!(translate_stage.error.is_none());

        let log = advanced.log_summary.join("\n");
        assert!(log.contains("Glossary check: 2 required term(s) not found"));
        assert!(log.contains("Fan -> 风扇"));
        assert!(log.contains("Secret -> 秘密"));
        assert!(log.contains("complete and unmodified"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_clean_glossary_adds_no_warning_line() {
        let root = temp_root("glossary-violation-absent");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job(&store, &repo);
        let executor = TranslationEngineFixtureExecutor::succeeding();

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
        assert!(!advanced.log_summary.join("\n").contains("Glossary check"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_book_auto_approves_translates_and_stops_at_expert_qa() {
        let root = temp_root("fake-translate-success");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job(&store, &repo);
        let executor = TranslationEngineFixtureExecutor::succeeding();

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(
            child_stage_status(&advanced, "approve_translation"),
            STATUS_COMPLETED
        );
        assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
        assert_eq!(child_stage_status(&advanced, "expert_qa"), STATUS_PENDING);
        assert_eq!(advanced.current_stage_id, "expert_qa");
        assert_eq!(
            executor.requested_units(),
            vec![vec!["chapter_001".to_string()]]
        );

        let child = &advanced.children[0];
        let gate = child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap();
        let approval_id = gate.approval_id.as_deref().expect("approval ID");
        let approval = advanced
            .approval_references
            .iter()
            .find(|approval| approval.approval_id == approval_id)
            .expect("approval reference");
        assert_eq!(approval.decision, "approved");
        assert_eq!(
            approval.bound_artifact_hashes,
            gate.approval_request
                .as_ref()
                .unwrap()
                .bound_artifact_hashes
        );

        let translated = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "chapter_translation")
            .expect("translated chapter artifact");
        assert_eq!(translated.producer_stage.as_deref(), Some("translate"));
        let translated_sha256 = sha256_file(Path::new(&translated.path)).unwrap();
        assert_eq!(
            translated.sha256.as_deref(),
            Some(translated_sha256.as_str())
        );
        let translate = child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "translate")
            .unwrap();
        let summary = translate.unit_summary.as_ref().unwrap();
        assert_eq!(
            (summary.total, summary.completed, summary.failed),
            (1, 1, 0)
        );
        assert!(translate.artifact_ids.contains(&translated.artifact_id));
        assert_eq!(translate.attempt, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_fast_book_runs_layered_qa_and_readies_promotion_gate() {
        let root = temp_root("fake-fast-expert-qa");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job(&store, &repo);
        let executor = TranslationEngineFixtureExecutor::succeeding();

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let translated =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(
            child_stage_status(&translated, "translate"),
            STATUS_COMPLETED
        );
        assert_eq!(child_stage_status(&waiting, "expert_qa"), STATUS_BLOCKED);
        let qa_stage = waiting.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "expert_qa")
            .unwrap();
        assert!(qa_stage
            .execution_owner
            .as_deref()
            .unwrap()
            .starts_with(AGENT_EXECUTION_OWNER_PREFIX));
        assert!(waiting.children[0]
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "expert_qa_handoff"));

        satisfy_qa_handoff(&waiting);
        let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(child_stage_status(&ready, "expert_qa"), STATUS_COMPLETED);
        assert_eq!(
            child_stage_status(&ready, "approve_promotion"),
            STATUS_READY
        );
        assert_eq!(ready.current_stage_id, "approve_promotion");
        let request = ready.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_promotion")
            .unwrap()
            .approval_request
            .as_ref()
            .unwrap();
        assert_eq!(request.gate_id, "promotion");
        assert_eq!(request.qa_policy.as_deref(), Some(TRANSLATION_MODE_FAST));
        assert!(request
            .bound_artifact_hashes
            .keys()
            .any(|key| key.starts_with("chapter_translation:")));
        assert!(request
            .bound_artifact_hashes
            .keys()
            .any(|key| key.starts_with("chapter_control:")));
        assert!(request
            .sample_evidence
            .keys()
            .any(|key| key.starts_with("chapter_control:")));
        let control = chapter_control(&ready, "chapter_001");
        assert_eq!(control["qaPolicy"], TRANSLATION_MODE_FAST);
        assert_eq!(control["checks"]["closure"], "pass");
        assert_eq!(control["unresolvedPolysemy"], 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_expert_book_waits_for_both_agent_handoffs_then_readies_promotion_gate() {
        let root = temp_root("fake-expert-handoffs");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job(&store, &repo);
        configure_expert_job(&store, &job_id);
        let executor = TranslationEngineFixtureExecutor::succeeding();

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let prepared = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(
            child_stage_status(&prepared, "approve_translation"),
            STATUS_READY
        );
        assert_eq!(executor.requested_units(), Vec::<Vec<String>>::new());
        approve_ready_translation_for_test(&store, &job_id);

        let translation_waiting =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(
            child_stage_status(&translation_waiting, "translate"),
            STATUS_BLOCKED
        );
        assert!(translation_waiting.children[0]
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "translation_handoff"));
        satisfy_translation_handoff(&translation_waiting);

        let translated =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(
            child_stage_status(&translated, "translate"),
            STATUS_COMPLETED
        );
        let qa_waiting =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(child_stage_status(&qa_waiting, "expert_qa"), STATUS_BLOCKED);
        satisfy_qa_handoff(&qa_waiting);

        let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(child_stage_status(&ready, "expert_qa"), STATUS_COMPLETED);
        assert_eq!(
            child_stage_status(&ready, "approve_promotion"),
            STATUS_READY
        );
        let request = ready.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_promotion")
            .unwrap()
            .approval_request
            .as_ref()
            .unwrap();
        assert_eq!(request.qa_policy.as_deref(), Some(TRANSLATION_MODE_EXPERT));
        assert_eq!(
            request.agent_profile_id.as_deref(),
            Some("fake-agent-profile")
        );
        assert_eq!(request.skill_ids, vec![EXPERT_QA_SKILL_ID.to_string()]);
        assert_eq!(executor.requested_units(), Vec::<Vec<String>>::new());
        let _ = fs::remove_dir_all(root);
    }

    // The Stages tab has always labelled a failure "retryable"; until now there
    // was no automatic retry behind that word. A stage that keeps failing must
    // spend a bounded budget and then say why it stopped.
    #[test]
    fn a_retryable_stage_failure_is_retried_to_the_budget_then_gives_up() {
        let root = temp_root("stage-retry-budget");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::failing_epubcheck();
        let (job_id, waiting) = fake_job_waiting_for_expert_qa(&store, &repo, &executor);
        satisfy_qa_handoff(&waiting);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        approve_ready_promotion_for_test(&store, &job_id);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        let stage = |job: &BookPipelineJob| {
            job.children[0]
                .stages
                .iter()
                .find(|stage| stage.stage_id == "validate_reading")
                .unwrap()
                .clone()
        };
        let validate = stage(&failed);
        assert_eq!(validate.status, STATUS_FAILED);
        assert_eq!(
            validate.attempt, DEFAULT_STAGE_MAX_ATTEMPTS,
            "the whole budget should be spent before giving up"
        );
        assert_eq!(
            validate.give_up_reason.as_deref(),
            Some(GIVE_UP_RETRY_BUDGET_EXHAUSTED)
        );
        assert!(
            validate.next_retry_at.is_none(),
            "nothing is scheduled once the budget is gone"
        );

        // The same story has to be readable from the progress the UI polls.
        let persisted = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        assert_eq!(persisted.progress.active_stage_id, "validate_reading");
        assert_eq!(persisted.progress.retry_attempts_remaining, 0);
        assert_eq!(
            persisted.progress.give_up_reason.as_deref(),
            Some(GIVE_UP_RETRY_BUDGET_EXHAUSTED)
        );

        // A give-up written for the automatic loop must not refuse the operator:
        // an advance still runs. It runs *once* — the budget counts the stage's
        // whole life, so clicking Advance cannot spin up a fresh ladder each time.
        let retried = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(stage(&retried).attempt, DEFAULT_STAGE_MAX_ATTEMPTS + 1);
        assert_eq!(
            stage(&retried).give_up_reason.as_deref(),
            Some(GIVE_UP_RETRY_BUDGET_EXHAUSTED)
        );

        let _ = fs::remove_dir_all(root);
    }

    // Expert QA blocks on a judgement call, not on a flaky process. Retrying it
    // automatically would burn the budget on something no retry can fix.
    #[test]
    fn a_non_retryable_failure_is_not_retried_automatically() {
        let stage = |code: &str, retryable: bool| BookPipelineStage {
            stage_id: "expert_qa".into(),
            status: STATUS_FAILED.into(),
            attempt: 1,
            safe_error: Some(BookPipelineSafeError {
                code: code.into(),
                summary: "redacted".into(),
                retryable,
                attempt: 1,
                stage_id: "expert_qa".into(),
                ..BookPipelineSafeError::default()
            }),
            ..BookPipelineStage::default()
        };
        let mut child = BookPipelineChildJob {
            stages: vec![stage("qa_blocked", false)],
            ..BookPipelineChildJob::default()
        };

        assert_eq!(schedule_stage_retry(&mut child, "expert_qa"), None);
        let blocked = stage_ref(&child, "expert_qa").unwrap();
        assert_eq!(
            blocked.give_up_reason.as_deref(),
            Some(GIVE_UP_NOT_RETRYABLE)
        );
        assert_eq!(
            blocked.attempt, 1,
            "a non-retryable failure must not spend an attempt"
        );
        assert_eq!(stage_attempts_remaining(blocked), 0);

        // The same stage classified as retryable does schedule one.
        let mut child = BookPipelineChildJob {
            stages: vec![stage("runner_failed", true)],
            ..BookPipelineChildJob::default()
        };
        assert_eq!(
            schedule_stage_retry(&mut child, "expert_qa"),
            Some(DEFAULT_STAGE_RETRY_BACKOFF_SECONDS[0])
        );
        let scheduled = stage_ref(&child, "expert_qa").unwrap();
        assert!(scheduled.give_up_reason.is_none());
        assert!(scheduled.next_retry_at.is_some());
        assert_eq!(
            stage_attempts_remaining(scheduled),
            DEFAULT_STAGE_MAX_ATTEMPTS - 1
        );
    }

    // A stage that carries its own policy is honoured over the default, which is
    // what makes the persisted table worth persisting.
    #[test]
    fn a_stage_policy_overrides_the_default_budget_and_backoff() {
        let mut child = BookPipelineChildJob {
            stages: vec![BookPipelineStage {
                stage_id: "translate".into(),
                status: STATUS_FAILED.into(),
                attempt: 1,
                max_attempts: 2,
                retry_backoff_seconds: vec![7],
                safe_error: Some(BookPipelineSafeError {
                    retryable: true,
                    ..BookPipelineSafeError::default()
                }),
                ..BookPipelineStage::default()
            }],
            ..BookPipelineChildJob::default()
        };

        assert_eq!(schedule_stage_retry(&mut child, "translate"), Some(7));

        // A one-entry table repeats its last entry rather than dropping to no wait.
        stage_mut(&mut child, "translate").unwrap().attempt = 2;
        assert_eq!(schedule_stage_retry(&mut child, "translate"), None);
        assert_eq!(
            stage_ref(&child, "translate")
                .unwrap()
                .give_up_reason
                .as_deref(),
            Some(GIVE_UP_RETRY_BUDGET_EXHAUSTED),
            "max_attempts 2 means the second failure is the last"
        );
    }

    #[test]
    fn translate_failure_retries_only_failed_units_and_recovers_stage() {
        let root = temp_root("fake-translate-retry");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job(&store, &repo);
        let handed_off = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        let project_root = child_project_root(&handed_off);
        fs::write(
            project_root.join("source").join("source.md"),
            "# Alpha\n\nFirst body.\n\n# Beta\n\nSecond body.\n",
        )
        .unwrap();
        let executor = TranslationEngineFixtureExecutor::failing_once("chapter_002");

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        // One transient unit failure is the runner's own problem now: the stage
        // fails, schedules itself and comes back without anyone pressing retry.
        let recovered = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        // The automatic attempt inherits the existing retry-scope trimming, so
        // the unit that already translated is not paid for twice.
        assert_eq!(
            executor.requested_units(),
            vec![
                vec!["chapter_001".to_string(), "chapter_002".to_string()],
                vec!["chapter_002".to_string()]
            ]
        );
        assert_eq!(
            child_stage_status(&recovered, "translate"),
            STATUS_COMPLETED
        );
        assert_eq!(child_stage_status(&recovered, "expert_qa"), STATUS_PENDING);
        assert_eq!(recovered.current_stage_id, "expert_qa");
        assert!(recovered.last_error.is_none());
        let translate = recovered.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "translate")
            .unwrap();
        let summary = translate.unit_summary.as_ref().unwrap();
        assert_eq!(
            (summary.total, summary.completed, summary.failed),
            (2, 2, 0)
        );
        assert_eq!(translate.attempt, 2);
        assert!(!translate
            .input_hashes
            .keys()
            .any(|key| key.starts_with("failedUnit:")));
        assert_eq!(
            recovered.children[0]
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == "chapter_translation")
                .count(),
            2
        );
        assert!(!recovered.children[0]
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "chapter_translation_degraded"));
        assert!(recovered.children[0]
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "chapter_translation")
            .all(|artifact| artifact.sha256.is_some()
                && artifact.producer_stage.as_deref() == Some("translate")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn expert_qa_retries_only_failed_unit_and_separates_fix_from_pass_attempt() {
        let root = temp_root("expert-qa-retry");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job(&store, &repo);
        let handed_off = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        let project_root = child_project_root(&handed_off);
        fs::write(
            project_root.join("source").join("source.md"),
            "# Alpha\n\nFirst body.\n\n# Beta\n\nSecond body.\n",
        )
        .unwrap();
        let executor = TranslationEngineFixtureExecutor::succeeding();

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let translated =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let chapter_two = child_project_root(&translated)
            .join("chapters")
            .join("translated")
            .join("chapter_002.md");
        fs::write(&chapter_two, "# Broken\n").unwrap();

        let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(child_stage_status(&failed, "expert_qa"), STATUS_FAILED);
        let qa_stage = failed.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "expert_qa")
            .unwrap();
        assert!(qa_stage.input_hashes.contains_key("failedUnit:chapter_002"));
        assert!(!qa_stage.input_hashes.contains_key("failedUnit:chapter_001"));
        assert_eq!(
            chapter_control(&failed, "chapter_001")["automationAttempt"],
            1
        );
        assert_eq!(
            chapter_control(&failed, "chapter_002")["automationAttempt"],
            1
        );

        let source_two = fs::read_to_string(
            child_project_root(&failed)
                .join("chapters")
                .join("src")
                .join("chapter_002.md"),
        )
        .unwrap();
        fs::write(
            &chapter_two,
            fixture_translation(&source_two, "chapter_002"),
        )
        .unwrap();
        let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(child_stage_status(&waiting, "expert_qa"), STATUS_BLOCKED);
        assert_eq!(
            chapter_control(&waiting, "chapter_001")["automationAttempt"],
            1
        );
        assert_eq!(
            chapter_control(&waiting, "chapter_002")["automationAttempt"],
            2
        );
        satisfy_qa_handoff(&waiting);
        let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(child_stage_status(&ready, "expert_qa"), STATUS_COMPLETED);
        assert_eq!(
            child_stage_status(&ready, "approve_promotion"),
            STATUS_READY
        );
        let control = chapter_control(&ready, "chapter_002");
        assert_eq!(control["fixAttempt"], 2);
        assert_eq!(control["closureEvidence"]["passAttempt"], 3);
        assert_ne!(
            control["fixAttempt"],
            control["closureEvidence"]["passAttempt"]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fast_qa_expands_a_defective_sample_until_the_next_clean_unit() {
        let root = temp_root("expert-qa-expansion");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job(&store, &repo);
        let handed_off = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        fs::write(
            child_project_root(&handed_off)
                .join("source")
                .join("source.md"),
            "# One\n\nBody one.\n\n# Two\n\nBody two.\n\n# Three\n\nBody three.\n",
        )
        .unwrap();
        let executor = TranslationEngineFixtureExecutor::succeeding();

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let initial_waiting =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let initial_handoff = qa_handoff(&initial_waiting);
        assert_eq!(initial_handoff.base_unit_ids.len(), 2);
        assert!(initial_handoff.expansion_unit_ids.is_empty());
        let defective = initial_handoff.base_unit_ids[0].clone();
        set_expert_review(&initial_waiting, &defective, "failed", 1);
        for unit_id in initial_handoff.base_unit_ids.iter().skip(1) {
            set_expert_review(&initial_waiting, unit_id, "pass", 0);
        }

        let expanded = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let expanded_handoff = qa_handoff(&expanded);
        assert_eq!(expanded_handoff.expansion_unit_ids.len(), 1);
        let expansion_unit = expanded_handoff.expansion_unit_ids[0].clone();
        set_expert_review(&expanded, &expansion_unit, "pass", 0);

        let clean_boundary =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(
            child_stage_status(&clean_boundary, "expert_qa"),
            STATUS_BLOCKED
        );
        assert_eq!(qa_handoff(&clean_boundary).expansion_unit_ids.len(), 1);
        set_expert_review(&clean_boundary, &defective, "pass", 0);

        let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(child_stage_status(&ready, "expert_qa"), STATUS_COMPLETED);
        assert_eq!(
            child_stage_status(&ready, "approve_promotion"),
            STATUS_READY
        );
        assert_eq!(qa_handoff(&ready).expansion_unit_ids, vec![expansion_unit]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn translation_and_control_hash_changes_invalidate_promotion_approval() {
        let root = temp_root("promotion-hash-invalidation");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let job_id = fake_handoff_ready_job(&store, &repo);
        let executor = TranslationEngineFixtureExecutor::succeeding();

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        satisfy_qa_handoff(&waiting);
        let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let first_approval = approve_ready_promotion_for_test(&store, &job_id);

        let translation_path = child_project_root(&ready)
            .join("chapters")
            .join("translated")
            .join("chapter_001.md");
        let translation = fs::read_to_string(&translation_path).unwrap();
        fs::write(
            &translation_path,
            translation.replace("Translated:", "Revised:"),
        )
        .unwrap();
        let translation_invalidated =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(
            child_stage_status(&translation_invalidated, "expert_qa"),
            STATUS_BLOCKED
        );
        assert_eq!(
            child_stage_status(&translation_invalidated, "approve_promotion"),
            STATUS_PENDING
        );
        assert!(!translation_invalidated
            .approval_references
            .iter()
            .any(|approval| approval.approval_id == first_approval));

        satisfy_qa_handoff(&translation_invalidated);
        let rereadied = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(
            child_stage_status(&rereadied, "approve_promotion"),
            STATUS_READY
        );
        let second_approval = approve_ready_promotion_for_test(&store, &job_id);
        let control_path = child_project_root(&rereadied)
            .join("qa")
            .join("chapter_controls")
            .join("chapter_001.json");
        let mut control: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&control_path).unwrap()).unwrap();
        control["externalNote"] = serde_json::json!("changed after approval");
        fs::write(
            &control_path,
            serde_json::to_string_pretty(&control).unwrap() + "\n",
        )
        .unwrap();

        let control_invalidated =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(
            child_stage_status(&control_invalidated, "approve_promotion"),
            STATUS_READY
        );
        assert!(!control_invalidated
            .approval_references
            .iter()
            .any(|approval| approval.approval_id == second_approval));
        assert!(control_invalidated.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_promotion")
            .unwrap()
            .approval_id
            .is_none());
        let _ = fs::remove_dir_all(root);
    }

    /// Drive the fake pipeline to a completed reading build so there is a real
    /// EPUB artifact, with a real digest, to record evidence against.
    fn completed_reading_job(
        store: &BookPipelineStore,
        repo: &Path,
        executor: &ReadingPipelineFixtureExecutor,
    ) -> (String, BookPipelineJob) {
        let (job_id, waiting) = fake_job_waiting_for_expert_qa(store, repo, executor);
        satisfy_qa_handoff(&waiting);
        advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
        let ready = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        approve_job_gate(
            store,
            &job_id,
            Some(&ready.children[0].id),
            "approve_promotion",
            true,
        )
        .unwrap();
        // promote, build_reading, validate_reading
        for _ in 0..3 {
            advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
        }
        let completed = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        (job_id, completed)
    }

    // Story 18's second half — "and a real reader" — had nowhere to land, so the
    // only place to note it was qa/status.md, which validate_reading rewrites.
    #[test]
    fn reader_evidence_survives_revalidation_and_reaches_qa_status() {
        let root = temp_root("reader-evidence-persists");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::passing();
        let (job_id, completed) = completed_reading_job(&store, &repo, &executor);
        assert_eq!(
            child_stage_status(&completed, "validate_reading"),
            STATUS_COMPLETED
        );
        let project_root = child_project_root(&completed);
        assert!(fs::read_to_string(project_root.join("qa/status.md"))
            .unwrap()
            .contains("- reader verification: not recorded"));

        let recorded = record_reader_evidence(
            &store,
            &job_id,
            Some(&completed.children[0].id),
            "reading_epub",
            "Apple Books",
            "7.2",
            "passed",
        )
        .unwrap();

        let evidence = &recorded.children[0].reader_evidence;
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].reader, "Apple Books");
        assert_eq!(evidence[0].conclusion, "passed");
        assert!(!evidence[0].stale);
        let epub_sha256 = recorded.children[0]
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "reading_epub")
            .and_then(|artifact| artifact.sha256.clone())
            .unwrap();
        assert_eq!(evidence[0].artifact_sha256, epub_sha256);

        // The record carries the artifact's identity, never its location.
        let payload = serde_json::to_string(&evidence[0]).unwrap();
        assert!(!payload.contains(&display_path(&project_root)), "{payload}");
        assert!(!payload.contains('/'), "{payload}");

        // Re-running validate_reading must not quietly erase it, and the report
        // it regenerates now says the same thing the job state does.
        run_validate_reading_stage(&recorded, &recorded.children[0], &executor).unwrap();
        let revalidated = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();

        assert_eq!(revalidated.children[0].reader_evidence, *evidence);
        let qa_status = fs::read_to_string(project_root.join("qa/status.md")).unwrap();
        assert!(
            qa_status.contains("- reader verification: Apple Books 7.2 on reading_epub — passed"),
            "{qa_status}"
        );
        assert!(qa_status.contains(&epub_sha256), "{qa_status}");

        let _ = fs::remove_dir_all(root);
    }

    // One reading session must not vouch for every later build of the book.
    #[test]
    fn rebuilding_the_epub_makes_reader_evidence_stale() {
        let root = temp_root("reader-evidence-stale");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::passing();
        let (job_id, completed) = completed_reading_job(&store, &repo, &executor);
        let child_id = completed.children[0].id.clone();
        let recorded = record_reader_evidence(
            &store,
            &job_id,
            Some(&child_id),
            "reading_epub",
            "Calibre",
            "8.4",
            "passed",
        )
        .unwrap();
        assert!(!recorded.children[0].reader_evidence[0].stale);

        // Rebuild the EPUB: same artifact, different bytes. The evidence still
        // describes what someone read, but no longer describes what is built.
        let mut state = store.load().unwrap();
        let job_index = find_job_index(&state, &job_id).unwrap();
        let artifact = state.jobs[job_index].children[0]
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.kind == "reading_epub")
            .unwrap();
        let epub_path = PathBuf::from(&artifact.path);
        fs::write(&epub_path, "rebuilt epub bytes").unwrap();
        let rebuilt_sha256 = sha256_file(&epub_path).unwrap();
        artifact.artifact_id = format!(
            "artifact-{}",
            sha256_str(&format!(
                "{}\0{}\0{}",
                artifact.kind, artifact.path, rebuilt_sha256
            ))
        );
        artifact.sha256 = Some(rebuilt_sha256);
        artifact.size_bytes = Some(fs::metadata(&epub_path).unwrap().len());
        derive_job(&mut state.jobs[job_index]);
        let rebuilt = state.jobs[job_index].clone();
        store.save(&state).unwrap();

        assert_eq!(rebuilt.children[0].reader_evidence.len(), 1);
        assert!(
            rebuilt.children[0].reader_evidence[0].stale,
            "evidence must not survive the artifact it was taken against"
        );
        assert_eq!(rebuilt.children[0].reader_evidence[0].reader, "Calibre");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reader_evidence_is_optional_and_validated() {
        let root = temp_root("reader-evidence-optional");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::passing();
        let (job_id, completed) = completed_reading_job(&store, &repo, &executor);
        let child_id = completed.children[0].id.clone();

        // Promotion already happened, and validation already completed, with no
        // reader evidence anywhere: recording it is not a precondition of either.
        assert_eq!(completed.status, STATUS_COMPLETED);
        assert!(completed.children[0].reader_evidence.is_empty());
        assert_eq!(
            child_stage_status(&completed, "approve_promotion"),
            STATUS_COMPLETED
        );

        for (kind, reader, version, conclusion) in [
            ("reading_markdown", "Apple Books", "7.2", "passed"),
            ("reading_epub", "", "7.2", "passed"),
            ("reading_epub", "Apple Books", "7.2", "looked fine to me"),
        ] {
            assert!(
                record_reader_evidence(
                    &store,
                    &job_id,
                    Some(&child_id),
                    kind,
                    reader,
                    version,
                    conclusion,
                )
                .is_err(),
                "{kind}/{reader}/{conclusion} should be rejected"
            );
        }

        // Re-reading the same book in the same app supersedes rather than piles up.
        for conclusion in ["passed", "failed"] {
            record_reader_evidence(
                &store,
                &job_id,
                Some(&child_id),
                "reading_epub",
                "Thorium",
                "3.1",
                conclusion,
            )
            .unwrap();
        }
        let stored = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        assert_eq!(stored.children[0].reader_evidence.len(), 1);
        assert_eq!(stored.children[0].reader_evidence[0].conclusion, "failed");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_pipeline_promotes_builds_validates_and_completes() {
        let root = temp_root("fake-reading-complete");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::passing();
        let (job_id, waiting) = fake_job_waiting_for_expert_qa(&store, &repo, &executor);
        let project_root = child_project_root(&waiting);
        fs::write(
            project_root.join("chapters/translated/unapproved.md"),
            "# Unapproved\n",
        )
        .unwrap();
        satisfy_qa_handoff(&waiting);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let ready = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        let approved = approve_job_gate(
            &store,
            &job_id,
            Some(&ready.children[0].id),
            "approve_promotion",
            true,
        )
        .unwrap();
        assert_eq!(
            child_stage_status(&approved, "approve_promotion"),
            STATUS_COMPLETED
        );

        let promoted = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(
            child_stage_status(&promoted, "approve_promotion"),
            STATUS_COMPLETED
        );
        assert_eq!(child_stage_status(&promoted, "promote"), STATUS_COMPLETED);
        assert!(!project_root.join("chapters/final/unapproved.md").exists());
        assert_eq!(
            promoted.children[0]
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == "chapter_final")
                .count(),
            1
        );
        assert!(promoted.children[0].artifacts.iter().any(|artifact| {
            artifact.kind == "promotion_manifest"
                && artifact.producer_stage.as_deref() == Some("promote")
        }));

        let built = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(
            child_stage_status(&built, "build_reading"),
            STATUS_COMPLETED
        );
        let completed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(completed.output_formats, default_output_formats());
        assert!(!project_root.join("output/book_bilingual.epub").exists());
        assert_eq!(
            child_stage_status(&completed, "validate_reading"),
            STATUS_COMPLETED
        );
        assert_eq!(
            child_stage_status(&completed, "build_digest"),
            STATUS_SKIPPED
        );
        for (kind, stage) in [
            ("reading_markdown", "build_reading"),
            ("reading_html", "build_reading"),
            ("reading_epub", "build_reading"),
            ("epubcheck_report", "validate_reading"),
        ] {
            assert!(completed.children[0].artifacts.iter().any(|artifact| {
                artifact.kind == kind && artifact.producer_stage.as_deref() == Some(stage)
            }));
        }
        assert!(!completed.children[0]
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "reading_bilingual_epub"));
        let report: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(project_root.join("output/epubcheck.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["checker"]["nFatal"], 0);
        assert_eq!(report["checker"]["nError"], 0);
        let qa_status = fs::read_to_string(project_root.join("qa/status.md")).unwrap();
        assert!(qa_status.contains("- reading output: passed"));
        assert!(qa_status.contains(
            "- accepted residual risks: 1 EPUBCheck warning(s), accepted for local reading output"
        ));
        assert_eq!(
            executor.command_labels(),
            vec![
                TRANSLATION_ENGINE_COMMAND_LABEL.to_string(),
                READING_BUILD_COMMAND_LABEL.to_string(),
                EPUBCHECK_COMMAND_LABEL.to_string(),
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_pipeline_builds_and_validates_bilingual_epub_when_selected() {
        let root = temp_root("fake-bilingual-complete");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::passing();
        let mut output_formats = default_output_formats();
        output_formats.push(OUTPUT_FORMAT_BILINGUAL.into());
        let job_id = fake_handoff_ready_job_with_output_formats(
            &store,
            &repo,
            false,
            false,
            output_formats.clone(),
        );
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(child_stage_status(&waiting, "expert_qa"), STATUS_BLOCKED);
        let project_root = child_project_root(&waiting);
        satisfy_qa_handoff(&waiting);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        approve_ready_promotion_for_test(&store, &job_id);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        let built = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(
            child_stage_status(&built, "build_reading"),
            STATUS_COMPLETED
        );
        assert!(project_root.join("output/book_bilingual.epub").is_file());
        assert!(built.children[0].artifacts.iter().any(|artifact| {
            artifact.kind == "reading_bilingual_epub"
                && artifact.producer_stage.as_deref() == Some("build_reading")
                && artifact.sha256.is_some()
        }));

        let completed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(completed.output_formats, output_formats);
        assert!(completed.children[0].artifacts.iter().any(|artifact| {
            artifact.kind == "bilingual_epubcheck_report"
                && artifact.producer_stage.as_deref() == Some("validate_reading")
                && artifact.sha256.is_some()
        }));
        assert!(project_root
            .join("output/epubcheck_bilingual.json")
            .is_file());
        let qa_status = fs::read_to_string(project_root.join("qa/status.md")).unwrap();
        assert!(qa_status.contains("- reading output: passed"));
        assert!(qa_status.contains(
            "- accepted residual risks: 2 EPUBCheck warning(s), accepted for local reading output"
        ));
        assert!(completed
            .log_summary
            .iter()
            .any(|line| line.contains("alignment=paragraph")));
        assert_eq!(
            executor.command_labels(),
            vec![
                TRANSLATION_ENGINE_COMMAND_LABEL.to_string(),
                READING_BUILD_COMMAND_LABEL.to_string(),
                BILINGUAL_BUILD_COMMAND_LABEL.to_string(),
                EPUBCHECK_COMMAND_LABEL.to_string(),
                EPUBCHECK_COMMAND_LABEL.to_string(),
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_bilingual_pipeline_logs_whole_chapter_fallback_for_mismatched_paragraphs() {
        let root = temp_root("fake-bilingual-fallback");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::passing_with_bilingual_fallback();
        let mut output_formats = default_output_formats();
        output_formats.push(OUTPUT_FORMAT_BILINGUAL.into());
        let job_id =
            fake_handoff_ready_job_with_output_formats(&store, &repo, false, false, output_formats);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        satisfy_qa_handoff(&waiting);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        approve_ready_promotion_for_test(&store, &job_id);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        let built = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(
            child_stage_status(&built, "build_reading"),
            STATUS_COMPLETED
        );
        assert!(built
            .log_summary
            .iter()
            .any(|line| line.contains("alignment=chapter-fallback")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_pipeline_builds_digest_when_book_intent_is_enabled() {
        let root = temp_root("fake-digest-complete");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::passing_with_digest();
        let (job_id, waiting) =
            fake_job_waiting_for_expert_qa_with_digest(&store, &repo, &executor, true);
        let project_root = child_project_root(&waiting);
        fs::write(
            project_root.join("digest.config.json"),
            "{\n  \"max_section_chars\": 2400\n}\n",
        )
        .unwrap();
        fs::write(
            project_root.join("metadata/book.yaml"),
            "title: Digest Fixture Title\nlanguage: zh-CN\n",
        )
        .unwrap();
        satisfy_qa_handoff(&waiting);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        approve_ready_promotion_for_test(&store, &job_id);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let validated = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(
            child_stage_status(&validated, "validate_reading"),
            STATUS_COMPLETED
        );
        assert_eq!(
            child_stage_status(&validated, "build_digest"),
            STATUS_PENDING
        );
        let completed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(
            child_stage_status(&completed, "build_digest"),
            STATUS_COMPLETED
        );
        for kind in [
            "digest_epub",
            "digest_xhtml",
            "digest_knowledge_map",
            "digest_review_checklist",
            "digest_report",
            "digest_epubcheck_report",
        ] {
            assert!(completed.children[0].artifacts.iter().any(|artifact| {
                artifact.kind == kind
                    && artifact.producer_stage.as_deref() == Some("build_digest")
                    && artifact.sha256.is_some()
            }));
        }
        let digest_stage = completed.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "build_digest")
            .unwrap();
        for key in [
            "readingEpubSha256",
            "epubcheckReportSha256",
            "sourceManifestSha256",
            "bookMetadataSha256",
            "digestConfigSha256",
        ] {
            assert!(
                digest_stage.input_hashes.contains_key(key),
                "missing key {key}"
            );
        }
        let report: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(project_root.join("qa/digest/digest_report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["status"], "PASS");
        assert_eq!(report["merged"], true);
        assert_eq!(report["source_epub"], "output/book.epub");
        assert_eq!(report["output_epub"], "output/book_digest.epub");
        let epubcheck_report: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(project_root.join("output/digest_epubcheck.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(epubcheck_report["checker"]["nFatal"], 0);
        assert_eq!(epubcheck_report["checker"]["nError"], 0);
        let config: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(project_root.join("digest.config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["max_section_chars"], 2400);
        assert_eq!(
            executor.command_labels(),
            vec![
                TRANSLATION_ENGINE_COMMAND_LABEL.to_string(),
                READING_BUILD_COMMAND_LABEL.to_string(),
                EPUBCHECK_COMMAND_LABEL.to_string(),
                DIGEST_BUILD_COMMAND_LABEL.to_string(),
                EPUBCHECK_COMMAND_LABEL.to_string(),
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fake_expert_pipeline_builds_digest_when_book_intent_is_enabled() {
        let root = temp_root("fake-expert-digest-complete");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::passing_with_digest();
        let job_id = fake_handoff_ready_job_with_digest(&store, &repo);
        let handed_off = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        fs::write(
            child_project_root(&handed_off).join("metadata/book.yaml"),
            "title: Digest Fixture Title\nlanguage: zh-CN\n",
        )
        .unwrap();
        configure_expert_job(&store, &job_id);

        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        approve_ready_translation_for_test(&store, &job_id);
        let translation_waiting =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        satisfy_translation_handoff(&translation_waiting);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let qa_waiting =
            advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        satisfy_qa_handoff(&qa_waiting);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        approve_ready_promotion_for_test(&store, &job_id);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let completed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(completed.translation_mode, TRANSLATION_MODE_EXPERT);
        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(
            child_stage_status(&completed, "build_digest"),
            STATUS_COMPLETED
        );
        assert!(completed.children[0].artifacts.iter().any(|artifact| {
            artifact.kind == "digest_epub"
                && artifact.producer_stage.as_deref() == Some("build_digest")
                && artifact.sha256.is_some()
        }));
        assert_eq!(
            executor.command_labels(),
            vec![
                READING_BUILD_COMMAND_LABEL.to_string(),
                EPUBCHECK_COMMAND_LABEL.to_string(),
                DIGEST_BUILD_COMMAND_LABEL.to_string(),
                EPUBCHECK_COMMAND_LABEL.to_string(),
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn digest_epubcheck_failure_marks_build_failed_and_can_retry() {
        let root = temp_root("fake-digest-epubcheck-failed");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::failing_digest_epubcheck();
        let (job_id, waiting) =
            fake_job_waiting_for_expert_qa_with_digest(&store, &repo, &executor, true);
        let project_root = child_project_root(&waiting);
        fs::write(
            project_root.join("metadata/book.yaml"),
            "title: Digest Fixture Title\nlanguage: zh-CN\n",
        )
        .unwrap();
        satisfy_qa_handoff(&waiting);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        approve_ready_promotion_for_test(&store, &job_id);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(failed.status, STATUS_FAILED);
        assert_eq!(failed.current_stage_id, "build_digest");
        assert_eq!(failed.current_step, "build_digest stage failed");
        assert_eq!(child_stage_status(&failed, "build_digest"), STATUS_FAILED);
        assert!(failed.last_error.as_deref().is_some_and(
            |error| error.contains("EPUBCheck reported 0 fatal finding(s) and 1 error(s)")
        ));
        assert!(failed.children[0].artifacts.iter().any(|artifact| {
            artifact.kind == "digest_epubcheck_report"
                && artifact.producer_stage.as_deref() == Some("build_digest")
        }));
        let failed_report: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(project_root.join("output/digest_epubcheck.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(failed_report["checker"]["nError"], 1);

        let retry_executor = ReadingPipelineFixtureExecutor::passing_with_digest();
        let completed =
            advance_job_with_executor(&store, &job_id, None, false, &retry_executor).unwrap();

        assert_eq!(completed.status, STATUS_COMPLETED);
        assert_eq!(
            child_stage_status(&completed, "build_digest"),
            STATUS_COMPLETED
        );
        // Three automatic attempts exhausted the stage's budget before the
        // operator's own retry, which is the fourth.
        assert_eq!(
            completed.children[0]
                .stages
                .iter()
                .find(|stage| stage.stage_id == "build_digest")
                .unwrap()
                .attempt,
            DEFAULT_STAGE_MAX_ATTEMPTS + 1
        );
        let passed_report: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(project_root.join("output/digest_epubcheck.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(passed_report["checker"]["nError"], 0);
        assert_eq!(
            retry_executor.command_labels(),
            vec![
                DIGEST_BUILD_COMMAND_LABEL.to_string(),
                EPUBCHECK_COMMAND_LABEL.to_string(),
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn epubcheck_failure_stops_at_validation_failed() {
        let root = temp_root("fake-reading-validation-failed");
        let repo = handoff_repo_fixture(&root);
        let store = BookPipelineStore::for_test(&root);
        let executor = ReadingPipelineFixtureExecutor::failing_epubcheck();
        let (job_id, waiting) = fake_job_waiting_for_expert_qa(&store, &repo, &executor);
        satisfy_qa_handoff(&waiting);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        approve_ready_promotion_for_test(&store, &job_id);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(failed.status, STATUS_FAILED);
        assert_eq!(failed.current_stage_id, "validate_reading");
        assert_eq!(failed.current_step, "validation_failed");
        assert_eq!(
            child_stage_status(&failed, "validate_reading"),
            STATUS_FAILED
        );
        assert_ne!(failed.status, STATUS_COMPLETED);
        assert!(failed.children[0]
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "epubcheck_report"));
        let qa_status =
            fs::read_to_string(child_project_root(&failed).join("qa/status.md")).unwrap();
        assert!(qa_status.contains("- reading output: failed"));
        assert!(qa_status.contains("fatal=0, error=1, warning=0"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn real_markdown_job_stops_at_promotion_gate() {
        let root = temp_root("real-promotion-gate");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# Real chapter\n\nBody paragraph.\n",
        );
        let executor = TranslationEngineFixtureExecutor::succeeding();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        approve_ready_translation_for_test(&store, &job_id);
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        satisfy_qa_handoff(&waiting);

        let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
        let stopped = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

        assert_eq!(
            child_stage_status(&ready, "approve_promotion"),
            STATUS_READY
        );
        assert_eq!(
            child_stage_status(&stopped, "approve_promotion"),
            STATUS_READY
        );
        assert_eq!(child_stage_status(&stopped, "promote"), STATUS_PENDING);
        assert_eq!(stopped.current_stage_id, "approve_promotion");
        assert!(!stopped.approval_references.iter().any(|approval| {
            approval.child_job_id == stopped.children[0].id
                && approval.stage_id == "approve_promotion"
        }));
        assert!(
            fs::read_dir(child_project_root(&stopped).join("chapters/final"))
                .unwrap()
                .next()
                .is_none()
        );
        let _ = fs::remove_dir_all(root);
    }

    fn stage_attempt(job: &BookPipelineJob, stage_id: &str) -> u32 {
        job.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == stage_id)
            .unwrap()
            .attempt
    }

    fn artifact_sha(job: &BookPipelineJob, kind: &str) -> Option<String> {
        job.children[0]
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == kind)
            .unwrap()
            .sha256
            .clone()
    }

    fn chapter_source_count(job: &BookPipelineJob) -> usize {
        job.children[0]
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "chapter_source")
            .count()
    }

    fn child_source_md(job: &BookPipelineJob) -> PathBuf {
        child_project_root(job).join("source").join("source.md")
    }

    #[test]
    fn advance_reuses_completed_stages_without_rerunning() {
        let root = temp_root("advance-idempotent");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
        );

        advance_job(&store, &job_id, None, false).unwrap();
        let prepared = advance_job(&store, &job_id, None, false).unwrap();
        let split_attempt = stage_attempt(&prepared, "split");
        let prepare_attempt = stage_attempt(&prepared, "prepare");
        let source_map_sha = artifact_sha(&prepared, "source_map");

        let again = advance_job(&store, &job_id, None, false).unwrap();

        assert_eq!(child_stage_status(&again, "prepare"), STATUS_COMPLETED);
        assert_eq!(
            child_stage_status(&again, "approve_translation"),
            STATUS_READY
        );
        assert_eq!(stage_attempt(&again, "split"), split_attempt);
        assert_eq!(stage_attempt(&again, "prepare"), prepare_attempt);
        assert_eq!(artifact_sha(&again, "source_map"), source_map_sha);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_change_before_prepare_reruns_split_without_blocking() {
        let root = temp_root("advance-resplit");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id =
            handoff_ready_child_job(&store, &repo, &source_path, "# Only\n\nOriginal body.\n");

        let split_once = advance_job(&store, &job_id, None, false).unwrap();
        assert_eq!(chapter_source_count(&split_once), 1);
        let first_map = artifact_sha(&split_once, "source_map");

        fs::write(
            child_source_md(&split_once),
            "# First\n\nNew body.\n\n# Second\n\nMore body.\n",
        )
        .unwrap();
        let resplit = advance_job(&store, &job_id, None, false).unwrap();

        assert_eq!(child_stage_status(&resplit, "split"), STATUS_COMPLETED);
        assert_eq!(stage_attempt(&resplit, "split"), 2);
        assert_eq!(chapter_source_count(&resplit), 2);
        assert_ne!(artifact_sha(&resplit, "source_map"), first_map);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_change_after_prepare_blocks_split_pending_invalidation() {
        let root = temp_root("advance-block");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
        );

        advance_job(&store, &job_id, None, false).unwrap();
        let prepared = advance_job(&store, &job_id, None, false).unwrap();
        assert_eq!(child_stage_status(&prepared, "prepare"), STATUS_COMPLETED);

        fs::write(
            child_source_md(&prepared),
            "# Rewritten\n\nDifferent body.\n",
        )
        .unwrap();
        let blocked = advance_job(&store, &job_id, None, false).unwrap();

        assert_eq!(child_stage_status(&blocked, "split"), STATUS_BLOCKED);
        // Committed downstream work is rolled back to pending, never silently lost.
        assert_eq!(child_stage_status(&blocked, "prepare"), STATUS_PENDING);
        assert_eq!(
            child_stage_status(&blocked, "approve_translation"),
            STATUS_PENDING
        );
        assert!(blocked.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap()
            .approval_request
            .is_none());
        assert_eq!(blocked.children[0].status, STATUS_BLOCKED);
        assert_eq!(
            blocked.children[0].last_error.as_deref(),
            Some("source_changed_downstream_exists")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_invalidation_reruns_split_and_prepare_from_new_source() {
        let root = temp_root("advance-invalidate");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
        );

        advance_job(&store, &job_id, None, false).unwrap();
        let prepared = advance_job(&store, &job_id, None, false).unwrap();
        fs::write(
            child_source_md(&prepared),
            "# Merged\n\nSingle chapter now.\n",
        )
        .unwrap();
        let blocked = advance_job(&store, &job_id, None, false).unwrap();
        assert_eq!(child_stage_status(&blocked, "split"), STATUS_BLOCKED);

        let unblocked = advance_job(&store, &job_id, None, true).unwrap();
        assert_eq!(child_stage_status(&unblocked, "split"), STATUS_COMPLETED);
        assert_eq!(chapter_source_count(&unblocked), 1);
        assert_eq!(child_stage_status(&unblocked, "prepare"), STATUS_PENDING);

        let reprepared = advance_job(&store, &job_id, None, false).unwrap();
        assert_eq!(child_stage_status(&reprepared, "prepare"), STATUS_COMPLETED);
        assert_eq!(
            reprepared.children[0]
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == "translation_task_manifest")
                .count(),
            1
        );
        assert!(!child_project_root(&reprepared)
            .join("chapters")
            .join("src")
            .join("chapter_002.md")
            .exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn advanced_split_and_prepare_survive_store_restart() {
        let root = temp_root("advance-restart");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let job_id = {
            let store = BookPipelineStore::for_test(&root);
            let job_id = handoff_ready_child_job(
                &store,
                &repo,
                &source_path,
                "# Alpha\n\nAlpha body.\n\n# Beta\n\nBeta body.\n",
            );
            advance_job(&store, &job_id, None, false).unwrap();
            advance_job(&store, &job_id, None, false).unwrap();
            job_id
        };

        let reopened = BookPipelineStore::for_test(&root);
        let state = reopened.load().unwrap();
        let job = state.jobs.iter().find(|job| job.id == job_id).unwrap();
        let child = &job.children[0];
        assert_eq!(
            child
                .stages
                .iter()
                .find(|stage| stage.stage_id == "split")
                .unwrap()
                .status,
            STATUS_COMPLETED
        );
        assert_eq!(
            child
                .stages
                .iter()
                .find(|stage| stage.stage_id == "prepare")
                .unwrap()
                .status,
            STATUS_COMPLETED
        );
        assert!(child
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "source_map" && artifact.sha256.is_some()));
        assert_eq!(
            child
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == "translation_task_manifest")
                .count(),
            2
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn split_and_prepare_keep_private_text_out_of_job_records() {
        let root = temp_root("advance-privacy");
        let repo = handoff_repo_fixture(&root);
        let source_path = root.join("source.md");
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# Distinctivechaptertitle\n\nConfidentialbodytext lives here.\n",
        );

        advance_job(&store, &job_id, None, false).unwrap();
        let advanced = advance_job(&store, &job_id, None, false).unwrap();

        let log = advanced.log_summary.join("\n");
        assert!(!log.contains("Distinctivechaptertitle"));
        assert!(!log.contains("Confidentialbodytext"));
        for stage in &advanced.children[0].stages {
            if let Some(error) = &stage.error {
                assert!(!error.contains("Confidentialbodytext"));
            }
        }
        for artifact in &advanced.children[0].artifacts {
            assert!(!artifact.path.contains("Confidentialbodytext"));
            assert!(!artifact.path.contains("Distinctivechaptertitle"));
        }

        // Traceability metadata still lives in the local (gitignored) source map.
        let source_map = fs::read_to_string(
            child_project_root(&advanced)
                .join("metadata")
                .join("source_map.json"),
        )
        .unwrap();
        assert!(source_map.contains("Distinctivechaptertitle"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn persisted_file_artifacts_have_complete_immutable_provenance() {
        let root = temp_root("artifact-provenance");
        let input = root.join("input");
        fs::create_dir_all(&input).unwrap();
        fs::write(input.join("sample.pdf"), "%PDF fixture").unwrap();
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            local_pdf_source(&input),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();

        let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
        let markdown = completed
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .expect("registered Markdown artifact");
        let markdown_path = PathBuf::from(&markdown.path);
        assert!(markdown.artifact_id.starts_with("artifact-"));
        assert_eq!(
            markdown.sha256.as_deref(),
            Some(sha256_file(&markdown_path).unwrap().as_str())
        );
        assert_eq!(
            markdown.size_bytes,
            Some(fs::metadata(&markdown_path).unwrap().len())
        );
        assert_eq!(markdown.producer.stage_id, "extract");
        assert_eq!(markdown.producer.attempt, 1);
        assert_eq!(
            markdown.producer.child_job_id.as_deref(),
            Some(completed.children[0].id.as_str())
        );
        assert!(!markdown.input_hashes.is_empty());
        assert!(!markdown.source_refs.source_ref_sha256.is_empty());
        assert_eq!(markdown.privacy, "private_text");
        assert!(markdown.validation.exists);
        assert!(markdown.validation.nonempty);
        assert!(markdown.validation.hash_matches);
        assert!(completed
            .artifacts
            .iter()
            .all(|artifact| Path::new(&artifact.path).is_file()));
        assert!(completed
            .children
            .iter()
            .flat_map(|child| child.stages.iter())
            .find(|stage| stage.stage_id == "extract")
            .is_some_and(|stage| stage.artifact_ids.contains(&markdown.artifact_id)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn persisted_artifact_identity_rejects_producer_mutation() {
        let root = temp_root("artifact-immutable-producer");
        let store = BookPipelineStore::for_test(&root);
        let queued = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let completed = run_job(&store, &ArtifactFixtureRunner, &queued.id).unwrap();
        let artifact_id = completed
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown")
            .unwrap()
            .artifact_id
            .clone();
        let mut state = store.load().unwrap();
        let job = state
            .jobs
            .iter_mut()
            .find(|job| job.id == queued.id)
            .unwrap();
        for artifact in job
            .artifacts
            .iter_mut()
            .chain(
                job.children
                    .iter_mut()
                    .flat_map(|child| child.artifacts.iter_mut()),
            )
            .chain(
                job.collection_items
                    .iter_mut()
                    .flat_map(|item| item.artifacts.iter_mut()),
            )
            .filter(|artifact| artifact.artifact_id == artifact_id)
        {
            artifact.producer.attempt += 1;
        }

        let error = store.save(&state).unwrap_err();

        assert!(error.contains("was mutated"));
        assert!(error.contains("producer"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn worker_output_persists_only_allowlisted_markers() {
        let root = temp_root("worker-marker-allowlist");
        let output = root.join("job-output");
        fs::create_dir_all(&output).unwrap();
        let hash = "a".repeat(64);
        let text = format!(
            "private source paragraph\nBOOK_PIPELINE_MARKER status=completed count=2 sha256={hash} path={}\nprompt=translate this secret\nhttps://private.example/file?X-Amz-Signature=secret",
            display_path(&output.join("result.md"))
        );

        let markers = parse_allowlisted_worker_markers(&text, &[output.as_path()]);

        assert_eq!(markers.len(), 1);
        assert!(markers[0].contains("status=completed"));
        assert!(markers[0].contains("count=2"));
        assert!(markers[0].contains(&hash));
        let persisted = markers.join("\n");
        assert!(!persisted.contains("private source paragraph"));
        assert!(!persisted.contains("prompt="));
        assert!(!persisted.contains("Signature"));
        assert!(!persisted.contains("secret"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn structured_errors_classify_and_redact_sensitive_payloads() {
        let error = safe_error_from_message(
            "extract",
            None,
            3,
            "ZOTERO_API_KEY=supersecret Authorization: Bearer nope https://private.example/file?X-Amz-Signature=secret prompt=private source text",
        );

        assert_eq!(error.code, "missing_credentials");
        assert_eq!(error.stage_id, "extract");
        assert_eq!(error.attempt, 3);
        assert!(error.retryable);
        assert!(!error.summary.contains("supersecret"));
        assert!(!error.summary.contains("private.example"));
        assert!(!error.summary.contains("prompt"));
        assert!(!error.summary.contains("source text"));
    }

    #[test]
    fn a_missing_key_message_names_no_secret_and_stays_legible() {
        let error = safe_error_from_message(
            "index",
            None,
            1,
            "Zotero item-scoped full-text index exited with status 1: RuntimeError: GOOGLE_API_KEY or GEMINI_API_KEY not set.",
        );

        assert_eq!(error.code, "missing_credentials");
        assert_eq!(
            error.summary,
            "Zotero item-scoped full-text index exited with status 1: RuntimeError: GOOGLE_API_KEY or GEMINI_API_KEY not set."
        );
    }

    #[test]
    fn diagnostic_profiles_have_monotonic_disclosure() {
        let root = temp_root("diagnostic-profiles");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();

        let local = serde_json::to_string(
            &build_book_pipeline_diagnostic(&completed, "local-full").unwrap(),
        )
        .unwrap();
        let support = serde_json::to_string(
            &build_book_pipeline_diagnostic(&completed, "redacted-support").unwrap(),
        )
        .unwrap();
        let public = serde_json::to_string(
            &build_book_pipeline_diagnostic(&completed, "public-issue").unwrap(),
        )
        .unwrap();

        assert!(local.contains(&completed.artifacts[0].path));
        assert!(local.contains(completed.artifacts[0].sha256.as_deref().unwrap()));
        assert!(!support.contains(&display_path(&root)));
        assert!(support.contains("<JOB_ROOT>"));
        assert!(support.contains(completed.artifacts[0].sha256.as_deref().unwrap()));
        assert!(!public.contains(&display_path(&root)));
        assert!(!public.contains(completed.artifacts[0].sha256.as_deref().unwrap()));
        assert!(!public.contains("artifactId"));
        for export in [&local, &support, &public] {
            assert!(!export.contains("stdout"));
            assert!(!export.contains("stderr"));
            assert!(!export.contains("providerPayload"));
            assert!(!export.contains("prompt"));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn diagnostic_bundle_lands_in_the_chosen_folder_under_a_contained_name() {
        // The disclosure test above covers what goes in; this covers the part
        // that makes it reachable at all -- the bundle has to be a file the user
        // can attach to a report, not a value returned in-process.
        let root = temp_root("diagnostic-write");
        let store = BookPipelineStore::for_test(&root);
        let job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
        let out = root.join("export");
        fs::create_dir_all(&out).unwrap();

        let document = build_book_pipeline_diagnostic(&completed, "public-issue").unwrap();
        let path =
            write_book_pipeline_diagnostic(&out, &completed.id, "public-issue", &document).unwrap();
        assert_eq!(path.parent(), Some(out.as_path()));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(format!("bibliosmith-diagnostic-{}-public-issue.json", completed.id).as_str())
        );
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["profile"], "public-issue");
        assert_eq!(written, document);

        // A job id that is not filename-safe must not steer the write out of the
        // folder the user picked.
        let escaped =
            write_book_pipeline_diagnostic(&out, "../../etc/pwned", "public-issue", &document)
                .unwrap();
        assert_eq!(escaped.parent(), Some(out.as_path()));
        assert_eq!(
            escaped.file_name().and_then(|name| name.to_str()),
            Some("bibliosmith-diagnostic-______etc_pwned-public-issue.json")
        );

        assert!(build_book_pipeline_diagnostic(&completed, "everything").is_err());
        let _ = fs::remove_dir_all(root);
    }

    fn open_target_fixture(
        root: &Path,
        status: &str,
        kind: &str,
        action_label: &str,
    ) -> BookPipelineJob {
        let store = BookPipelineStore::for_test(root);
        let mut job = queue_job(
            &store,
            fake_source(None),
            "conversion_only".into(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap();
        let target_root = root.join("registered-job-root");
        fs::create_dir_all(&target_root).unwrap();
        let target_path = if kind.ends_with("_directory") || kind == "workspace" {
            target_root.join(kind)
        } else {
            target_root.join(format!("{kind}.json"))
        };
        if target_path.extension().is_some() {
            fs::write(&target_path, "fixture").unwrap();
        } else {
            fs::create_dir_all(&target_path).unwrap();
        }
        job.status = status.into();
        job.navigation_targets = vec![BookPipelineNavigationTarget {
            target_id: format!("target-{kind}"),
            kind: kind.into(),
            path: display_path(&target_path),
            allowed_root: display_path(&target_root),
            artifact_id: None,
        }];
        job.open_target = select_book_pipeline_open_target(&job);
        assert_eq!(
            job.open_target
                .as_ref()
                .map(|target| target.action_label.as_str()),
            Some(action_label)
        );
        job
    }

    #[test]
    fn every_job_status_resolves_a_deterministic_registered_open_target() {
        let cases = [
            (STATUS_PENDING, "workspace", "Open workspace"),
            (STATUS_READY, "workspace", "Open workspace"),
            (STATUS_RUNNING, "workspace", "Open workspace"),
            (
                STATUS_WAITING_FOR_APPROVAL,
                "approval_packet",
                "Review approval",
            ),
            (STATUS_BLOCKED, "blocker_evidence", "Review blocker"),
            (STATUS_FAILED, "failure_evidence", "Open failure evidence"),
            (STATUS_PARTIAL, "partial_results", "Inspect partial results"),
            (
                STATUS_COMPLETED,
                "reading_output_directory",
                "Open reading output",
            ),
            (
                STATUS_SKIPPED,
                "verified_evidence",
                "Open verified evidence",
            ),
        ];
        for (index, (status, kind, action_label)) in cases.into_iter().enumerate() {
            let root = temp_root(&format!("open-status-{index}"));
            let job = open_target_fixture(&root, status, kind, action_label);
            let allowed = root.join("registered-job-root");

            let resolved = resolve_book_pipeline_open_target(&job, &[allowed]).unwrap();

            assert_eq!(resolved.kind, kind);
            assert_eq!(resolved.action_label, action_label);
            assert!(resolved.path.exists());
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn completed_collection_uses_collection_results_action() {
        let root = temp_root("open-collection-results");
        let mut job = open_target_fixture(&root, STATUS_PENDING, "workspace", "Open workspace");
        job.kind = "collection".into();
        job.status = STATUS_COMPLETED.into();
        job.navigation_targets[0].kind = "collection_results".into();
        job.open_target = select_book_pipeline_open_target(&job);

        assert_eq!(
            job.open_target
                .as_ref()
                .map(|target| target.action_label.as_str()),
            Some("Open collection results")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn skipped_collection_opens_its_hashed_manifest_as_verified_evidence() {
        let root = temp_root("open-skipped-collection");
        let store = BookPipelineStore::for_test(&root);
        let mut job = queue_job(
            &store,
            fake_collection_source(),
            "conversion_only".into(),
            BookPipelinePreviewConfig {
                has_paddleocr_credentials: true,
                has_mineru_credentials: true,
                route_overrides: BTreeMap::new(),
            },
        )
        .unwrap();
        for stage in &mut job.stages {
            stage.status = STATUS_COMPLETED.into();
        }
        for child in &mut job.children {
            for stage in &mut child.stages {
                stage.status = STATUS_SKIPPED.into();
            }
        }
        let output_dir = PathBuf::from(job.output_dir.as_deref().unwrap());
        let manifest_path = output_dir.join("collection-summary.json");
        fs::write(&manifest_path, "{\"schema\":\"fixture\"}\n").unwrap();
        job.artifacts.push(
            required_stage_artifact("collection_manifest", &manifest_path, "discover").unwrap(),
        );

        derive_job(&mut job);
        let resolved = resolve_book_pipeline_open_target(&job, &[output_dir]).unwrap();

        assert_eq!(job.status, STATUS_SKIPPED);
        assert_eq!(resolved.kind, "verified_evidence");
        assert_eq!(resolved.action_label, "Open verified evidence");
        assert_eq!(resolved.path, fs::canonicalize(manifest_path).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn open_target_rejects_traversal_missing_paths_and_source_pdf_fallback() {
        let root = temp_root("open-target-guardrails");
        let allowed = root.join("registered-job-root");
        fs::create_dir_all(&allowed).unwrap();
        let escaped = root.join("escaped.txt");
        fs::write(&escaped, "outside").unwrap();
        let mut job = open_target_fixture(
            &root,
            STATUS_FAILED,
            "failure_evidence",
            "Open failure evidence",
        );
        job.navigation_targets[0].path = display_path(&allowed.join("..").join("escaped.txt"));
        assert!(
            resolve_book_pipeline_open_target(&job, std::slice::from_ref(&allowed))
                .unwrap_err()
                .contains("open_target_invalid")
        );

        job.navigation_targets[0].path = display_path(&allowed.join("missing.json"));
        assert!(
            resolve_book_pipeline_open_target(&job, std::slice::from_ref(&allowed))
                .unwrap_err()
                .contains("open_target_invalid")
        );

        let source_pdf = root.join("private-source.pdf");
        fs::write(&source_pdf, "%PDF private").unwrap();
        job.source.path = Some(display_path(&source_pdf));
        job.navigation_targets.clear();
        job.open_target = None;
        assert!(
            resolve_book_pipeline_open_target(&job, std::slice::from_ref(&allowed))
                .unwrap_err()
                .contains("open_target_invalid")
        );
        let _ = fs::remove_dir_all(root);
    }

    fn excerpt_fixture_job(artifact_path: &Path) -> BookPipelineJob {
        serde_json::from_value(serde_json::json!({
            "id": "job-excerpt",
            "mode": "convert_then_translate",
            "source": { "kind": "local_pdf_folder", "title": "Fixture" },
            "route": [],
            "status": "waiting_for_approval",
            "currentStep": "approve_translation",
            "lastError": null,
            "logSummary": [],
            "artifacts": [{
                "artifactId": "art-1",
                "kind": "extraction_markdown",
                "path": display_path(artifact_path),
                "sha256": null,
                "zoteroKey": null
            }],
            "outputDir": null,
            "attempts": 1,
            "createdAt": "2026-07-18T00:00:00Z",
            "updatedAt": "2026-07-18T00:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn artifact_excerpt_returns_truncated_head_within_allowlist() {
        let root = std::env::temp_dir().join(format!("bp-excerpt-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let artifact_path = root.join("book.md");
        let body = format!("# Title\n\n{}", "正文abc".repeat(400));
        fs::write(&artifact_path, &body).unwrap();
        let job = excerpt_fixture_job(&artifact_path);

        let excerpt =
            read_artifact_excerpt(&job, "art-1", Some(64), std::slice::from_ref(&root)).unwrap();
        assert_eq!(excerpt.artifact_id, "art-1");
        assert_eq!(excerpt.kind, "extraction_markdown");
        assert!(excerpt.truncated);
        assert_eq!(excerpt.excerpt.chars().count(), 64);
        assert!(excerpt.excerpt.starts_with("# Title"));

        let full = read_artifact_excerpt(&job, "art-1", Some(4000), std::slice::from_ref(&root));
        assert!(full.is_ok());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_excerpt_rejects_paths_outside_allowlist_and_unknown_artifacts() {
        let root = std::env::temp_dir().join(format!("bp-excerpt-out-{}", std::process::id()));
        let elsewhere =
            std::env::temp_dir().join(format!("bp-excerpt-else-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        let artifact_path = elsewhere.join("secret.md");
        fs::write(&artifact_path, "secret body").unwrap();
        let job = excerpt_fixture_job(&artifact_path);

        let outside = read_artifact_excerpt(&job, "art-1", None, std::slice::from_ref(&root));
        assert!(outside.unwrap_err().contains("artifact_excerpt_invalid"));

        let unknown = read_artifact_excerpt(&job, "missing", None, std::slice::from_ref(&root));
        assert!(unknown.unwrap_err().contains("not registered"));

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(elsewhere);
    }
}
