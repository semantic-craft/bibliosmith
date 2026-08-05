import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ActionResult,
  BookPipelineActionResult,
  BookPipelineArtifactExcerpt,
  BookPipelineCleanupCandidate,
  BookPipelineCleanupPreview,
  BookPipelineStructureCorrectionDraft,
  BookPipelineStructureCorrectionInput,
  BookPipelineJob,
  BookPipelinePreviewConfig,
  BookPipelineProjectMigration,
  BookPipelineRouteItem,
  BookPipelineShelfSelection,
  BookPipelineSource,
  BookPipelineState,
  BookPipelineOcrSampleReport,
  BookPipelineTranslationSampleReport,
  BookPipelineTranslationIntent,
  BookPipelineZoteroDiscoveryResult,
  DiagnosticLogSettings,
  DownloadProgress,
  EmbeddingConnectionResult,
  EmbeddingStatus,
  WorkspaceState,
  ModelCatalog,
  ModelConnectionResult,
  NetworkProxySettings,
  OcrConnectionResult,
  OcrCredentialsStatus,
  ProxyAutoDetectResult,
  ProxyTestResult,
  RuntimeStatus,
  TranslationPromptPackCatalog,
  TranslationPromptPackDefinition,
  TranslationPromptPackReference,
  TranslationPromptPackRevision,
  TranslationPromptPackRevisionDiff,
  TranslationPromptPackRevisionDraft,
  TranslationPromptPreview,
} from "./types";
import builtinTranslationPromptPacks from "../src-tauri/resources/translation-prompt-packs.json";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

// Exported because the input island registers the native drag-drop listener
// only under Tauri; in the browser preview there are no file paths to receive.
export function isTauriRuntime() {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

function previewWorkspaceState(): WorkspaceState {
  return {
    workspaceRoot: "Documents/BiblioSmith",
    recommendedWorkspaceRoot: "Documents/BiblioSmith",
    workspaceReady: true,
    workspaceStatus: "ready",
    proxyConfigured: false,
    platform: "preview",
  };
}

let previewBookPipelineJobs: BookPipelineJob[] = [];
let previewBookPipelineRevision = 0;
let previewTranslationPromptCatalog = structuredClone(
  builtinTranslationPromptPacks,
) as TranslationPromptPackCatalog;
const previewDeletedTranslationPromptPacks: TranslationPromptPackDefinition[] = [];
const previewTranslationPromptDefaults = new Map<string, TranslationPromptPackReference>();
export const BOOK_PIPELINE_STATE_SCHEMA_VERSION = "book-pipeline-state-v5";
const BOOK_PIPELINE_JOB_SCHEMA_VERSION = "book-pipeline-job-v5";
const PREVIEW_STRUCTURE_PROMPT_PACK: TranslationPromptPackReference = {
  packId: "builtin.structure-fidelity",
  revisionId: "2026.08.05-1",
  contentSha256: "fb5dae8c498d46a1a3501acd0d6b00645b7dfe4c5c797e8e71732482c5a0c26f",
};

function bookPipelineNow() {
  return new Date().toISOString();
}

function isBookPipelineZoteroSource(source: BookPipelineSource) {
  return source.kind === "zotero_attachment" || source.kind === "zotero_collection" || source.kind === "zotero_filter";
}

function isBookPipelineZoteroBatchSource(source: BookPipelineSource) {
  return source.kind === "zotero_collection" || source.kind === "zotero_filter";
}

function routeIsRunnableForSource(source: BookPipelineSource, route: BookPipelineRouteItem[]) {
  if (!route.length) return false;
  if (isBookPipelineZoteroBatchSource(source)) return route.some((item) => item.routeKind !== "translation_handoff" && item.canRun);
  if (isBookPipelineZoteroSource(source)) return route.every((item) => item.canRun);
  return route.some((item) => item.canRun);
}

// Mirrors should_handoff_after_run in book_pipeline.rs, exclusions and all.
// Listing the two translating modes instead read as the same answer but gave a
// mode this mirror has not heard of the retired conversion-only shape, which is
// the one shape the backend now refuses to hand out. The two modes that stop
// after extraction are the retired conversion_only, which never translated, and
// layout_preserving, whose single pass already is the translation.
function shouldHandoffAfterRun(mode: string) {
  return mode !== "conversion_only" && mode !== "layout_preserving";
}

// Mirrors ENQUEUEABLE_MODES in book_pipeline/contract.rs.
const ENQUEUEABLE_MODES = ["convert_then_translate", "translate_only", "layout_preserving"];

// Mirrors validate_enqueue_mode in book_pipeline.rs, wording included. The
// preview is where the wizard's modes get exercised while the UI is built, so a
// mode the real backend refuses has to come back refused here rather than as a
// job that only fails once the work is inside the packaged app.
function enqueueModeRejection(mode: string) {
  if (ENQUEUEABLE_MODES.includes(mode)) return null;
  if (mode === "conversion_only") {
    return "Book Pipeline mode conversion_only was retired: conversion now always continues into translation. Enqueue convert_then_translate instead. Jobs queued before the retirement keep running and stay readable.";
  }
  return `Unknown Book Pipeline mode ${mode}. Valid modes: ${ENQUEUEABLE_MODES.join(", ")}.`;
}

function withTranslationHandoff(source: BookPipelineSource, mode: string, route: BookPipelineRouteItem[]) {
  if (!shouldHandoffAfterRun(mode) || !routeIsRunnableForSource(source, route)) return route;
  return [
    ...route,
    {
      id: "translation-handoff",
      title: "Local reading project handoff",
      sourceKind: source.kind,
      sourceRef: "books/local/zh-Hans",
      routeKind: "translation_handoff",
      canRun: true,
      blockedReason: null,
      summary: "Cleaned Markdown will be copied into a local reading project after conversion.",
    },
  ];
}

function previewBookPipelineRoutes(source: BookPipelineSource, mode: string, config?: BookPipelinePreviewConfig | null): BookPipelineRouteItem[] {
  if (source.kind === "fake") {
    return withTranslationHandoff(source, mode, [{
      id: "fake-source",
      title: source.title || "Fake source",
      sourceKind: "fake",
      sourceRef: source.selector || "fake://source",
      routeKind: "direct_text",
      canRun: true,
      blockedReason: null,
      summary: `Fake source will run in ${mode} mode through the fake CLI runner.`,
    }]);
  }
  if (source.kind === "markdown_source") {
    return withTranslationHandoff(source, mode, [{
      id: "markdown-source",
      title: source.title || "Markdown source",
      sourceKind: "markdown_source",
      sourceRef: source.path || "",
      routeKind: "translation_ready",
      canRun: Boolean(source.path),
      blockedReason: source.path ? null : "Choose a Markdown or source-text file before translation.",
      summary: "Selected source text will be copied into a local reading project without OCR.",
    }]);
  }
  if (source.kind === "external_adapter") {
    const command = source.adapterCommand || source.path || "";
    return [{
      id: "external-adapter",
      title: source.title || "External adapter",
      sourceKind: "external_adapter",
      sourceRef: command,
      routeKind: "external_adapter",
      canRun: Boolean(command),
      blockedReason: command ? null : "External adapter command path is missing.",
      summary: "External adapter will run with --input and --output-dir, then Book Pipeline will normalize artifacts.",
    }];
  }
  if (source.kind === "local_pdf_folder") {
    // Mirrors the backend: a path that is itself an .epub routes to extraction
    // and never waits on an OCR credential. The browser cannot stat a folder,
    // so only the single-file shape is previewable here.
    if (/\.epub$/i.test(source.path || "")) {
      return withTranslationHandoff(source, mode, [{
        id: "local-epub-1",
        title: source.path?.split(/[/\\]/).pop() || "EPUB",
        sourceKind: "local_pdf_folder",
        sourceRef: source.path || "",
        routeKind: "epub_source",
        canRun: true,
        blockedReason: null,
        summary: "Extract EPUB chapters straight to Markdown through scripts/epub_to_markdown.py; no OCR engine runs.",
      }]);
    }
    // Mirrors the backend's own no-books-found placeholder, which since #137
    // names no engine: whether a PDF costs anything is a per-book question the
    // text-layer probe answers, and the browser can neither list the folder nor
    // run the probe.
    return withTranslationHandoff(source, mode, [{
      id: "local-pdf-folder",
      title: source.title || "Local PDF folder",
      sourceKind: "local_pdf_folder",
      sourceRef: source.path || "",
      routeKind: "local_pdf_folder",
      canRun: Boolean(source.path),
      blockedReason: source.path ? null : "Choose a folder before running conversion.",
      summary: "The existing local PDF conversion wrapper will decide the PDF extraction details.",
    }]);
  }
  // Mirrors the backend: a Zotero route is only previewable from discovery
  // evidence; without it the source stays blocked instead of fabricating demo
  // attachments (which used to be queued as real children).
  const items = source.fakeZoteroItems ?? [];
  if (!items.length) {
    return [{
      id: "zotero-no-attachments",
      title: source.title || source.selector || "Zotero source",
      sourceKind: source.kind,
      sourceRef: source.selector || "",
      routeKind: "blocked_no_attachment",
      canRun: false,
      blockedReason: "No matching Zotero attachment was discovered for this source.",
      summary: "Adjust the search or filter, or select a specific attachment from bibliographic discovery.",
    }];
  }
  const hasPaddle = Boolean(config?.hasPaddleocrCredentials);
  const hasMineru = Boolean(config?.hasMineruCredentials);
  return withTranslationHandoff(source, mode, items.map((item) => {
    const base = {
      id: item.key,
      title: item.title,
      sourceKind: source.kind,
      sourceRef: item.attachmentPath || `zotero://${item.key}`,
    };
    if (item.alreadyConverted) {
      return {
        ...base,
        routeKind: "already_converted",
        canRun: false,
        blockedReason: "Converted Markdown already exists for this attachment.",
        summary: "Already converted; no full conversion will start from preview.",
      };
    }
    if (item.dirtyTextLayer) {
      return {
        ...base,
        routeKind: "blocked_dirty_text_layer",
        canRun: false,
        blockedReason: "Dirty embedded text layer detected; route requires manual MinerU review.",
        summary: "Blocked to avoid silently converting degraded Chinese text.",
      };
    }
    if (item.preferMineru) {
      return {
        ...base,
        routeKind: hasMineru ? "mineru" : "missing_credentials",
        canRun: hasMineru,
        blockedReason: hasMineru ? null : "MinerU credentials are missing.",
        summary: hasMineru ? "Route preview selects MinerU for this layout-sensitive item." : "MinerU candidate is blocked until credentials are configured.",
      };
    }
    if (item.hasTextLayer && !item.scanned) {
      return {
        ...base,
        routeKind: "direct_text",
        canRun: true,
        blockedReason: null,
        summary: "Direct embedded text extraction can run without remote OCR credentials.",
      };
    }
    return {
      ...base,
      routeKind: hasPaddle ? "remote_paddleocr" : "missing_credentials",
      canRun: hasPaddle,
      blockedReason: hasPaddle ? null : "Remote PaddleOCR credentials are missing.",
      summary: hasPaddle ? "Scanned or low-text PDF will use the existing remote PaddleOCR workflow." : "Scanned or low-text PDF is blocked until OCR credentials are configured.",
    };
  }));
}

function previewBookPipelineJob(source: BookPipelineSource, mode: string, config?: BookPipelinePreviewConfig | null): BookPipelineJob {
  const now = bookPipelineNow();
  const route = previewBookPipelineRoutes(source, mode, config);
  const id = `preview-${crypto.randomUUID()}`;
  const executionRoutes = route.filter((item) => item.routeKind !== "translation_handoff");
  const childRoutes = isBookPipelineZoteroBatchSource(source) ? executionRoutes.map((item) => [item]) : [executionRoutes];
  const children = childRoutes.map((routes, index) => {
    const selected = routes[0];
    const skipped = selected?.routeKind === "already_converted";
    const runnable = routes.length > 0 && routes.every((item) => item.canRun);
    // Mirrors ordered_child_stage_ids in book_pipeline.rs, including the
    // item-scoped "index" stage the backend only runs for Zotero attachments.
    const wantsItemIndex = isBookPipelineZoteroSource(source);
    // Answered ahead of the handoff question, as ordered_child_stage_ids answers
    // it: the layout track's single pass is the whole run, and it is the one mode
    // ensure_item_index_stage refuses to give an "index" stage to. The last arm
    // is the retired conversion-only shape, which only jobs stored before the
    // retirement still carry -- reached through the same exclusion the backend
    // uses, so an unfamiliar mode gets the translation shape rather than one that
    // silently stops after extraction.
    const stageIds =
      mode === "layout_preserving"
        ? ["route", "extract"]
        : shouldHandoffAfterRun(mode)
          ? ["route", "extract", "index", "handoff", "split", "prepare", "approve_translation", "translate", "expert_qa", "approve_promotion", "promote", "build_reading", "validate_reading", "build_digest"]
          : ["route", "extract", "index"];
    const stages = stageIds.map((stageId) => ({
      stageId,
      status: skipped ? "skipped" : stageId === "route" ? (runnable ? "completed" : "blocked") : stageId === "extract" ? (runnable ? "ready" : "pending") : stageId === "index" ? (wantsItemIndex ? "pending" : "skipped") : stageId === "build_digest" ? "skipped" : "pending",
      attempt: 0,
      error: null,
      contractVersion: BOOK_PIPELINE_JOB_SCHEMA_VERSION,
      startedAt: null,
      finishedAt: null,
      inputHashes: {},
      artifactIds: [],
      unitSummary: null,
      approvalId: null,
    }));
    return {
      id: `${id}-${selected?.id || index + 1}`,
      parentJobId: id,
      status: skipped ? "skipped" : runnable ? "ready" : "blocked",
      currentStageId: skipped ? stageIds.at(-1) || "extract" : runnable ? "extract" : "route",
      source: selected
        ? { ...source, kind: isBookPipelineZoteroSource(source) ? "zotero_attachment" : source.kind, title: selected.title, path: selected.sourceRef, selector: selected.id, fakeZoteroItems: null }
        : source,
      route: routes,
      stages,
      artifacts: [],
      attempts: 0,
      lastError: null,
      promptPackReference: PREVIEW_STRUCTURE_PROMPT_PACK,
      promptPackSelectionSource: "default",
    };
  });
  const job: BookPipelineJob = {
    schemaVersion: BOOK_PIPELINE_JOB_SCHEMA_VERSION,
    id,
    kind: isBookPipelineZoteroBatchSource(source) ? "collection" : "single",
    mode,
    promptPackReference: PREVIEW_STRUCTURE_PROMPT_PACK,
    promptPackSelectionSource: "default",
    source,
    route,
    status: routeIsRunnableForSource(source, route) ? "ready" : "blocked",
    currentStageId: isBookPipelineZoteroBatchSource(source) ? "children" : children[0]?.currentStageId || "route",
    currentStep: "Route preview recorded",
    lastError: null,
    logSummary: ["Route preview recorded"],
    artifacts: [],
    collectionItems: [],
    outputDir: null,
    attempts: 0,
    stages: isBookPipelineZoteroBatchSource(source)
      ? [{ stageId: "discover", status: "completed", attempt: 1, error: null, contractVersion: BOOK_PIPELINE_JOB_SCHEMA_VERSION, startedAt: now, finishedAt: now, inputHashes: {}, artifactIds: [], unitSummary: null, approvalId: null }]
      : [],
    children,
    membership: isBookPipelineZoteroBatchSource(source)
      ? { revision: 1, frozenAt: now, discoveryStageId: "discover", childJobIds: children.map((child) => child.id) }
      : null,
    summary: { total: children.length, pending: 0, ready: children.filter((child) => child.status === "ready").length, running: 0, waitingForApproval: 0, blocked: children.filter((child) => child.status === "blocked").length, failed: 0, completed: 0, skipped: children.filter((child) => child.status === "skipped").length },
    progress: { stageTotal: 0, stageCompleted: 0, percent: 0, activeStageId: "", unitSummary: null },
    notificationDeliveries: [],
    approvalReferences: [],
    createdAt: now,
    updatedAt: now,
  };
  derivePreviewBookPipelineJob(job);
  return job;
}

function previewPipelineStage(job: BookPipelineJob, childIndex: number, stageId: string) {
  return job.children[childIndex]?.stages.find((stage) => stage.stageId === stageId);
}

function derivePreviewBookPipelineJob(job: BookPipelineJob) {
  for (const child of job.children) {
    const active = child.stages.find((stage) => stage.status !== "completed" && stage.status !== "skipped");
    child.status = active?.status || (child.stages.every((stage) => stage.status === "skipped") ? "skipped" : "completed");
    child.currentStageId = active?.stageId || child.stages.at(-1)?.stageId || "";
  }
  const count = (status: string) => job.children.filter((child) => child.status === status).length;
  job.summary = {
    total: job.children.length,
    pending: count("pending"),
    ready: count("ready"),
    running: count("running"),
    waitingForApproval: count("waiting_for_approval"),
    blocked: count("blocked"),
    failed: count("failed"),
    completed: count("completed"),
    skipped: count("skipped"),
  };
  const summary = job.summary;
  if (summary.running) job.status = "running";
  else if (summary.waitingForApproval) job.status = "waiting_for_approval";
  else if (summary.ready) job.status = "ready";
  else if (summary.completed && summary.completed + summary.skipped === summary.total) job.status = "completed";
  else if (summary.completed + summary.skipped && summary.failed + summary.blocked) job.status = "partial";
  else if (summary.failed && summary.failed === summary.total - summary.skipped) job.status = "failed";
  else if (!summary.completed && summary.blocked && summary.failed + summary.blocked === summary.total - summary.skipped) job.status = "blocked";
  else if (summary.skipped === summary.total) job.status = "skipped";
  else job.status = "pending";
  job.currentStageId = job.kind === "collection" ? "children" : job.children[0]?.currentStageId || "";
  const stages = [...job.stages, ...job.children.flatMap((child) => child.stages)];
  const completed = stages.filter((stage) => stage.status === "completed" || stage.status === "skipped").length;
  const active = ["running", "waiting_for_approval", "ready", "failed", "blocked", "pending"]
    .map((status) => stages.find((stage) => stage.status === status))
    .find(Boolean);
  job.progress = {
    stageTotal: stages.length,
    stageCompleted: completed,
    percent: stages.length ? Math.floor((completed * 100) / stages.length) : 0,
    activeStageId: active?.stageId || job.currentStageId,
    unitSummary: active?.unitSummary || null,
  };
}

function previewBookPipelineCleanupCandidates(): BookPipelineCleanupCandidate[] {
  return previewBookPipelineJobs
    .filter((job) => job.artifacts.some((artifact) => artifact.kind === "markdown") || isBookPipelineZoteroSource(job.source))
    .map((job) => {
      const markdown = job.artifacts.find((artifact) => artifact.kind === "markdown") ?? null;
      const outputDir = job.outputDir || job.artifacts.find((artifact) => artifact.kind === "output_dir")?.path || null;
      const zoteroKey = markdown?.zoteroKey || null;
      const checks = [
        {
          kind: "markdown_output",
          ok: Boolean(markdown?.sha256),
          detail: markdown?.sha256 ? "Markdown output exists and checksum is recorded." : "Missing Markdown artifact.",
          path: markdown?.path ?? null,
          zoteroKey,
        },
        {
          kind: "local_output",
          ok: Boolean(outputDir),
          detail: outputDir ? "Local output directory is recorded." : "Missing local output directory or deliverable artifact.",
          path: outputDir,
          zoteroKey: null,
        },
        {
          kind: "zotero_child_attachment",
          ok: Boolean(zoteroKey),
          detail: zoteroKey ? "Zotero Markdown child attachment key is recorded." : "Missing Zotero Markdown child attachment key.",
          path: null,
          zoteroKey,
        },
      ];
      return {
        id: `cleanup-${job.id}`,
        jobId: job.id,
        title: job.source.title || job.source.selector || job.source.path || "Book Pipeline source",
        sourceKind: job.source.kind,
        sourceRef: job.source.selector || job.source.path || job.id,
        sourcePath: job.source.path ?? null,
        sourcePdfKey: job.source.selector ?? null,
        markdownPath: markdown?.path ?? null,
        localOutputPath: outputDir,
        zoteroChildAttachmentKey: zoteroKey,
        checks,
        canApprove: checks.every((check) => check.ok),
      };
    });
}

function previewZoteroDiscovery(source: BookPipelineSource): BookPipelineZoteroDiscoveryResult {
  const selector = source.selector || "preview";
  return {
    sources: [
      {
        kind: "zotero_attachment",
        title: "Preview born-digital attachment",
        selector: `${selector}-DIRECT`,
        fakeZoteroItems: [{
          key: `${selector}-DIRECT`,
          title: "Preview born-digital attachment",
          attachmentPath: `zotero://${selector}/direct.pdf`,
          hasTextLayer: true,
          dirtyTextLayer: false,
          scanned: false,
          alreadyConverted: false,
          preferMineru: false,
        }],
      },
      {
        kind: "zotero_attachment",
        title: "Preview scanned attachment",
        selector: `${selector}-SCAN`,
        fakeZoteroItems: [{
          key: `${selector}-SCAN`,
          title: "Preview scanned attachment",
          attachmentPath: `zotero://${selector}/scan.pdf`,
          hasTextLayer: false,
          dirtyTextLayer: false,
          scanned: true,
          alreadyConverted: false,
          preferMineru: false,
        }],
      },
      {
        kind: "zotero_collection",
        title: selector,
        selector,
      },
      {
        kind: "zotero_filter",
        title: "Books",
        selector: "parent_item_type=book",
      },
    ],
    logSummary: ["Preview Zotero discovery returned fake sources"],
  };
}

export function getWorkspaceState() {
  if (!isTauriRuntime()) {
    return Promise.resolve(previewWorkspaceState());
  }
  return invoke<WorkspaceState>("get_workspace_state");
}

export function createRecommendedWorkspace() {
  if (!isTauriRuntime()) {
    return Promise.resolve<WorkspaceState>({
      ...previewWorkspaceState(),
      workspaceReady: true,
      workspaceStatus: "ready",
    });
  }
  return invoke<WorkspaceState>("create_recommended_workspace");
}

export function chooseAndCreateWorkspace() {
  if (!isTauriRuntime()) {
    return Promise.resolve<WorkspaceState | null>({
      ...previewWorkspaceState(),
      workspaceReady: true,
      workspaceStatus: "ready",
    });
  }
  return invoke<WorkspaceState | null>("choose_and_create_workspace");
}

export function getDiagnosticLogSettings() {
  if (!isTauriRuntime()) {
    return Promise.resolve<DiagnosticLogSettings>({
      saveLogs: true,
      logDir: "BiblioSmith/launcher/logs",
      logFile: "BiblioSmith/launcher/logs/bibliosmith-launcher.log",
      maxBytes: 4 * 1024 * 1024,
      backupCount: 5,
      maxTotalBytes: 24 * 1024 * 1024,
    });
  }
  return invoke<DiagnosticLogSettings>("get_diagnostic_log_settings");
}

export function setSaveLogsEnabled(saveLogs: boolean) {
  if (!isTauriRuntime()) {
    return Promise.resolve<DiagnosticLogSettings>({
      saveLogs,
      logDir: "BiblioSmith/launcher/logs",
      logFile: "BiblioSmith/launcher/logs/bibliosmith-launcher.log",
      maxBytes: 4 * 1024 * 1024,
      backupCount: 5,
      maxTotalBytes: 24 * 1024 * 1024,
    });
  }
  return invoke<DiagnosticLogSettings>("set_save_logs_enabled", { saveLogs });
}

export function getProxySettings() {
  if (!isTauriRuntime()) {
    return Promise.resolve<NetworkProxySettings>({
      enabled: false,
      scheme: "http",
      host: "127.0.0.1",
      port: 7890,
    });
  }
  return invoke<NetworkProxySettings>("get_proxy_settings");
}

export function saveProxySettings(proxy: NetworkProxySettings) {
  if (!isTauriRuntime()) {
    return Promise.resolve<NetworkProxySettings>(proxy);
  }
  return invoke<NetworkProxySettings>("save_proxy_settings", { proxy });
}

export function testProxySettings(proxy: NetworkProxySettings) {
  if (!isTauriRuntime()) {
    return Promise.resolve<ProxyTestResult>({
      ok: true,
      message: "Preview mode: proxy test succeeded in 38 ms.",
      elapsedMs: 38,
      httpVersion: "HTTP/2",
      targetUrl: "https://api.github.com/",
    });
  }
  return invoke<ProxyTestResult>("test_proxy_settings", { proxy });
}

export function autoDetectProxySettings(force = true) {
  if (!isTauriRuntime()) {
    return Promise.resolve<ProxyAutoDetectResult>({
      detected: true,
      proxy: { enabled: true, scheme: "http", host: "127.0.0.1", port: 7890 },
      test: {
        ok: true,
        message: "Preview mode: proxy auto detection succeeded.",
        elapsedMs: 36,
        httpVersion: "HTTP/2",
        targetUrl: "https://api.github.com/",
      },
      message: "Preview mode: detected local proxy.",
    });
  }
  return invoke<ProxyAutoDetectResult>("auto_detect_proxy_settings", { force });
}

export function getModelCatalog() {
  if (!isTauriRuntime()) {
    // Preview mode: a couple of slots, one already "configured", so the panel
    // renders without a Tauri backend.
    return Promise.resolve<ModelCatalog>({
      slots: [
        {
          profileId: "deepseek",
          configId: "deepseek-default",
          providerType: "openai-compatible",
          defaultModel: "deepseek-v4-flash",
          configured: true,
        },
        {
          profileId: "qwen",
          configId: "payg",
          providerType: "openai-responses",
          defaultModel: "qwen3.7-max",
          configured: false,
          workspaceId: null,
          webSearchEnabled: false,
        },
        {
          profileId: "doubao",
          configId: "cn-beijing",
          providerType: "openai-responses",
          defaultModel: "doubao-seed-2-1-pro-260628",
          configured: false,
        },
      ],
      active: {
        profileId: "deepseek",
        configId: "deepseek-default",
        model: "deepseek-v4-flash",
      },
    });
  }
  return invoke<ModelCatalog>("get_model_catalog");
}

export function saveModelCredential(
  profileId: string,
  configId: string,
  apiKey: string,
) {
  if (!isTauriRuntime()) {
    return Promise.resolve();
  }
  return invoke<void>("save_model_credential", { profileId, configId, apiKey });
}

export function saveQwenSettings(
  workspaceId: string,
  webSearchEnabled: boolean,
) {
  if (!isTauriRuntime()) {
    return Promise.resolve();
  }
  return invoke<void>("save_qwen_settings", {
    workspaceId,
    webSearchEnabled,
  });
}

export function deleteModelCredential(profileId: string, configId: string) {
  if (!isTauriRuntime()) {
    return Promise.resolve();
  }
  return invoke<void>("delete_model_credential", { profileId, configId });
}

export function setActiveModel(
  profileId: string,
  configId: string,
  model: string,
) {
  if (!isTauriRuntime()) {
    return Promise.resolve();
  }
  return invoke<void>("set_active_model", { profileId, configId, model });
}

export function testModelConnection(
  profileId: string,
  configId: string,
  model: string,
  apiKey?: string,
) {
  if (!isTauriRuntime()) {
    return Promise.resolve<ModelConnectionResult>({
      ok: true,
      message: "Preview mode: connection test succeeded.",
    });
  }
  return invoke<ModelConnectionResult>("test_model_connection", {
    profileId,
    configId,
    model,
    apiKey: apiKey ?? null,
  });
}

export function getEmbeddingStatus() {
  if (!isTauriRuntime()) {
    return Promise.resolve<EmbeddingStatus>({ backend: "gemini", configured: false });
  }
  return invoke<EmbeddingStatus>("get_embedding_status");
}

export function saveEmbeddingCredential(apiKey: string) {
  if (!isTauriRuntime()) {
    return Promise.resolve();
  }
  return invoke<void>("save_embedding_credential", { apiKey });
}

export function deleteEmbeddingCredential() {
  if (!isTauriRuntime()) {
    return Promise.resolve();
  }
  return invoke<void>("delete_embedding_credential");
}

export function testEmbeddingConnection(apiKey?: string) {
  if (!isTauriRuntime()) {
    return Promise.resolve<EmbeddingConnectionResult>({
      ok: true,
      message: "Preview mode: connection test succeeded.",
    });
  }
  return invoke<EmbeddingConnectionResult>("test_embedding_connection", {
    apiKey: apiKey ?? null,
  });
}

export function getOcrCredentialsStatus() {
  if (!isTauriRuntime()) {
    return Promise.resolve<OcrCredentialsStatus>({
      paddleocr: { configured: false, source: null },
      mineru: { configured: true, source: "env" },
    });
  }
  return invoke<OcrCredentialsStatus>("get_ocr_credentials_status");
}

export function saveOcrCredential(service: "paddleocr" | "mineru", apiKey: string) {
  if (!isTauriRuntime()) {
    return Promise.resolve();
  }
  return invoke<void>("save_ocr_credential", { service, apiKey });
}

export function deleteOcrCredential(service: "paddleocr" | "mineru") {
  if (!isTauriRuntime()) {
    return Promise.resolve();
  }
  return invoke<void>("delete_ocr_credential", { service });
}

export function testOcrConnection(service: "paddleocr" | "mineru", apiKey?: string) {
  if (!isTauriRuntime()) {
    return Promise.resolve<OcrConnectionResult>({
      ok: true,
      message: "Preview mode: connection test succeeded.",
    });
  }
  return invoke<OcrConnectionResult>("test_ocr_connection", {
    service,
    apiKey: apiKey ?? null,
  });
}

export function getRuntimeStatus() {
  if (!isTauriRuntime()) {
    return Promise.resolve<RuntimeStatus>({
      ready: true,
      privateReady: true,
      running: false,
      runtimeRoot: "BiblioSmith/runtimes",
      python: {
        ready: true,
        privateReady: true,
        version: "3.12.10",
        source: "private",
        path: "BiblioSmith/runtimes/python/python.exe",
        message: "Python private runtime is ready.",
      },
      java: {
        ready: true,
        privateReady: true,
        version: "17.0.19",
        source: "private",
        path: "BiblioSmith/runtimes/java/bin/java.exe",
        message: "Java private runtime is ready.",
      },
    });
  }
  return invoke<RuntimeStatus>("get_runtime_status");
}

export function startRuntimePrepare() {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode." });
  }
  return invoke<ActionResult>("start_runtime_prepare");
}

export function exportLauncherLogs() {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode." });
  }
  return invoke<ActionResult>("export_launcher_logs");
}

export function recordFrontendActivity(level: string, message: string) {
  if (!isTauriRuntime()) {
    void level;
    void message;
    return Promise.resolve();
  }
  return invoke<void>("record_frontend_activity", { level, message });
}

export function minimizeMainWindow() {
  if (!isTauriRuntime()) return Promise.resolve();
  return invoke<void>("minimize_main_window");
}

export function toggleMainWindowMaximized() {
  if (!isTauriRuntime()) return Promise.resolve(false);
  return invoke<boolean>("toggle_main_window_maximized");
}

export function closeMainWindowToTray() {
  if (!isTauriRuntime()) return Promise.resolve();
  return invoke<void>("close_main_window_to_tray");
}

export function getBookPipelineState() {
  if (!isTauriRuntime()) {
    return Promise.resolve<BookPipelineState>({
      schemaVersion: BOOK_PIPELINE_STATE_SCHEMA_VERSION,
      revision: previewBookPipelineRevision,
      jobs: previewBookPipelineJobs,
    });
  }
  return invoke<BookPipelineState>("get_book_pipeline_state");
}

function promptPackReference(revision: TranslationPromptPackRevision): TranslationPromptPackReference {
  return {
    packId: revision.packId,
    revisionId: revision.revisionId,
    contentSha256: revision.contentSha256,
  };
}

function canonicalPromptPackJson(value: unknown): string {
  if (value === null || typeof value === "boolean" || typeof value === "number") return JSON.stringify(value);
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalPromptPackJson).join(",")}]`;
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record)
      .filter((key) => record[key] !== undefined)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalPromptPackJson(record[key])}`)
      .join(",")}}`;
  }
  throw new Error("Prompt pack revision contains a non-JSON value.");
}

async function previewPromptPackContentSha256(revision: TranslationPromptPackRevision): Promise<string> {
  const snapshot: Record<string, unknown> = structuredClone(revision);
  Reflect.deleteProperty(snapshot, "contentSha256");
  const bytes = new TextEncoder().encode(canonicalPromptPackJson(snapshot));
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function previewPromptPackRevision(reference: TranslationPromptPackReference) {
  return [...previewTranslationPromptCatalog.packs, ...previewDeletedTranslationPromptPacks]
    .find((pack) => pack.packId === reference.packId)
    ?.revisions.find((revision) => revision.revisionId === reference.revisionId && revision.contentSha256 === reference.contentSha256);
}

function previewStringRecord(value: unknown, field: string): Record<string, string> | undefined {
  if (value === undefined) return undefined;
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`Prompt pack ${field} must be a string map.`);
  }
  const entries = Object.entries(value);
  if (entries.some(([, item]) => typeof item !== "string")) {
    throw new Error(`Prompt pack ${field} must be a string map.`);
  }
  return Object.fromEntries(entries) as Record<string, string>;
}

function previewPromptPackDefaultKey(executor: string, sourceLanguage: string, targetLanguage: string) {
  return `${executor}:${sourceLanguage}:${targetLanguage}`;
}

export function listTranslationPromptPacks() {
  if (!isTauriRuntime()) return Promise.resolve(structuredClone(previewTranslationPromptCatalog));
  return invoke<TranslationPromptPackCatalog>("list_translation_prompt_packs");
}

export function getTranslationPromptPackDefault(
  executor: "programmatic" | "expert-agent",
  sourceLanguage = "auto",
  targetLanguage = "zh-Hans",
) {
  if (!isTauriRuntime()) {
    const key = previewPromptPackDefaultKey(executor, sourceLanguage, targetLanguage);
    const selected = previewTranslationPromptDefaults.get(key);
    if (selected) return Promise.resolve(structuredClone(selected));
    const packId = executor === "programmatic" ? "builtin.structure-fidelity" : "builtin.context-backtracking";
    const revision = previewTranslationPromptCatalog.packs.find((pack) => pack.packId === packId)?.revisions.at(-1);
    if (!revision) return Promise.reject(new Error("Preview prompt pack default not found."));
    return Promise.resolve(promptPackReference(revision));
  }
  return invoke<TranslationPromptPackReference>("get_translation_prompt_pack_default", {
    executor,
    sourceLanguage,
    targetLanguage,
  });
}

export async function copyTranslationPromptPack(sourceReference: TranslationPromptPackReference, displayName: string) {
  if (!isTauriRuntime()) {
    const source = previewPromptPackRevision(sourceReference);
    if (!source) throw new Error("Preview prompt pack revision not found.");
    const packId = `local.preview-${crypto.randomUUID()}`;
    const revision: TranslationPromptPackRevision = {
      ...structuredClone(source),
      packId,
      revisionId: "local-1",
      contentSha256: "",
      displayName: displayName.trim(),
      source: {
        ...structuredClone(source.source),
        kind: "local-copy",
        sourcePackId: sourceReference.packId,
        sourceRevisionId: sourceReference.revisionId,
        sourceContentSha256: sourceReference.contentSha256,
      },
    };
    revision.contentSha256 = await previewPromptPackContentSha256(revision);
    const pack: TranslationPromptPackDefinition = {
      packId,
      kind: "custom",
      summary: `“${source.displayName}”的本地副本`,
      revisions: [revision],
    };
    previewTranslationPromptCatalog = {
      ...previewTranslationPromptCatalog,
      packs: [...previewTranslationPromptCatalog.packs, pack],
    };
    return structuredClone(pack);
  }
  return invoke<TranslationPromptPackDefinition>("copy_translation_prompt_pack", { sourceReference, displayName });
}

export async function saveTranslationPromptPackRevision(draft: TranslationPromptPackRevisionDraft) {
  if (!isTauriRuntime()) {
    const pack = previewTranslationPromptCatalog.packs.find((item) => item.packId === draft.packId);
    const previous = pack?.revisions.at(-1);
    if (!pack || pack.kind === "builtin" || !previous) throw new Error("Only local prompt packs can be edited.");
    if (!draft.displayName.trim()) throw new Error("displayName is required.");
    if (draft.stages.length !== previous.stages.length || draft.stages.some((stage, index) => {
      const locked = previous.stages[index];
      return !locked
        || stage.stageId !== locked.stageId
        || stage.label !== locked.label
        || !stage.template.trim();
    })) {
      throw new Error("Prompt pack executor contract is read-only.");
    }
    if (Object.entries(draft.parameters).some(([key, value]) =>
      !["qualityFocus", "styleGuidance"].includes(key)
      || !value.trim()
      || [...value].length > 2_000)) {
      throw new Error("Prompt pack parameter is not editable.");
    }
    const revision: TranslationPromptPackRevision = {
      ...structuredClone(previous),
      revisionId: `local-${pack.revisions.length + 1}`,
      contentSha256: "",
      displayName: draft.displayName.trim(),
      parameters: structuredClone(draft.parameters),
      stages: structuredClone(draft.stages),
    };
    revision.contentSha256 = await previewPromptPackContentSha256(revision);
    pack.revisions.push(revision);
    return structuredClone(revision);
  }
  return invoke<TranslationPromptPackRevision>("save_translation_prompt_pack_revision", { draft });
}

export function deleteTranslationPromptPack(packId: string) {
  if (!isTauriRuntime()) {
    const pack = previewTranslationPromptCatalog.packs.find((item) => item.packId === packId);
    if (!pack || pack.kind === "builtin") return Promise.reject(new Error("Only local prompt packs can be deleted."));
    if ([...previewTranslationPromptDefaults.values()].some((reference) => reference.packId === packId)) {
      return Promise.reject(new Error("The current default prompt pack cannot be deleted."));
    }
    previewDeletedTranslationPromptPacks.push({
      ...structuredClone(pack),
      deletedAt: new Date().toISOString(),
    });
    previewTranslationPromptCatalog = {
      ...previewTranslationPromptCatalog,
      packs: previewTranslationPromptCatalog.packs.filter((item) => item.packId !== packId),
    };
    return Promise.resolve();
  }
  return invoke<void>("delete_translation_prompt_pack", { packId });
}

export function setTranslationPromptPackDefault(
  executor: "programmatic" | "expert-agent",
  promptPackReference: TranslationPromptPackReference,
  sourceLanguage = "auto",
  targetLanguage = "zh-Hans",
) {
  if (!isTauriRuntime()) {
    const revision = previewPromptPackRevision(promptPackReference);
    if (!revision || revision.executor !== executor) return Promise.reject(new Error("Prompt pack executor mismatch."));
    previewTranslationPromptDefaults.set(
      previewPromptPackDefaultKey(executor, sourceLanguage, targetLanguage),
      structuredClone(promptPackReference),
    );
    return Promise.resolve();
  }
  return invoke<void>("set_translation_prompt_pack_default", {
    executor,
    sourceLanguage,
    targetLanguage,
    promptPackReference,
  });
}

export function diffTranslationPromptPackRevisions(
  before: TranslationPromptPackReference,
  after: TranslationPromptPackReference,
) {
  if (!isTauriRuntime()) {
    const left = previewPromptPackRevision(before);
    const right = previewPromptPackRevision(after);
    if (!left || !right || left.packId !== right.packId) return Promise.reject(new Error("Prompt pack revisions must belong to the same pack."));
    const stageIds = new Set([...left.stages, ...right.stages].map((stage) => stage.stageId));
    const stages = [...stageIds].flatMap((stageId) => {
      const beforeTemplate = left.stages.find((stage) => stage.stageId === stageId)?.template;
      const afterTemplate = right.stages.find((stage) => stage.stageId === stageId)?.template;
      return beforeTemplate === afterTemplate ? [] : [{ stageId, beforeTemplate, afterTemplate }];
    });
    const diff: TranslationPromptPackRevisionDiff = {
      before,
      after,
      beforeMetadata: {
        displayName: left.displayName,
        executor: left.executor,
        sourceLanguage: left.sourceLanguage,
        targetLanguage: left.targetLanguage,
        costHint: left.costHint,
        source: structuredClone(left.source),
        contextPolicy: left.contextPolicy,
        requiredSkillIds: structuredClone(left.requiredSkillIds ?? []),
        requiredEvidence: structuredClone(left.requiredEvidence ?? []),
        excludedResponsibilities: structuredClone(left.excludedResponsibilities ?? []),
        parameters: structuredClone(left.parameters ?? {}),
        evidencePolicy: structuredClone(left.evidencePolicy),
      },
      afterMetadata: {
        displayName: right.displayName,
        executor: right.executor,
        sourceLanguage: right.sourceLanguage,
        targetLanguage: right.targetLanguage,
        costHint: right.costHint,
        source: structuredClone(right.source),
        contextPolicy: right.contextPolicy,
        requiredSkillIds: structuredClone(right.requiredSkillIds ?? []),
        requiredEvidence: structuredClone(right.requiredEvidence ?? []),
        excludedResponsibilities: structuredClone(right.excludedResponsibilities ?? []),
        parameters: structuredClone(right.parameters ?? {}),
        evidencePolicy: structuredClone(right.evidencePolicy),
      },
      stages,
    };
    return Promise.resolve(diff);
  }
  return invoke<TranslationPromptPackRevisionDiff>("diff_translation_prompt_pack_revisions", { before, after });
}

export function previewBookPipelineRoute(source: BookPipelineSource, mode: string, config?: BookPipelinePreviewConfig | null) {
  if (!isTauriRuntime()) {
    return Promise.resolve<BookPipelineRouteItem[]>(previewBookPipelineRoutes(source, mode, config));
  }
  return invoke<BookPipelineRouteItem[]>("preview_book_pipeline_route", { source, mode, config });
}

export function discoverBookPipelineZoteroSources(source: BookPipelineSource, limit = 20) {
  if (!isTauriRuntime()) {
    return Promise.resolve<BookPipelineZoteroDiscoveryResult>(previewZoteroDiscovery(source));
  }
  return invoke<BookPipelineZoteroDiscoveryResult>("discover_book_pipeline_zotero_sources", { source, limit });
}

export function queueBookPipelineJob(
  source: BookPipelineSource,
  mode: string,
  translationIntent: BookPipelineTranslationIntent,
  config?: BookPipelinePreviewConfig | null,
) {
  if (!isTauriRuntime()) {
    const rejection = enqueueModeRejection(mode);
    if (rejection) return Promise.reject(new Error(rejection));
    const job = previewBookPipelineJob(source, mode, config);
    job.translationMode = translationIntent.translationMode;
    job.translationProfileId = translationIntent.profileId;
    job.translationConfigId = translationIntent.configId;
    job.translationSkillIds = translationIntent.skillIds;
    job.promptPackReference = translationIntent.promptPackReference;
    job.secondPassEnabled = translationIntent.secondPassEnabled;
    job.textCleanup = translationIntent.textCleanup;
    job.digestMode = translationIntent.digestMode;
    job.outputFormats = translationIntent.outputFormats;
    for (const child of job.children) {
      child.promptPackReference = translationIntent.promptPackReference;
      const digest = child.stages.find((stage) => stage.stageId === "build_digest");
      if (digest && child.status !== "skipped") digest.status = translationIntent.digestMode ? "pending" : "skipped";
    }
    previewBookPipelineJobs = [job, ...previewBookPipelineJobs];
    previewBookPipelineRevision += 1;
    return Promise.resolve<BookPipelineJob>(job);
  }
  return invoke<BookPipelineJob>("queue_book_pipeline_job", { source, mode, translationIntent, config });
}

export function selectBookTranslationPromptPack(
  jobId: string,
  childId: string | null,
  promptPackReference: TranslationPromptPackReference,
) {
  if (!isTauriRuntime()) {
    const job = previewBookPipelineJobs.find((item) => item.id === jobId);
    if (!job) return Promise.reject(new Error("Preview job not found."));
    const child = childId ? job.children.find((item) => item.id === childId) : job.children[0];
    if (!child) return Promise.reject(new Error("Preview child job not found."));
    const revision = previewPromptPackRevision(promptPackReference);
    const expectedExecutor = job.translationMode === "expert" ? "expert-agent" : "programmatic";
    if (!revision || revision.executor !== expectedExecutor) return Promise.reject(new Error("Prompt pack executor mismatch."));
    child.promptPackReference = structuredClone(promptPackReference);
    child.promptPackSelectionSource = "book-override";
    if (job.children.length === 1) {
      job.promptPackReference = structuredClone(promptPackReference);
      job.promptPackSelectionSource = "book-override";
    }
    job.approvalReferences = job.approvalReferences.filter((approval) => approval.childJobId !== child.id);
    job.updatedAt = bookPipelineNow();
    previewBookPipelineRevision += 1;
    return Promise.resolve<BookPipelineJob>({ ...job });
  }
  return invoke<BookPipelineJob>("select_book_translation_prompt_pack", {
    jobId,
    childId,
    promptPackReference,
  });
}

export function getBookPipelineStructureCorrectionDraft(
  jobId: string,
  childId: string | null,
) {
  if (!isTauriRuntime()) {
    return Promise.reject(new Error("Structure correction is available only in the desktop app."));
  }
  return invoke<BookPipelineStructureCorrectionDraft>(
    "get_book_pipeline_structure_correction_draft",
    { jobId, childId },
  );
}

export function saveBookPipelineStructureCorrection(
  jobId: string,
  childId: string | null,
  correction: BookPipelineStructureCorrectionInput,
) {
  if (!isTauriRuntime()) {
    return Promise.reject(new Error("Structure correction is available only in the desktop app."));
  }
  return invoke<BookPipelineJob>("save_book_pipeline_structure_correction", {
    jobId,
    childId,
    correction,
  });
}

export function previewBookTranslationPrompt(jobId: string, childId: string | null) {
  if (!isTauriRuntime()) {
    const job = previewBookPipelineJobs.find((item) => item.id === jobId);
    const child = childId ? job?.children.find((item) => item.id === childId) : job?.children[0];
    if (!job || !child) return Promise.reject(new Error("Preview job not found."));
    const revision = previewPromptPackRevision(child.promptPackReference);
    if (!revision) return Promise.reject(new Error("Prompt pack revision not found."));
    const parameterBlock = Object.entries(revision.parameters ?? {})
      .map(([key, value]) => `${key}: ${value}`)
      .join("\n");
    return Promise.resolve({
      promptPackReference: child.promptPackReference,
      stages: revision.stages.map((stage) => ({
        stageId: stage.stageId,
        label: stage.label,
        actualPrompt: `${stage.template}${parameterBlock ? `\n\n[开放方案参数]\n${parameterBlock}` : ""}\n\n[执行器保护约束]\n结构、术语、占位符与私人文本边界由 BiblioSmith 执行器拥有。`,
        injections: ["template", "current-source", "neighbor-context:none-for-first-segment", "glossary", "executor-safety"],
      })),
      contextPolicy: revision.contextPolicy,
      requiredSkillIds: revision.requiredSkillIds,
      skillDependencyVersions: previewStringRecord(revision.source.skillVersions, "source.skillVersions"),
      requiredEvidence: revision.requiredEvidence,
      excludedResponsibilities: revision.excludedResponsibilities,
      parameters: revision.parameters,
    });
  }
  return invoke<TranslationPromptPreview>("preview_book_translation_prompt", { jobId, childId });
}

export function runBookPipelineJob(jobId: string) {
  if (!isTauriRuntime()) {
    const job = previewBookPipelineJobs.find((item) => item.id === jobId);
    if (!job) return Promise.reject(new Error("Preview job not found."));
    const eligible = job.children.some((child) => child.stages.some((stage) => stage.stageId === "extract" && (stage.status === "ready" || stage.status === "failed")));
    if (!eligible) return Promise.reject(new Error("No eligible extraction stage is ready to run or retry."));
    job.attempts += 1;
    if (job.source.runnerBehavior === "always_fail" || (job.source.runnerBehavior === "fail_once" && job.attempts === 1)) {
      job.currentStep = "Failed";
      job.lastError = "Fake CLI runner failed intentionally.";
      for (const child of job.children) {
        const extract = child.stages.find((stage) => stage.stageId === "extract" && (stage.status === "ready" || stage.status === "failed"));
        if (extract) {
          extract.status = "failed";
          extract.attempt += 1;
          extract.error = job.lastError;
          child.lastError = job.lastError;
        }
      }
      derivePreviewBookPipelineJob(job);
      job.updatedAt = bookPipelineNow();
      job.logSummary = [...job.logSummary, `Attempt ${job.attempts} failed`];
      previewBookPipelineRevision += 1;
      return Promise.resolve({ ...job });
    }
    const handoffAfterRun = shouldHandoffAfterRun(job.mode);
    const batchSource = isBookPipelineZoteroBatchSource(job.source);
    const blockedBatchItems = batchSource ? job.route.filter((item) => item.routeKind !== "translation_handoff" && !item.canRun) : [];
    job.currentStep = handoffAfterRun ? "Translation handoff ready" : blockedBatchItems.length ? "Collection summary: completed=1 failed=0 blocked=1 skipped=1" : "Completed";
    job.lastError = null;
    job.outputDir = "/tmp/bibliosmith-preview/book-pipeline";
    const markdown = { kind: "markdown", path: `${job.outputDir}/preview.md`, sha256: "preview-sha256", zoteroKey: null };
    job.artifacts = [
      { kind: "output_dir", path: job.outputDir, sha256: null, zoteroKey: null },
      markdown,
      { kind: "html", path: `${job.outputDir}/preview.html`, sha256: "preview-sha256", zoteroKey: null },
      { kind: "epub", path: `${job.outputDir}/preview.epub`, sha256: "preview-sha256", zoteroKey: null },
    ];
    if (batchSource) {
      job.collectionItems = job.route.filter((item) => item.routeKind !== "translation_handoff").map((item) => ({
        id: item.id,
        title: item.title,
        routeKind: item.routeKind,
        status: item.canRun ? "completed" : item.routeKind === "already_converted" ? "skipped" : "blocked",
        lastError: item.canRun ? null : item.blockedReason || item.summary,
        artifacts: item.canRun ? [markdown] : [],
        attempts: job.attempts,
      }));
    }
    for (const child of job.children) {
      const extract = child.stages.find((stage) => stage.stageId === "extract" && (stage.status === "ready" || stage.status === "failed"));
      if (!extract) continue;
      extract.status = "completed";
      extract.attempt += 1;
      extract.error = null;
      child.artifacts = [markdown];
      child.attempts = job.attempts;
      child.lastError = null;
      if (handoffAfterRun) {
        const handoff = child.stages.find((stage) => stage.stageId === "handoff");
        const split = child.stages.find((stage) => stage.stageId === "split");
        if (handoff) handoff.status = "completed";
        if (split) split.status = "ready";
      }
    }
    if (handoffAfterRun) {
      job.artifacts.push(
        { kind: "translation_project", path: "/tmp/bibliosmith-preview/books/local/zh-Hans/001_preview", sha256: null, zoteroKey: null },
        { kind: "translation_source", path: "/tmp/bibliosmith-preview/books/local/zh-Hans/001_preview/source/source.md", sha256: markdown.sha256, zoteroKey: null },
      );
    }
    job.updatedAt = bookPipelineNow();
    derivePreviewBookPipelineJob(job);
    job.logSummary = [
      ...job.logSummary,
      `Attempt ${job.attempts} completed`,
      ...(handoffAfterRun ? [job.mode === "convert_then_translate" ? "Conversion completed; translation handoff started" : "Source preparation completed; translation handoff started", "Translation handoff ready"] : []),
    ];
    previewBookPipelineRevision += 1;
    return Promise.resolve({ ...job });
  }
  return invoke<BookPipelineJob>("run_book_pipeline_job", { jobId });
}

export function retryBookPipelineJob(jobId: string) {
  if (!isTauriRuntime()) {
    return runBookPipelineJob(jobId);
  }
  return invoke<BookPipelineJob>("retry_book_pipeline_job", { jobId });
}

export function removeBooksFromShelf(selections: BookPipelineShelfSelection[]) {
  if (!isTauriRuntime()) {
    const grouped = new Map<string, Set<string | null>>();
    for (const selection of selections) {
      const children = grouped.get(selection.jobId) ?? new Set<string | null>();
      children.add(selection.childId ?? null);
      grouped.set(selection.jobId, children);
    }
    previewBookPipelineJobs = previewBookPipelineJobs.flatMap((job) => {
      const selectedChildren = grouped.get(job.id);
      if (!selectedChildren) return [job];
      if (selectedChildren.has(null)) return [];
      const children = job.children.filter((child) => !selectedChildren.has(child.id));
      return children.length === 0 ? [] : [{ ...job, children }];
    });
    previewBookPipelineRevision += 1;
    return Promise.resolve<BookPipelineState>({
      schemaVersion: BOOK_PIPELINE_STATE_SCHEMA_VERSION,
      revision: previewBookPipelineRevision,
      jobs: previewBookPipelineJobs,
    });
  }
  return invoke<BookPipelineState>("remove_books_from_shelf", {
    selections,
    explicitApproval: true,
  });
}

export function inspectBookPipelineProjectMigration(jobId: string, childId?: string | null) {
  if (!isTauriRuntime()) {
    return Promise.resolve<BookPipelineProjectMigration>({
      required: false,
      sourceRoot: "",
      destinationRoot: "",
    });
  }
  return invoke<BookPipelineProjectMigration>("inspect_book_pipeline_project_migration", {
    jobId,
    childId: childId ?? null,
  });
}

export function migrateBookPipelineProject(jobId: string, childId?: string | null) {
  if (!isTauriRuntime()) {
    return getBookPipelineState();
  }
  return invoke<BookPipelineState>("migrate_book_pipeline_project", {
    jobId,
    childId: childId ?? null,
    explicitApproval: true,
  });
}

export function advanceBookPipelineJob(jobId: string, childId?: string | null, invalidateDownstream = false) {
  if (!isTauriRuntime()) {
    const job = previewBookPipelineJobs.find((item) => item.id === jobId);
    if (!job) return Promise.reject(new Error("Preview job not found."));
    const child = childId ? job.children.find((item) => item.id === childId) : job.children[0];
    if (!child) return Promise.reject(new Error("Preview child job not found."));
    const stage = child.stages.find((item) => item.status !== "completed" && item.status !== "skipped");
    if (!stage) return Promise.resolve({ ...job });
    if (stage.stageId === "approve_translation" || stage.stageId === "approve_promotion") {
      return Promise.reject(new Error("Explicit gate approval is required before this stage can advance."));
    }
    void invalidateDownstream;
    stage.status = "completed";
    stage.attempt += 1;
    stage.finishedAt = bookPipelineNow();
    stage.error = null;
    const next = child.stages.find((item) => item.status !== "completed" && item.status !== "skipped");
    if (next?.status === "pending") next.status = "ready";
    job.currentStep = `Completed ${stage.stageId} stage`;
    job.updatedAt = bookPipelineNow();
    job.logSummary = [...job.logSummary, job.currentStep];
    derivePreviewBookPipelineJob(job);
    previewBookPipelineRevision += 1;
    return Promise.resolve({ ...job });
  }
  return invoke<BookPipelineJob>("advance_book_pipeline_job", { jobId, childId, invalidateDownstream });
}

export function approveBookPipelineGate(jobId: string, childId: string, stageId: "approve_translation" | "approve_promotion") {
  if (!isTauriRuntime()) {
    const job = previewBookPipelineJobs.find((item) => item.id === jobId);
    const child = job?.children.find((item) => item.id === childId);
    const stage = child?.stages.find((item) => item.stageId === stageId);
    if (!job || !child || !stage) return Promise.reject(new Error("Preview approval gate not found."));
    if (stage.status !== "ready") return Promise.reject(new Error("Preview approval gate is not ready."));
    stage.status = "completed";
    stage.attempt += 1;
    stage.approvalId = `preview-approval-${crypto.randomUUID()}`;
    stage.finishedAt = bookPipelineNow();
    const next = child.stages.find((item) => item.status !== "completed" && item.status !== "skipped");
    if (next?.status === "pending") next.status = "ready";
    job.currentStep = `Approved ${stageId} gate`;
    job.updatedAt = bookPipelineNow();
    job.logSummary = [...job.logSummary, `Explicit ${stageId} approval recorded`];
    derivePreviewBookPipelineJob(job);
    previewBookPipelineRevision += 1;
    return Promise.resolve({ ...job });
  }
  return invoke<BookPipelineJob>("approve_book_pipeline_gate", { jobId, childId, stageId, explicitApproval: true });
}

/** Record that a person opened the built book in a real reader. */
/** Re-route a book the pipeline held back, without deleting and re-queueing it. */
export async function setBookPipelineRouteOverride(
  jobId: string,
  childId: string,
  routeItemId: string,
  routeOverride: string,
  config?: BookPipelinePreviewConfig,
): Promise<BookPipelineJob> {
  return invoke<BookPipelineJob>("set_book_pipeline_route_override", {
    jobId,
    childId,
    routeItemId,
    routeOverride,
    config,
  });
}

// applyToJob stays false for a plain sample: trying a model out must not decide
// what the full book runs on. Adopting one is setBookPipelineTranslationProvider.
export function runBookPipelineTranslationSample(
  jobId: string,
  childId: string,
  providerProfileId: string,
  providerConfigId: string,
  applyToJob = false,
) {
  if (!isTauriRuntime()) {
    return Promise.reject(new Error("Translation samples require the desktop runtime."));
  }
  return invoke<BookPipelineJob>("run_book_pipeline_translation_sample", {
    jobId,
    childId,
    providerProfileId,
    providerConfigId,
    applyToJob,
  });
}

export function setBookPipelineTranslationProvider(
  jobId: string,
  childId: string,
  providerProfileId: string,
  providerConfigId: string,
) {
  if (!isTauriRuntime()) {
    return Promise.reject(new Error("Changing the translation provider requires the desktop runtime."));
  }
  return invoke<BookPipelineJob>("set_book_pipeline_translation_provider", {
    jobId,
    childId,
    providerProfileId,
    providerConfigId,
  });
}

export function handoffBookPipelineMarkdown(jobId: string, artifactPath?: string | null) {
  if (!isTauriRuntime()) {
    const job = previewBookPipelineJobs.find((item) => item.id === jobId);
    if (!job) return Promise.reject(new Error("Preview job not found."));
    const markdown = artifactPath
      ? job.artifacts.find((artifact) => artifact.kind === "markdown" && artifact.path === artifactPath)
      : job.artifacts.find((artifact) => artifact.kind === "markdown");
    if (!markdown) return Promise.reject(new Error("This job has no cleaned Markdown artifact to hand off."));
    job.currentStep = "Translation handoff ready";
    job.lastError = null;
    job.artifacts = [
      ...job.artifacts,
      { kind: "translation_project", path: "/tmp/bibliosmith-preview/books/local/zh-Hans/001_preview", sha256: null, zoteroKey: null },
      { kind: "translation_source", path: "/tmp/bibliosmith-preview/books/local/zh-Hans/001_preview/source/source.md", sha256: markdown.sha256, zoteroKey: markdown.zoteroKey ?? null },
    ];
    job.logSummary = [...job.logSummary, "Translation handoff ready"];
    job.updatedAt = bookPipelineNow();
    const childIndex = job.children.findIndex((child) => child.stages.some((stage) => stage.stageId === "extract" && stage.status === "completed"));
    if (childIndex >= 0) {
      const child = job.children[childIndex];
      if (!child.stages.some((stage) => stage.stageId === "handoff")) {
        child.stages.push(
          { stageId: "handoff", status: "completed", attempt: 1, error: null, contractVersion: BOOK_PIPELINE_JOB_SCHEMA_VERSION, startedAt: null, finishedAt: job.updatedAt, inputHashes: {}, artifactIds: [], unitSummary: null, approvalId: null },
          { stageId: "split", status: "ready", attempt: 0, error: null, contractVersion: BOOK_PIPELINE_JOB_SCHEMA_VERSION, startedAt: null, finishedAt: null, inputHashes: {}, artifactIds: [], unitSummary: null, approvalId: null },
        );
      } else {
        const handoff = previewPipelineStage(job, childIndex, "handoff");
        const split = previewPipelineStage(job, childIndex, "split");
        if (handoff) handoff.status = "completed";
        if (split) split.status = "ready";
      }
      child.artifacts = [...child.artifacts, ...job.artifacts.filter((artifact) => artifact.kind.startsWith("translation_"))];
    }
    derivePreviewBookPipelineJob(job);
    previewBookPipelineRevision += 1;
    return Promise.resolve({ ...job });
  }
  return invoke<BookPipelineJob>("handoff_book_pipeline_markdown", { jobId, artifactPath });
}

export function previewBookPipelineCleanup() {
  if (!isTauriRuntime()) {
    const candidates = previewBookPipelineCleanupCandidates();
    return Promise.resolve<BookPipelineCleanupPreview>({
      candidates,
      logSummary: [`Found ${candidates.length} cleanup candidate(s) from Book Pipeline job history`],
    });
  }
  return invoke<BookPipelineCleanupPreview>("preview_book_pipeline_cleanup");
}

export function approveBookPipelineCleanup(candidateId: string, explicitApproval: boolean) {
  if (!isTauriRuntime()) {
    if (!explicitApproval) return Promise.reject(new Error("Explicit cleanup approval is required."));
    const candidate = previewBookPipelineCleanupCandidates().find((item) => item.id === candidateId);
    if (!candidate) return Promise.reject(new Error("Cleanup candidate not found."));
    if (!candidate.canApprove) {
      const missing = candidate.checks.filter((check) => !check.ok).map((check) => check.kind).join(", ");
      return Promise.reject(new Error(`Cleanup approval blocked; missing evidence: ${missing}`));
    }
    const job = previewBookPipelineJobs.find((item) => item.id === candidate.jobId);
    if (job) {
      job.logSummary = [
        ...job.logSummary,
        `Cleanup approval recorded at ${bookPipelineNow()} for ${candidate.sourceRef}; existing cleanup wrapper remains the deletion path`,
      ];
    }
    return Promise.resolve<BookPipelineActionResult>({
      ok: true,
      message: "Cleanup approval recorded. The launcher did not delete any source PDF; existing cleanup scripts remain the deletion path.",
      path: candidate.sourcePath || candidate.markdownPath || null,
    });
  }
  return invoke<BookPipelineActionResult>("approve_book_pipeline_cleanup", { candidateId, explicitApproval });
}

export function chooseBookPipelinePdfFolder() {
  if (!isTauriRuntime()) {
    return Promise.resolve<BookPipelineSource | null>({
      kind: "local_pdf_folder",
      title: "Preview PDF folder",
      path: "/tmp/bibliosmith-preview/pdfs",
    });
  }
  return invoke<BookPipelineSource | null>("choose_book_pipeline_pdf_folder");
}

export function openBookPipelineOutput(jobId: string) {
  if (!isTauriRuntime()) {
    return Promise.resolve<BookPipelineActionResult>({ ok: true, message: `Preview output opened for ${jobId}`, path: "/tmp/bibliosmith-preview/book-pipeline" });
  }
  return invoke<BookPipelineActionResult>("open_book_pipeline_output", { jobId });
}

export function readBookPipelineArtifactExcerpt(jobId: string, artifactId: string, maxChars?: number) {
  if (!isTauriRuntime()) {
    // No fabricated sample in preview mode — the gate card falls back to its
    // no-sample layout.
    return Promise.reject(new Error("Artifact excerpts require the desktop runtime."));
  }
  return invoke<BookPipelineArtifactExcerpt>("read_book_pipeline_artifact_excerpt", { jobId, artifactId, maxChars });
}

export function readBookPipelineTranslationSample(jobId: string, childId: string) {
  if (!isTauriRuntime()) {
    return Promise.reject(new Error("Translation samples require the desktop runtime."));
  }
  return invoke<BookPipelineTranslationSampleReport>("read_book_pipeline_translation_sample", { jobId, childId });
}

// Both OCR engines over the same sampled interior pages. Like the translation
// sample this decides nothing on its own: adopting the winner is a separate
// route override, so a comparison can be run and then ignored.
export function runBookPipelineOcrSample(jobId: string, childId: string, samplePages?: number) {
  if (!isTauriRuntime()) {
    return Promise.reject(new Error("OCR samples require the desktop runtime."));
  }
  return invoke<BookPipelineJob>("run_book_pipeline_ocr_sample", { jobId, childId, samplePages });
}

export function readBookPipelineOcrSample(jobId: string, childId: string) {
  if (!isTauriRuntime()) {
    return Promise.reject(new Error("OCR samples require the desktop runtime."));
  }
  return invoke<BookPipelineOcrSampleReport>("read_book_pipeline_ocr_sample", { jobId, childId });
}


function listenDownloadProgress(
  eventName: string,
  callback: (payload: DownloadProgress) => void,
) {
  if (!isTauriRuntime()) return Promise.resolve(() => undefined);
  return listen<DownloadProgress>(eventName, (event) => {
    callback(event.payload);
  }).catch((error) => {
    const message = `frontend event listen failed event=${eventName} error=${String(error)}`;
    console.warn(`Unable to listen for ${eventName}:`, error);
    void recordFrontendActivity("warning", message).catch(() => undefined);
    return () => undefined;
  });
}

export function listenRuntimeProgress(
  callback: (payload: DownloadProgress) => void,
) {
  return listenDownloadProgress("runtime-install-progress", callback);
}
