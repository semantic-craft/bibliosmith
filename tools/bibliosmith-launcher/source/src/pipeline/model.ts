import type {
  BookPipelineApprovalReference,
  BookPipelineArtifact,
  BookPipelineChildJob,
  BookPipelineJob,
  BookPipelineRouteItem,
  BookPipelineSource,
  BookPipelineStage,
  BookPipelineOutputFormat,
} from "../types";
import type { PipelineCopy } from "./copy";

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
  | "folder"
  | "markdown"
  | "zotero"
  | "handoff"
  | "cleanup"
  | "cleanupApproval"
  | "open"
  | null;

/** Conversion route override a user can pick per route item in the wizard. */
export type RouteOverride = "auto" | "direct" | "paddle" | "mineru" | "keep";

export type PipelineDraft = {
  sourceKind: BookPipelineSource["kind"];
  mode: "conversion_only" | "convert_then_translate" | "translate_only";
  fakeBehavior: "succeed" | "fail_once" | "always_fail";
  localPdfFolder: string;
  localPdfTitle: string;
  markdownPath: string;
  markdownTitle: string;
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
  reflectionTranslation: boolean;
  externalAdapterCommand: string;
  externalAdapterInput: string;
  zoteroSelector: string;
  hasPaddleocrCredentials: boolean;
  hasMineruCredentials: boolean;
};

// The wizard only offers local PDF / Zotero / Markdown cards, so the default
// must be one of them; "fake" would be selectable-by-omission and launchable.
export const defaultPipelineDraft: PipelineDraft = {
  sourceKind: "local_pdf_folder",
  mode: "conversion_only",
  fakeBehavior: "succeed",
  localPdfFolder: "",
  localPdfTitle: "Local PDF folder",
  markdownPath: "",
  markdownTitle: "Markdown source",
  translationMode: "fast",
  providerProfileId: "openai-compatible",
  providerConfigId: "openai-default",
  secondPassEnabled: true,
  textCleanup: false,
  digestMode: false,
  outputFormats: ["md", "html", "epub"],
  reflectionTranslation: false,
  externalAdapterCommand: "",
  externalAdapterInput: "",
  zoteroSelector: "reading-queue",
  hasPaddleocrCredentials: false,
  hasMineruCredentials: true,
};

/* ---------- 12-stage contract ---------- */

export const PIPELINE_STAGE_ORDER = [
  "route",
  "extract",
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
] as const;

export const GATE_STAGE_IDS = new Set(["approve_translation", "approve_promotion"]);

// The config_id each provider profile resolves to. Defined once so the new-job
// wizard (App) and the per-book drawer cannot disagree about which config a
// profile maps to; the keys must match the profiles in the engine's
// providers.toml.
export const PROVIDER_DEFAULT_CONFIG: Record<
  PipelineDraft["providerProfileId"],
  string
> = {
  "openai-compatible": "openai-default",
  "gemini-native": "gemini-default",
  deepseek: "deepseek-default",
};

// A persisted job's translationProfileId is a loose string (it may predate a
// profile being renamed), so resolve it through this rather than indexing the
// typed map directly. The wizard, whose profile is the union, indexes the map
// straight and stays exhaustively checked.
export function providerDefaultConfig(profileId: string): string {
  return (
    (PROVIDER_DEFAULT_CONFIG as Record<string, string>)[profileId] ?? "default"
  );
}

export function stageLabel(stageId: string, copy: PipelineCopy): string {
  const labels: Record<string, string> = {
    route: copy.stageRoute,
    extract: copy.stageExtract,
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
    discover: copy.stageDiscover,
    build_digest: copy.stageBuildDigest,
  };
  return labels[stageId] ?? stageId.replaceAll("_", " ");
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
export const FOUR_STEPS = [
  { key: "ingest", stageIds: ["discover", "route"] },
  { key: "tidy", stageIds: ["extract", "handoff", "split"] },
  { key: "translate", stageIds: ["prepare", "approve_translation", "translate", "expert_qa"] },
  { key: "produce", stageIds: ["approve_promotion", "promote", "build_reading", "validate_reading", "build_digest"] },
] as const;

export type StepState = "done" | "current" | "gate" | "error" | "todo" | "none";

const GATE_PENDING_STATUSES = new Set(["waiting_for_approval", "ready"]);

export function fourStepStates(unit: BookUnit): StepState[] {
  const stages = focusStages(unit);
  const grouped = FOUR_STEPS.map((step) =>
    stages.filter((stage) => (step.stageIds as readonly string[]).includes(stage.stageId)),
  );
  const states: StepState[] = grouped.map((members) => {
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
  // Promote the first open step to "current" so an idle/queued book still
  // shows where it stands rather than four grey circles.
  const active = states.findIndex((state) => state === "current" || state === "gate" || state === "error");
  if (active === -1 && unit.status !== "completed" && unit.status !== "skipped") {
    const firstOpen = states.findIndex((state) => state === "todo");
    if (firstOpen !== -1) states[firstOpen] = "current";
  }
  return states;
}

/** Index of the step the caption should talk about; -1 when everything is closed. */
export function activeStepIndex(states: StepState[]): number {
  return states.findIndex((state) => state === "gate" || state === "error" || state === "current");
}

export function stepName(index: number, copy: PipelineCopy): string {
  return [copy.step1, copy.step2, copy.step3, copy.step4][index] ?? "";
}

/** One beginner-voiced line: "翻译 · 12/26" — drives the strip caption and shelf card. */
export function stepCaption(unit: BookUnit, copy: PipelineCopy): string {
  const states = fourStepStates(unit);
  const index = activeStepIndex(states);
  if (index === -1) {
    if (unit.status === "completed" || unit.status === "skipped") return copy.capAllDone;
    return copy.capQueued;
  }
  const name = stepName(index, copy);
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
    return errorText ? `${name} · ${errorText}` : `${name} · ${copy.capNeedsAttention}`;
  }
  const stage = currentStage(unit);
  if (stage?.status === "running" && stage.unitSummary && stage.unitSummary.total > 0) {
    return `${name} · ${stage.unitSummary.completed}/${stage.unitSummary.total}`;
  }
  if (unit.status === "running") return `${name} · ${copy.capWorking}`;
  return `${name} · ${copy.capQueued}`;
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
    already_converted: copy.routeAlreadyConverted,
    missing_credentials: copy.routeMissingCredentials,
    translation_handoff: copy.routeTranslationHandoff,
    translation_ready: copy.routeTranslationReady,
    external_adapter: copy.routeExternalAdapter,
  };
  return labels[routeKind] ?? routeKind.replaceAll("_", " ");
}

export type RouteTone = "ok" | "info" | "block" | "warn" | "neutral";

export function routeTone(routeKind: string): RouteTone {
  if (routeKind === "direct_text" || routeKind === "translation_ready") return "ok";
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
    if (job.children.length > 1) {
      for (const child of job.children) {
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
    const child = job.children[0] ?? null;
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

export function formatDateTime(iso?: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleString(undefined, { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
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

export function approvalBadge(unit: BookUnit, copy: PipelineCopy): { count: number; invalid: boolean } {
  const gates = pendingGates(unit, copy);
  return { count: gates.length, invalid: gates.some((gate) => gate.invalidated) };
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
