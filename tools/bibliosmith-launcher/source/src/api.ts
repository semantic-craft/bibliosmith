import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ActionResult,
  BookPipelineActionResult,
  BookPipelineArtifactExcerpt,
  BookPipelineCleanupCandidate,
  BookPipelineCleanupPreview,
  BookPipelineCustomInstructions,
  BookPipelineDiagnosticProfile,
  BookPipelineJob,
  BookPipelinePreviewConfig,
  BookPipelineRouteItem,
  BookPipelineSource,
  BookPipelineState,
  BookPipelineTranslationSampleReport,
  BookPipelineTranslationIntent,
  BookPipelineZoteroDiscoveryResult,
  DiagnosticLogSettings,
  DownloadProgress,
  EmbeddingConnectionResult,
  EmbeddingStatus,
  LauncherState,
  BiblioSmithUpdateInfo,
  ModelCatalog,
  ModelConnectionResult,
  NetworkProxySettings,
  OcrConnectionResult,
  OcrCredentialsStatus,
  NodeModulesStatus,
  ProjectDocument,
  ProxyAutoDetectResult,
  ProxyTestResult,
  RuntimeStatus,
} from "./types";

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

function previewState(): LauncherState {
  return {
    repoRoot: "BiblioSmith-PublicDomain-Translator",
    repoReady: true,
    repoStatus: "ready",
    branch: "main",
    localCommit: "preview",
    localCommitShort: "preview",
    remoteUrl: "origin",
    dirty: false,
    proxyConfigured: false,
    platform: "preview",
  };
}

let previewBookPipelineJobs: BookPipelineJob[] = [];
let previewBookPipelineRevision = 0;
export const BOOK_PIPELINE_STATE_SCHEMA_VERSION = "book-pipeline-state-v5";
const BOOK_PIPELINE_JOB_SCHEMA_VERSION = "book-pipeline-job-v5";

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

function shouldHandoffAfterRun(mode: string) {
  return mode === "convert_then_translate" || mode === "translate_only";
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
    return withTranslationHandoff(source, mode, [{
      id: "local-pdf-folder",
      title: source.title || "Local PDF folder",
      sourceKind: "local_pdf_folder",
      sourceRef: source.path || "",
      routeKind: "remote_paddleocr",
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
  const id = `preview-${Date.now()}`;
  const executionRoutes = route.filter((item) => item.routeKind !== "translation_handoff");
  const childRoutes = isBookPipelineZoteroBatchSource(source) ? executionRoutes.map((item) => [item]) : [executionRoutes];
  const children = childRoutes.map((routes, index) => {
    const selected = routes[0];
    const skipped = selected?.routeKind === "already_converted";
    const runnable = routes.length > 0 && routes.every((item) => item.canRun);
    // Mirrors ordered_child_stage_ids in book_pipeline.rs, including the
    // item-scoped "index" stage the backend only runs for Zotero attachments.
    const wantsItemIndex = isBookPipelineZoteroSource(source);
    const stageIds = shouldHandoffAfterRun(mode)
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
    };
  });
  const job: BookPipelineJob = {
    schemaVersion: BOOK_PIPELINE_JOB_SCHEMA_VERSION,
    id,
    kind: isBookPipelineZoteroBatchSource(source) ? "collection" : "single",
    mode,
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

export function getLauncherState() {
  if (!isTauriRuntime()) {
    return Promise.resolve(previewState());
  }
  return invoke<LauncherState>("get_launcher_state");
}

export function chooseRepoFolder() {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode.", repoRoot: "D:\\BiblioSmith", requiresDownload: false });
  }
  return invoke<ActionResult>("choose_repo_folder");
}

export function setRepoFolder(repoRoot: string) {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode.", repoRoot, requiresDownload: false });
  }
  return invoke<ActionResult>("set_repo_folder", { repoRoot });
}

export function checkBiblioSmithUpdates(locale = "en") {
  if (!isTauriRuntime()) {
    const commits = [
      {
        hash: "a1b2c3d",
        date: "2025-05-25 10:15",
        title: "新增书籍：《时间简史》全文初译",
        summary: "添加《时间简史》第一版全文初译，包含第1-10章内容。",
      },
      {
        hash: "d4e5f6a",
        date: "2025-05-25 09:02",
        title: "优化术语库匹配算法",
        summary: "改进术语匹配逻辑，提高长句和复合句的识别准确率。",
      },
      {
        hash: "b7c8d9e",
        date: "2025-05-24 22:47",
        title: "修复章节导出格式问题",
        summary: "修复 Markdown 导出时标题层级丢失的问题。",
      },
      {
        hash: "e0f1a2b",
        date: "2025-05-24 18:33",
        title: "更新贡献指南",
        summary: "补充翻译规范说明，新增常见问题解答部分。",
      },
      {
        hash: "c3d4e5f",
        date: "2025-05-24 16:11",
        title: "新增西班牙语翻译支持",
        summary: "添加西班牙语语言包与基础术语库支持。",
      },
      {
        hash: "f6a7b8c",
        date: "2025-05-24 12:05",
        title: "改进 Web 编辑器体验",
        summary: "优化段落导航与快捷键提示，提升编辑效率。",
      },
      {
        hash: "9d8c7bb",
        date: "2025-05-23 23:19",
        title: "修复图片引用路径问题",
        summary: "修复部分书籍中图片相对路径失效的问题。",
      },
    ].map((commit) => ({
      ...commit,
      fullMessage: `${commit.title}\n\nZH:\n- ${commit.summary}\n\nEN:\n- Preview English summary for ${commit.hash}.\n\nJA:\n- ${commit.hash} のプレビュー概要。`,
    }));

    return Promise.resolve<BiblioSmithUpdateInfo>({
      repoRoot: "BiblioSmith-PublicDomain-Translator",
      currentCommit: "preview",
      remoteRef: "origin/main",
      behindCount: 7,
      aheadCount: 0,
      hasUpdate: true,
      commits,
    });
  }
  return invoke<BiblioSmithUpdateInfo>("check_bibliosmith_updates", { locale });
}

export function prepareBiblioSmithProject(locale = "en") {
  if (!isTauriRuntime()) {
    return checkBiblioSmithUpdates(locale);
  }
  return invoke<BiblioSmithUpdateInfo>("prepare_bibliosmith_project", { locale });
}

export function syncBiblioSmithProject(locale = "en") {
  if (!isTauriRuntime()) {
    return checkBiblioSmithUpdates(locale);
  }
  return invoke<BiblioSmithUpdateInfo>("sync_bibliosmith_project", { locale });
}

export function updateBiblioSmith() {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode." });
  }
  return invoke<ActionResult>("update_bibliosmith");
}

export function cancelBiblioSmithUpdate() {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode." });
  }
  return invoke<ActionResult>("cancel_bibliosmith_update");
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

export function getNodeModulesStatus() {
  if (!isTauriRuntime()) {
    return Promise.resolve<NodeModulesStatus>({
      ready: true,
      running: false,
      autoInstall: true,
      repoReady: true,
      booksDir: "BiblioSmith/books",
      nodeModulesDir: "BiblioSmith/books/node_modules",
    });
  }
  return invoke<NodeModulesStatus>("get_node_modules_status");
}

export function setAutoInstallNodeModules(enabled: boolean) {
  if (!isTauriRuntime()) {
    return Promise.resolve<NodeModulesStatus>({
      ready: true,
      running: false,
      autoInstall: enabled,
      repoReady: true,
      booksDir: "BiblioSmith/books",
      nodeModulesDir: "BiblioSmith/books/node_modules",
    });
  }
  return invoke<NodeModulesStatus>("set_auto_install_node_modules", { enabled });
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

export function startNodeModulesInstall() {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode." });
  }
  return invoke<ActionResult>("start_node_modules_install");
}

export function cancelNodeModulesInstall(removePartial = false) {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode." });
  }
  return invoke<ActionResult>("cancel_node_modules_install", { removePartial });
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

export function readProjectDocument(kind: "readme" | "howto", locale: string) {
  if (!isTauriRuntime()) {
    const content = kind === "readme"
      ? `# BiblioSmith 本地阅读翻译工作台

<table align="center">
  <tr>
    <td align="center"><h3><a href="./README.zh-CN.md">简体中文</a></h3></td>
    <td align="center"><h3><a href="./docs/guides/how-to-use-local-reading.zh-CN.md">How to use</a></h3></td>
  </tr>
</table>

BiblioSmith 用于处理用户已经拥有的本地 EPUB、PDF、论文和书稿。

## 快速开始

- 打开 [How to use](./docs/guides/how-to-use-local-reading.zh-CN.md)
- 从本地文件创建 \`books/local/\` 项目
`
      : `# How to use

## 开始使用

- 在 BiblioSmith Launcher 的流水线页选择本地 EPUB、PDF 或 Markdown 并新建任务。
- 完成抽取、翻译、审校后，从 output/reading/ 打开产物。
- 阅读 [README](./README.zh-CN.md)。
`;
    return Promise.resolve<ProjectDocument>({
      kind,
      path: `preview/${kind}.md`,
      title: kind === "readme" ? "README" : "How to use",
      content,
    });
  }
  return invoke<ProjectDocument>("read_project_document", { kind, locale });
}

export function readProjectDocumentPath(relativePath: string, locale: string) {
  if (!isTauriRuntime()) {
    return readProjectDocument(relativePath.toLowerCase().includes("how-to-use") ? "howto" : "readme", locale);
  }
  return invoke<ProjectDocument>("read_project_document_path", { relativePath, locale });
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

export function openRepoFolder() {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode." });
  }
  return invoke<ActionResult>("open_repo_folder");
}

export function openBooksFolder() {
  if (!isTauriRuntime()) {
    return Promise.resolve<ActionResult>({ ok: true, message: "Preview mode." });
  }
  return invoke<ActionResult>("open_books_folder");
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
    const job = previewBookPipelineJob(source, mode, config);
    job.translationMode = translationIntent.translationMode;
    job.translationProfileId = translationIntent.profileId;
    job.translationConfigId = translationIntent.configId;
    job.translationSkillIds = translationIntent.skillIds;
    job.secondPassEnabled = translationIntent.secondPassEnabled;
    job.textCleanup = translationIntent.textCleanup;
    job.digestMode = translationIntent.digestMode;
    job.outputFormats = translationIntent.outputFormats;
    for (const child of job.children) {
      const digest = child.stages.find((stage) => stage.stageId === "build_digest");
      if (digest && child.status !== "skipped") digest.status = translationIntent.digestMode ? "pending" : "skipped";
    }
    previewBookPipelineJobs = [job, ...previewBookPipelineJobs];
    previewBookPipelineRevision += 1;
    return Promise.resolve<BookPipelineJob>(job);
  }
  return invoke<BookPipelineJob>("queue_book_pipeline_job", { source, mode, translationIntent, config });
}

export function saveBookPipelineCustomInstructions(
  jobId: string,
  childId: string | null,
  customInstructions: BookPipelineCustomInstructions,
) {
  if (!isTauriRuntime()) {
    const job = previewBookPipelineJobs.find((item) => item.id === jobId);
    if (!job) return Promise.reject(new Error("Preview job not found."));
    const child = childId ? job.children.find((item) => item.id === childId) : job.children[0];
    if (!child) return Promise.reject(new Error("Preview child job not found."));
    const translation = customInstructions.translation?.trim() ? customInstructions.translation : null;
    const reflection = customInstructions.reflection?.trim() ? customInstructions.reflection : null;
    child.customInstructions = translation || reflection ? { translation, reflection } : null;
    job.updatedAt = bookPipelineNow();
    previewBookPipelineRevision += 1;
    return Promise.resolve<BookPipelineJob>({ ...job });
  }
  return invoke<BookPipelineJob>("save_book_pipeline_custom_instructions", {
    jobId,
    childId,
    customInstructions,
  });
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

export function deleteBookPipelineJob(jobId: string, childId?: string | null) {
  if (!isTauriRuntime()) {
    previewBookPipelineJobs = previewBookPipelineJobs.filter((job) => job.id !== jobId);
    previewBookPipelineRevision += 1;
    return Promise.resolve<BookPipelineState>({
      schemaVersion: BOOK_PIPELINE_STATE_SCHEMA_VERSION,
      revision: previewBookPipelineRevision,
      jobs: previewBookPipelineJobs,
    });
  }
  return invoke<BookPipelineState>("delete_book_pipeline_job", {
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
    stage.approvalId = `preview-approval-${Date.now()}`;
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

export function listenBiblioSmithProgress(
  callback: (payload: DownloadProgress) => void,
) {
  return listenDownloadProgress("bibliosmith-project-progress", callback);
}

export function listenNodeModulesProgress(
  callback: (payload: DownloadProgress) => void,
) {
  return listenDownloadProgress("node-modules-install-progress", callback);
}

export function listenRuntimeProgress(
  callback: (payload: DownloadProgress) => void,
) {
  return listenDownloadProgress("runtime-install-progress", callback);
}
