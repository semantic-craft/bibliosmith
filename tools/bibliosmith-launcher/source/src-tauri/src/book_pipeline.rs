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

const LIVE_PROGRESS_SCHEMA: &str = "book-pipeline-progress-v1";
const LIVE_PROGRESS_FILE: &str = ".book-pipeline-progress";
const LIVE_PROGRESS_PATH_ENV: &str = "BIBLIOSMITH_PROGRESS_PATH";

mod contract;
mod domain;
mod migrate;
mod store;

pub(crate) use contract::*;
pub use domain::*;
pub(crate) use migrate::*;
pub(crate) use store::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedBookPipelineOpenTarget {
    path: PathBuf,
    kind: String,
    action_label: String,
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
        operation: None,
        retry_attempts_remaining: active.map(stage_attempts_remaining).unwrap_or_default(),
        next_retry_at: active.and_then(|stage| stage.next_retry_at.clone()),
        give_up_reason: active.and_then(|stage| stage.give_up_reason.clone()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LiveWorkerProgress {
    schema: String,
    stage_id: String,
    #[serde(default)]
    scope_id: Option<String>,
    completed: u32,
    #[serde(default)]
    total: Option<u32>,
    unit_kind: String,
    phase: String,
    activity_at: String,
}

fn read_live_worker_progress(path: &Path) -> Option<BookPipelineOperationProgress> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > 16 * 1024 {
        return None;
    }
    let value: LiveWorkerProgress = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let valid_unit = matches!(
        value.unit_kind.as_str(),
        "pages" | "chapters" | "chunks" | "items"
    );
    let valid_phase = matches!(
        value.phase.as_str(),
        "starting"
            | "uploading"
            | "extracting"
            | "downloading"
            | "translating"
            | "reviewing"
            | "assembling"
    );
    let valid_total = value
        .total
        .is_none_or(|total| total > 0 && value.completed <= total);
    if value.schema != LIVE_PROGRESS_SCHEMA
        || ordered_stage_index(&value.stage_id).is_none()
        || !valid_unit
        || !valid_phase
        || !valid_total
        || chrono::DateTime::parse_from_rfc3339(&value.activity_at).is_err()
        || value.scope_id.as_deref().is_some_and(str::is_empty)
    {
        return None;
    }
    Some(BookPipelineOperationProgress {
        stage_id: value.stage_id,
        scope_id: value.scope_id,
        completed: value.completed,
        total: value.total,
        unit_kind: value.unit_kind,
        phase: value.phase,
        activity_at: value.activity_at,
    })
}

fn live_progress_paths(store: &dyn BookPipelineStateStore, job: &BookPipelineJob) -> Vec<PathBuf> {
    let output_root = store.job_output_dir(&job.id);
    let mut paths = vec![output_root.join(LIVE_PROGRESS_FILE)];
    if let Some(output_dir) = job.output_dir.as_deref() {
        paths.push(Path::new(output_dir).join(LIVE_PROGRESS_FILE));
    }
    for child in &job.children {
        if let Some(identity) = &child.source_identity {
            let component = clean_path_component(&identity.pdf_attachment_key);
            if !component.is_empty() {
                paths.push(output_root.join(component).join(LIVE_PROGRESS_FILE));
            }
        }
        if let Some(project_root) = child.local_project_root.as_deref() {
            paths.push(Path::new(project_root).join(LIVE_PROGRESS_FILE));
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn live_progress_matches_stage(
    stage: &BookPipelineStage,
    operation: &BookPipelineOperationProgress,
) -> bool {
    if stage.stage_id != operation.stage_id || stage.status != STATUS_RUNNING {
        return false;
    }
    let Some(started_at) = stage.started_at.as_deref() else {
        return true;
    };
    let Ok(started_at) = chrono::DateTime::parse_from_rfc3339(started_at) else {
        return false;
    };
    let Ok(activity_at) = chrono::DateTime::parse_from_rfc3339(&operation.activity_at) else {
        return false;
    };
    activity_at >= started_at
}

fn overlay_live_progress(job: &mut BookPipelineJob, operation: BookPipelineOperationProgress) {
    let mut matching_stage = if let Some(scope_id) = operation.scope_id.as_deref() {
        job.children
            .iter_mut()
            .find(|child| child.id == scope_id)
            .and_then(|child| {
                child
                    .stages
                    .iter_mut()
                    .find(|stage| live_progress_matches_stage(stage, &operation))
            })
    } else {
        job.children
            .iter_mut()
            .flat_map(|child| child.stages.iter_mut())
            .find(|stage| live_progress_matches_stage(stage, &operation))
    };
    if matching_stage.is_none() {
        matching_stage = job
            .stages
            .iter_mut()
            .find(|stage| live_progress_matches_stage(stage, &operation));
    }
    let Some(stage) = matching_stage else {
        return;
    };
    if let Some(total) = operation.total {
        let running = u32::from(operation.completed < total);
        let summary = BookPipelineUnitSummary {
            total,
            completed: operation.completed,
            running,
            pending: total
                .saturating_sub(operation.completed)
                .saturating_sub(running),
            ..BookPipelineUnitSummary::default()
        };
        stage.unit_summary = Some(summary.clone());
        job.progress.unit_summary = Some(summary);
    }
    job.progress.active_stage_id = operation.stage_id.clone();
    job.progress.operation = Some(operation);
}

fn load_state_with_live_progress(
    store: &dyn BookPipelineStateStore,
) -> Result<BookPipelineState, String> {
    let mut state = store.load()?;
    for job in &mut state.jobs {
        overlay_current_mineru_source_evidence(job);
        let operation = live_progress_paths(store, job)
            .into_iter()
            .filter_map(|path| read_live_worker_progress(&path))
            .max_by(|left, right| left.activity_at.cmp(&right.activity_at));
        if let Some(operation) = operation {
            overlay_live_progress(job, operation);
        }
    }
    Ok(state)
}

/// The route records how extraction originally started, but an explicitly
/// replaced source can become the authoritative downstream input later. Show
/// that current provenance only when the local manifest, MinerU manifest, and
/// source Markdown digest agree; this is a public-state overlay and never
/// rewrites historical execution evidence on disk.
fn overlay_current_mineru_source_evidence(job: &mut BookPipelineJob) {
    for child in &mut job.children {
        let Some(project_root) = child.local_project_root.as_deref().map(Path::new) else {
            continue;
        };
        let source_path = project_root.join("source/source.md");
        let manifest_path = project_root.join("metadata/source_manifest.json");
        let Ok(manifest_text) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&manifest_text) else {
            continue;
        };
        let Some(engine) = manifest
            .get("extraction_engine")
            .and_then(serde_json::Value::as_str)
            .filter(|engine| engine.starts_with("MinerU Precision"))
        else {
            continue;
        };
        let Some(expected_sha256) = manifest
            .get("source_sha256")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        if sha256_file(&source_path).ok().as_deref() != Some(expected_sha256) {
            continue;
        }
        let Some(mineru_manifest) = manifest
            .get("mineru_manifest_path")
            .and_then(serde_json::Value::as_str)
            .map(Path::new)
            .filter(|path| {
                !path.is_absolute()
                    && path.components().all(|component| {
                        matches!(
                            component,
                            std::path::Component::Normal(_) | std::path::Component::CurDir
                        )
                    })
            })
        else {
            continue;
        };
        if !project_root.join(mineru_manifest).is_file() {
            continue;
        }
        if let Some(route) = child
            .route
            .iter_mut()
            .find(|route| route.route_kind != "translation_handoff")
        {
            let original_route = route.route_kind.clone();
            route.route_kind = "mineru".into();
            route.summary = format!(
                "Current translation source verified as {engine} Markdown; the earlier {original_route} extraction output is superseded for downstream work."
            );
        }
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
        "markdown" | "html" | "epub" | "pdf" | "metadata" | "index" | "ocr_sample_report" => {
            "extract"
        }
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
            // Excerpts of the book's own pages, exactly like the translation
            // sample: shown on screen, never logged.
            | "ocr_sample_report"
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
        // The layout track's whole deliverable is one PDF sitting in the job
        // output directory -- it builds no reading project, so without this the
        // finished book offers "Open workspace" and leaves the user to find the
        // file themselves.
        if job.mode == MODE_LAYOUT_PRESERVING {
            if let Some(artifact) = job
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == "pdf" && artifact.validation.hash_matches)
            {
                register_navigation_target(
                    &mut targets,
                    "bilingual_pdf",
                    Path::new(&artifact.path),
                    Path::new(output_dir),
                    Some(&artifact.artifact_id),
                );
            }
        }
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
            if let Some(target) = find(&["bilingual_pdf"]) {
                (Some(target), "Open bilingual PDF")
            } else if let Some(target) = find(&["reading_output_directory"]) {
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
    let children = children
        .iter()
        .filter(|child| child.removed_at.is_none())
        .collect::<Vec<_>>();
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
    let mut progress = job.progress.clone();
    if let Some(summary) = progress.unit_summary.as_mut() {
        // Unit identifiers and failure categories are local diagnostic state.
        // Terminal webhooks carry only aggregate progress per ADR 0002.
        summary.failures.clear();
    }
    BookPipelineTerminalEvent {
        schema_version: TERMINAL_EVENT_SCHEMA_VERSION.into(),
        event_id: sha256_str(&identity),
        job_id: job.id.clone(),
        job_kind: job.kind.clone(),
        status: job.status.clone(),
        current_stage_id: job.current_stage_id.clone(),
        progress,
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
    {
        let stage_order = ordered_stage_index(&next.stage_id);
        let exact_audit_hash = |key: &str| {
            (next.input_hashes.len() == 1)
                .then(|| next.input_hashes.get(key))
                .flatten()
        };
        let approval_invalidation = stage_order
            .zip(ordered_stage_index("approve_translation"))
            .is_some_and(|(stage, gate)| stage > gate)
            && exact_audit_hash("translationApprovalBindingSha256").is_some();
        let split_policy_invalidation = stage_order
            .zip(ordered_stage_index("split"))
            .is_some_and(|(stage, split)| stage > split)
            && exact_audit_hash("splitPolicyVersion")
                .is_some_and(|version| version == SPLIT_POLICY_VERSION);
        if approval_invalidation || split_policy_invalidation {
            return previous.input_hashes != next.input_hashes;
        }
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
pub async fn get_book_pipeline_state() -> Result<BookPipelineState, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        load_state_with_live_progress(&store)
    })
    .await
}

#[tauri::command]
pub async fn preview_book_pipeline_route(
    source: BookPipelineSource,
    mode: String,
    config: Option<BookPipelinePreviewConfig>,
) -> Result<Vec<BookPipelineRouteItem>, String> {
    crate::run_blocking(move || {
        preview_book_pipeline_route_with_executor(
            &SystemCommandExecutor,
            &source,
            &mode,
            config.unwrap_or_default(),
        )
    })
    .await
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
            mode,
            config,
            20,
            &book_ocr_conversion_root(),
        );
    }
    Ok(preview_route(source, mode, config))
}

#[tauri::command]
pub async fn queue_book_pipeline_job(
    source: BookPipelineSource,
    mode: String,
    translation_intent: BookPipelineTranslationIntent,
    config: Option<BookPipelinePreviewConfig>,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        queue_job_with_translation_intent(
            &store,
            source,
            mode,
            translation_intent,
            config.unwrap_or_default(),
        )
    })
    .await
}

#[tauri::command]
pub async fn save_book_pipeline_custom_instructions(
    job_id: String,
    child_id: Option<String>,
    custom_instructions: BookPipelineCustomInstructions,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        save_book_custom_instructions(&store, &job_id, child_id.as_deref(), custom_instructions)
    })
    .await
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
pub async fn run_book_pipeline_job(job_id: String) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        let job = run_job_to_quiescence(&store, &SystemPipelineRunner, &job_id)?;
        dispatch_configured_terminal_notification(&store, job)
    })
    .await
}

#[tauri::command]
pub async fn retry_book_pipeline_job(job_id: String) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        let job = retry_job_from_ui(&store, &SystemPipelineRunner, &job_id)?;
        dispatch_configured_terminal_notification(&store, job)
    })
    .await
}

fn retry_job_from_ui(
    store: &dyn BookPipelineStateStore,
    runner: &dyn PipelineRunner,
    job_id: &str,
) -> Result<BookPipelineJob, String> {
    let state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let job = &state.jobs[job_index];
    let live = live_children(job).collect::<Vec<_>>();
    let staged_child_id = (live.len() == 1)
        .then(|| live[0])
        .filter(|child| {
            child.local_project_root.is_some()
                && stage_ref(child, "handoff").is_some_and(|stage| stage.status == STATUS_COMPLETED)
                && child.stages.iter().any(|stage| {
                    ordered_stage_index(&stage.stage_id)
                        .is_some_and(|order| order > ordered_stage_index("handoff").unwrap_or(0))
                        && stage.status == STATUS_FAILED
                })
        })
        .map(|child| child.id.clone());
    drop(state);
    if let Some(child_id) = staged_child_id {
        return advance_job(store, job_id, Some(&child_id), false);
    }
    retry_job_to_quiescence(store, runner, job_id)
}

/// Remove a job from the shelf. Files on disk (extraction output, the local
/// reading project, Zotero attachments) are deliberately left untouched — this
/// only forgets the job, so a re-queued book can reuse the converted Markdown.
#[tauri::command]
pub async fn delete_book_pipeline_job(
    job_id: String,
    child_id: Option<String>,
    explicit_approval: bool,
) -> Result<BookPipelineState, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        delete_job(&store, &job_id, child_id.as_deref(), explicit_approval)
    })
    .await
}

/// A book still on the shelf. Everything that counts, schedules or displays a
/// collection's books goes through this, so a dropped one stays in the frozen
/// membership without being worked on or shown again.
fn live_children(job: &BookPipelineJob) -> impl Iterator<Item = &BookPipelineChildJob> {
    job.children
        .iter()
        .filter(|child| child.removed_at.is_none())
}

fn delete_job(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
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
    // The shelf shows one row per book and a collection queues many under one
    // job, so removing the job for the book the user pointed at took the rest of
    // the batch with it. Drop just that book when others would remain.
    if let Some(child_id) = child_id {
        let job = &mut state.jobs[job_index];
        let remaining = live_children(job)
            .filter(|child| child.id != child_id)
            .count();
        let target = job
            .children
            .iter_mut()
            .find(|child| child.id == child_id && child.removed_at.is_none())
            .ok_or_else(|| "Book Pipeline child not found.".to_string())?;
        if remaining > 0 {
            target.removed_at = Some(now_label());
            let title = source_title(&target.source);
            job.current_step = "Removed one book from this batch".into();
            job.log_summary
                .push(format!("Removed {title} from this batch"));
            job.log_summary = trim_log_summary(&job.log_summary);
            job.updated_at = now_label();
            derive_job(job);
            store.save(&state)?;
            return store.load();
        }
        // Nothing would be left on the shelf, so the row and the job are the
        // same thing again and an empty batch is not worth keeping.
    }
    state.jobs.remove(job_index);
    store.save(&state)?;
    store.load()
}

fn job_is_actively_running(job: &BookPipelineJob) -> bool {
    matches!(job.status.as_str(), STATUS_RUNNING | STATUS_HANDOFF_RUNNING)
        || live_children(job).any(|child| {
            child
                .stages
                .iter()
                .any(|stage| stage.status == STATUS_RUNNING)
        })
}

#[tauri::command]
pub async fn advance_book_pipeline_job(
    job_id: String,
    child_id: Option<String>,
    invalidate_downstream: Option<bool>,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        let job = advance_job(
            &store,
            &job_id,
            child_id.as_deref(),
            invalidate_downstream.unwrap_or(false),
        )?;
        dispatch_configured_terminal_notification(&store, job)
    })
    .await
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
pub async fn approve_book_pipeline_gate(
    job_id: String,
    child_id: Option<String>,
    stage_id: String,
    explicit_approval: bool,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        approve_job_gate(
            &store,
            &job_id,
            child_id.as_deref(),
            &stage_id,
            explicit_approval,
        )
    })
    .await
}

/// Re-route a book the pipeline held back, in place. Before this the Overview
/// tab offered the same three choices as the wizard's preflight step but with
/// every button disabled, so a held book could only be dealt with by deleting it
/// and queueing it again — which for a collection took the whole batch with it.
#[tauri::command]
pub async fn set_book_pipeline_route_override(
    job_id: String,
    child_id: Option<String>,
    route_item_id: String,
    route_override: String,
    config: Option<BookPipelinePreviewConfig>,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        set_route_override(
            &store,
            &job_id,
            child_id.as_deref(),
            &route_item_id,
            &route_override,
            &config.unwrap_or_default(),
        )
    })
    .await
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
pub async fn record_book_pipeline_reader_evidence(
    job_id: String,
    child_id: Option<String>,
    artifact_kind: String,
    reader: String,
    reader_version: String,
    conclusion: String,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
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
    })
    .await
}

/// Sampling is a "try before you decide" action, so by default it leaves the
/// job's own translation provider alone: the caller passes `apply_to_job` only
/// for the explicit "translate this book with this model" action, which goes
/// through `set_book_pipeline_translation_provider` in the normal case.
#[tauri::command]
pub async fn run_book_pipeline_translation_sample(
    job_id: String,
    child_id: String,
    provider_profile_id: String,
    provider_config_id: String,
    apply_to_job: bool,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
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
    })
    .await
}

/// Adopt a provider slot as the job's own, which is what the full-book run will
/// use. Separate from sampling so that trying a model out cannot silently
/// redirect the book; the approval gate is rebound here exactly as it is after a
/// sample, so an approval that predates the change does not survive it.
#[tauri::command]
pub async fn set_book_pipeline_translation_provider(
    job_id: String,
    child_id: String,
    provider_profile_id: String,
    provider_config_id: String,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        set_translation_provider_in_store(
            &store,
            &job_id,
            Some(&child_id),
            &provider_profile_id,
            &provider_config_id,
        )
    })
    .await
}

/// Synchronous by convention, not by necessity. rfd's blocking pickers wrap
/// their whole body in `run_on_main`, which dispatches to the main thread when
/// they are called from anywhere else, so calling one from a worker is
/// supported — see `save_book_pipeline_diagnostic`, which does exactly that.
/// What is left here is a trade rather than a rule: these two commands are a
/// dialog plus a few lines of struct building, so taking them off the main
/// thread would buy nothing worth the divergence. Anything with real work
/// around the dialog should go async instead.
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
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    }))
}

#[tauri::command]
pub async fn discover_book_pipeline_zotero_sources(
    source: BookPipelineSource,
    limit: Option<u32>,
) -> Result<BookPipelineZoteroDiscoveryResult, String> {
    crate::run_blocking(move || {
        discover_zotero_sources(&SystemCommandExecutor, &source, limit.unwrap_or(20))
    })
    .await
}

#[tauri::command]
pub async fn open_book_pipeline_output(job_id: String) -> Result<BookPipelineActionResult, String> {
    crate::run_blocking(move || {
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
    })
    .await
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
pub async fn read_book_pipeline_artifact_excerpt(
    job_id: String,
    artifact_id: String,
    max_chars: Option<usize>,
) -> Result<BookPipelineArtifactExcerpt, String> {
    crate::run_blocking(move || {
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
    })
    .await
}

/// Run both OCR engines over the same sampled interior pages so the conversion
/// route can be chosen on evidence rather than on a guess about the scan. Like
/// the translation sample, it changes nothing about the job: adopting a winner
/// is the separate, explicit route-override action.
#[tauri::command]
pub async fn run_book_pipeline_ocr_sample(
    job_id: String,
    child_id: String,
    sample_pages: Option<u32>,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        run_ocr_sample_with_executor(
            &store,
            &job_id,
            Some(&child_id),
            sample_pages.unwrap_or(OCR_SAMPLE_PAGE_COUNT),
            &SystemCommandExecutor,
        )
    })
    .await
}

#[tauri::command]
pub async fn read_book_pipeline_ocr_sample(
    job_id: String,
    child_id: String,
) -> Result<BookPipelineOcrSampleReport, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        let state = store.load()?;
        let job = state
            .jobs
            .iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| "Book Pipeline job not found.".to_string())?;
        let sample_dir = ocr_sample_dir(&store, &job_id, &child_id);
        read_ocr_sample_report(job, &child_id, &sample_dir)
    })
    .await
}

#[tauri::command]
pub async fn read_book_pipeline_translation_sample(
    job_id: String,
    child_id: String,
) -> Result<BookPipelineTranslationSampleReport, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        let state = store.load()?;
        let job = state
            .jobs
            .iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| "Book Pipeline job not found.".to_string())?;
        read_translation_sample_report(job, &child_id)
    })
    .await
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
pub async fn export_book_pipeline_diagnostic(
    job_id: String,
    profile: String,
) -> Result<serde_json::Value, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        let state = store.load()?;
        let job = state
            .jobs
            .iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| "Book Pipeline job not found.".to_string())?;
        build_book_pipeline_diagnostic(job, &profile)
    })
    .await
}

/// The same three profiles, written to a folder the user picks, because a
/// diagnostic bundle is only useful if it can be attached to a report. The
/// command above returns the value in-process and stays as it is.
///
/// Deliberately async, dialog included: rfd's blocking `pick_folder` runs its
/// body inside `run_on_main`, so calling it from this worker dispatches the
/// panel to the main thread and hands the result back. Only the panel itself
/// needs that thread — loading the store, building the document and writing the
/// bundle all stay off it. Do not "fix" this back to a synchronous `fn`; that
/// would put the file work back on the main thread to no end.
#[tauri::command]
pub async fn save_book_pipeline_diagnostic(
    job_id: String,
    profile: String,
) -> Result<BookPipelineActionResult, String> {
    crate::run_blocking(move || {
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
    })
    .await
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
pub async fn handoff_book_pipeline_markdown(
    job_id: String,
    artifact_path: Option<String>,
) -> Result<BookPipelineJob, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        handoff_job_markdown(
            &store,
            &job_id,
            artifact_path.as_deref(),
            &local_reading_repo_root()?,
        )
    })
    .await
}

#[tauri::command]
pub async fn preview_book_pipeline_cleanup() -> Result<BookPipelineCleanupPreview, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        preview_cleanup_candidates(&store)
    })
    .await
}

#[tauri::command]
pub async fn approve_book_pipeline_cleanup(
    candidate_id: String,
    explicit_approval: bool,
) -> Result<BookPipelineActionResult, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        approve_cleanup_candidate(&store, &candidate_id, explicit_approval)
    })
    .await
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
        preview_zotero_route_from_worker(executor, &source, &mode, config, 20, root)?
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
        removed_at: None,
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
    // The log is a ring buffer; a record that only lives there is a record that
    // expires. The two pipeline gates already persist their decisions as
    // approval references, and a decision to delete someone's source PDF is not
    // a weaker claim than a decision to promote a translation.
    let child_job_id = state.jobs[index]
        .children
        .first()
        .map(|child| child.id.clone())
        .unwrap_or_default();
    let bound_artifact_hashes = cleanup_bound_artifact_hashes(&state.jobs[index]);
    state.jobs[index]
        .approval_references
        .retain(|approval| approval.gate_id != CLEANUP_GATE_ID);
    state.jobs[index]
        .approval_references
        .push(BookPipelineApprovalReference {
            approval_id: new_approval_id(),
            gate_id: CLEANUP_GATE_ID.into(),
            child_job_id,
            stage_id: "validate_reading".into(),
            decision: "approved".into(),
            bound_artifact_hashes,
            decided_at: now.clone(),
        });
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

const CLEANUP_GATE_ID: &str = "source_cleanup";

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookPipelineCleanupApprovalStatus {
    pub source_ref: String,
    #[serde(default)]
    pub job_id: String,
    #[serde(default)]
    pub approval_id: String,
    #[serde(default)]
    pub decided_at: String,
    pub approved: bool,
    pub reason: String,
}

/// The reading artifacts a cleanup decision is taken against, keyed by artifact
/// id. Deleting a source PDF is only safe while *these* are the files on disk, so
/// rebuilding the book invalidates the approval rather than silently carrying it.
fn cleanup_bound_artifact_hashes(job: &BookPipelineJob) -> BTreeMap<String, String> {
    job.children
        .iter()
        .flat_map(|child| child.artifacts.iter())
        .chain(job.artifacts.iter())
        .filter(|artifact| artifact.kind.starts_with("reading_"))
        .filter_map(|artifact| {
            artifact
                .sha256
                .as_deref()
                .map(|sha256| (artifact.artifact_id.clone(), sha256.to_string()))
        })
        .collect()
}

/// Whether a recorded cleanup approval still describes the book on disk. This is
/// the check an out-of-band deletion script needs: an approval is a statement
/// about specific bytes, and it stops applying the moment those bytes change.
fn cleanup_approval_is_current(job: &BookPipelineJob) -> bool {
    let Some(approval) = job
        .approval_references
        .iter()
        .find(|approval| approval.gate_id == CLEANUP_GATE_ID)
    else {
        return false;
    };
    if approval.bound_artifact_hashes.is_empty() {
        return false;
    }
    let current = cleanup_bound_artifact_hashes(job);
    approval.bound_artifact_hashes == current
}

/// Read-only entry point for the out-of-band cleanup scripts: given a source
/// reference they already know, say whether a still-current approval exists. The
/// launcher deletes nothing here and never has.
#[tauri::command]
pub async fn verify_book_pipeline_cleanup_approval(
    source_ref: String,
) -> Result<BookPipelineCleanupApprovalStatus, String> {
    crate::run_blocking(move || {
        let store = BookPipelineStore::default()?;
        Ok(cleanup_approval_status(&store.load()?, &source_ref))
    })
    .await
}

fn cleanup_approval_status(
    state: &BookPipelineState,
    source_ref: &str,
) -> BookPipelineCleanupApprovalStatus {
    let matched = state.jobs.iter().find(|job| {
        job.source.selector.as_deref() == Some(source_ref)
            || job.source.path.as_deref() == Some(source_ref)
            || job.id == source_ref
    });
    let Some(job) = matched else {
        return BookPipelineCleanupApprovalStatus {
            source_ref: source_ref.to_string(),
            approved: false,
            reason: "No Book Pipeline job matches this source reference.".into(),
            ..BookPipelineCleanupApprovalStatus::default()
        };
    };
    let approval = job
        .approval_references
        .iter()
        .find(|approval| approval.gate_id == CLEANUP_GATE_ID);
    let Some(approval) = approval else {
        return BookPipelineCleanupApprovalStatus {
            source_ref: source_ref.to_string(),
            job_id: job.id.clone(),
            approved: false,
            reason: "No source-cleanup approval has been recorded for this book.".into(),
            ..BookPipelineCleanupApprovalStatus::default()
        };
    };
    let current = cleanup_approval_is_current(job);
    BookPipelineCleanupApprovalStatus {
        source_ref: source_ref.to_string(),
        job_id: job.id.clone(),
        approval_id: approval.approval_id.clone(),
        decided_at: approval.decided_at.clone(),
        approved: current,
        reason: if current {
            "A source-cleanup approval is recorded and still matches the built artifacts.".into()
        } else {
            "The approved artifacts changed since the approval; re-approve before deleting.".into()
        },
    }
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
        cleanup_validated_reading_check(job),
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
    // `html`/`epub`/`translation_source` are conversion-stage outputs; the built
    // book is `reading_*`. Accepting the former let a book whose reading build
    // never ran look like it had a deliverable worth deleting the source for.
    if let Some(artifact) = job
        .children
        .iter()
        .flat_map(|child| child.artifacts.iter())
        .chain(job.artifacts.iter())
        .find(|artifact| artifact.kind.starts_with("reading_"))
    {
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

/// The candidate list never looked at a single stage status, so a book whose
/// reading validation failed — or which never reached `build_reading` at all —
/// counted as "evidence complete" and offered to have its source PDF deleted.
fn cleanup_validated_reading_check(job: &BookPipelineJob) -> BookPipelineCleanupEvidence {
    let validated = job.children.iter().any(|child| {
        stage_ref(child, "validate_reading").is_some_and(|stage| stage.status == STATUS_COMPLETED)
    });
    if validated {
        cleanup_evidence(
            "validated_reading",
            true,
            "Reading validation completed for this book.",
            None,
            None,
        )
    } else {
        cleanup_evidence(
            "validated_reading",
            false,
            "Reading validation has not completed for this book.",
            None,
            None,
        )
    }
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

/// Whether `run` may take its two source-kind shortcuts instead of building a
/// command.
///
/// Both shortcuts assume the reflow track. The layout track is dispatched inside
/// the command builder, so it has to skip them -- a Zotero title search is a
/// "batch" source, and a layout job queued from one would otherwise be handed to
/// the OCR worker with its mode silently ignored.
fn takes_reflow_source_shortcut(job: &BookPipelineJob) -> bool {
    job.mode != MODE_LAYOUT_PRESERVING
        && (job.source.kind == "markdown_source" || is_zotero_batch_source(&job.source))
}

impl<E: RunnerCommandExecutor> PipelineRunner for CommandPipelineRunner<E> {
    fn run(&self, job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerOutput, String> {
        if takes_reflow_source_shortcut(job) {
            if job.source.kind == "markdown_source" {
                return run_markdown_source_job(job, output_dir);
            }
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
    if command_uses_ocr_credentials(&command) {
        inject_ocr_credentials(&mut command);
    }
    if command.label == LAYOUT_PDF_COMMAND_LABEL {
        inject_layout_pdf_model_env(&mut command)?;
    }
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
    if command.label == LAYOUT_PDF_COMMAND_LABEL {
        // Stated unconditionally because BabelDOC has no runtime warning for it,
        // so there is no marker to forward -- and a reader who opens the
        // bibliography of a finished book deserves to know before they conclude
        // the translation is broken.
        log_summary.push(LAYOUT_PDF_REFERENCE_LIMITATION.into());
    }
    Ok(RunnerOutput {
        log_summary: trim_log_summary(&log_summary),
        artifacts,
        collection_items: Vec::new(),
        output_dir: Some(command.output_dir),
        current_step: None,
    })
}

fn command_uses_ocr_credentials(command: &RunnerCommand) -> bool {
    matches!(
        command.label.as_str(),
        ZOTERO_CONVERSION_COMMAND_LABEL | "MinerU Precision batch" | "local PDF conversion wrapper"
    )
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

/// Point BabelDOC at the model the user chose in Settings.
///
/// Unlike the OCR injection above there is no fallback: BabelDOC has no registry
/// and no `.env` lookup of its own, so a missing key or a non-OpenAI provider has
/// to fail here with something the user can act on rather than a stack trace from
/// inside the subprocess.
fn inject_layout_pdf_model_env(command: &mut RunnerCommand) -> Result<(), String> {
    let repo_root = translation_engine_repo_root()?;
    let endpoint = crate::model_settings::resolve_openai_compatible_endpoint(&repo_root)?;
    command
        .env
        .push((LAYOUT_PDF_BASE_URL_ENV.into(), endpoint.base_url));
    command
        .env
        .push((LAYOUT_PDF_API_KEY_ENV.into(), endpoint.api_key));
    command
        .env
        .push((LAYOUT_PDF_MODEL_ENV.into(), endpoint.model));
    Ok(())
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
    // Dispatched on the mode before the source kind: this track replaces the
    // conversion step outright rather than being one more way to convert. The
    // eligibility rule -- a single `direct_text` PDF -- lives in the builder.
    if job.mode == MODE_LAYOUT_PRESERVING {
        return build_layout_pdf_command(job, output_dir);
    }
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
    let mineru_source = source_path.with_extension("mineru");
    if mineru_source.is_dir() {
        copy_directory_tree(&mineru_source, &copied.with_extension("mineru"))?;
    }
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
    let runnable_routes: Vec<&BookPipelineRouteItem> = job
        .route
        .iter()
        .filter(|route| route.can_run && route.route_kind != "translation_handoff")
        .collect();
    let mineru_routes = runnable_routes
        .iter()
        .filter(|route| route.route_kind == "mineru")
        .count();
    if mineru_routes > 0 {
        if mineru_routes != runnable_routes.len() {
            return Err(
                "One local PDF folder job cannot mix MinerU and non-MinerU routes. Split the files into separate jobs so no selected engine is silently ignored."
                    .into(),
            );
        }
        let script = root.join("mineru.py");
        if !script.is_file() {
            return Err(format!(
                "MinerU Precision client not found at {}",
                display_path(&script)
            ));
        }
        let mut args = ocr_python_args(&script);
        args.extend([
            input_dir.into(),
            "--output-dir".into(),
            display_path(output_dir),
            "--mode".into(),
            "batch".into(),
            "--model-version".into(),
            "vlm".into(),
        ]);
        return Ok(RunnerCommand {
            kind: RunnerCommandKind::Process,
            label: "MinerU Precision batch".into(),
            program: PathBuf::from("uv"),
            args,
            env: Vec::new(),
            cwd: Some(root.to_path_buf()),
            output_dir: output_dir.to_path_buf(),
            attempts: job.attempts,
            accepted_exit_codes: vec![0],
        });
    }
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

/// The repository root, having checked that the layout-preserving package is
/// actually there. `uv run --package` resolves against the workspace root, so a
/// repoRoot pointing somewhere without this member fails inside uv with a much
/// worse message than this one.
fn layout_pdf_repo_root() -> Result<PathBuf, String> {
    let repo_root = local_reading_repo_root()?;
    let package_manifest = repo_root
        .join("packages")
        .join("layout-pdf")
        .join("pyproject.toml");
    if !package_manifest.is_file() {
        return Err(format!(
            "Layout-preserving PDF package not found at {}",
            display_path(&package_manifest)
        ));
    }
    Ok(repo_root)
}

/// The source PDF for a layout-track route.
///
/// `source_ref` carries the worker's fingerprint as a `#source_md5=` fragment,
/// so the raw value is a path that does not exist and whose extension is not
/// `.pdf`. Every real Zotero attachment arrives this way; only hand-built
/// fixtures come through clean, which is exactly why this is easy to get wrong.
fn layout_pdf_source_path(route: &BookPipelineRouteItem) -> Result<PathBuf, String> {
    // Split on the exact marker from the right, never on a bare `#`: `#` is a
    // legal filename character, and `draft#2.pdf` would otherwise be truncated
    // to `draft`. Taking the last occurrence keeps the one this code appended.
    let raw = route
        .source_ref
        .rsplit_once("#source_md5=")
        .map(|(path, _)| path)
        .unwrap_or(&route.source_ref)
        .trim();
    if raw.is_empty() {
        return Err("The layout-preserving track has no source PDF path.".into());
    }
    let path = PathBuf::from(raw);
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
        != Some("pdf")
    {
        return Err(format!(
            "The layout-preserving track only accepts PDFs, not {}",
            display_path(&path)
        ));
    }
    if !path.is_file() {
        return Err(format!("Source PDF not found at {}", display_path(&path)));
    }
    Ok(path)
}

/// The one route this track is allowed to run.
///
/// `direct_text` means the worker found a usable text layer. A scanned book has
/// nothing for BabelDOC to translate, and a multi-item source has no single
/// document to preserve the layout of, so both are refused here rather than
/// failing halfway through a run.
fn layout_pdf_route(job: &BookPipelineJob) -> Result<&BookPipelineRouteItem, String> {
    let runnable: Vec<&BookPipelineRouteItem> = job
        .route
        .iter()
        .filter(|route| route.can_run && route.route_kind != "translation_handoff")
        .collect();
    match runnable.as_slice() {
        [] => Err(
            "The layout-preserving track has no runnable item. Only a text PDF is eligible."
                .into(),
        ),
        [route] if route.route_kind == "direct_text" => Ok(route),
        [route] => Err(format!(
            "The layout-preserving track is only available for text PDFs, and this item routed as {}.",
            route.route_kind
        )),
        _ => Err(
            "The layout-preserving track handles one book at a time. Queue each PDF separately."
                .into(),
        ),
    }
}

fn build_layout_pdf_command(
    job: &BookPipelineJob,
    output_dir: &Path,
) -> Result<RunnerCommand, String> {
    build_layout_pdf_command_for_root(job, output_dir, &layout_pdf_repo_root()?)
}

fn build_layout_pdf_command_for_root(
    job: &BookPipelineJob,
    output_dir: &Path,
    root: &Path,
) -> Result<RunnerCommand, String> {
    let source_path = layout_pdf_source_path(layout_pdf_route(job)?)?;
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: LAYOUT_PDF_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args: vec![
            "run".into(),
            "--package".into(),
            "layout-pdf".into(),
            // BabelDOC is an optional extra so the shared venv every other suite
            // runs in stays light; asking for it here is what installs it, from
            // uv's cache after the first run. See packages/layout-pdf/README.md.
            "--extra".into(),
            "babeldoc".into(),
            "layout-pdf".into(),
            "--input".into(),
            display_path(&source_path),
            "--output-dir".into(),
            display_path(output_dir),
        ],
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
    command
        .env
        .push(("BIBLIOSMITH_PROGRESS_SCOPE".into(), child.id.clone()));
    Ok(command)
}

fn build_zotero_conversion_command_for_source(
    source: &BookPipelineSource,
    route: &BookPipelineRouteItem,
    attempts: u32,
    output_dir: &Path,
    root: &Path,
) -> Result<RunnerCommand, String> {
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
    mode: &str,
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
    // Same handoff contract preview_route applies to every other source kind:
    // a runnable route in a handoff mode carries the handoff row. Appended
    // after the overrides, which can flip whether the route runs at all.
    if should_handoff_after_run(mode) && route_is_runnable_for_source(source, &routes) {
        routes.push(translation_handoff_route_item(source));
    }
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
    let live_progress_path = command.output_dir.join(LIVE_PROGRESS_FILE);
    let _ = fs::remove_file(&live_progress_path);
    process.env(LIVE_PROGRESS_PATH_ENV, &live_progress_path);
    if let Some(path_value) = runner_path_env_value() {
        process.env("PATH", path_value);
    }
    process.envs(command.env.iter().map(|(key, value)| (key, value)));
    if let Some(cwd) = &command.cwd {
        process.current_dir(cwd);
    }
    let timeout = runner_command_timeout(command);
    let output = crate::command_output_with_timeout(&mut process, timeout).map_err(|err| {
        if err.kind() == std::io::ErrorKind::TimedOut {
            // Named, not folded into a generic IO failure: a stage that stops
            // because a provider hung is a different problem from one that
            // could not start, and only one of them is worth retrying as-is.
            format!(
                "{} timed out after {}s and was stopped.",
                command.label,
                timeout.as_secs()
            )
        } else {
            format!(
                "Failed to start {} with {}: {err}",
                command.label,
                display_path(&program)
            )
        }
    })?;
    command_result_from_output(command, output)
}

/// A stalled child used to hold its stage `running` forever, because nothing
/// ever stopped waiting for it. Translation gets hours because that is how long
/// a real book legitimately takes; everything else is minutes, and a bound that
/// generous is still a bound.
fn runner_command_timeout(command: &RunnerCommand) -> Duration {
    let hours = match command.label.as_str() {
        TRANSLATION_ENGINE_COMMAND_LABEL => 12,
        ZOTERO_CONVERSION_COMMAND_LABEL => 6,
        // Measured, not guessed: 37s/page on dense academic pages and 23s/page
        // on lighter ones, against Qwen at the default qps of 4. Twelve hours
        // therefore covers a book of roughly 1100 dense pages, which is well
        // past anything in the library, and absorbs the one-off model download
        // (~4 minutes) on the first run.
        LAYOUT_PDF_COMMAND_LABEL => 12,
        _ => 2,
    };
    Duration::from_secs(hours * 60 * 60)
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
                // A classification, never the warning text. BabelDOC warns in
                // free English that interpolates page numbers and sometimes file
                // paths; `layout_pdf/warnings.py` counts by kind so none of it
                // has to cross into a job log.
                "warning" => LAYOUT_PDF_WARNING_KINDS.contains(&value),
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

/// `.state/chunks` under an OCR output root, where `zotero_llm_worker.py` splits
/// a long book into page ranges before uploading them. Those splits are PDFs, so
/// once `artifact_kind` learned to recognise PDFs they would otherwise be
/// registered as deliverables -- dozens of `pages-0001-0050.pdf` per book. The
/// worker's other private subtrees hold no PDFs and are left alone; `.state`
/// itself must stay scanned, because the finished Markdown lives in
/// `.state/staging`.
fn is_ocr_worker_chunk_dir(path: &Path) -> bool {
    path.file_name().and_then(OsStr::to_str) == Some("chunks")
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(OsStr::to_str)
            == Some(".state")
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
            // A sibling *.mineru tree is supporting material for its cleaned
            // Markdown, not a second set of handoff candidates. In particular,
            // MinerU's per-part Markdown must never outrank the assembled file.
            if path.extension().and_then(|extension| extension.to_str()) == Some("mineru") {
                continue;
            }
            // The OCR engine comparison is evidence for choosing a route, not
            // conversion output, and it lands under the same job output root
            // this scan walks. Registered here, its report would become a
            // `metadata` artifact of a conversion that never produced it.
            if path.file_name().and_then(|name| name.to_str()) == Some(OCR_SAMPLE_DIR_NAME) {
                continue;
            }
            if is_ocr_worker_chunk_dir(&path) {
                continue;
            }
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
    let mineru_source = markdown_path.with_extension("mineru");
    let source_resources_path = if mineru_source.is_dir() {
        let directory_name = mineru_source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "MinerU artifact directory has an invalid name.".to_string())?;
        let target = project_root.join("source").join(directory_name);
        copy_directory_tree(&mineru_source, &target)?;
        Some(format!("source/{directory_name}"))
    } else {
        None
    };
    let source_sha256 = sha256_file(&markdown_path)?;
    write_source_manifest(
        &project_root,
        &markdown_path,
        &source_sha256,
        "cleaned_markdown_ready",
        source_resources_path.as_deref(),
    )?;
    let manifest = project_root.join("metadata").join("source_manifest.json");
    let artifacts = vec![
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
    let log_summary = vec![format!(
        "Translation handoff ready at {}",
        display_path(&project_root)
    )];
    Ok(TranslationHandoffOutput {
        log_summary,
        artifacts,
    })
}

fn copy_directory_tree(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|err| err.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Refusing to copy a symlink from the MinerU artifact tree: {}",
            display_path(source)
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "MinerU artifact path is not a directory: {}",
            display_path(source)
        ));
    }
    fs::create_dir_all(target).map_err(|err| err.to_string())?;
    for entry in fs::read_dir(source).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let entry_metadata = fs::symlink_metadata(&source_path).map_err(|err| err.to_string())?;
        if entry_metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to copy a symlink from the MinerU artifact tree: {}",
                display_path(&source_path)
            ));
        }
        if entry_metadata.is_dir() {
            copy_directory_tree(&source_path, &target_path)?;
        } else if entry_metadata.is_file() {
            fs::copy(&source_path, &target_path).map_err(|err| err.to_string())?;
        } else {
            return Err(format!(
                "Unsupported entry in MinerU artifact tree: {}",
                display_path(&source_path)
            ));
        }
    }
    Ok(())
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
    source_resources_path: Option<&str>,
) -> Result<(), String> {
    let mut manifest = serde_json::json!({
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
    if let Some(path) = source_resources_path {
        manifest["source_resources_path"] = serde_json::Value::String(path.to_string());
    }
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
        // The layout-preserving track's deliverable. `collect_artifacts` skips
        // the OCR worker's chunk directory, which is the only other place a PDF
        // appears under a job output root.
        Some("pdf") => Some("pdf"),
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
    #[cfg(test)]
    {
        let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if let Some(repo_root) = start.ancestors().find(|path| {
            path.join("AGENTS.md").is_file()
                && path
                    .join("tools")
                    .join("create_local_book_project.py")
                    .is_file()
        }) {
            return Ok(repo_root.to_path_buf());
        }
    }

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
const MAX_TRANSLATION_UNIT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy)]
struct MarkdownFence {
    marker: u8,
    width: usize,
}

fn update_markdown_fence(line: &str, state: &mut Option<MarkdownFence>) -> bool {
    let bytes = line.as_bytes();
    let leading_spaces = bytes.iter().take_while(|byte| **byte == b' ').count();
    if leading_spaces > 3 || leading_spaces == bytes.len() {
        return false;
    }
    let marker = bytes[leading_spaces];
    if marker != b'`' && marker != b'~' {
        return false;
    }
    let width = bytes[leading_spaces..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    if width < 3 {
        return false;
    }

    if let Some(active) = state {
        let remainder = &bytes[leading_spaces + width..];
        if marker == active.marker
            && width >= active.width
            && remainder.iter().all(u8::is_ascii_whitespace)
        {
            *state = None;
            return true;
        }
        return false;
    }

    let remainder = &bytes[leading_spaces + width..];
    if marker == b'`' && remainder.contains(&b'`') {
        return false;
    }
    *state = Some(MarkdownFence { marker, width });
    true
}

fn split_source_markdown(text: &str) -> SplitPlan {
    let lines: Vec<&str> = text.lines().collect();
    let mut fence = None;
    let heading_levels: Vec<Option<usize>> = lines
        .iter()
        .map(|line| {
            if update_markdown_fence(line, &mut fence) {
                return None;
            }
            if fence.is_some() {
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
    let chapters = bound_oversized_chapters(chapters, &lines, &heading_levels, primary);
    SplitPlan {
        primary_heading_level: primary.unwrap_or(0),
        chapters,
    }
}

fn bound_oversized_chapters(
    chapters: Vec<SplitChapter>,
    lines: &[&str],
    heading_levels: &[Option<usize>],
    primary: Option<usize>,
) -> Vec<SplitChapter> {
    let mut slices = Vec::new();
    for chapter in chapters {
        let start = chapter.start_line.saturating_sub(1);
        let end = chapter.end_line;
        if chapter.text.len() <= MAX_TRANSLATION_UNIT_BYTES {
            slices.push((chapter.title, start, end));
            continue;
        }
        let deeper_level = heading_levels[start..end]
            .iter()
            .flatten()
            .copied()
            .filter(|level| primary.is_none_or(|primary| *level > primary))
            .min();
        let boundaries = if let Some(deeper_level) = deeper_level {
            heading_levels[start..end]
                .iter()
                .enumerate()
                .filter_map(|(offset, level)| {
                    (*level == Some(deeper_level)).then_some(start + offset)
                })
                .collect::<Vec<_>>()
        } else {
            paragraph_start_lines(lines, start, end)
        };
        if boundaries.is_empty() {
            slices.push((chapter.title, start, end));
            continue;
        }

        let mut unit_starts = vec![start];
        unit_starts.extend(boundaries.iter().skip(1).copied());
        let mut group_start = start;
        let mut group_title = chapter.title;
        for (position, &unit_start) in unit_starts.iter().enumerate().skip(1) {
            let unit_end = unit_starts.get(position + 1).copied().unwrap_or(end);
            if rendered_slice_len(&lines[group_start..unit_end]) > MAX_TRANSLATION_UNIT_BYTES {
                slices.push((group_title, group_start, unit_start));
                group_start = unit_start;
                let title = heading_levels[unit_start]
                    .is_some()
                    .then(|| heading_title(lines[unit_start]))
                    .filter(|title| !title.is_empty());
                group_title = title.unwrap_or_else(|| "Continuation".into());
            }
        }
        slices.push((group_title, group_start, end));
    }

    let chapters = slices
        .into_iter()
        .enumerate()
        .map(|(index, (title, start, end))| {
            build_chapter(index + 1, &title, start + 1, end, &lines[start..end])
        })
        .collect();
    hard_bound_unstructured_text(chapters)
}

fn hard_bound_unstructured_text(chapters: Vec<SplitChapter>) -> Vec<SplitChapter> {
    let mut bounded = Vec::new();
    for chapter in chapters {
        let pieces = hard_bound_text(&chapter.text);
        if pieces.len() == 1 {
            bounded.push(chapter);
            continue;
        }

        let mut start_line = chapter.start_line;
        for (piece_index, text) in pieces.into_iter().enumerate() {
            let newline_count = text.bytes().filter(|byte| *byte == b'\n').count();
            let end_line = if newline_count == 0 {
                start_line
            } else {
                start_line + newline_count - usize::from(text.ends_with('\n'))
            };
            let title = if piece_index == 0 {
                chapter.title.clone()
            } else {
                "Continuation".into()
            };
            bounded.push(build_chapter_from_text(
                bounded.len() + 1,
                &title,
                start_line,
                end_line,
                text,
            ));
            start_line += newline_count;
        }
    }

    for (index, chapter) in bounded.iter_mut().enumerate() {
        chapter.ordinal = index + 1;
        chapter.id = format!("chapter_{:03}", index + 1);
        chapter.blocks = paragraph_blocks_for_text(&chapter.text, chapter.start_line, &chapter.id);
    }
    bounded
}

fn hard_bound_text(text: &str) -> Vec<String> {
    if text.len() <= MAX_TRANSLATION_UNIT_BYTES {
        return vec![text.to_string()];
    }

    let mut pieces = Vec::new();
    let mut current = String::new();
    for (atom, splittable) in structural_text_atoms(text) {
        let atom_pieces = if splittable {
            hard_bound_plain_text(atom)
        } else {
            vec![atom.to_string()]
        };
        for piece in atom_pieces {
            if piece.len() > MAX_TRANSLATION_UNIT_BYTES {
                if !current.is_empty() {
                    pieces.push(std::mem::take(&mut current));
                }
                pieces.push(piece);
            } else if current.len() + piece.len() <= MAX_TRANSLATION_UNIT_BYTES {
                current.push_str(&piece);
            } else {
                pieces.push(std::mem::take(&mut current));
                current = piece;
            }
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

fn hard_bound_plain_text(text: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut remaining = text;
    while remaining.len() > MAX_TRANSLATION_UNIT_BYTES {
        let mut boundary = MAX_TRANSLATION_UNIT_BYTES;
        while !remaining.is_char_boundary(boundary) {
            boundary -= 1;
        }
        let preferred = remaining[..boundary]
            .char_indices()
            .rev()
            .find_map(|(index, character)| {
                character
                    .is_whitespace()
                    .then_some(index + character.len_utf8())
            })
            .filter(|preferred| *preferred >= MAX_TRANSLATION_UNIT_BYTES / 2);
        let split_at = preferred.unwrap_or(boundary);
        pieces.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }
    if !remaining.is_empty() {
        pieces.push(remaining.to_string());
    }
    pieces
}

fn structural_text_atoms(text: &str) -> Vec<(&str, bool)> {
    let mut atoms = Vec::new();
    let mut fence = None;
    let mut plain_start = 0;
    let mut fence_start = None;
    let mut offset = 0;
    for segment in text.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        let before = fence;
        let fence_line = update_markdown_fence(line, &mut fence);
        let next_offset = offset + segment.len();
        if fence_line && before.is_none() && fence.is_some() {
            if plain_start < offset {
                atoms.push((&text[plain_start..offset], true));
            }
            fence_start = Some(offset);
        } else if fence_line && before.is_some() && fence.is_none() {
            let start = fence_start.take().unwrap_or(offset);
            atoms.push((&text[start..next_offset], false));
            plain_start = next_offset;
        }
        offset = next_offset;
    }
    if let Some(start) = fence_start {
        atoms.push((&text[start..], false));
    } else if plain_start < text.len() {
        atoms.push((&text[plain_start..], true));
    }
    atoms
}

fn rendered_slice_len(slice: &[&str]) -> usize {
    slice.iter().map(|line| line.len() + 1).sum()
}

fn paragraph_start_lines(lines: &[&str], start: usize, end: usize) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut in_paragraph = false;
    let mut fence = None;
    for (index, line) in lines.iter().enumerate().take(end).skip(start) {
        if update_markdown_fence(line, &mut fence) {
            if !in_paragraph {
                starts.push(index);
                in_paragraph = true;
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if line.trim().is_empty() {
            in_paragraph = false;
        } else if !in_paragraph {
            starts.push(index);
            in_paragraph = true;
        }
    }
    starts
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

fn build_chapter_from_text(
    ordinal: usize,
    title: &str,
    start_line: usize,
    end_line: usize,
    text: String,
) -> SplitChapter {
    let id = format!("chapter_{ordinal:03}");
    let blocks = paragraph_blocks_for_text(&text, start_line, &id);
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

fn paragraph_blocks_for_text(
    text: &str,
    slice_start_line: usize,
    chapter_id: &str,
) -> Vec<SplitBlock> {
    let lines = text.lines().collect::<Vec<_>>();
    paragraph_blocks(&lines, slice_start_line, chapter_id)
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
    let source_text =
        rewrite_mineru_asset_references_for_chapters(&project_root.join("source"), &source_text)?;
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

fn rewrite_mineru_asset_references_for_chapters(
    source_dir: &Path,
    markdown: &str,
) -> Result<String, String> {
    let mut directory_names = fs::read_dir(source_dir)
        .map_err(|err| err.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.ends_with(".mineru").then_some(name)
        })
        .collect::<Vec<_>>();
    directory_names.sort();
    let mut rewritten = markdown.to_string();
    for name in directory_names {
        let target = format!("../../source/{name}/");
        for opening in [format!("]({name}/"), format!("](<{name}/")] {
            let replacement = if opening.starts_with("](<") {
                format!("](<{target}")
            } else {
                format!("]({target}")
            };
            rewritten = rewritten.replace(&opening, &replacement);
        }
        for attribute in ["src", "href"] {
            for quote in ['\"', '\''] {
                let opening = format!("{attribute}={quote}{name}/");
                let replacement = format!("{attribute}={quote}{target}");
                rewritten = rewritten.replace(&opening, &replacement);
            }
        }
    }
    Ok(rewritten)
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
    #[serde(default)]
    error: Option<TranslationEngineUnitErrorReport>,
    // Terms the engine demanded of the model and did not find in the output. A
    // warning, never a failure: Chinese compounding makes false positives likely
    // enough that rejecting a chapter over one would cost more than it saves.
    // Optional so a report from an older engine still parses.
    #[serde(default)]
    glossary_violations: Vec<TranslationEngineGlossaryViolation>,
}

#[derive(Deserialize)]
struct TranslationEngineUnitErrorReport {
    code: String,
    retryable: bool,
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
    failures: Vec<BookPipelineUnitFailure>,
    // "source -> required translation", deduplicated across units and sorted, so
    // one term missed throughout a book reads as one problem to fix rather than
    // one per chapter.
    glossary_violations: BTreeSet<String>,
}

fn valid_translation_error_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn translation_failure_summary(failures: &[BookPipelineUnitFailure]) -> String {
    format!(
        "Translation failed for {} unit(s). See failed-unit details.",
        failures.len()
    )
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

/// Inject the slot's per-user runtime settings into the engine subprocess. API
/// keys come from Keychain; Qwen's optional workspace URL and web-search choice
/// come from Launcher config. Missing values preserve the engine registry and
/// root .env defaults.
fn inject_model_runtime_env(command: &mut RunnerCommand, profile_id: &str, config_id: &str) {
    let Ok(repo_root) = translation_engine_repo_root() else {
        return;
    };
    if let Some((key_env, value)) =
        crate::model_settings::resolve_credential_env(&repo_root, profile_id, config_id)
    {
        command.env.push((key_env, value));
    }
    if let Some((key_env, value)) =
        crate::model_settings::resolve_base_url_env(&repo_root, profile_id, config_id)
    {
        command.env.push((key_env, value));
    }
    if let Some((key_env, value)) =
        crate::model_settings::resolve_web_search_env(&repo_root, profile_id, config_id)
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
        env: vec![("BIBLIOSMITH_PROGRESS_SCOPE".into(), child.id.clone())],
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
    inject_model_runtime_env(&mut command, provider_profile_id, provider_config_id);
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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookPipelineOcrSampleReport {
    schema: String,
    total_pages: u32,
    sampled_pages: Vec<u32>,
    character_budget: usize,
    engines: Vec<BookPipelineOcrSampleEngine>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookPipelineOcrSampleEngine {
    engine: String,
    status: String,
    markdown_excerpt: String,
    character_count: usize,
    elapsed_ms: u64,
    error: Option<String>,
}

/// The engines a sample always compares, in the order the report lists them.
fn ocr_sample_engines() -> [&'static str; 2] {
    [OCR_SAMPLE_ENGINE_PADDLEOCR, OCR_SAMPLE_ENGINE_MINERU]
}

/// The local file a source reference points at, or None when it names no file
/// on this disk.
///
/// `zotero_source_ref` builds these, and both of its shapes need handling: it
/// appends `#source_md5=<fingerprint>` to the attachment's storage path, and it
/// falls back to a `zotero://attachment/<key>` URI when Zotero reported no path
/// at all. Left alone, the fingerprint becomes part of the file extension --
/// `book.pdf#source_md5=abc` has extension `pdf#source_md5=abc` -- so every
/// real Zotero book reads as "not a PDF". Stripped by exact suffix rather than
/// by splitting on `#`, because `#` is legal in a filename.
fn ocr_sample_local_path(source_ref: &str) -> Option<PathBuf> {
    let path = source_ref
        .rsplit_once("#source_md5=")
        .map_or(source_ref, |(base, _)| base)
        .trim();
    if path.is_empty() || path.starts_with("zotero://") {
        return None;
    }
    Some(PathBuf::from(path))
}

/// The PDF the sample reads. Zotero attachment children carry the storage path
/// their route was built from, and local-folder route items name the file
/// directly; nothing here goes looking on disk for a file the state does not
/// already point at.
fn ocr_sample_source_pdf(child: &BookPipelineChildJob) -> Result<PathBuf, String> {
    let references = std::iter::once(child.source.path.as_deref())
        .chain(
            child
                .route
                .iter()
                .map(|route| Some(route.source_ref.as_str())),
        )
        .flatten()
        .collect::<Vec<_>>();
    let path = references
        .iter()
        .filter_map(|source_ref| ocr_sample_local_path(source_ref))
        .find(|path| is_pdf_path(path));
    let Some(path) = path else {
        // An attachment Zotero knows about but has no local file for reads as a
        // sync problem, not as a missing path the user was never shown.
        if references
            .iter()
            .any(|source_ref| source_ref.starts_with("zotero://"))
        {
            return Err(
                "This book's PDF is not stored locally. Sync the Zotero attachment first.".into(),
            );
        }
        return Err("This book has no local PDF to sample.".into());
    };
    if !path.is_file() {
        return Err("The source PDF for this book is missing.".into());
    }
    Ok(path)
}

fn is_pdf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

/// Per-child, so two books sampled from one collection cannot overwrite each
/// other's report or scratch pages.
fn ocr_sample_dir(store: &dyn BookPipelineStateStore, job_id: &str, child_id: &str) -> PathBuf {
    store
        .job_output_dir(job_id)
        .join(clean_path_component(child_id))
        .join("qa")
        .join(OCR_SAMPLE_DIR_NAME)
}

fn build_ocr_sample_command(
    manifest_path: &Path,
    sample_dir: &Path,
) -> Result<RunnerCommand, String> {
    let root = book_ocr_conversion_root();
    let script = root.join("sample_compare.py");
    if !script.is_file() {
        return Err(format!(
            "OCR sample compare client not found at {}",
            display_path(&script)
        ));
    }
    let mut args = ocr_python_args(&script);
    args.extend(["--manifest".into(), display_path(manifest_path)]);
    Ok(RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: OCR_SAMPLE_COMPARE_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args,
        env: Vec::new(),
        cwd: Some(root),
        output_dir: sample_dir.to_path_buf(),
        // Only the fake-command runner reads this, and the OCR sample is always
        // a real process.
        attempts: 0,
        accepted_exit_codes: vec![0],
    })
}

fn validate_ocr_sample_report(
    report: &BookPipelineOcrSampleReport,
    requested_pages: u32,
) -> Result<(), String> {
    if report.schema != OCR_SAMPLE_COMPARE_REPORT_SCHEMA {
        return Err("OCR sample report has an unsupported schema.".into());
    }
    if report.total_pages < 3 {
        return Err("OCR sample report claims a book with no interior pages.".into());
    }
    if report.sampled_pages.is_empty() || report.sampled_pages.len() > requested_pages as usize {
        return Err("OCR sample report has an unexpected sampled page count.".into());
    }
    let mut previous = 0;
    for page in &report.sampled_pages {
        // Strictly increasing, and never the endpoints: a report that showed
        // the cover would be evidence of the least representative page in the
        // book, which is the whole reason the sampler skips them.
        if *page <= previous || *page < 2 || *page >= report.total_pages {
            return Err("OCR sample report sampled a cover, endpoint or unordered page.".into());
        }
        previous = *page;
    }
    let expected = ocr_sample_engines();
    if report.engines.len() != expected.len()
        || report
            .engines
            .iter()
            .zip(expected)
            .any(|(result, engine)| result.engine != engine)
    {
        return Err("OCR sample report does not compare both engines.".into());
    }
    for result in &report.engines {
        match result.status.as_str() {
            "ok" => {
                if result.error.is_some() {
                    return Err("OCR sample report marked a successful engine as failed.".into());
                }
            }
            "failed" => {
                if non_empty(result.error.as_deref()).is_none() {
                    return Err("OCR sample report has a failure with no reason.".into());
                }
            }
            _ => return Err("OCR sample report has an invalid engine status.".into()),
        }
        if result.markdown_excerpt.chars().count() > report.character_budget {
            return Err("OCR sample report exceeded its excerpt budget.".into());
        }
    }
    if report
        .engines
        .iter()
        .all(|result| result.status == "failed")
    {
        return Err(format!(
            "Both OCR engines failed on the sampled pages: {}",
            report
                .engines
                .iter()
                .filter_map(|result| result
                    .error
                    .as_deref()
                    .map(|error| format!("{}: {error}", result.engine)))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    Ok(())
}

/// Sample both OCR engines over the same interior pages so the route can be
/// chosen on evidence. Deliberately free of side effects on the route itself:
/// picking a winner is a separate, explicit action, exactly as running a
/// translation sample does not adopt the provider it previewed.
fn run_ocr_sample_with_executor(
    store: &dyn BookPipelineStateStore,
    job_id: &str,
    child_id: Option<&str>,
    sample_pages: u32,
    executor: &dyn RunnerCommandExecutor,
) -> Result<BookPipelineJob, String> {
    if !(1..=OCR_SAMPLE_MAX_PAGES).contains(&sample_pages) {
        return Err(format!(
            "OCR sampling takes between 1 and {OCR_SAMPLE_MAX_PAGES} pages."
        ));
    }

    let mut state = store.load()?;
    let job_index = find_job_index(&state, job_id)?;
    let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
    let child = &state.jobs[job_index].children[child_index];
    let child_id = child.id.clone();
    // Sampling is a pre-conversion decision. Once extraction is under way the
    // engine has already been chosen and the comparison would either race the
    // runner or answer a question that is no longer open.
    let extract_status = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .map(|stage| stage.status.clone())
        .ok_or_else(|| "This book has no extract stage to sample.".to_string())?;
    if matches!(extract_status.as_str(), STATUS_RUNNING | STATUS_COMPLETED) {
        return Err("OCR samples run only before conversion starts.".into());
    }
    let source_pdf = ocr_sample_source_pdf(child)?;

    let sample_dir = ocr_sample_dir(store, job_id, &child_id);
    fs::create_dir_all(&sample_dir).map_err(|err| err.to_string())?;
    // Each run writes its own report file rather than overwriting a fixed name.
    // A re-sample that fails would otherwise have already clobbered the report
    // the previous run registered, leaving a good artifact record pointing at
    // content whose digest no longer matches -- the last working comparison
    // destroyed by the attempt to replace it.
    static OCR_SAMPLE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let run_tag = format!(
        "{}-{}",
        std::process::id(),
        OCR_SAMPLE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let report_name = format!("report-{run_tag}.json");
    let report_path = sample_dir.join(&report_name);
    let manifest = serde_json::json!({
        "schema": OCR_SAMPLE_COMPARE_SCHEMA,
        "projectRoot": display_path(&sample_dir),
        "sourcePdfPath": display_path(&source_pdf),
        "reportPath": report_name,
        "workDir": "work",
        "samplePages": sample_pages,
        "characterBudget": OCR_SAMPLE_CHARACTER_BUDGET,
        "engines": ocr_sample_engines(),
    });
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())? + "\n";
    let manifest_path = sample_dir.join(format!("manifest-{run_tag}.json"));
    fs::write(&manifest_path, manifest_json).map_err(|err| err.to_string())?;

    let mut command = build_ocr_sample_command(&manifest_path, &sample_dir)?;
    inject_ocr_credentials(&mut command);
    let command_result = match executor.execute(&command) {
        Ok(result) => {
            fs::remove_file(&manifest_path).map_err(|err| {
                format!("Failed to remove the temporary OCR sample manifest: {err}")
            })?;
            result
        }
        Err(err) => {
            let _ = fs::remove_file(&manifest_path);
            return Err(err);
        }
    };
    // Read from disk rather than stdout: the report is a durable artifact the
    // UI reads back long after this process is gone, so the file is the source
    // of truth and stdout is only progress.
    //
    // A report this run wrote but cannot register is deleted on the way out.
    // The names are per-run, so leaving them would pile up one file per failed
    // attempt -- and "both engines failed" is the ordinary first-run outcome on
    // a machine with no tokens configured yet.
    let report_json = fs::read_to_string(&report_path)
        .map_err(|err| format!("OCR sample report was not written: {err}"))?;
    let accept = || -> Result<BookPipelineOcrSampleReport, String> {
        let report: BookPipelineOcrSampleReport = serde_json::from_str(&report_json)
            .map_err(|err| format!("OCR sample compare returned invalid report JSON: {err}"))?;
        validate_ocr_sample_report(&report, sample_pages)?;
        Ok(report)
    };
    let report = match accept() {
        Ok(report) => report,
        Err(err) => {
            let _ = fs::remove_file(&report_path);
            return Err(err);
        }
    };
    let report_sha256 = sha256_str(&report_json);

    let job = &mut state.jobs[job_index];
    let previous_report_path = job.children[child_index]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "ocr_sample_report")
        .map(|artifact| PathBuf::from(&artifact.path));
    replace_stage_artifacts(
        &mut job.children[child_index].artifacts,
        &["ocr_sample_report"],
        vec![BookPipelineArtifact {
            kind: "ocr_sample_report".into(),
            path: display_path(&report_path),
            sha256: Some(report_sha256),
            producer_stage: Some("extract".into()),
            ..BookPipelineArtifact::default()
        }],
    );
    job.current_step = "Compared OCR engines on sampled pages".into();
    job.log_summary
        .extend(redact_log_lines(&command_result.log_summary));
    job.log_summary.push(format!(
        "Sampled {} page(s) through {}",
        report.sampled_pages.len(),
        report
            .engines
            .iter()
            .map(|result| format!("{} ({})", result.engine, result.status))
            .collect::<Vec<_>>()
            .join(" and ")
    ));
    job.log_summary = trim_log_summary(&job.log_summary);
    job.updated_at = now_label();
    derive_job(job);
    let result = job.clone();
    store.save(&state)?;
    // Only now that the new report is the registered one. Confined to this
    // child's own sample directory so a hand-edited artifact path cannot turn
    // a re-sample into a delete somewhere else.
    if let Some(previous_path) = previous_report_path.filter(|previous_path| {
        previous_path != &report_path && previous_path.parent() == Some(sample_dir.as_path())
    }) {
        let _ = fs::remove_file(previous_path);
    }
    Ok(result)
}

fn read_ocr_sample_report(
    job: &BookPipelineJob,
    child_id: &str,
    sample_dir: &Path,
) -> Result<BookPipelineOcrSampleReport, String> {
    let child = job
        .children
        .iter()
        .find(|child| child.id == child_id)
        .ok_or_else(|| "Book Pipeline child job not found.".to_string())?;
    let artifact = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "ocr_sample_report")
        .ok_or_else(|| "OCR sample report is not registered.".to_string())?;
    let path = fs::canonicalize(&artifact.path)
        .map_err(|_| "OCR sample report file is missing.".to_string())?;
    // The state file names the path, so the read is confined to the directory
    // this child's samples are written to. Without it a tampered artifact
    // record would make the reader hand any readable file to the UI.
    let canonical_sample_dir =
        fs::canonicalize(sample_dir).map_err(|_| "OCR sample directory is missing.".to_string())?;
    if !path.starts_with(&canonical_sample_dir) {
        return Err("OCR sample report is outside the job's sample directory.".into());
    }
    let expected_sha256 = artifact
        .sha256
        .as_deref()
        .ok_or_else(|| "OCR sample report has no SHA-256.".to_string())?;
    if sha256_file(&path)? != expected_sha256 {
        return Err("OCR sample report changed after registration.".into());
    }
    let report: BookPipelineOcrSampleReport =
        serde_json::from_str(&fs::read_to_string(path).map_err(|err| err.to_string())?)
            .map_err(|err| format!("OCR sample report is invalid: {err}"))?;
    validate_ocr_sample_report(&report, OCR_SAMPLE_MAX_PAGES)?;
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
    let mut failures = Vec::new();
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
                if unit.error.is_some() {
                    return Err(format!(
                        "Completed translation unit {} reported an error",
                        unit.unit_id
                    ));
                }
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
                let error = unit.error.as_ref().ok_or_else(|| {
                    format!("Failed translation unit {} has no error", unit.unit_id)
                })?;
                if !valid_translation_error_code(&error.code) {
                    return Err(format!(
                        "Failed translation unit {} has an invalid error code",
                        unit.unit_id
                    ));
                }
                failures.push(BookPipelineUnitFailure {
                    unit_id: unit.unit_id.clone(),
                    code: error.code.clone(),
                    retryable: error.retryable,
                });
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
        failures,
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
    inject_model_runtime_env(
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
            failures: validated.failures.clone(),
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
            failures: validated.failures.clone(),
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
    if !validated.failures.is_empty() {
        log_summary.push(translation_failure_summary(&validated.failures));
    }
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
    let error =
        (!validated.failures.is_empty()).then(|| translation_failure_summary(&validated.failures));
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
                let is_math_superscript = open > 0
                    && text[..open].ends_with('^')
                    && is_inside_latex_math(text, open, index + character.len_utf8());
                if !is_math_superscript
                    && token.len() > 2
                    && !token.chars().any(char::is_whitespace)
                {
                    *tokens.entry(token.to_string()).or_insert(0) += 1;
                }
                start = None;
            }
            _ => {}
        }
    }
    tokens
}

fn is_inside_latex_math(text: &str, start: usize, end: usize) -> bool {
    let before = &text[..start];
    let after = &text[end..];
    [(r"\(", r"\)"), (r"\[", r"\]")]
        .into_iter()
        .any(|(opening, closing)| {
            let Some(last_opening) = before.rfind(opening) else {
                return false;
            };
            let last_closing = before.rfind(closing);
            last_closing.is_none_or(|position| position < last_opening) && after.contains(closing)
        })
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
        decided_at: now_label(),
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
        decided_at: now_label(),
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
        .join("tools")
        .join("bibliosmith-launcher")
        .join("source")
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

    let standard_epub_path = reading_dir.join("book.epub");
    let html_dir = reading_dir.join("html");
    if wants_html || wants_epub {
        let script_path = prepare_reading_builder(&project_root)?;
        input_hashes.insert("buildScriptSha256".into(), sha256_file(&script_path)?);
        command_results.push(executor.execute(&build_reading_command(child, &script_path)?)?);

        if wants_html {
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
        } else if html_dir.is_dir() {
            fs::remove_dir_all(&html_dir).map_err(|err| err.to_string())?;
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
        if html_dir.is_dir() {
            fs::remove_dir_all(&html_dir).map_err(|err| err.to_string())?;
        }
        let work_dir = project_root.join("output").join("epub_work");
        if work_dir.is_dir() {
            fs::remove_dir_all(work_dir).map_err(|err| err.to_string())?;
        }
    }

    let bilingual_epub_path = reading_dir.join("book_bilingual.epub");
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
    let status_updates = [
        ("- split:", "- split: passed".to_string()),
        ("- translation:", "- translation: passed".to_string()),
        ("- expert QA:", "- expert QA: passed".to_string()),
        (
            "- reading output:",
            format!("- reading output: {reading_status}"),
        ),
        ("- EPUBCheck:", format!("- EPUBCheck: {reading_status}")),
    ];
    let mut replaced_statuses = vec![false; status_updates.len()];
    let mut lines = Vec::new();
    let mut in_previous_section = false;
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
        if let Some((index, (_, replacement))) = status_updates
            .iter()
            .enumerate()
            .find(|(_, (prefix, _))| line.starts_with(prefix))
        {
            lines.push(replacement.clone());
            replaced_statuses[index] = true;
        } else {
            lines.push(line.to_string());
        }
    }
    for ((_, replacement), replaced) in status_updates.iter().zip(replaced_statuses) {
        if !replaced {
            lines.push(replacement.clone());
        }
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
    let reading_dir = project_root.join("output").join("reading");
    let expected_source_epub = reading_dir.join("book.epub");
    let output_epub = reading_dir.join("book_digest.epub");
    if source_epub != expected_source_epub {
        return Err(format!(
            "Digest source EPUB must be output/reading/book.epub, found {}",
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
        ("source_epub", serde_json::json!("output/reading/book.epub")),
        (
            "output_epub",
            serde_json::json!("output/reading/book_digest.epub"),
        ),
        ("title", serde_json::json!(title)),
        ("language", serde_json::json!(language)),
    ] {
        config_object.insert(key.into(), value);
    }
    let config_text = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())? + "\n";
    fs::write(&config_path, &config_text).map_err(|err| err.to_string())?;

    let outputs = [
        output_epub.clone(),
        project_root.join("output/reading/digest/digest.xhtml"),
        project_root.join("output/reading/digest/knowledge_map.svg"),
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
            != Some("output/reading/book.epub")
        || report
            .get("output_epub")
            .and_then(serde_json::Value::as_str)
            != Some("output/reading/book_digest.epub")
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
    policy_changed: bool,
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
    let policy_changed = split
        .input_hashes
        .get("splitPolicyVersion")
        .map(String::as_str)
        != Some(SPLIT_POLICY_VERSION);
    let source_changed = split.input_hashes.get("sourceMarkdownSha256") != Some(&current);

    let split_order = ordered_stage_index("split").unwrap_or(0);
    let downstream = || {
        child.stages.iter().filter(|stage| {
            ordered_stage_index(&stage.stage_id).is_some_and(|order| order > split_order)
        })
    };
    let downstream_running =
        downstream().any(|stage| stage.status == STATUS_RUNNING || is_agent_handoff_waiting(stage));
    if policy_changed && downstream_running {
        return Ok(None);
    }
    if source_changed && downstream_running {
        return Ok(None);
    }
    let downstream_committed = downstream().any(|stage| stage.status == STATUS_COMPLETED);

    if split.status == STATUS_BLOCKED {
        return Ok(invalidate_downstream.then_some(SplitFreshnessChange {
            action: SplitFreshnessAction::InvalidateDownstreamAndRerun,
            new_source_hash: current,
            policy_changed,
            stop_after: false,
        }));
    }
    if !policy_changed && !source_changed {
        return Ok(None);
    }
    let policy_upgrade_only = policy_changed && !source_changed;
    let action = if policy_upgrade_only || invalidate_downstream {
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
        policy_changed,
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

fn invalidate_all_downstream(child: &mut BookPipelineChildJob, after_stage: &str) {
    let after_order = ordered_stage_index(after_stage).unwrap_or(0);
    child.artifacts.retain(|artifact| {
        let producer_stage = if artifact.producer.stage_id.is_empty() {
            artifact.producer_stage.as_deref()
        } else {
            Some(artifact.producer.stage_id.as_str())
        };
        !producer_stage.is_some_and(|stage_id| {
            ordered_stage_index(stage_id).is_some_and(|order| order > after_order)
        })
    });
    for stage in &mut child.stages {
        let is_downstream =
            ordered_stage_index(&stage.stage_id).is_some_and(|order| order > after_order);
        if is_downstream && stage.status != STATUS_SKIPPED {
            stage.input_hashes.clear();
            stage
                .input_hashes
                .insert("splitPolicyVersion".into(), SPLIT_POLICY_VERSION.into());
            stage.status = STATUS_PENDING.into();
            stage.attempt = 0;
            stage.error = None;
            stage.safe_error = None;
            stage.started_at = None;
            stage.finished_at = None;
            stage.artifact_ids.clear();
            stage.unit_summary = None;
            stage.approval_id = None;
            stage.approval_request = None;
            stage.execution_owner = None;
            stage.index_evidence = None;
            stage.give_up_reason = None;
            stage.next_retry_at = None;
        }
    }
}

/// Repair state written before downstream invalidation cleared unit-scoped
/// evidence. The prepared task manifests are the active chapter set; records
/// for any other unit belong to an older split and must not reach QA or build.
fn reconcile_prepared_unit_scope(child: &mut BookPipelineChildJob) -> bool {
    let active_unit_ids = child
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "translation_task_manifest")
        .filter_map(|artifact| {
            artifact.producer.unit_id.clone().or_else(|| {
                Path::new(&artifact.path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::to_string)
            })
        })
        .collect::<BTreeSet<_>>();
    if active_unit_ids.is_empty() {
        return false;
    }
    let translate_order = ordered_stage_index("translate").unwrap_or(0);
    let pending_stages = child
        .stages
        .iter()
        .filter(|stage| {
            stage.status == STATUS_PENDING
                && ordered_stage_index(&stage.stage_id)
                    .is_some_and(|order| order >= translate_order)
        })
        .map(|stage| stage.stage_id.clone())
        .collect::<BTreeSet<_>>();
    let original_artifact_count = child.artifacts.len();
    child.artifacts.retain(|artifact| {
        let producer_stage = if artifact.producer.stage_id.is_empty() {
            artifact.producer_stage.as_deref()
        } else {
            Some(artifact.producer.stage_id.as_str())
        };
        if producer_stage.is_some_and(|stage_id| pending_stages.contains(stage_id)) {
            return false;
        }
        if producer_stage == Some("translate") {
            return artifact
                .producer
                .unit_id
                .as_ref()
                .cloned()
                // Compatibility records may predate producer.unitId.
                // Translation artifact file names still carry the unit id.
                // Keep the run manifest, which has no unit id.
                .or_else(|| translation_artifact_unit_id(artifact))
                .is_none_or(|unit_id| active_unit_ids.contains(&unit_id));
        }
        true
    });
    let mut changed = child.artifacts.len() != original_artifact_count;
    for stage in &mut child.stages {
        if !pending_stages.contains(&stage.stage_id) {
            continue;
        }
        changed |= stage.attempt != 0
            || stage.input_hashes.len() != 1
            || stage
                .input_hashes
                .get("splitPolicyVersion")
                .map(String::as_str)
                != Some(SPLIT_POLICY_VERSION)
            || stage.error.is_some()
            || stage.safe_error.is_some()
            || stage.started_at.is_some()
            || stage.finished_at.is_some()
            || !stage.artifact_ids.is_empty()
            || stage.unit_summary.is_some()
            || stage.approval_id.is_some()
            || stage.approval_request.is_some()
            || stage.execution_owner.is_some()
            || stage.index_evidence.is_some()
            || stage.give_up_reason.is_some()
            || stage.next_retry_at.is_some();
        stage.input_hashes.clear();
        stage
            .input_hashes
            .insert("splitPolicyVersion".into(), SPLIT_POLICY_VERSION.into());
        stage.attempt = 0;
        stage.error = None;
        stage.safe_error = None;
        stage.started_at = None;
        stage.finished_at = None;
        stage.artifact_ids.clear();
        stage.unit_summary = None;
        stage.approval_id = None;
        stage.approval_request = None;
        stage.execution_owner = None;
        stage.index_evidence = None;
        stage.give_up_reason = None;
        stage.next_retry_at = None;
    }
    changed
}

fn apply_split_freshness(child: &mut BookPipelineChildJob, change: &SplitFreshnessChange) {
    // Blocking and explicit invalidation both roll back completed downstream
    // stages so the re-blocked/re-readied split stays a valid ordered state; the
    // rollback is a no-op when nothing downstream was committed. Generated files
    // stay on disk and are overwritten only when split actually re-runs.
    if matches!(
        change.action,
        SplitFreshnessAction::Block | SplitFreshnessAction::InvalidateDownstreamAndRerun
    ) {
        invalidate_all_downstream(child, "split");
    } else {
        invalidate_completed_downstream(child, "split");
    }
    if let Some(split) = stage_mut(child, "split") {
        split.input_hashes.insert(
            "sourceMarkdownSha256".into(),
            change.new_source_hash.clone(),
        );
        if change.policy_changed
            && matches!(
                change.action,
                SplitFreshnessAction::InvalidateDownstreamAndRerun
            )
        {
            split
                .input_hashes
                .insert("splitPolicyVersion".into(), SPLIT_POLICY_VERSION.into());
        }
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
            .position(|child| child.id == child_id && child.removed_at.is_none())
            .ok_or_else(|| "Book Pipeline child not found.".to_string()),
        None => job
            .children
            .iter()
            .position(|child| {
                child.removed_at.is_none() && deterministic_stage_to_run(child).is_some()
            })
            .or_else(|| {
                job.children
                    .iter()
                    .position(|child| child.removed_at.is_none())
            })
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

    // Phase 0.125: self-heal legacy jobs whose source was already re-split by a
    // build that reset statuses but kept old unit evidence. This is intentionally
    // driven by the prepared task manifests, not by filenames left on disk.
    if !retrying_stage {
        let mut state = store.load()?;
        let job_index = find_job_index(&state, job_id)?;
        let child_index = locate_child_index(&state.jobs[job_index], child_id)?;
        if reconcile_prepared_unit_scope(&mut state.jobs[job_index].children[child_index]) {
            state.jobs[job_index].log_summary.push(
                "Reconciled downstream records to the prepared translation unit scope".into(),
            );
            state.jobs[job_index].log_summary =
                trim_log_summary(&state.jobs[job_index].log_summary);
            state.jobs[job_index].updated_at = now_label();
            derive_job(&mut state.jobs[job_index]);
            store.save(&state)?;
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
mod tests;
