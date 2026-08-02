//! Forward migration of state written by older builds. Every function here reads a shape this build no longer produces -- flat single-item jobs, collection execution status strings, jobs predating the child-job model -- and rewrites it into the current schema on load. Nothing outside a migration path may call these; they exist to be deleted once the oldest supported state file is gone.

use super::*;

pub(crate) fn migrate_legacy_state(state: &mut BookPipelineState) {
    state.schema_version = STATE_SCHEMA_VERSION.into();
    for job in &mut state.jobs {
        migrate_legacy_job(job);
    }
    derive_state(state);
}

pub(crate) fn migrate_legacy_job(job: &mut BookPipelineJob) {
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
    let mode = job.mode.clone();
    for child in &mut job.children {
        ensure_item_index_stage(child, &mode);
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

pub(crate) fn collection_children_from_job(job: &BookPipelineJob) -> Vec<BookPipelineChildJob> {
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

pub(crate) fn legacy_route_from_collection_item(
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

pub(crate) fn collection_child_from_route(
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
        removed_at: None,
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

pub(crate) fn migrate_legacy_collection_execution_status(
    job: &mut BookPipelineJob,
    legacy_status: &str,
) {
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

pub(crate) fn ordered_child_stage_ids(
    mode: &str,
    legacy_translation_state: bool,
) -> Vec<&'static str> {
    // Ahead of the legacy check on purpose: this mode postdates every state file
    // a legacy flag can describe, so a job carrying it is never mid-translation.
    // Spelled out rather than left to fall through the `else` below, which now
    // exists only to keep pre-retirement `conversion_only` jobs readable.
    if mode == MODE_LAYOUT_PRESERVING {
        return vec!["route", "extract"];
    }
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
        // The retired conversion-only shape. Only jobs stored before the mode
        // was retired reach this arm; enqueue refuses the mode outright, so no
        // new job is given a pipeline that stops short of translation.
        vec!["route", "extract", "index"]
    }
}

pub(crate) fn ensure_item_index_stage(child: &mut BookPipelineChildJob, mode: &str) {
    // The item index is built from the Markdown the reflow track produces. The
    // layout track never produces any, and its stage list is exactly two long --
    // inserting a third here would leave every job stalled on a stage with no
    // input.
    if mode == MODE_LAYOUT_PRESERVING {
        return;
    }
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

pub(crate) fn child_requires_item_index(source: &BookPipelineSource) -> bool {
    source.kind == "zotero_attachment"
}

pub(crate) fn stage_contract_version(stage_id: &str) -> &'static str {
    if stage_id == "index" {
        ITEM_INDEX_CONTRACT_VERSION
    } else {
        JOB_SCHEMA_VERSION
    }
}

pub(crate) fn legacy_child_from_job(
    job: &BookPipelineJob,
    legacy_status: &str,
) -> BookPipelineChildJob {
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
        removed_at: None,
    }
}
