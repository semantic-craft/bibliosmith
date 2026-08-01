import type {
  BookPipelineApprovalReference,
  BookPipelineArtifact,
  BookPipelineChildJob,
  BookPipelineJob,
  BookPipelineRouteItem,
  BookPipelineSource,
  BookPipelineStage,
  BookPipelineOutputFormat,
  BookPipelineOperationProgress,
  ModelSlotView,
} from "../types";
import type { PipelineCopy } from "./copy";
import { MODEL_BRANDS } from "../pages/settings/modelCatalog";

export type PipelineBusy =
  | "loading"
  | "preview"
  | "queue"
  | "run"
  | "retry"
  | "delete"
  | "advance"
  | "sample"
  | "gateApproval"
  | "customInstructions"
  | "routeOverride"
  | "readerEvidence"
  | "folder"
  | "markdown"
  | "zotero"
  | "handoff"
  | "cleanup"
  | "cleanupApproval"
  | "open"
  | "diagnostic"
  | null;

export function unitFailureLabel(code: string, copy: PipelineCopy): string {
  switch (code) {
    case "provider_timeout":
      return copy.failureProviderTimeout;
    case "provider_http_5xx":
      return copy.failureProviderServer;
    case "provider_rate_limited":
      return copy.failureProviderRateLimit;
    case "provider_transient_error":
    case "provider_unavailable":
      return copy.failureProviderUnavailable;
    case "translation_structure_invalid":
    case "translation_incomplete":
      return copy.failureStructureInvalid;
    case "provider_fatal_error":
      return copy.failureProviderFatal;
    default:
      return code;
  }
}

export function translationFailureSummary(
  stage: BookPipelineStage | null | undefined,
  copy: PipelineCopy,
): string | null {
  const failures = stage?.unitSummary?.failures;
  if (!failures?.length) return null;
  const counts = new Map<string, number>();
  for (const failure of failures) {
    const label = unitFailureLabel(failure.code, copy);
    counts.set(label, (counts.get(label) || 0) + 1);
  }
  const reasons = [...counts.entries()]
    .map(([label, count]) => `${label} ${count}`)
    .join(copy.failureReasonSeparator);
  return copy.translationFailureSummary(failures.length, reasons);
}

/** Conversion route override a user can pick per route item in the wizard. */
export type RouteOverride = "auto" | "direct" | "paddle" | "mineru" | "keep";

export type PipelineDraft = {
  sourceKind: BookPipelineSource["kind"];
  // The union is the backend contract and keeps its three arms, but the input
  // island never sets anything other than convert_then_translate: conversion
  // without translation left the UI with the mode selector, and translate_only
  // waits on the EPUB input route.
  mode: "conversion_only" | "convert_then_translate" | "translate_only";
  localPdfFolder: string;
  localPdfTitle: string;
  translationMode: "fast" | "expert";
  // A provider slot: profile picks the brand/client, config the billing plan.
  // Open strings, not a union, because the set of slots lives in the engine
  // registry and the models settings, not in this type.
  providerProfileId: string;
  providerConfigId: string;
  secondPassEnabled: boolean;
  textCleanup: boolean;
  digestMode: boolean;
  outputFormats: BookPipelineOutputFormat[];
  zoteroSelector: string;
  hasPaddleocrCredentials: boolean;
  hasMineruCredentials: boolean;
};

// The island offers a local PDF folder or a Zotero item, so the default must be
// one of them; "fake" would be selectable-by-omission and launchable.
export const defaultPipelineDraft: PipelineDraft = {
  sourceKind: "local_pdf_folder",
  mode: "convert_then_translate",
  localPdfFolder: "",
  localPdfTitle: "Local PDF folder",
  translationMode: "fast",
  providerProfileId: "openai-compatible",
  providerConfigId: "openai-default",
  secondPassEnabled: true,
  textCleanup: false,
  digestMode: false,
  outputFormats: ["md", "html", "epub"],
  zoteroSelector: "",
  hasPaddleocrCredentials: false,
  hasMineruCredentials: true,
};

/* ---------- Stage contract ---------- */

/**
 * Every stage id the backend can persist, in its own order. Must stay in step
 * with `ordered_stage_index` in book_pipeline.rs; `discover` is the collection
 * parent's own single stage and has no slot in that child-order list, so it
 * leads here. A stage missing from this list sorts to the bottom of the Stages
 * tab and reads as the last thing that happened, which is how `index` used to
 * render after `validate_reading`.
 */
export const PIPELINE_STAGE_ORDER = [
  "discover",
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
] as const;

export type PipelineStageId = (typeof PIPELINE_STAGE_ORDER)[number];

export const GATE_STAGE_IDS = new Set(["approve_translation", "approve_promotion"]);

// The config_id each provider profile resolves to, derived from the same catalog
// Settings and the new-job wizard render. It used to be a hand-written map of
// three brands while the catalog carried six, so the per-book drawer silently
// offered half the providers the user could actually configure. A brand's first
// slot is its default config (Qwen and MiMo bill two ways and list two slots),
// which is only a fallback now that every picker offers the slots themselves.
const PROVIDER_DEFAULT_CONFIG: Record<string, string> = Object.fromEntries(
  MODEL_BRANDS.flatMap((brand) =>
    brand.slots[0] ? [[brand.profileId, brand.slots[0].configId] as const] : [],
  ),
);

// A persisted job's translationProfileId is a loose string (it may predate a
// profile being renamed), so resolve it through this rather than indexing the
// map directly.
export function providerDefaultConfig(profileId: string): string {
  return PROVIDER_DEFAULT_CONFIG[profileId] ?? "default";
}

// Whether the wizard's provider picker decides anything for this draft. App.tsx
// builds the translation intent with the "expert-agent" profile for expert mode
// and never reaches the translate stage for a conversion-only job, so in both
// cases the picked slot is inert and an unconfigured one is not a problem.
export function providerSelectionApplies(draft: PipelineDraft): boolean {
  return draft.mode !== "conversion_only" && draft.translationMode === "fast";
}

// Whether the draft's provider slot has no stored key, which is what makes a job
// run OCR for a quarter of an hour and then die on provider auth. The catalog is
// the backend's answer (it reads the Keychain per slot); an empty one means the
// best-effort read failed, and reporting that as "nothing is configured" would
// hold back every job on a transient error, so unknown counts as fine.
export function providerCredentialMissing(
  draft: PipelineDraft,
  slots: ModelSlotView[],
): boolean {
  if (!providerSelectionApplies(draft)) return false;
  if (!slots.length) return false;
  return !slots.some(
    (slot) =>
      slot.profileId === draft.providerProfileId &&
      slot.configId === draft.providerConfigId &&
      slot.configured,
  );
}

// Slot keys the catalog reports a stored key for, for labelling the picker.
export function configuredSlotKeys(slots: ModelSlotView[]): Set<string> {
  return new Set(
    slots.filter((slot) => slot.configured).map((slot) => `${slot.profileId}:${slot.configId}`),
  );
}

// Typed as a total map over the contract, so adding a stage without giving it a
// label is a compile error rather than a raw untranslated id in the timeline.
export function stageLabel(stageId: string, copy: PipelineCopy): string {
  const labels: Record<PipelineStageId, string> = {
    discover: copy.stageDiscover,
    route: copy.stageRoute,
    extract: copy.stageExtract,
    index: copy.stageIndex,
    handoff: copy.stageHandoff,
    split: copy.stageSplit,
    prepare: copy.stagePrepare,
    approve_translation: copy.stageApproveTranslation,
    translate: copy.stageTranslate,
    expert_qa: copy.stageExpertQa,
    approve_promotion: copy.stageApprovePromotion,
    promote: copy.stagePromote,
    build_reading: copy.stageBuildReading,
    validate_reading: copy.stageValidateReading,
    build_digest: copy.stageBuildDigest,
  };
  return (labels as Record<string, string>)[stageId] ?? stageId.replaceAll("_", " ");
}

/** Canonical stage ordering: known contract stages first, extras appended in place. */
export function orderedStages(stages: BookPipelineStage[]): BookPipelineStage[] {
  const rank = (stageId: string) => {
    const index = (PIPELINE_STAGE_ORDER as readonly string[]).indexOf(stageId);
    return index === -1 ? PIPELINE_STAGE_ORDER.length : index;
  };
  return [...stages].sort((a, b) => rank(a.stageId) - rank(b.stageId));
}

/* ---------- Four-step beginner view ---------- */

/**
 * The beginner-facing shelf folds the 12 internal stages into four steps.
 * Stage ids missing from a job (e.g. translation stages on conversion_only)
 * leave that step "none" so the strip stays honest about what this job does.
 */
export const PHASES = [
  { key: "convert", stageIds: ["discover", "route", "extract", "index", "handoff", "split"] },
  {
    key: "translate",
    stageIds: ["prepare", "approve_translation", "translate", "expert_qa", "approve_promotion", "promote"],
  },
  // build_digest is retired and no longer requested, but it stays in the stage
  // contract, so it stays grouped: an ungrouped stage is invisible below.
  { key: "build", stageIds: ["build_reading", "validate_reading", "build_digest"] },
] as const satisfies readonly { key: string; stageIds: readonly PipelineStageId[] }[];

type AssertNever<T extends never> = T;

/**
 * A stage left out of every phase contributes to no phase, so its failure turns
 * no circle red and the shelf reports a book as fine while it is stuck —
 * exactly what `index` did. Make that omission a type error instead.
 */
export type PhasesCoverEveryStage = AssertNever<
  Exclude<PipelineStageId, (typeof PHASES)[number]["stageIds"][number]>
>;

export type PhaseState = "done" | "current" | "gate" | "error" | "todo" | "none";

const GATE_PENDING_STATUSES = new Set(["waiting_for_approval", "ready"]);

export function phaseStates(unit: BookUnit): PhaseState[] {
  const stages = focusStages(unit);
  const grouped = PHASES.map((step) =>
    stages.filter((stage) => (step.stageIds as readonly string[]).includes(stage.stageId)),
  );
  const states: PhaseState[] = grouped.map((members) => {
    if (!members.length) return "none";
    if (
      members.some(
        (stage) => stage.status === "failed" || (stage.status === "blocked" && !GATE_STAGE_IDS.has(stage.stageId)),
      )
    ) {
      return "error";
    }
    if (members.some((stage) => GATE_STAGE_IDS.has(stage.stageId) && GATE_PENDING_STATUSES.has(stage.status))) {
      return "gate";
    }
    if (members.every((stage) => stage.status === "completed" || stage.status === "skipped")) return "done";
    if (members.some((stage) => stage.status === "running")) return "current";
    return "todo";
  });
  // A pending step stays pending. Highlighting it as current made queued work
  // look indistinguishable from work the runner had actually started.
  return states;
}

/** Index of the step the caption should talk about; -1 when everything is closed. */
export function activePhaseIndex(states: PhaseState[]): number {
  return states.findIndex((state) => state === "gate" || state === "error" || state === "current");
}

export function phaseName(index: number, copy: PipelineCopy): string {
  return [copy.phase1, copy.phase2, copy.phase3][index] ?? "";
}

/** Literal status text for one visible phase. */
export function phaseStatusCaption(unit: BookUnit, index: number, copy: PipelineCopy): string {
  const step = PHASES[index];
  if (!step) return copy.phaseNotInJob;
  const members = focusStages(unit).filter((stage) =>
    (step.stageIds as readonly string[]).includes(stage.stageId),
  );
  if (!members.length) return copy.phaseNotInJob;
  if (
    members.some(
      (stage) => stage.status === "failed" || (stage.status === "blocked" && !GATE_STAGE_IDS.has(stage.stageId)),
    )
  ) {
    return copy.capNeedsAttention;
  }
  if (members.some((stage) => GATE_STAGE_IDS.has(stage.stageId) && GATE_PENDING_STATUSES.has(stage.status))) {
    return copy.capWaitingConfirmation;
  }
  if (members.every((stage) => stage.status === "completed" || stage.status === "skipped")) {
    return copy.capCompleted;
  }
  if (members.some((stage) => stage.status === "running")) return copy.capWorking;
  return copy.capNotStarted;
}

/** One beginner-voiced line: "翻译 · 12/26" — drives the strip caption and shelf card. */
export function phaseCaption(unit: BookUnit, copy: PipelineCopy): string {
  const states = phaseStates(unit);
  const index = activePhaseIndex(states);
  if (index === -1) {
    if (unit.status === "completed" || unit.status === "skipped") return copy.capAllDone;
    const stages = focusStages(unit);
    if (
      stages.find((stage) => stage.stageId === "translate")?.status === "completed" &&
      ["pending", "ready"].includes(
        stages.find((stage) => stage.stageId === "expert_qa")?.status ?? "",
      )
    ) {
      return copy.capTranslationQaPending;
    }
    if (
      stages.find((stage) => stage.stageId === "approve_promotion")?.status === "completed" &&
      stages.some(
        (stage) =>
          ["promote", "build_reading", "validate_reading"].includes(stage.stageId) &&
          ["pending", "ready"].includes(stage.status),
      )
    ) {
      return copy.capPromotionApprovedPending;
    }
    return copy.capQueued;
  }
  const name = phaseName(index, copy);
  const state = states[index];
  if (state === "gate") {
    const gateStage = focusStages(unit).find(
      (stage) => GATE_STAGE_IDS.has(stage.stageId) && GATE_PENDING_STATUSES.has(stage.status),
    );
    return gateStage?.stageId === "approve_promotion" ? copy.capGatePromotion : copy.capGateTranslation;
  }
  if (state === "error") {
    const stage = currentStage(unit);
    const errorText = stage?.safeError?.summary || stage?.error || unit.child?.lastError || unit.job.lastError;
    if (sourceChangedRequiresRebuild(unit)) return `${name} · ${copy.sourceChangedTitle}`;
    return errorText ? `${name} · ${errorText}` : `${name} · ${copy.capNeedsAttention}`;
  }
  const stage = currentStage(unit);
  if (stage?.status === "running" && stage.unitSummary && stage.unitSummary.total > 0) {
    const runningName = stageLabel(stage.stageId, copy);
    const count = `${stage.unitSummary.completed}/${stage.unitSummary.total}`;
    return runningName === name ? `${name} · ${count}` : `${name} · ${runningName} ${count}`;
  }
  if (unit.status === "running") return `${name} · ${copy.capWorking}`;
  return `${name} · ${copy.capQueued}`;
}

/**
 * Beginner summary that makes the handoff between two visible steps explicit.
 * A checkmark alone made the just-finished step easy to miss, especially when
 * the next long-running stage immediately took over the main caption.
 */
export function phaseSummaryCaption(unit: BookUnit, copy: PipelineCopy): string {
  const states = phaseStates(unit);
  const active = activePhaseIndex(states);
  const current = phaseCaption(unit, copy);
  if (active <= 0 || states[active - 1] !== "done") return current;
  return copy.phaseSummaryPair(phaseName(active - 1, copy), current);
}

export function unitOperationProgress(unit: BookUnit): BookPipelineOperationProgress | null {
  const operation = unit.job.progress.operation;
  if (!operation || operation.stageId !== currentStage(unit)?.stageId) return null;
  if (operation.scopeId && operation.scopeId !== unit.child?.id) return null;
  return operation;
}

export function operationUnitLabel(unitKind: string, copy: PipelineCopy): string {
  const labels: Record<string, string> = {
    pages: copy.progressUnitPages,
    chapters: copy.progressUnitChapters,
    chunks: copy.progressUnitChunks,
    items: copy.progressUnitItems,
  };
  return labels[unitKind] ?? copy.progressUnitItems;
}

export function operationPhaseLabel(phase: string, copy: PipelineCopy): string {
  const labels: Record<string, string> = {
    starting: copy.progressStarting,
    uploading: copy.progressUploading,
    extracting: copy.progressExtracting,
    downloading: copy.progressDownloading,
    translating: copy.progressTranslating,
    reviewing: copy.progressReviewing,
    assembling: copy.progressAssembling,
  };
  return labels[phase] ?? copy.progressWorking;
}

/* ---------- Status vocabulary ---------- */

export type StatusTone = "info" | "amber" | "purple" | "red" | "jade" | "gray";

export function statusTone(status: string): StatusTone {
  switch (status) {
    case "running":
      return "info";
    case "waiting_for_approval":
      return "amber";
    case "blocked":
      return "purple";
    case "failed":
    case "partial":
      return "red";
    case "completed":
      return "jade";
    default:
      return "gray";
  }
}

export function statusLabel(status: string, copy: PipelineCopy): string {
  const labels: Record<string, string> = {
    running: copy.statusRunning,
    waiting_for_approval: copy.statusWaiting,
    blocked: copy.statusBlocked,
    failed: copy.statusFailed,
    partial: copy.statusPartial,
    completed: copy.statusCompleted,
    pending: copy.statusQueued,
    ready: copy.statusReady,
    skipped: copy.statusSkipped,
  };
  return labels[status] ?? status;
}

export function routeKindLabel(routeKind: string, copy: PipelineCopy): string {
  const labels: Record<string, string> = {
    direct_text: copy.routeDirectText,
    remote_paddleocr: copy.routeRemotePaddle,
    mineru: copy.routeMineru,
    blocked_dirty_text_layer: copy.routeDirty,
    blocked_no_attachment: copy.routeNoAttachment,
    already_converted: copy.routeAlreadyConverted,
    missing_credentials: copy.routeMissingCredentials,
    translation_handoff: copy.routeTranslationHandoff,
    translation_ready: copy.routeTranslationReady,
    external_adapter: copy.routeExternalAdapter,
    epub_source: copy.routeEpubSource,
  };
  return labels[routeKind] ?? routeKind.replaceAll("_", " ");
}

export type RouteTone = "ok" | "info" | "block" | "warn" | "neutral";

export function routeTone(routeKind: string): RouteTone {
  // epub_source is "ok" alongside direct_text: both extract text the source
  // already carries, so neither waits on an OCR credential.
  if (routeKind === "direct_text" || routeKind === "translation_ready" || routeKind === "epub_source") return "ok";
  if (routeKind === "remote_paddleocr" || routeKind === "mineru") return "info";
  if (routeKind.startsWith("blocked")) return "block";
  if (routeKind === "missing_credentials") return "warn";
  if (routeKind === "already_converted") return "neutral";
  return "neutral";
}

/* ---------- Book units (list rows) ---------- */

export type BookUnit = {
  key: string;
  job: BookPipelineJob;
  child: BookPipelineChildJob | null;
  title: string;
  status: string;
};

function sourceTitle(source: BookPipelineSource | undefined, fallback: string): string {
  if (!source) return fallback;
  return source.title || source.selector || source.path || fallback;
}

/**
 * The inbox lists books, not batch containers: a collection job contributes one
 * row per attachment child; a single job contributes one row for its child (or
 * for itself when the backend has not materialized children yet).
 */
export function flattenBookUnits(jobs: BookPipelineJob[]): BookUnit[] {
  const units: BookUnit[] = [];
  for (const job of jobs) {
    // A book dropped from a batch stays in the job — its collection membership
    // is frozen and cannot shrink — so the shelf is where it stops existing.
    const live = job.children.filter((child) => !child.removedAt);
    if (live.length > 1) {
      for (const child of live) {
        units.push({
          key: `${job.id}/${child.id}`,
          job,
          child,
          title: sourceTitle(child.source, child.id),
          status: child.status,
        });
      }
      continue;
    }
    const child = live[0] ?? null;
    units.push({
      key: job.id,
      job,
      child,
      title: sourceTitle(child?.source ?? job.source, job.id),
      status: child?.status ?? job.status,
    });
  }
  return units;
}

/* ---------- Per-unit derivation ---------- */

export function focusStages(unit: BookUnit): BookPipelineStage[] {
  return orderedStages(unit.child?.stages ?? unit.job.stages);
}

export function currentStage(unit: BookUnit): BookPipelineStage | null {
  const stages = focusStages(unit);
  const active = stages.find((stage) => stage.status !== "completed" && stage.status !== "skipped");
  return active ?? stages.at(-1) ?? null;
}

const SOURCE_CHANGED_DOWNSTREAM_EXISTS = "source_changed_downstream_exists";

/** The extracted source changed after downstream artifacts had already been produced. */
export function sourceChangedRequiresRebuild(unit: BookUnit): boolean {
  const values = [
    unit.child?.lastError,
    unit.job.lastError,
    ...focusStages(unit).flatMap((stage) => [stage.error, stage.safeError?.code, stage.safeError?.summary]),
  ];
  return values.some((value) => value === SOURCE_CHANGED_DOWNSTREAM_EXISTS);
}

/** Progress in [0,1] for running rows; null when nothing meaningful can be derived. */
export function unitProgress(unit: BookUnit): number | null {
  if (unit.status === "completed" || unit.status === "skipped") return 1;
  const stages = focusStages(unit);
  if (!stages.length) return null;
  const running = stages.find((stage) => stage.status === "running");
  if (running?.unitSummary && running.unitSummary.total > 0) {
    const done = running.unitSummary.completed + running.unitSummary.skipped;
    const stageIndex = stages.indexOf(running);
    return Math.min(1, (stageIndex + done / running.unitSummary.total) / stages.length);
  }
  const closed = stages.filter((stage) => stage.status === "completed" || stage.status === "skipped").length;
  if (unit.status === "running" || closed > 0) return closed / stages.length;
  return null;
}

export function gateLabel(stageId: string, copy: PipelineCopy): string {
  return stageId === "approve_promotion" ? copy.gatePromotion : copy.gateTranslation;
}

/** One-line explanation under the book title in list rows. */
export function unitNote(unit: BookUnit, copy: PipelineCopy): string {
  const stage = currentStage(unit);
  const stageName = stage ? stageLabel(stage.stageId, copy) : "";
  const errorText =
    stage?.safeError?.summary ||
    stage?.error ||
    unit.child?.lastError ||
    unit.job.lastError ||
    "";
  switch (unit.status) {
    case "running": {
      if (stage?.unitSummary && stage.unitSummary.total > 0) {
        const done = stage.unitSummary.completed;
        return `${stageName} · ${done}/${stage.unitSummary.total}`;
      }
      return `${stageName} · ${copy.statusRunning}`;
    }
    case "waiting_for_approval":
      return `${stage ? gateLabel(stage.stageId, copy) : copy.statusWaiting} · ${copy.stageWaitingYou}`;
    case "blocked":
      return errorText || `${stageName} · ${copy.statusBlocked}`;
    case "failed":
      return errorText || `${stageName} · ${copy.statusFailed}`;
    case "partial":
      return copy.statusPartial;
    case "completed": {
      const formats = readingFormatSummary(unit, copy);
      return formats ? `${copy.statusCompleted} · ${formats}` : copy.statusCompleted;
    }
    case "skipped":
      return copy.statusSkipped;
    case "ready":
      return copy.statusReady;
    default:
      return copy.statusQueued;
  }
}

function readingFormatSummary(unit: BookUnit, copy: PipelineCopy): string {
  const kinds = new Set(allArtifacts(unit).map((artifact) => artifact.kind));
  const parts: string[] = [];
  if (kinds.has("reading_markdown") || kinds.has("markdown")) parts.push("MD");
  if (kinds.has("reading_html") || kinds.has("html")) parts.push("HTML");
  if (kinds.has("reading_epub") || kinds.has("epub")) parts.push("EPUB");
  if (kinds.has("reading_bilingual_epub") || kinds.has("bilingual_epub")) parts.push(copy.formatBilingualShort);
  return parts.join(" / ");
}

/** Route chip for the detail header: the unit's decided extraction route. */
export function unitRoute(unit: BookUnit): BookPipelineRouteItem | null {
  const routes = unit.child?.route ?? unit.job.route;
  if (!routes.length) return null;
  return routes.find((item) => item.routeKind !== "translation_handoff") ?? routes[0];
}

export function allArtifacts(unit: BookUnit): BookPipelineArtifact[] {
  const seen = new Set<string>();
  const merged: BookPipelineArtifact[] = [];
  for (const artifact of [...(unit.child?.artifacts ?? []), ...unit.job.artifacts]) {
    const key = artifact.artifactId ?? `${artifact.kind}:${artifact.path}`;
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(artifact);
  }
  return merged;
}

export function firstMarkdownArtifact(job: BookPipelineJob): BookPipelineArtifact | null {
  return job.artifacts.find((artifact) => artifact.kind === "markdown") ?? null;
}

export function hashShort(hash?: string | null): string {
  if (!hash) return "—";
  if (hash.length <= 12) return hash;
  return `${hash.slice(0, 4)}…${hash.slice(-4)}`;
}

export function formatTime(iso?: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

/* ---------- Approval gate view model ---------- */

export type BoundHashEntry = {
  artifactId: string;
  bound: string;
  current: string | null;
  match: boolean | "unknown";
};

export type GateCheck = {
  ok: boolean | "unknown";
  title: string;
  detail: string;
};

export type GateView = {
  stageId: "approve_translation" | "approve_promotion";
  stage: BookPipelineStage;
  reference: BookPipelineApprovalReference | null;
  boundEntries: BoundHashEntry[];
  invalidated: boolean;
  checks: GateCheck[];
};

function approvalReferenceFor(
  job: BookPipelineJob,
  child: BookPipelineChildJob | null,
  stage: BookPipelineStage,
): BookPipelineApprovalReference | null {
  if (stage.approvalId) {
    const byId = job.approvalReferences.find((ref) => ref.approvalId === stage.approvalId);
    if (byId) return byId;
  }
  const childId = child?.id;
  const pending = job.approvalReferences.filter(
    (ref) => ref.stageId === stage.stageId && ref.decision !== "approved" && ref.decision !== "rejected",
  );
  return (
    pending.find((ref) => (childId ? ref.childJobId === childId : true)) ??
    pending[0] ??
    null
  );
}

export function buildGateView(
  unit: BookUnit,
  stageId: "approve_translation" | "approve_promotion",
  copy: PipelineCopy,
): GateView | null {
  const stage = focusStages(unit).find((candidate) => candidate.stageId === stageId);
  // The backend surfaces a reached gate as "ready"; "waiting_for_approval" is
  // the unified-model vocabulary. Accept both so pending gates always render.
  if (!stage || (stage.status !== "waiting_for_approval" && stage.status !== "ready")) return null;
  const reference = approvalReferenceFor(unit.job, unit.child, stage);
  const artifacts = allArtifacts(unit);

  const boundEntries: BoundHashEntry[] = Object.entries(reference?.boundArtifactHashes ?? {}).map(
    ([artifactId, bound]) => {
      const artifact = artifacts.find((candidate) => candidate.artifactId === artifactId);
      if (!artifact || !artifact.sha256) return { artifactId, bound, current: null, match: "unknown" as const };
      return {
        artifactId,
        bound,
        current: artifact.sha256,
        match: artifact.sha256 === bound,
      };
    },
  );
  const invalidated = boundEntries.some((entry) => entry.match === false);
  const hashesCheck: GateCheck =
    boundEntries.length === 0
      ? { ok: "unknown", title: copy.checkHashes, detail: copy.hashUnknown }
      : invalidated
        ? {
            ok: false,
            title: copy.checkHashes,
            detail: boundEntries
              .filter((entry) => entry.match === false)
              .map((entry) => `${entry.artifactId} ${hashShort(entry.bound)} → ${hashShort(entry.current)}`)
              .join(" · "),
          }
        : boundEntries.some((entry) => entry.match === "unknown")
          ? { ok: "unknown", title: copy.checkHashes, detail: copy.hashUnknown }
          : {
              ok: true,
              title: copy.checkHashes,
              detail: boundEntries.map((entry) => `${entry.artifactId} ${hashShort(entry.bound)}`).join(" · "),
            };

  const checks: GateCheck[] = [
    {
      ok: true,
      title: copy.checkPacket,
      detail: `${gateLabel(stageId, copy)}${stage.unitSummary ? ` · ${stage.unitSummary.completed}/${stage.unitSummary.total}` : ""}`,
    },
    hashesCheck,
  ];

  if (stageId === "approve_translation") {
    const profile = unit.job.translationProfileId;
    const config = unit.job.translationConfigId;
    checks.push({
      ok: profile || config ? true : "unknown",
      title: copy.checkProvider,
      detail: profile || config ? `${profile ?? "—"} · ${config ?? "—"}` : copy.providerUnknown,
    });
  } else {
    const qa = focusStages(unit).find((candidate) => candidate.stageId === "expert_qa");
    const qaClosed =
      qa?.status === "completed" &&
      (!qa.unitSummary || (qa.unitSummary.failed === 0 && qa.unitSummary.blocked === 0));
    checks.push({
      ok: qaClosed,
      title: copy.checkQa,
      detail: qaClosed
        ? qa?.unitSummary
          ? `PASS ${qa.unitSummary.completed}/${qa.unitSummary.total}`
          : copy.checkQa
        : copy.qaNotClosed,
    });
  }

  const stageIndex = focusStages(unit).findIndex((candidate) => candidate.stageId === stageId);
  const upstreamOpen = focusStages(unit)
    .slice(0, stageIndex)
    .some((candidate) => candidate.status === "failed" || candidate.status === "blocked");
  checks.push({
    ok: !upstreamOpen,
    title: copy.checkNoBlocker,
    detail: upstreamOpen ? copy.blockerFound : "✓",
  });

  return { stageId, stage, reference, boundEntries, invalidated, checks };
}

export function pendingGates(unit: BookUnit, copy: PipelineCopy): GateView[] {
  const gates = [
    buildGateView(unit, "approve_translation", copy),
    buildGateView(unit, "approve_promotion", copy),
  ];
  return gates.filter((gate): gate is GateView => gate !== null);
}

/** Recorded (immutable) approval decisions for the focused child. */
export function approvalRecords(unit: BookUnit): BookPipelineApprovalReference[] {
  const childId = unit.child?.id;
  return unit.job.approvalReferences.filter(
    (ref) =>
      (ref.decision === "approved" || ref.decision === "rejected") &&
      (childId ? ref.childJobId === childId || !ref.childJobId : true),
  );
}

/* ---------- Manual advance ---------- */

const ADVANCEABLE_STAGE_IDS = new Set([
  "split",
  "prepare",
  "translate",
  "expert_qa",
  "promote",
  "build_reading",
  "validate_reading",
  "build_digest",
]);

export type UnitAdvanceAction = { childId: string; stageId: string; stageStatus: string };

/** Next stage the user can push forward by hand (gates go through the approval tab instead). */
export function unitAdvanceAction(unit: BookUnit): UnitAdvanceAction | null {
  const child = unit.child;
  if (!child) return null;
  const stage = orderedStages(child.stages).find(
    (candidate) => candidate.status !== "completed" && candidate.status !== "skipped",
  );
  if (!stage || !ADVANCEABLE_STAGE_IDS.has(stage.stageId)) return null;
  const runnable =
    stage.status === "pending" ||
    stage.status === "ready" ||
    (stage.status === "blocked" && (stage.stageId === "translate" || stage.stageId === "expert_qa"));
  if (!runnable) return null;
  return { childId: child.id, stageId: stage.stageId, stageStatus: stage.status };
}
