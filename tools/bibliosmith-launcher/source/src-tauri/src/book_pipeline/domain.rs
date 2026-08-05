//! The serde contract shared with the frontend and the state file on disk. Every struct here is camelCase-renamed and round-trips through book-pipeline-state-v5, so field names, rename_all and default attributes are a compatibility surface -- src/types.ts mirrors them. Also holds the normalisation helpers that keep those fields well-formed on the way in.

use super::*;

pub(crate) fn default_output_formats() -> Vec<String> {
    [OUTPUT_FORMAT_MD, OUTPUT_FORMAT_HTML, OUTPUT_FORMAT_EPUB]
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub(crate) fn normalize_output_formats(formats: &[String]) -> Result<Vec<String>, String> {
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

pub(crate) fn output_format_enabled(job: &BookPipelineJob, format: &str) -> bool {
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
    pub prompt_pack_reference: PromptPackReference,
    pub prompt_pack_selection_source: String,
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
    pub prompt_pack_reference: PromptPackReference,
    pub prompt_pack_selection_source: String,
    #[serde(default)]
    pub reader_evidence: Vec<BookPipelineReaderEvidence>,
    /// When this book was dropped from the shelf. A collection's membership
    /// snapshot is a frozen integrity anchor — `validate_state` requires
    /// `child_job_ids` to equal the children exactly and
    /// `validate_state_transitions` rejects any membership change — so removing
    /// one book of a batch cannot mean removing the child. It means marking it,
    /// and everything that walks the children skipping it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_at: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<BookPipelineUnitFailure>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineUnitFailure {
    pub unit_id: String,
    pub code: String,
    pub retryable: bool,
}

/// Ephemeral, aggregate-only progress reported by a running worker. It is
/// overlaid when the UI polls and is never written into durable jobs.json.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineOperationProgress {
    pub stage_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub completed: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    pub unit_kind: String,
    pub phase: String,
    pub activity_at: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<BookPipelineOperationProgress>,
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
    /// When the decision was taken. `#[serde(default)]` because the two pipeline
    /// gates have been writing this record without one since before it existed.
    #[serde(default)]
    pub decided_at: String,
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
    pub prompt_pack_reference: PromptPackReference,
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
    pub prompt_pack_reference: PromptPackReference,
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
