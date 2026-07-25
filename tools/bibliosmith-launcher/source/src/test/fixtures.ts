import type {
  BookPipelineApprovalReference,
  BookPipelineArtifact,
  BookPipelineChildJob,
  BookPipelineJob,
  BookPipelineRouteItem,
  BookPipelineStage,
  BookPipelineUnitSummary,
} from "../types";
import type { BookUnit } from "../pipeline/model";

/**
 * Builders for the pipeline view model. The backend shapes carry a lot of
 * bookkeeping (contract versions, hash maps, counters) that no view-model
 * function reads, so each builder fills that in and takes an override object
 * for the handful of fields a given test actually cares about. Tests then read
 * as the scenario they describe rather than as forty lines of scaffolding.
 */

export function unitSummary(over: Partial<BookPipelineUnitSummary> = {}): BookPipelineUnitSummary {
  return {
    total: 0,
    pending: 0,
    ready: 0,
    running: 0,
    blocked: 0,
    failed: 0,
    completed: 0,
    skipped: 0,
    ...over,
  };
}

export function stage(
  stageId: string,
  status: BookPipelineStage["status"],
  over: Partial<BookPipelineStage> = {},
): BookPipelineStage {
  return {
    stageId,
    status,
    attempt: 1,
    contractVersion: "1",
    inputHashes: {},
    artifactIds: [],
    ...over,
  };
}

export function artifact(
  kind: string,
  over: Partial<BookPipelineArtifact> = {},
): BookPipelineArtifact {
  return { kind, path: `/out/${kind}`, ...over };
}

export function routeItem(over: Partial<BookPipelineRouteItem> = {}): BookPipelineRouteItem {
  return {
    id: "route-1",
    title: "A Book",
    sourceKind: "zotero_attachment",
    sourceRef: "ABCD1234",
    routeKind: "direct_text",
    canRun: true,
    summary: "",
    ...over,
  };
}

export function approvalRef(
  over: Partial<BookPipelineApprovalReference> = {},
): BookPipelineApprovalReference {
  return {
    approvalId: "approval-1",
    gateId: "gate-1",
    childJobId: "child-1",
    stageId: "approve_translation",
    decision: "pending",
    boundArtifactHashes: {},
    ...over,
  };
}

export function childJob(over: Partial<BookPipelineChildJob> = {}): BookPipelineChildJob {
  return {
    id: "child-1",
    parentJobId: "job-1",
    status: "pending",
    currentStageId: "route",
    source: { kind: "zotero_attachment", title: "A Book" },
    route: [],
    stages: [],
    artifacts: [],
    attempts: 1,
    ...over,
  };
}

export function job(over: Partial<BookPipelineJob> = {}): BookPipelineJob {
  return {
    schemaVersion: "1",
    id: "job-1",
    kind: "single",
    mode: "convert_then_translate",
    translationMode: "fast",
    source: { kind: "zotero_attachment", title: "A Book" },
    route: [],
    status: "pending",
    currentStageId: "route",
    currentStep: "ingest",
    logSummary: [],
    artifacts: [],
    attempts: 1,
    stages: [],
    children: [],
    summary: {
      total: 1,
      pending: 1,
      ready: 0,
      running: 0,
      waitingForApproval: 0,
      blocked: 0,
      failed: 0,
      completed: 0,
      skipped: 0,
    },
    progress: { stageTotal: 1, stageCompleted: 0, percent: 0, activeStageId: "route" },
    notificationDeliveries: [],
    approvalReferences: [],
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...over,
  };
}

/**
 * A single-book unit whose stages hang off a child job — the shape the drawer
 * and shelf actually get, since `focusStages` prefers the child's stages.
 */
export function bookUnit(over: {
  stages?: BookPipelineStage[];
  status?: string;
  title?: string;
  jobOver?: Partial<BookPipelineJob>;
  childOver?: Partial<BookPipelineChildJob>;
} = {}): BookUnit {
  const stages = over.stages ?? [];
  const status = over.status ?? "pending";
  const child = childJob({ stages, status, ...over.childOver });
  const parent = job({ children: [child], status, ...over.jobOver });
  return {
    key: parent.id,
    job: parent,
    child,
    title: over.title ?? "A Book",
    status,
  };
}
