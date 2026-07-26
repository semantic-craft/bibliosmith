import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import {
  BookOpen,
  Download,
  FolderOpen,
} from "lucide-react";
import {
  BOOK_PIPELINE_STATE_SCHEMA_VERSION,
  autoDetectProxySettings,
  cancelBiblioSmithUpdate,
  cancelNodeModulesInstall,
  chooseRepoFolder,
  checkLauncherUpdates,
  chooseBookPipelineMarkdownSource,
  chooseBookPipelinePdfFolder,
  advanceBookPipelineJob,
  setBookPipelineRouteOverride,
  approveBookPipelineGate,
  deleteBookPipelineJob,
  discoverBookPipelineZoteroSources,
  exportLauncherLogs,
  getBookPipelineState,
  getModelCatalog,
  getOcrCredentialsStatus,
  getDiagnosticLogSettings,
  getLauncherState,
  getNodeModulesStatus,
  getProxySettings,
  getRuntimeStatus,
  handoffBookPipelineMarkdown,
  listenBiblioSmithProgress,
  listenNodeModulesProgress,
  listenRuntimeProgress,
  openBooksFolder,
  openBookPipelineOutput,
  openRepoFolder,
  prepareBiblioSmithProject,
  previewBookPipelineRoute,
  queueBookPipelineJob,
  readProjectDocument,
  readProjectDocumentPath,
  recordFrontendActivity,
  retryBookPipelineJob,
  runBookPipelineTranslationSample,
  saveBookPipelineDiagnostic,
  setBookPipelineTranslationProvider,
  runBookPipelineJob,
  saveBookPipelineCustomInstructions,
  setRepoFolder,
  setSaveLogsEnabled,
  setAutoInstallNodeModules,
  saveProxySettings,
  syncBiblioSmithProject,
  startNodeModulesInstall,
  startRuntimePrepare,
  testProxySettings,
} from "./api";
import {
  ActivityItem,
  BookPipelineCustomInstructions,
  BookPipelineJob,
  BookPipelinePreviewConfig,
  BookPipelineDiagnosticProfile,
  BookPipelineRouteItem,
  BookPipelineSource,
  BookPipelineState,
  DiagnosticLogSettings,
  LauncherSettings,
  LauncherState,
  LauncherUpdateInfo,
  BiblioSmithUpdateInfo,
  ModelSlotView,
  NetworkProxySettings,
  NodeModulesStatus,
  DownloadProgress,
  ProjectDocument,
  ProxyTestResult,
  RuntimeStatus,
  RuntimeToolStatus,
} from "./types";
import { copies, detectLocale, type LanguageSetting, type Locale } from "./i18n";
import { type ProductCardProps } from "./components";
import { OverviewPage } from "./pages/overview";
import { UpdatesPage } from "./pages/updates";
import { GuidePage } from "./pages/guide";
import { SettingsPage } from "./pages/settings";
import { LogsPage } from "./pages/logs";
import { pipelineJobOutcomeSucceeded, translationHandoffReady } from "./lib/pipeline-status";
import {
  ConfirmDialog,
  FloatingFeedback,
  RuntimeBootstrapScreen,
  Sidebar,
  Titlebar,
  type ConfirmDialogState,
  type DownloadHudState,
  type FloatingToast,
  type RuntimeBootstrapState,
  type TabId,
  type ToastTone,
  type TutorialHistoryEntry,
  type TutorialKind,
} from "./shell";
import { UNKNOWN_VALUE, commitDate, formatDownloadProgress, nowLabel, sleep, versionFromDate } from "./lib/format";
import launcherVersionManifest from "../launcher-version.json";
import {
  PipelineWorkbench,
  defaultPipelineDraft,
  pipelineCopy,
  type PipelineBusy,
  type PipelineDraft,
  type RouteOverride,
} from "./pipeline";

const SETTINGS_KEY = "bibliosmith-launcher-settings";
const LANGUAGE_KEY = "bibliosmith-launcher-language";
const LAUNCHER_VERSION = `v${launcherVersionManifest.version}`;


const defaultSettings: LauncherSettings = {
  autoStart: false,
  checkLauncherOnLaunch: true,
  saveLogsToLocal: true,
};

function upsertPipelineJob(state: BookPipelineState, job: BookPipelineJob): BookPipelineState {
  const existing = state.jobs.filter((item) => item.id !== job.id);
  return { ...state, revision: state.revision + 1, jobs: [job, ...existing] };
}

function runtimeToolStatusLog(tool: RuntimeToolStatus) {
  return `ready=${tool.ready} privateReady=${tool.privateReady} source=${tool.source ?? "-"} path=${tool.path ?? "-"} version=${tool.version || "-"}`;
}

function runtimeStatusLogKey(status: RuntimeStatus) {
  return [
    status.ready,
    status.privateReady,
    status.running,
    status.runtimeRoot,
    status.python.ready,
    status.python.privateReady,
    status.python.source ?? "",
    status.python.path ?? "",
    status.java.ready,
    status.java.privateReady,
    status.java.source ?? "",
    status.java.path ?? "",
  ].join("|");
}

function runtimeStatusLogMessage(status: RuntimeStatus) {
  return `runtime status ready=${status.ready} privateReady=${status.privateReady} running=${status.running} root=${status.runtimeRoot} python=[${runtimeToolStatusLog(status.python)}] java=[${runtimeToolStatusLog(status.java)}]`;
}

function loadSettings(): LauncherSettings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    return raw ? { ...defaultSettings, ...JSON.parse(raw) } : defaultSettings;
  } catch {
    return defaultSettings;
  }
}

function saveSettings(settings: LauncherSettings) {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
}

function loadLanguageSetting(): LanguageSetting {
  try {
    const raw = localStorage.getItem(LANGUAGE_KEY);
    if (raw === "zh-CN" || raw === "zh-TW" || raw === "ja" || raw === "en") return raw;
  } catch {
    // Fall through to the default below.
  }
  return "zh-CN";
}

export default function App() {
  const [languageSetting, setLanguageSetting] = useState<LanguageSetting>(loadLanguageSetting);
  const locale = useMemo<Locale>(
    () => (languageSetting === "system" ? detectLocale() : languageSetting),
    [languageSetting],
  );
  const copy = copies[locale];
  const updateLanguageSetting = useCallback((value: LanguageSetting) => {
    setLanguageSetting(value);
    try {
      localStorage.setItem(LANGUAGE_KEY, value);
    } catch {
      // Ignore storage failures; the choice still applies for this session.
    }
  }, []);
  const bookPipelineCopy = useMemo(() => pipelineCopy(locale), [locale]);
  const [activeTab, setActiveTab] = useState<TabId>("overview");
  const [state, setState] = useState<LauncherState | null>(null);
  const [launcherUpdate, setLauncherUpdate] = useState<LauncherUpdateInfo | null>(null);
  const [biblioSmithUpdate, setBiblioSmithUpdate] = useState<BiblioSmithUpdateInfo | null>(null);
  const [tutorialKind, setTutorialKind] = useState<TutorialKind>("howto");
  const [tutorialDoc, setTutorialDoc] = useState<ProjectDocument | null>(null);
  const [tutorialHistory, setTutorialHistory] = useState<TutorialHistoryEntry[]>([]);
  const [tutorialLoading, setTutorialLoading] = useState(false);
  const [settings, setSettings] = useState<LauncherSettings>(loadSettings);
  const [diagnosticLogSettings, setDiagnosticLogSettings] = useState<DiagnosticLogSettings | null>(null);
  const [proxySettings, setProxySettings] = useState<NetworkProxySettings>({
    enabled: false,
    scheme: "http",
    host: "127.0.0.1",
    port: 7890,
  });
  const [proxyTestResult, setProxyTestResult] = useState<ProxyTestResult | null>(null);
  const [proxyBusy, setProxyBusy] = useState<"test" | "detect" | null>(null);
  const [runtimeStatus, setRuntimeStatus] = useState<RuntimeStatus | null>(null);
  const [runtimeProgress, setRuntimeProgress] = useState<DownloadProgress | null>(null);
  const [runtimeBootstrapState, setRuntimeBootstrapState] = useState<RuntimeBootstrapState>("ready");
  const [runtimeBootstrapMessage, setRuntimeBootstrapMessage] = useState<string | null>(null);
  const [runtimeBootstrapBlocking, setRuntimeBootstrapBlocking] = useState(false);
  const [nodeModulesStatus, setNodeModulesStatus] = useState<NodeModulesStatus | null>(null);
  const [nodeModulesProgress, setNodeModulesProgress] = useState<DownloadProgress | null>(null);
  const [nodeModulesDownloadState, setNodeModulesDownloadState] = useState<DownloadHudState>("idle");
  const [nodeModulesDownloadMessage, setNodeModulesDownloadMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [refreshInProgress, setRefreshInProgress] = useState(false);
  const [lastRefreshAt, setLastRefreshAt] = useState("");
  const [biblioSmithPreparing, setBiblioSmithPreparing] = useState(false);
  const [biblioSmithSyncing, setBiblioSmithSyncing] = useState(false);
  const [biblioSmithProgress, setBiblioSmithProgress] = useState<DownloadProgress | null>(null);
  const [biblioSmithDownloadState, setBiblioSmithDownloadState] = useState<DownloadHudState>("idle");
  const [biblioSmithDownloadMessage, setBiblioSmithDownloadMessage] = useState<string | null>(null);
  const [biblioSmithDownloadDismissed, setBiblioSmithDownloadDismissed] = useState(false);
  const [biblioSmithRetryMode, setBiblioSmithRetryMode] = useState<"prepare" | "sync">("sync");
  const [showAllCommits, setShowAllCommits] = useState(true);
  const [quickActionsOpen, setQuickActionsOpen] = useState(false);
  const [floatingToast, setFloatingToast] = useState<FloatingToast | null>(null);
  const [confirmDialog, setConfirmDialog] = useState<ConfirmDialogState | null>(null);
  const [pipelineState, setPipelineState] = useState<BookPipelineState>({
    schemaVersion: BOOK_PIPELINE_STATE_SCHEMA_VERSION,
    revision: 0,
    jobs: [],
  });
  const [pipelineDraft, setPipelineDraft] = useState<PipelineDraft>(defaultPipelineDraft);
  // Slots keep their `configured` flag so the wizard can say which providers have
  // a key. Stays empty when the catalog cannot be read, which the wizard reads as
  // "unknown" rather than "none configured".
  const [modelSlots, setModelSlots] = useState<ModelSlotView[]>([]);
  // Default a new job's provider to whatever the user chose in Settings → Models,
  // so the wizard reflects that choice instead of the OpenAI fallback.
  useEffect(() => {
    void getModelCatalog()
      .then((catalog) => {
        setModelSlots(catalog.slots);
        if (!catalog.active) return;
        setPipelineDraft((draft) => ({
          ...draft,
          providerProfileId: catalog.active!.profileId,
          providerConfigId: catalog.active!.configId,
        }));
      })
      .catch(() => undefined);
  }, []);
  // Seed the wizard's OCR-credential chips from what is actually configured
  // (Keychain or repo-root .env) instead of hard-coded defaults. The chips
  // stay clickable as manual overrides for the route preview.
  useEffect(() => {
    void getOcrCredentialsStatus()
      .then((status) => {
        setPipelineDraft((draft) => ({
          ...draft,
          hasPaddleocrCredentials: status.paddleocr.configured,
          hasMineruCredentials: status.mineru.configured,
        }));
      })
      .catch(() => undefined);
  }, []);
  const [pipelinePreview, setPipelinePreview] = useState<BookPipelineRouteItem[]>([]);
  const [pipelineRouteOverrides, setPipelineRouteOverrides] = useState<Record<string, RouteOverride>>({});
  const [pipelineZoteroSources, setPipelineZoteroSources] = useState<BookPipelineSource[]>([]);
  const [pipelineBusy, setPipelineBusy] = useState<PipelineBusy>(null);
  const [activities, setActivities] = useState<ActivityItem[]>([
    { id: "welcome", time: nowLabel(), level: "info", message: copy.welcome },
  ]);
  const runtimeBootstrapReleaseTimer = useRef<number | null>(null);
  const runtimeBootstrapStartedRef = useRef(false);
  const runtimeStatusLogKeyRef = useRef<string | null>(null);
  const refreshInProgressRef = useRef(false);
  const biblioSmithSyncingRef = useRef(false);
  const biblioSmithDownloadDismissedRef = useRef(false);
  const nodeModulesAutoStartRef = useRef(false);
  const startupInitializedRef = useRef(false);
  const launcherCheckInProgressRef = useRef(false);
  const floatingToastTimer = useRef<number | null>(null);

  const addActivity = useCallback((level: ActivityItem["level"], message: string) => {
    void recordFrontendActivity(level, message).catch(() => undefined);
    setActivities((items) => [
      { id: `${Date.now()}-${Math.random()}`, time: nowLabel(), level, message },
      ...items,
    ].slice(0, 80));
  }, []);

  const logRuntimeStatusIfChanged = useCallback((status: RuntimeStatus) => {
    const key = runtimeStatusLogKey(status);
    if (runtimeStatusLogKeyRef.current === key) return;
    runtimeStatusLogKeyRef.current = key;
    void recordFrontendActivity("info", runtimeStatusLogMessage(status)).catch(() => undefined);
  }, []);

  useEffect(() => {
    const onError = (event: ErrorEvent) => {
      const message = event.error?.stack || event.message || "Unknown frontend error";
      void recordFrontendActivity("error", `frontend error: ${message}`).catch(() => undefined);
    };
    const onUnhandledRejection = (event: PromiseRejectionEvent) => {
      const reason = event.reason instanceof Error ? event.reason.stack || event.reason.message : String(event.reason);
      void recordFrontendActivity("error", `frontend unhandled rejection: ${reason}`).catch(() => undefined);
    };
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onUnhandledRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onUnhandledRejection);
    };
  }, []);

  const showFloatingToast = useCallback((message: string, tone: ToastTone = "info") => {
    if (floatingToastTimer.current) {
      window.clearTimeout(floatingToastTimer.current);
    }
    setFloatingToast({ id: Date.now(), message, tone });
    floatingToastTimer.current = window.setTimeout(() => {
      setFloatingToast(null);
      floatingToastTimer.current = null;
    }, 2200);
  }, []);

  const pipelineConfig = useMemo<BookPipelinePreviewConfig>(() => ({
    hasPaddleocrCredentials: pipelineDraft.hasPaddleocrCredentials,
    hasMineruCredentials: pipelineDraft.hasMineruCredentials,
    routeOverrides: pipelineRouteOverrides,
  }), [pipelineDraft.hasMineruCredentials, pipelineDraft.hasPaddleocrCredentials, pipelineRouteOverrides]);

  const buildPipelineSource = useCallback((): BookPipelineSource => {
    if (pipelineDraft.sourceKind === "fake") {
      return {
        kind: "fake",
        title: "Fake source",
        selector: "fake://source",
        runnerBehavior: pipelineDraft.fakeBehavior,
      };
    }
    if (pipelineDraft.sourceKind === "local_pdf_folder") {
      return {
        kind: "local_pdf_folder",
        title: pipelineDraft.localPdfTitle || "Local PDF folder",
        path: pipelineDraft.localPdfFolder,
      };
    }
    if (pipelineDraft.sourceKind === "markdown_source") {
      return {
        kind: "markdown_source",
        title: pipelineDraft.markdownTitle || "Markdown source",
        path: pipelineDraft.markdownPath,
        translationStrategy: pipelineDraft.reflectionTranslation ? "reflection" : null,
      };
    }
    if (pipelineDraft.sourceKind === "external_adapter") {
      return {
        kind: "external_adapter",
        title: "External adapter",
        path: pipelineDraft.externalAdapterInput,
        adapterCommand: pipelineDraft.externalAdapterCommand,
      };
    }
    const discovered = pipelineZoteroSources.find((source) => source.kind === pipelineDraft.sourceKind && source.selector === pipelineDraft.zoteroSelector);
    if (discovered) return discovered;
    return {
      kind: pipelineDraft.sourceKind,
      title: pipelineDraft.zoteroSelector,
      selector: pipelineDraft.zoteroSelector,
    };
  }, [pipelineDraft, pipelineZoteroSources]);

  const refreshBookPipelineState = useCallback(async () => {
    setPipelineBusy((current) => current ?? "loading");
    try {
      setPipelineState(await getBookPipelineState());
    } catch (error) {
      addActivity("warning", String(error));
    } finally {
      setPipelineBusy((current) => (current === "loading" ? null : current));
    }
  }, [addActivity]);

  const previewPipeline = useCallback(async () => {
    setPipelineBusy("preview");
    try {
      const route = await previewBookPipelineRoute(buildPipelineSource(), pipelineDraft.mode, pipelineConfig);
      setPipelinePreview(route);
      addActivity("info", `Book Pipeline route preview: ${route.length} item(s)`);
      showFloatingToast(`Book Pipeline route preview: ${route.length} item(s)`, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, buildPipelineSource, pipelineConfig, pipelineDraft.mode, showFloatingToast]);

  const changePipelineRouteOverride = useCallback((routeItemId: string, override: RouteOverride) => {
    setPipelineRouteOverrides((current) => {
      const next = { ...current };
      if (override === "auto") delete next[routeItemId];
      else next[routeItemId] = override;
      return next;
    });
  }, []);

  // Re-preview whenever an override changes so the route chips and the launch
  // counts reflect the backend's decision rather than a client-side guess.
  const routeOverrideSignature = JSON.stringify(pipelineRouteOverrides);
  useEffect(() => {
    if (pipelinePreview.length === 0) return;
    void previewPipeline();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routeOverrideSignature]);

  const queueAndRunPipeline = useCallback(async (): Promise<boolean> => {
    setPipelineBusy("queue");
    try {
      const source = buildPipelineSource();
      // Fake routes stay zero-cost; real fast routes use the versioned provider
      // registry IDs introduced by #60.
      const translationIntent =
        pipelineDraft.translationMode === "expert"
          ? { translationMode: "expert" as const, profileId: "expert-agent", configId: "default", skillIds: ["expert-translation-quality"], secondPassEnabled: false, textCleanup: false, digestMode: pipelineDraft.mode !== "conversion_only" && pipelineDraft.digestMode, outputFormats: pipelineDraft.outputFormats }
          : source.kind === "fake"
            ? { translationMode: "fast" as const, profileId: "fake-provider-profile", configId: "fake-provider-config", skillIds: [], secondPassEnabled: pipelineDraft.mode !== "conversion_only" && pipelineDraft.secondPassEnabled, textCleanup: pipelineDraft.mode !== "conversion_only" && pipelineDraft.textCleanup, digestMode: pipelineDraft.mode !== "conversion_only" && pipelineDraft.digestMode, outputFormats: pipelineDraft.outputFormats }
            : { translationMode: "fast" as const, profileId: pipelineDraft.providerProfileId, configId: pipelineDraft.providerConfigId, skillIds: [], secondPassEnabled: pipelineDraft.mode !== "conversion_only" && pipelineDraft.secondPassEnabled, textCleanup: pipelineDraft.mode !== "conversion_only" && pipelineDraft.textCleanup, digestMode: pipelineDraft.mode !== "conversion_only" && pipelineDraft.digestMode, outputFormats: pipelineDraft.outputFormats };
      const queued = await queueBookPipelineJob(source, pipelineDraft.mode, translationIntent, pipelineConfig);
      setPipelineState((current) => upsertPipelineJob(current, queued));
      setPipelinePreview(queued.route);
      if (!queued.route.some((item) => item.canRun)) {
        addActivity("warning", "Book Pipeline job blocked by route preview");
        showFloatingToast("Book Pipeline job blocked", "warning");
        return true;
      }
      setPipelineBusy("run");
      const completed = await runBookPipelineJob(queued.id);
      setPipelineState((current) => upsertPipelineJob(current, completed));
      const succeeded = pipelineJobOutcomeSucceeded(completed);
      addActivity(succeeded ? "success" : completed.status === "failed" ? "error" : "warning", `Book Pipeline job ${completed.status}: ${completed.id}`);
      showFloatingToast(`Book Pipeline job ${completed.status}`, succeeded ? "success" : "warning");
      return true;
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
      return false;
    } finally {
      setPipelineBusy(null);
    }
    // providerConfigId selects the billing slot within a brand and is read at
    // the fast-route branch above; leaving it out of the deps kept the memoized
    // callback on the slot that was selected when the profile last changed.
  }, [addActivity, buildPipelineSource, pipelineConfig, pipelineDraft.digestMode, pipelineDraft.mode, pipelineDraft.outputFormats, pipelineDraft.providerConfigId, pipelineDraft.providerProfileId, pipelineDraft.secondPassEnabled, pipelineDraft.textCleanup, pipelineDraft.translationMode, showFloatingToast]);

  const retryPipeline = useCallback(async (jobId: string) => {
    setPipelineBusy("retry");
    try {
      const job = await retryBookPipelineJob(jobId);
      setPipelineState((current) => upsertPipelineJob(current, job));
      addActivity(pipelineJobOutcomeSucceeded(job) ? "success" : "warning", `Book Pipeline retry ${job.status}: ${job.id}`);
      showFloatingToast(`Retry ${job.status}`, pipelineJobOutcomeSucceeded(job) ? "success" : "warning");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const deletePipeline = useCallback(async (jobId: string) => {
    setPipelineBusy("delete");
    try {
      const state = await deleteBookPipelineJob(jobId);
      setPipelineState(state);
      addActivity("success", `Book Pipeline job deleted: ${jobId}`);
      showFloatingToast(bookPipelineCopy.deleteBookDone, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, bookPipelineCopy.deleteBookDone, showFloatingToast]);

  const advancePipeline = useCallback(async (jobId: string, childId: string) => {
    setPipelineBusy("advance");
    try {
      const job = await advanceBookPipelineJob(jobId, childId);
      setPipelineState((current) => upsertPipelineJob(current, job));
      addActivity(job.status === "failed" ? "error" : "info", `Book Pipeline advanced: ${job.currentStep}`);
      showFloatingToast(job.currentStep, job.status === "failed" ? "error" : "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const savePipelineCustomInstructions = useCallback(async (
    jobId: string,
    childId: string,
    customInstructions: BookPipelineCustomInstructions,
  ) => {
    setPipelineBusy("customInstructions");
    try {
      const job = await saveBookPipelineCustomInstructions(jobId, childId, customInstructions);
      setPipelineState((current) => upsertPipelineJob(current, job));
      addActivity("success", `Book Pipeline custom instructions saved: ${jobId}/${childId}`);
      showFloatingToast(bookPipelineCopy.customInstructionsSaved, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, bookPipelineCopy.customInstructionsSaved, showFloatingToast]);

  // Re-route a held book in place. The credentials the backend needs to decide
  // whether a forced provider is usable are the same ones the wizard sends.
  const overridePipelineRoute = useCallback(async (
    jobId: string,
    childId: string,
    routeItemId: string,
    routeOverride: string,
  ) => {
    setPipelineBusy("routeOverride");
    try {
      const job = await setBookPipelineRouteOverride(jobId, childId, routeItemId, routeOverride, {
        hasPaddleocrCredentials: pipelineDraft.hasPaddleocrCredentials,
        hasMineruCredentials: pipelineDraft.hasMineruCredentials,
        routeOverrides: {},
      });
      setPipelineState((current) => upsertPipelineJob(current, job));
      addActivity("info", `Book Pipeline route override: ${routeOverride}`);
      showFloatingToast(job.currentStep, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, pipelineDraft.hasMineruCredentials, pipelineDraft.hasPaddleocrCredentials, showFloatingToast]);

  const approvePipelineGate = useCallback(async (
    jobId: string,
    childId: string,
    stageId: "approve_translation" | "approve_promotion",
  ) => {
    setPipelineBusy("gateApproval");
    try {
      const job = await approveBookPipelineGate(jobId, childId, stageId);
      setPipelineState((current) => upsertPipelineJob(current, job));
      addActivity("success", `Book Pipeline approval recorded: ${stageId}`);
      showFloatingToast(`Approval recorded: ${stageId}`, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const samplePipelineTranslation = useCallback(async (
    jobId: string,
    childId: string,
    providerProfileId: string,
    providerConfigId: string,
  ) => {
    setPipelineBusy("sample");
    try {
      const job = await runBookPipelineTranslationSample(jobId, childId, providerProfileId, providerConfigId);
      setPipelineState((current) => upsertPipelineJob(current, job));
      addActivity("success", `Translation sample ready: ${providerProfileId} · ${providerConfigId}`);
      showFloatingToast(bookPipelineCopy.sampleReady, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, bookPipelineCopy.sampleReady, showFloatingToast]);

  // The explicit half of the sample flow: adopt the slot the sample was run with
  // as the book's own, which is what the full run uses.
  const applyPipelineTranslationProvider = useCallback(async (
    jobId: string,
    childId: string,
    providerProfileId: string,
    providerConfigId: string,
  ) => {
    setPipelineBusy("sample");
    try {
      const job = await setBookPipelineTranslationProvider(jobId, childId, providerProfileId, providerConfigId);
      setPipelineState((current) => upsertPipelineJob(current, job));
      addActivity("success", `Translation provider set: ${providerProfileId} · ${providerConfigId}`);
      showFloatingToast(bookPipelineCopy.appliedSampleProvider, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, bookPipelineCopy.appliedSampleProvider, showFloatingToast]);

  // The three redaction profiles were configured and tested on the backend with
  // no way to reach them, so a user reporting a problem had only screenshots.
  const exportPipelineDiagnostic = useCallback(async (
    jobId: string,
    profile: BookPipelineDiagnosticProfile,
  ) => {
    setPipelineBusy("diagnostic");
    try {
      const result = await saveBookPipelineDiagnostic(jobId, profile);
      addActivity(result.ok ? "success" : "info", result.message);
      showFloatingToast(result.message, result.ok ? "success" : "info");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const handoffPipelineMarkdown = useCallback(async (jobId: string, artifactPath?: string | null) => {
    setPipelineBusy("handoff");
    try {
      const job = await handoffBookPipelineMarkdown(jobId, artifactPath);
      setPipelineState((current) => upsertPipelineJob(current, job));
      const ready = translationHandoffReady(job);
      addActivity(ready ? "success" : "warning", `Book Pipeline handoff ${job.status}: ${job.id}`);
      showFloatingToast(`Handoff ${job.status}`, ready ? "success" : "warning");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const choosePipelinePdfFolder = useCallback(async () => {
    setPipelineBusy("folder");
    try {
      const source = await chooseBookPipelinePdfFolder();
      if (!source) return;
      setPipelineDraft((draft) => ({
        ...draft,
        sourceKind: "local_pdf_folder",
        localPdfFolder: source.path || "",
        localPdfTitle: source.title || "Local PDF folder",
      }));
      setPipelinePreview([]);
      addActivity("info", `Selected PDF folder: ${source.path || ""}`);
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const choosePipelineMarkdownSource = useCallback(async () => {
    setPipelineBusy("markdown");
    try {
      const source = await chooseBookPipelineMarkdownSource();
      if (!source) return;
      setPipelineDraft((draft) => ({
        ...draft,
        sourceKind: "markdown_source",
        mode: "translate_only",
        markdownPath: source.path || "",
        markdownTitle: source.title || "Markdown source",
      }));
      setPipelinePreview([]);
      addActivity("info", `Selected Markdown source: ${source.path || ""}`);
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const discoverPipelineZoteroSources = useCallback(async () => {
    setPipelineBusy("zotero");
    try {
      const result = await discoverBookPipelineZoteroSources(buildPipelineSource(), 20);
      setPipelineZoteroSources(result.sources);
      setPipelinePreview([]);
      for (const line of result.logSummary.slice(-3)) {
        addActivity("info", line);
      }
      if (result.sources[0]) {
        setPipelineDraft((draft) => ({
          ...draft,
          sourceKind: result.sources[0].kind,
          zoteroSelector: result.sources[0].selector || result.sources[0].title || draft.zoteroSelector,
        }));
      }
      addActivity("success", `Discovered ${result.sources.length} Zotero source(s)`);
      showFloatingToast(`Discovered ${result.sources.length} Zotero source(s)`, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, buildPipelineSource, showFloatingToast]);

  // A dedicated search, not a reuse of discoverPipelineZoteroSources: that one
  // reads the current draft via buildPipelineSource(), so calling it right
  // after onDraftChange({ zoteroSelector }) in the same handler would race the
  // draft update and discover with the previous selector. Building the request
  // from the typed query directly sidesteps that.
  const discoverZoteroByQuery = useCallback(async (query: string) => {
    setPipelineBusy("zotero");
    try {
      const source: BookPipelineSource = {
        kind: "zotero_filter",
        title: "Title search",
        selector: `query=${query}`,
      };
      const result = await discoverBookPipelineZoteroSources(source, 20);
      setPipelineZoteroSources(result.sources);
      setPipelinePreview([]);
      for (const line of result.logSummary.slice(-3)) {
        addActivity("info", line);
      }
      setPipelineDraft((draft) => ({
        ...draft,
        sourceKind: result.sources[0]?.kind ?? "zotero_filter",
        zoteroSelector: result.sources[0]?.selector || result.sources[0]?.title || source.selector || draft.zoteroSelector,
      }));
      addActivity("success", `Found ${result.sources.length} Zotero source(s) for "${query}"`);
      showFloatingToast(`Found ${result.sources.length} Zotero source(s)`, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const openPipelineOutput = useCallback(async (jobId: string) => {
    setPipelineBusy("open");
    try {
      const result = await openBookPipelineOutput(jobId);
      addActivity(result.ok ? "success" : "warning", result.message);
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const refreshDiagnosticLogSettings = useCallback(async () => {
    try {
      const info = await getDiagnosticLogSettings();
      setDiagnosticLogSettings(info);
      setSettings((current) => {
        const next = { ...current, saveLogsToLocal: info.saveLogs };
        saveSettings(next);
        return next;
      });
    } catch (error) {
      addActivity("warning", copy.logSettingsLoadFailed(String(error)));
    }
  }, [addActivity, copy]);

  const refreshProxySettings = useCallback(async () => {
    try {
      const proxy = await getProxySettings();
      setProxySettings(proxy);
    } catch (error) {
      addActivity("warning", String(error));
    }
  }, [addActivity]);

  const refreshRuntimeStatus = useCallback(async () => {
    try {
      const status = await getRuntimeStatus();
      setRuntimeStatus(status);
      logRuntimeStatusIfChanged(status);
      return status;
    } catch (error) {
      addActivity("warning", copy.runtimeStatusLoadFailed(String(error)));
      return null;
    }
  }, [addActivity, copy, logRuntimeStatusIfChanged]);

  const startRuntimeBootstrap = useCallback(async (blocking: boolean) => {
    if (runtimeBootstrapReleaseTimer.current) {
      window.clearTimeout(runtimeBootstrapReleaseTimer.current);
      runtimeBootstrapReleaseTimer.current = null;
    }
    setRuntimeBootstrapBlocking(blocking);
    setRuntimeBootstrapState("checking");
    setRuntimeBootstrapMessage(copy.runtimeBootstrapChecking);
    setRuntimeProgress({
      percent: 0.01,
      downloadedBytes: 0,
      totalBytes: 100,
      message: copy.runtimeBootstrapChecking,
      state: "downloading",
    });
    try {
      void recordFrontendActivity("info", `runtime bootstrap start blocking=${blocking}`).catch(() => undefined);
      const status = await getRuntimeStatus();
      setRuntimeStatus(status);
      logRuntimeStatusIfChanged(status);
      if (status.ready) {
        setRuntimeBootstrapState("ready");
        setRuntimeBootstrapMessage(copy.runtimeBootstrapReady);
        setRuntimeProgress({
          percent: 100,
          downloadedBytes: 100,
          totalBytes: 100,
          message: copy.runtimeBootstrapReady,
          state: "success",
        });
        runtimeBootstrapReleaseTimer.current = window.setTimeout(() => {
          setRuntimeBootstrapBlocking(false);
          setRuntimeProgress(null);
          runtimeBootstrapReleaseTimer.current = null;
        }, blocking ? 450 : 800);
        return;
      }
      setRuntimeBootstrapState("preparing");
      setRuntimeBootstrapMessage(copy.runtimeBootstrapPreparing);
      const result = await startRuntimePrepare();
      if (result.requiresDownload === false) {
        const refreshed = await refreshRuntimeStatus();
        setRuntimeBootstrapState("ready");
        setRuntimeBootstrapMessage(refreshed?.ready ? copy.runtimeBootstrapReady : result.message);
        setRuntimeProgress({
          percent: 100,
          downloadedBytes: 100,
          totalBytes: 100,
          message: refreshed?.ready ? copy.runtimeBootstrapReady : result.message,
          state: "success",
        });
        runtimeBootstrapReleaseTimer.current = window.setTimeout(() => {
          setRuntimeBootstrapBlocking(false);
          setRuntimeProgress(null);
          runtimeBootstrapReleaseTimer.current = null;
        }, blocking ? 450 : 800);
        return;
      }
      addActivity("info", copy.runtimePrepareStarted);
    } catch (error) {
      const message = copy.runtimePrepareFailed(String(error));
      setRuntimeBootstrapState("failed");
      setRuntimeBootstrapMessage(message);
      setRuntimeProgress({
        percent: 100,
        downloadedBytes: 0,
        totalBytes: 0,
        message,
        state: "failed",
      });
      addActivity("warning", message);
      if (blocking) {
        runtimeBootstrapReleaseTimer.current = window.setTimeout(() => {
          setRuntimeBootstrapBlocking(false);
          runtimeBootstrapReleaseTimer.current = null;
        }, 1400);
      }
    }
  }, [addActivity, copy, logRuntimeStatusIfChanged, refreshRuntimeStatus]);

  const refreshNodeModulesStatus = useCallback(async () => {
    try {
      const status = await getNodeModulesStatus();
      setNodeModulesStatus(status);
      if (status.ready) {
        setNodeModulesDownloadState("idle");
        setNodeModulesDownloadMessage(null);
        setNodeModulesProgress(null);
      } else if (status.running) {
        setNodeModulesDownloadState("downloading");
      } else {
        setNodeModulesDownloadState((current) => {
          if (current === "downloading" || current === "cancelling") return "failed";
          return current;
        });
        setNodeModulesDownloadMessage((current) => current || (status.repoReady ? copy.nodeModulesIncomplete : null));
      }
    } catch (error) {
      addActivity("warning", copy.nodeModulesStatusFailed(String(error)));
    }
  }, [addActivity, copy]);

  const startBiblioSmithProgress = useCallback((mode: "prepare" | "sync") => {
    setBiblioSmithRetryMode(mode);
    setBiblioSmithDownloadDismissed(false);
    biblioSmithDownloadDismissedRef.current = false;
    setBiblioSmithDownloadState("downloading");
    setBiblioSmithDownloadMessage(null);
    setBiblioSmithProgress({
      percent: 0.01,
      downloadedBytes: 0,
      totalBytes: 100,
      message: mode === "prepare" ? copy.preparingBiblioSmith : copy.biblioSmithUpdateStarted,
    });
  }, [copy.biblioSmithUpdateStarted, copy.preparingBiblioSmith]);

  const finishBiblioSmithProgress = useCallback((message: string) => {
    setBiblioSmithProgress(() => ({
      percent: 100,
      downloadedBytes: 100,
      totalBytes: 100,
      message,
    } satisfies DownloadProgress));
    setBiblioSmithDownloadState("idle");
    window.setTimeout(() => {
      setBiblioSmithProgress(null);
      setBiblioSmithDownloadMessage(null);
    }, 900);
  }, []);

  const failBiblioSmithProgress = useCallback((error: unknown) => {
    const raw = String(error);
    const stopped = raw.includes("已停止") || raw.toLowerCase().includes("stopped");
    const message = stopped ? copy.biblioSmithDownloadStopped : copy.biblioSmithDownloadFailed;
    setBiblioSmithDownloadMessage(message);
    setBiblioSmithDownloadState(stopped ? "stopped" : "failed");
    addActivity(stopped ? "warning" : "error", stopped ? message : copy.biblioSmithUpdateStopped(raw));
    if (biblioSmithDownloadDismissedRef.current) {
      window.setTimeout(() => setBiblioSmithDownloadState("idle"), 900);
    } else {
      showFloatingToast(message, stopped ? "warning" : "error");
    }
  }, [addActivity, copy, showFloatingToast]);

  const startNodeModulesInBackground = useCallback(async (silent = false) => {
    if (nodeModulesDownloadState === "downloading" || nodeModulesDownloadState === "cancelling") return;
    setNodeModulesDownloadState("downloading");
    setNodeModulesDownloadMessage(null);
    setNodeModulesProgress({
      percent: 0.01,
      downloadedBytes: 0,
      totalBytes: 100,
      message: copy.nodeModulesInstalling,
      state: "downloading",
    });
    try {
      const result = await startNodeModulesInstall();
      if (!silent) {
        addActivity("info", result.message || copy.nodeModulesInstallStarted);
        showFloatingToast(copy.nodeModulesInstallStarted, "info");
      }
      await refreshNodeModulesStatus();
    } catch (error) {
      const message = copy.nodeModulesInstallFailed(String(error));
      setNodeModulesDownloadMessage(message);
      setNodeModulesDownloadState("failed");
      addActivity("error", message);
      showFloatingToast(message, "error");
    }
  }, [
    addActivity,
    copy,
    nodeModulesDownloadState,
    refreshNodeModulesStatus,
    showFloatingToast,
  ]);

  const stopNodeModulesInstall = useCallback(async (removePartial = false) => {
    setNodeModulesDownloadState("cancelling");
    try {
      await cancelNodeModulesInstall(removePartial);
    } catch (error) {
      const message = copy.nodeModulesInstallFailed(String(error));
      setNodeModulesDownloadMessage(message);
      setNodeModulesDownloadState("failed");
      addActivity("error", message);
      showFloatingToast(message, "error");
    }
  }, [addActivity, copy, showFloatingToast]);

  const retryRuntimePrepare = useCallback(() => {
    runtimeBootstrapStartedRef.current = true;
    void startRuntimeBootstrap(false);
  }, [startRuntimeBootstrap]);

  const continueAfterRuntimeBootstrap = useCallback(() => {
    if (runtimeBootstrapReleaseTimer.current) {
      window.clearTimeout(runtimeBootstrapReleaseTimer.current);
      runtimeBootstrapReleaseTimer.current = null;
    }
    setRuntimeBootstrapBlocking(false);
  }, []);

  const askConfirm = useCallback((options: Omit<ConfirmDialogState, "resolve">) => {
    return new Promise<boolean>((resolve) => {
      setConfirmDialog({ ...options, resolve });
    });
  }, []);

  const resolveConfirmDialog = useCallback((value: boolean) => {
    setConfirmDialog((dialog) => {
      dialog?.resolve(value);
      return null;
    });
  }, []);

  const refreshState = useCallback(async () => {
    try {
      setState(await getLauncherState());
    } catch (error) {
      setState(null);
      addActivity("error", String(error));
    }
  }, [addActivity]);

  const chooseRepo = useCallback(async () => {
    setBusy("repo-choose");
    try {
      const selected = await chooseRepoFolder();
      addActivity(selected.ok ? "info" : "info", selected.message);
      if (selected.ok && selected.repoRoot) {
        const confirmed = await askConfirm({
          title: copy.confirmProjectDirectoryTitle,
          message: selected.requiresDownload
            ? copy.confirmProjectDirectoryDownload(selected.repoRoot)
            : copy.confirmProjectDirectoryUse(selected.repoRoot),
          confirmLabel: copy.yes,
          cancelLabel: copy.no,
        });
        if (!confirmed) {
          addActivity("info", copy.projectDirectoryChangeCancelled);
          return;
        }
        const result = await setRepoFolder(selected.repoRoot);
        addActivity(result.ok ? "success" : "info", result.message);
        setTutorialDoc(null);
        await refreshState();
        if (biblioSmithSyncingRef.current) return;
        biblioSmithSyncingRef.current = true;
        setBiblioSmithPreparing(true);
        startBiblioSmithProgress("prepare");
        addActivity("info", copy.preparingBiblioSmith);
        try {
          const info = await prepareBiblioSmithProject(locale);
          setBiblioSmithUpdate(info);
          finishBiblioSmithProgress(copy.biblioSmithReady);
          addActivity("success", copy.biblioSmithReady);
        } catch (error) {
          failBiblioSmithProgress(error);
        } finally {
          biblioSmithSyncingRef.current = false;
          setBiblioSmithPreparing(false);
        }
        await refreshState();
        await refreshNodeModulesStatus();
      }
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setBusy(null);
    }
  }, [addActivity, askConfirm, copy, failBiblioSmithProgress, finishBiblioSmithProgress, locale, refreshNodeModulesStatus, refreshState, showFloatingToast, startBiblioSmithProgress]);

  const doOpenRepoFolder = useCallback(async () => {
    try {
      const result = await openRepoFolder();
      addActivity(result.ok ? "success" : "warning", result.message);
    } catch (error) {
      addActivity("error", String(error));
    }
  }, [addActivity]);

  const doOpenBooksFolder = useCallback(async () => {
    try {
      const result = await openBooksFolder();
      addActivity(result.ok ? "success" : "warning", result.message);
    } catch (error) {
      addActivity("error", String(error));
    }
  }, [addActivity]);

  const prepareBiblioSmith = useCallback(async () => {
    if (biblioSmithSyncingRef.current) return;
    biblioSmithSyncingRef.current = true;
    setBiblioSmithPreparing(true);
    startBiblioSmithProgress("prepare");
    addActivity("info", copy.preparingBiblioSmith);
    try {
      const info = await prepareBiblioSmithProject(locale);
      setBiblioSmithUpdate(info);
      finishBiblioSmithProgress(copy.biblioSmithReady);
      addActivity("success", copy.biblioSmithReady);
      await refreshState();
      await refreshNodeModulesStatus();
    } catch (error) {
      failBiblioSmithProgress(error);
      await refreshState();
      await refreshNodeModulesStatus();
    } finally {
      biblioSmithSyncingRef.current = false;
      setBiblioSmithPreparing(false);
    }
  }, [addActivity, copy, failBiblioSmithProgress, finishBiblioSmithProgress, locale, refreshNodeModulesStatus, refreshState, startBiblioSmithProgress]);

  const prepareBiblioSmithInBackground = useCallback(async () => {
    if (biblioSmithSyncingRef.current) return;
    biblioSmithSyncingRef.current = true;
    startBiblioSmithProgress("prepare");
    addActivity("info", copy.preparingBiblioSmith);
    try {
      const info = await prepareBiblioSmithProject(locale);
      setBiblioSmithUpdate(info);
      finishBiblioSmithProgress(copy.biblioSmithReady);
      addActivity("success", copy.biblioSmithReady);
      await refreshState();
      await refreshNodeModulesStatus();
    } catch (error) {
      failBiblioSmithProgress(error);
      await refreshState();
      await refreshNodeModulesStatus();
    } finally {
      biblioSmithSyncingRef.current = false;
    }
  }, [addActivity, copy, failBiblioSmithProgress, finishBiblioSmithProgress, locale, refreshNodeModulesStatus, refreshState, startBiblioSmithProgress]);

  const syncBiblioSmithNow = useCallback(async () => {
    if (biblioSmithSyncingRef.current) return;
    biblioSmithSyncingRef.current = true;
    setBiblioSmithSyncing(true);
    startBiblioSmithProgress("sync");
    addActivity("info", copy.biblioSmithUpdateStarted);
    try {
      const info = await syncBiblioSmithProject(locale);
      setBiblioSmithUpdate(info);
      const doneMessage = info.hasUpdate ? copy.biblioSmithFound(info.behindCount) : copy.biblioSmithUpdateComplete;
      finishBiblioSmithProgress(doneMessage);
      addActivity(info.hasUpdate ? "warning" : "success", doneMessage);
      await refreshState();
      await refreshNodeModulesStatus();
    } catch (error) {
      failBiblioSmithProgress(error);
      await refreshState();
      await refreshNodeModulesStatus();
    } finally {
      biblioSmithSyncingRef.current = false;
      setBiblioSmithSyncing(false);
    }
  }, [addActivity, copy, failBiblioSmithProgress, finishBiblioSmithProgress, locale, refreshNodeModulesStatus, refreshState, startBiblioSmithProgress]);

  // The outcome is read off the response instead of being asserted: this used to
  // log "up to date" unconditionally, so a reported update was still announced as
  // latest. `promptWhenUpdate` was accepted and then never used, which left the
  // two user-initiated call sites with no feedback beyond the activity list.
  const checkLauncher = useCallback(async (promptWhenUpdate = false, background = false) => {
    if (launcherCheckInProgressRef.current) return;
    launcherCheckInProgressRef.current = true;
    if (!background) setBusy("launcher-check");
    addActivity("info", copy.checkingLauncher);
    try {
      const info = await checkLauncherUpdates();
      setLauncherUpdate(info);
      const message = info.hasUpdate ? copy.launcherFound(info.latestVersion) : copy.launcherLatest;
      addActivity(info.hasUpdate ? "warning" : "success", message);
      if (promptWhenUpdate) showFloatingToast(message, info.hasUpdate ? "warning" : "success");
    } catch (error) {
      const message = copy.launcherCheckFailed(String(error));
      addActivity("error", message);
      if (promptWhenUpdate) showFloatingToast(message, "error");
    } finally {
      launcherCheckInProgressRef.current = false;
      if (!background) setBusy((value) => (value === "launcher-check" ? null : value));
    }
  }, [addActivity, copy, showFloatingToast]);

  useEffect(() => {
    const unlistenRuntime = listenRuntimeProgress((progress) => {
      setRuntimeProgress(progress);
      setRuntimeBootstrapMessage(progress.message ?? null);
      if (progress.state === "success") {
        setRuntimeBootstrapState("ready");
        void refreshRuntimeStatus();
        if (runtimeBootstrapReleaseTimer.current) window.clearTimeout(runtimeBootstrapReleaseTimer.current);
        runtimeBootstrapReleaseTimer.current = window.setTimeout(() => {
          setRuntimeBootstrapBlocking(false);
          setRuntimeProgress(null);
          runtimeBootstrapReleaseTimer.current = null;
        }, 650);
      } else if (progress.state === "failed") {
        setRuntimeBootstrapState("failed");
        addActivity("warning", progress.message || copy.runtimeBootstrapFailed);
        void refreshRuntimeStatus();
        if (runtimeBootstrapReleaseTimer.current) window.clearTimeout(runtimeBootstrapReleaseTimer.current);
        runtimeBootstrapReleaseTimer.current = window.setTimeout(() => {
          setRuntimeBootstrapBlocking(false);
          runtimeBootstrapReleaseTimer.current = null;
        }, 1400);
      } else if (progress.percent > 0 && progress.percent < 100) {
        setRuntimeBootstrapState("preparing");
      }
    });
    if (!runtimeBootstrapStartedRef.current) {
      runtimeBootstrapStartedRef.current = true;
      void startRuntimeBootstrap(false);
    }
    return () => {
      unlistenRuntime.then((fn) => fn()).catch(() => undefined);
    };
  }, [addActivity, copy, refreshRuntimeStatus, startRuntimeBootstrap]);

  useEffect(() => {
    if (runtimeBootstrapState !== "preparing") return undefined;
    const timer = window.setInterval(async () => {
      const status = await refreshRuntimeStatus();
      if (!status) return;
      if (status.ready) {
        setRuntimeBootstrapState("ready");
        setRuntimeBootstrapMessage(copy.runtimeBootstrapReady);
        setRuntimeProgress({
          percent: 100,
          downloadedBytes: 100,
          totalBytes: 100,
          message: copy.runtimeBootstrapReady,
          state: "success",
        });
        if (runtimeBootstrapReleaseTimer.current) window.clearTimeout(runtimeBootstrapReleaseTimer.current);
        runtimeBootstrapReleaseTimer.current = window.setTimeout(() => {
          setRuntimeBootstrapBlocking(false);
          setRuntimeProgress(null);
          runtimeBootstrapReleaseTimer.current = null;
        }, 450);
      } else if (!status.running) {
        const message = copy.runtimeBootstrapFailed;
        setRuntimeBootstrapState("failed");
        setRuntimeBootstrapMessage(message);
        setRuntimeProgress({
          percent: 100,
          downloadedBytes: 0,
          totalBytes: 0,
          message,
          state: "failed",
        });
        addActivity("warning", message);
        if (runtimeBootstrapReleaseTimer.current) window.clearTimeout(runtimeBootstrapReleaseTimer.current);
        runtimeBootstrapReleaseTimer.current = window.setTimeout(() => {
          setRuntimeBootstrapBlocking(false);
          runtimeBootstrapReleaseTimer.current = null;
        }, 1400);
      }
    }, 1500);
    return () => window.clearInterval(timer);
  }, [addActivity, copy, refreshRuntimeStatus, runtimeBootstrapState]);

  const loadTutorial = useCallback(async (kind: TutorialKind) => {
    setTutorialKind(kind);
    setTutorialHistory([]);
    if (!state?.repoReady) {
      setTutorialDoc(null);
      showFloatingToast(copy.tutorialUnavailable, "warning");
      return;
    }
    setTutorialLoading(true);
    try {
      const doc = await readProjectDocument(kind, locale);
      setTutorialDoc(doc);
    } catch (error) {
      addActivity("error", copy.tutorialLoadFailed(String(error)));
      showFloatingToast(copy.tutorialLoadFailed(String(error)), "error");
    } finally {
      setTutorialLoading(false);
    }
  }, [addActivity, copy, locale, showFloatingToast, state?.repoReady]);

  const openTutorialLink = useCallback(async (href: string) => {
    if (!state?.repoReady) {
      setTutorialDoc(null);
      showFloatingToast(copy.tutorialUnavailable, "warning");
      return;
    }
    setTutorialLoading(true);
    try {
      const doc = await readProjectDocumentPath(href, locale);
      if (tutorialDoc) {
        setTutorialHistory((items) => [...items.slice(-19), { kind: tutorialKind, document: tutorialDoc }]);
      }
      setTutorialKind(doc.kind === "howto" ? "howto" : "readme");
      setTutorialDoc(doc);
    } catch (error) {
      addActivity("error", copy.tutorialLoadFailed(String(error)));
      showFloatingToast(copy.tutorialLoadFailed(String(error)), "error");
    } finally {
      setTutorialLoading(false);
    }
  }, [addActivity, copy, locale, showFloatingToast, state?.repoReady, tutorialDoc, tutorialKind]);

  const goBackTutorial = useCallback(() => {
    setTutorialHistory((items) => {
      const previous = items[items.length - 1];
      if (!previous) return items;
      setTutorialKind(previous.kind);
      setTutorialDoc(previous.document);
      return items.slice(0, -1);
    });
  }, []);

  const refreshAllStatus = useCallback(async () => {
    setActiveTab("updates");
    if (refreshInProgressRef.current) return;
    refreshInProgressRef.current = true;
    setRefreshInProgress(true);
    addActivity("info", copy.refreshAllStarted);
    await sleep(80);
    try {
      await refreshState();
      if (!biblioSmithSyncingRef.current) {
        biblioSmithSyncingRef.current = true;
        startBiblioSmithProgress("sync");
        try {
          const info = await syncBiblioSmithProject(locale);
          setBiblioSmithUpdate(info);
          const doneMessage = info.hasUpdate ? copy.biblioSmithFound(info.behindCount) : copy.biblioSmithLatest;
          finishBiblioSmithProgress(doneMessage);
          addActivity(info.hasUpdate ? "warning" : "success", doneMessage);
        } catch (error) {
          failBiblioSmithProgress(error);
        } finally {
          biblioSmithSyncingRef.current = false;
        }
      }
      await refreshState();
      await refreshNodeModulesStatus();
      addActivity("success", copy.refreshAllDone);
    } catch (error) {
      addActivity("error", copy.biblioSmithUpdateStopped(String(error)));
      await refreshState();
      await refreshNodeModulesStatus();
    } finally {
      refreshInProgressRef.current = false;
      setRefreshInProgress(false);
      setLastRefreshAt(nowLabel());
    }
  }, [addActivity, copy, failBiblioSmithProgress, finishBiblioSmithProgress, locale, refreshNodeModulesStatus, refreshState, startBiblioSmithProgress]);

  const stopBiblioSmithDownload = useCallback(async (dismissAfterStop = false) => {
    if (biblioSmithDownloadState !== "downloading" && biblioSmithDownloadState !== "cancelling") return;
    biblioSmithDownloadDismissedRef.current = dismissAfterStop;
    setBiblioSmithDownloadDismissed(dismissAfterStop);
    setBiblioSmithDownloadState("cancelling");
    try {
      const result = await cancelBiblioSmithUpdate();
      addActivity(result.ok ? "warning" : "error", result.message);
    } catch (error) {
      addActivity("error", copy.biblioSmithUpdateStopped(String(error)));
      showFloatingToast(copy.biblioSmithUpdateStopped(String(error)), "error");
    }
  }, [addActivity, copy, biblioSmithDownloadState, showFloatingToast]);

  const retryBiblioSmithDownload = useCallback(() => {
    setBiblioSmithDownloadDismissed(false);
    biblioSmithDownloadDismissedRef.current = false;
    if (biblioSmithRetryMode === "prepare") {
      void prepareBiblioSmith();
    } else {
      void syncBiblioSmithNow();
    }
  }, [biblioSmithRetryMode, prepareBiblioSmith, syncBiblioSmithNow]);

  const closeBiblioSmithDownloadHud = useCallback(() => {
    setBiblioSmithDownloadDismissed(true);
    biblioSmithDownloadDismissedRef.current = true;
    if (biblioSmithDownloadState !== "downloading" && biblioSmithDownloadState !== "cancelling") {
      setBiblioSmithDownloadState("idle");
    }
  }, [biblioSmithDownloadState]);

  const updateSetting = useCallback(
    async (key: keyof LauncherSettings, value: boolean) => {
      const next = { ...settings, [key]: value };
      setSettings(next);
      saveSettings(next);
      if (key === "saveLogsToLocal") {
        try {
          const info = await setSaveLogsEnabled(value);
          setDiagnosticLogSettings(info);
          addActivity("success", copy.logSettingsSaved);
        } catch (error) {
          addActivity("error", copy.logSettingsSaveFailed(String(error)));
          void refreshDiagnosticLogSettings();
        }
      }
      if (key === "autoStart") {
        try {
          if (value) {
            await enable();
            addActivity("success", copy.autoStartEnabled);
          } else {
            await disable();
            addActivity("info", copy.autoStartDisabled);
          }
        } catch (error) {
          addActivity("error", copy.autoStartFailed(String(error)));
        }
      }
    },
    [addActivity, copy, refreshDiagnosticLogSettings, settings],
  );

  const doExportLauncherLogs = useCallback(async () => {
    addActivity("info", copy.exportingLogs);
    try {
      const result = await exportLauncherLogs();
      addActivity(result.ok ? "success" : "info", result.message);
    } catch (error) {
      addActivity("error", copy.logExportFailed(String(error)));
      showFloatingToast(copy.logExportFailed(String(error)), "error");
    }
  }, [addActivity, copy, showFloatingToast]);

  const updateProxySettingsDraft = useCallback((next: NetworkProxySettings) => {
    setProxySettings(next);
    setProxyTestResult(null);
    if (!next.enabled) {
      void saveProxySettings(next)
        .then((saved) => {
          setProxySettings(saved);
          void refreshState();
          addActivity("info", copy.proxyDisabledStatus);
        })
        .catch((error) => {
          const message = copy.proxySettingsFailed(String(error));
          addActivity("error", message);
          showFloatingToast(message, "error");
        });
    }
  }, [addActivity, copy, refreshState, showFloatingToast]);

  const doTestProxySettings = useCallback(async () => {
    setProxyBusy("test");
    setProxyTestResult(null);
    addActivity("info", copy.proxyTesting);
    try {
      const result = await testProxySettings(proxySettings);
      setProxyTestResult(result);
      if (!result.ok) {
        const message = result.message || copy.proxyTestFailed(copy.proxyUntested);
        addActivity("warning", message);
        showFloatingToast(message, "warning");
        return;
      }
      const saved = await saveProxySettings(proxySettings);
      setProxySettings(saved);
      await refreshState();
      const elapsed = result.elapsedMs ?? 0;
      const version = result.httpVersion ?? "";
      const message = copy.proxyTestAndApplied(elapsed, version);
      addActivity("success", message);
      showFloatingToast(message, "success");
    } catch (error) {
      const message = copy.proxyTestFailed(String(error));
      setProxyTestResult(proxyFailureResult(message));
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setProxyBusy(null);
    }
  }, [addActivity, copy, proxySettings, refreshState, showFloatingToast]);

  const doAutoDetectProxySettings = useCallback(async (force = true, silent = false) => {
    if (proxyBusy) return;
    setProxyBusy("detect");
    if (!silent) {
      addActivity("info", copy.proxyAutoDetecting);
    }
    try {
      const result = await autoDetectProxySettings(force);
      if (result.detected) {
        const detected = result.proxy ?? await getProxySettings();
        setProxySettings(detectedProxySettings(detected));
        setProxyTestResult(null);
      } else if (result.proxy) {
        setProxySettings(result.proxy);
      }
      if (result.test) {
        setProxyTestResult(result.test);
      }
      await refreshState();
      const message = result.detected ? copy.proxyAutoDetected : result.message || copy.proxyAutoDetectNotFound;
      if (!silent || Boolean(result.test)) {
        addActivity(result.detected ? "success" : "warning", message);
        showFloatingToast(message, result.detected ? "success" : "warning");
      }
    } catch (error) {
      if (!silent) {
        const message = copy.proxyTestFailed(String(error));
        setProxyTestResult(proxyFailureResult(message));
        addActivity("error", message);
        showFloatingToast(message, "error");
      }
    } finally {
      setProxyBusy(null);
    }
  }, [addActivity, copy, proxyBusy, refreshState, showFloatingToast]);

  const updateAutoInstallNodeModules = useCallback(async (enabled: boolean) => {
    try {
      if (!enabled && (nodeModulesDownloadState === "downloading" || nodeModulesDownloadState === "cancelling")) {
        void stopNodeModulesInstall(false);
      }
      const status = await setAutoInstallNodeModules(enabled);
      setNodeModulesStatus(status);
      if (!enabled) {
        setNodeModulesProgress(null);
        setNodeModulesDownloadMessage(null);
        setNodeModulesDownloadState("idle");
        nodeModulesAutoStartRef.current = false;
        return;
      }
      if (enabled && status.repoReady && !status.ready && !status.running) {
        nodeModulesAutoStartRef.current = false;
        void startNodeModulesInBackground(false);
      }
    } catch (error) {
      const message = copy.nodeModulesStatusFailed(String(error));
      addActivity("error", message);
      showFloatingToast(message, "error");
    }
  }, [
    addActivity,
    copy,
    nodeModulesDownloadState,
    showFloatingToast,
    startNodeModulesInBackground,
    stopNodeModulesInstall,
  ]);

  useEffect(() => {
    if (runtimeBootstrapBlocking) return undefined;
    if (!startupInitializedRef.current) {
      startupInitializedRef.current = true;
      void recordFrontendActivity("info", "frontend startup initialization begin").catch(() => undefined);
      refreshState();
      void refreshProxySettings();
      void doAutoDetectProxySettings(false, true);
      void refreshRuntimeStatus();
      void refreshNodeModulesStatus();
      isEnabled()
        .then((enabled) => {
          setSettings((old) => {
            const next = { ...old, autoStart: enabled };
            saveSettings(next);
            return next;
          });
        })
        .catch(() => undefined);
    }

    const unlistenBiblioSmith = listenBiblioSmithProgress((progress) => {
      setBiblioSmithProgress(progress);
      setBiblioSmithDownloadMessage(progress.message ?? null);
      if (progress.percent > 0 && progress.percent < 100) {
        setBiblioSmithDownloadState((current) => current === "idle" ? "downloading" : current);
      } else if (progress.percent >= 100 || progress.state === "success") {
        setBiblioSmithDownloadState("idle");
      } else if (progress.state === "failed") {
        setBiblioSmithDownloadState("failed");
      } else if (progress.state === "stopped") {
        setBiblioSmithDownloadState("stopped");
      }
    });
    const unlistenNodeModules = listenNodeModulesProgress((progress) => {
      setNodeModulesProgress(progress);
      setNodeModulesDownloadMessage(progress.message ?? null);
      if (progress.state === "success") {
        setNodeModulesDownloadState("idle");
        void refreshNodeModulesStatus();
        showFloatingToast(copy.nodeModulesReady, "success");
      } else if (progress.state === "failed") {
        setNodeModulesDownloadState("failed");
        addActivity("error", progress.message || copy.nodeModulesMissing);
        void refreshNodeModulesStatus();
      } else if (progress.state === "stopped") {
        setNodeModulesDownloadState("stopped");
        addActivity("warning", copy.nodeModulesInstallStopped);
        void refreshNodeModulesStatus();
      } else if (progress.percent > 0 && progress.percent < 100) {
        setNodeModulesDownloadState((current) => current === "idle" ? "downloading" : current);
      }
    });
    return () => {
      unlistenBiblioSmith.then((fn) => fn()).catch(() => undefined);
      unlistenNodeModules.then((fn) => fn()).catch(() => undefined);
    };
  }, [
    addActivity,
    copy,
    doAutoDetectProxySettings,
    refreshNodeModulesStatus,
    refreshProxySettings,
    refreshRuntimeStatus,
    refreshState,
    runtimeBootstrapBlocking,
    showFloatingToast,
  ]);

  useEffect(() => {
    if (nodeModulesDownloadState !== "downloading" && nodeModulesDownloadState !== "cancelling") return undefined;
    const timer = window.setInterval(() => {
      void refreshNodeModulesStatus();
    }, 1500);
    return () => window.clearInterval(timer);
  }, [nodeModulesDownloadState, refreshNodeModulesStatus]);

  useEffect(() => {
    void refreshDiagnosticLogSettings();
  }, [refreshDiagnosticLogSettings]);

  useEffect(() => {
    void refreshBookPipelineState();
  }, [refreshBookPipelineState]);

  useEffect(() => {
    if (pipelineBusy !== "run" && pipelineBusy !== "retry" && pipelineBusy !== "advance") return undefined;
    let cancelled = false;
    let requestInFlight = false;
    const poll = async () => {
      if (requestInFlight) return;
      requestInFlight = true;
      try {
        const state = await getBookPipelineState();
        if (!cancelled) setPipelineState(state);
      } catch {
        // The foreground run/retry action reports terminal errors. Polling is
        // best-effort so a transient read cannot create duplicate activity.
      } finally {
        requestInFlight = false;
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 750);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [pipelineBusy]);

  useEffect(() => {
    if (!nodeModulesStatus?.repoReady) {
      nodeModulesAutoStartRef.current = false;
      return;
    }
    if (nodeModulesStatus.ready) {
      nodeModulesAutoStartRef.current = true;
      return;
    }
    if (!nodeModulesStatus.autoInstall || nodeModulesStatus.running || nodeModulesAutoStartRef.current) return;
    nodeModulesAutoStartRef.current = true;
    void startNodeModulesInBackground(true);
  }, [nodeModulesStatus, startNodeModulesInBackground]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void recordFrontendActivity(
        "info",
        `frontend startup automation begin checkLauncher=${settings.checkLauncherOnLaunch}`,
      ).catch(() => undefined);
      void prepareBiblioSmithInBackground();
      if (settings.checkLauncherOnLaunch) void checkLauncher(false, true);
    }, 600);
    return () => window.clearTimeout(timer);
    // Startup automation should run once after first paint using the initial persisted settings.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (activeTab === "tutorial" && state?.repoReady && !tutorialDoc && !tutorialLoading) {
      void loadTutorial(tutorialKind);
    }
  }, [activeTab, loadTutorial, state?.repoReady, tutorialDoc, tutorialKind, tutorialLoading]);

  useEffect(() => {
    setTutorialDoc(null);
  }, [state?.repoRoot]);

  useEffect(() => {
    if (!state?.repoReady) {
      setTutorialDoc(null);
      setTutorialLoading(false);
      setBiblioSmithUpdate(null);
      setShowAllCommits(true);
    }
  }, [state?.repoReady, state?.repoRoot, state?.repoStatus]);

  useEffect(() => {
    return () => {
      if (floatingToastTimer.current) {
        window.clearTimeout(floatingToastTimer.current);
      }
    };
  }, []);

  const commits = biblioSmithUpdate?.commits ?? [];
  const displayedCommits = showAllCommits ? commits : commits.slice(0, 1);
  const firstCommit = commits[0];
  const repoReady = Boolean(state?.repoReady);
  const repoStatus = state?.repoStatus ?? "missing";
  // Only empty when the launcher state could not be read at all, so there is no
  // path to name. It used to fall back to a Windows path, which Settings and the
  // workspace-unavailable copy then showed verbatim on macOS.
  const repoPath = state?.repoRoot || UNKNOWN_VALUE;
  const repoCanAutoPrepare = !repoReady && (repoStatus === "missing" || repoStatus === "empty");
  const repoIsEmpty = !repoReady && repoStatus === "empty";
  const repoIsOccupied = !repoReady && repoStatus === "occupied";
  const workspaceUnavailableDescription = repoIsOccupied
    ? copy.workspaceOccupiedDescription(repoPath)
    : repoIsEmpty
      ? copy.workspaceEmptyDescription(repoPath)
    : copy.workspaceMissingDescription(repoPath);
  const workspaceUnavailableHelp = repoIsOccupied
    ? copy.workspaceOccupiedHelp
    : repoIsEmpty
      ? copy.workspaceEmptyHelp
      : copy.workspaceMissingHelp;
  const biblioSmithBusy = biblioSmithPreparing || biblioSmithSyncing;
  const unavailableRepoLabel = repoIsOccupied ? copy.repoInvalid : repoIsEmpty ? copy.repoEmpty : copy.repoMissing;
  const latestBiblioSmithVersion = firstCommit ? versionFromDate(firstCommit.date) : repoReady ? copy.projectReady : unavailableRepoLabel;
  const currentBiblioSmithVersion = repoReady
    ? state?.localCommitShort === "preview"
      ? "v2025.05.25"
      : state?.localCommitShort || copy.projectReady
    : biblioSmithBusy
      ? copy.preparing
      : unavailableRepoLabel;
  const biblioSmithStatus = biblioSmithBusy ? copy.preparing : repoReady ? copy.projectReady : unavailableRepoLabel;
  const biblioSmithStatusTone: "success" | "warning" | "muted" = biblioSmithBusy ? "warning" : repoReady ? "success" : "muted";
  // A ready repo with no commit list yet has no timestamp to show. This used to
  // call commitDate(undefined) on purpose, which returned a hardcoded date the
  // card then presented as the project's real last-updated time.
  const latestBiblioSmithUpdated = firstCommit
    ? commitDate(firstCommit)
    : repoReady
      ? UNKNOWN_VALUE
      : unavailableRepoLabel;
  const biblioSmithPrimaryLabel = repoReady ? copy.openBooks : repoCanAutoPrepare ? copy.prepareProject : copy.changeProjectPath;
  const BiblioSmithPrimaryIcon = repoReady ? FolderOpen : repoCanAutoPrepare ? Download : FolderOpen;
  const biblioSmithSecondaryLabel = repoReady ? copy.viewProject : repoCanAutoPrepare ? copy.changeProjectPath : copy.viewProject;
  const biblioSmithSecondaryIcon = FolderOpen;
  const biblioSmithMoreLabel = repoReady ? copy.updateBiblioSmithProject : repoCanAutoPrepare ? copy.prepareProject : copy.changeProjectPath;
  const showingBiblioSmithDownloadHud = biblioSmithDownloadState !== "idle" && !biblioSmithDownloadDismissed;
  const biblioSmithProgressLabel = formatDownloadProgress(copy, biblioSmithProgress);
  const biblioSmithHudMessage = biblioSmithDownloadMessage || biblioSmithProgressLabel || copy.biblioSmithProgressDefault;
  const nodeModulesProgressLabel = formatDownloadProgress(copy, nodeModulesProgress);
  const nodeModulesHudMessage = nodeModulesDownloadMessage || nodeModulesProgressLabel || copy.nodeModulesInstalling;
  const runtimeProgressLabel = formatDownloadProgress(copy, runtimeProgress);

  const visibleActivities = useMemo(() => activities.slice(0, 5), [activities]);

  const biblioSmithCard: ProductCardProps = {
    accent: "blue",
    icon: BookOpen,
    title: copy.biblioSmithTitle,
    subtitle: copy.biblioSmithSubtitle,
    current: currentBiblioSmithVersion,
    latest: latestBiblioSmithVersion,
    status: biblioSmithStatus,
    statusTone: biblioSmithStatusTone,
    latestUpdated: latestBiblioSmithUpdated,
    primaryLabel: biblioSmithPrimaryLabel,
    primaryIcon: BiblioSmithPrimaryIcon,
    secondaryLabel: biblioSmithSecondaryLabel,
    secondaryIcon: biblioSmithSecondaryIcon,
    busy: busy === "repo-choose",
    busyText: copy.working,
    onPrimary: repoReady ? doOpenBooksFolder : repoCanAutoPrepare ? prepareBiblioSmith : chooseRepo,
    onSecondary: repoReady ? doOpenRepoFolder : repoCanAutoPrepare ? chooseRepo : doOpenRepoFolder,
    onMore: repoReady ? syncBiblioSmithNow : repoCanAutoPrepare ? prepareBiblioSmith : chooseRepo,
    moreLabel: biblioSmithMoreLabel,
    moreBusy: biblioSmithSyncing,
    moreDisabled: biblioSmithSyncing,
    copy,
  };

  if (runtimeBootstrapBlocking) {
    return (
      <RuntimeBootstrapScreen
        copy={copy}
        state={runtimeBootstrapState}
        progress={runtimeProgress}
        message={runtimeBootstrapMessage || runtimeProgressLabel || copy.runtimeBootstrapChecking}
        onRetry={retryRuntimePrepare}
        onContinue={continueAfterRuntimeBootstrap}
      />
    );
  }

  return (
    <div className="launcher-frame">
      <Titlebar
        copy={copy}
        version={LAUNCHER_VERSION}
        proxyConfigured={Boolean(state?.proxyConfigured)}
        autoStart={settings.autoStart}
        projectReady={repoReady}
        projectStatusValue={biblioSmithStatus}
        quickActionsOpen={quickActionsOpen}
        onToggleQuickActions={() => setQuickActionsOpen((value) => !value)}
        onSelectRepo={() => {
          setQuickActionsOpen(false);
          void chooseRepo();
        }}
        onOpenRepo={() => {
          setQuickActionsOpen(false);
          void doOpenRepoFolder();
        }}
        onOpenBooks={() => {
          setQuickActionsOpen(false);
          void doOpenBooksFolder();
        }}
        onCheckLauncher={() => {
          setQuickActionsOpen(false);
          void checkLauncher(true);
        }}
      />

      <main className="app-shell">
        <Sidebar
          copy={copy}
          pipelineNavLabel={bookPipelineCopy.nav}
          version={LAUNCHER_VERSION}
          activeTab={activeTab}
          pipelineLoading={pipelineBusy === "loading"}
          updateAvailable={Boolean(launcherUpdate?.hasUpdate || biblioSmithUpdate?.hasUpdate)}
          onSelectTab={setActiveTab}
        />

        <section className="workspace">
          <FloatingFeedback
            toast={floatingToast}
            biblioSmithVisible={showingBiblioSmithDownloadHud}
            biblioSmithTitle={copy.biblioSmithProgressTitle}
            biblioSmithState={biblioSmithDownloadState}
            biblioSmithProgress={biblioSmithProgress}
            biblioSmithMessage={biblioSmithHudMessage}
            copy={copy}
            onStopBiblioSmith={() => void stopBiblioSmithDownload(false)}
            onCancelBiblioSmith={() => void stopBiblioSmithDownload(true)}
            onRetryBiblioSmith={retryBiblioSmithDownload}
            onCloseBiblioSmith={closeBiblioSmithDownloadHud}
          />

          {activeTab === "overview" && (
            <OverviewPage
              copy={copy}
              pipelineCopy={bookPipelineCopy}
              projectStatusLine={repoReady ? copy.running : biblioSmithStatus}
              biblioSmithCard={biblioSmithCard}
              biblioSmithUpdateAvailable={Boolean(biblioSmithUpdate?.hasUpdate)}
              visibleActivities={visibleActivities}
              pipelineState={pipelineState}
              onViewLogs={() => setActiveTab("logs")}
              onOpenPipeline={() => setActiveTab("pipeline")}
              onGoUpdates={() => setActiveTab("updates")}
            />
          )}

          {activeTab === "updates" && (
            <UpdatesPage
              copy={copy}
              biblioSmithCard={biblioSmithCard}
              launcherVersion={LAUNCHER_VERSION}
              launcherLatest={launcherUpdate?.latestVersion ?? ""}
              launcherBusy={busy === "launcher-check"}
              onCheckLauncher={() => void checkLauncher(true)}
              commits={commits}
              displayedCommits={displayedCommits}
              latestBiblioSmithVersion={latestBiblioSmithVersion}
              showAllCommits={showAllCommits}
              commitEmptyMessage={repoReady ? copy.noCommits : copy.noCommitsUnavailable}
              refreshInProgress={refreshInProgress}
              lastRefreshAt={lastRefreshAt}
              onToggleShowAllCommits={() => setShowAllCommits((value) => !value)}
              onCheckAll={() => void refreshAllStatus()}
            />
          )}

          {activeTab === "tutorial" && (
            <GuidePage
              copy={copy}
              kind={tutorialKind}
              document={tutorialDoc}
              loading={tutorialLoading}
              canGoBack={tutorialHistory.length > 0}
              repoReady={repoReady}
              unavailableTitle={copy.workspaceUnavailableTitle}
              unavailableDescription={workspaceUnavailableDescription}
              unavailableHelp={workspaceUnavailableHelp}
              recoverLabel={repoCanAutoPrepare ? copy.prepareProject : copy.changeProjectPath}
              onRecover={repoCanAutoPrepare ? prepareBiblioSmith : chooseRepo}
              onChangeProject={chooseRepo}
              onSelect={(kind) => void loadTutorial(kind)}
              onBack={goBackTutorial}
              onOpenLink={(href) => void openTutorialLink(href)}
            />
          )}

          {activeTab === "pipeline" && (
            <PipelineWorkbench
              copy={bookPipelineCopy}
              state={pipelineState}
              draft={pipelineDraft}
              preview={pipelinePreview}
              zoteroSources={pipelineZoteroSources}
              modelSlots={modelSlots}
              busy={pipelineBusy}
              onDraftChange={(patch) => {
                setPipelineDraft((draft) => ({ ...draft, ...patch }));
                setPipelinePreview([]);
              }}
              onPreview={() => void previewPipeline()}
              onQueueRun={queueAndRunPipeline}
              onChooseFolder={() => void choosePipelinePdfFolder()}
              onChooseMarkdown={() => void choosePipelineMarkdownSource()}
              onDiscoverZotero={() => void discoverPipelineZoteroSources()}
              onSearchZotero={(query) => void discoverZoteroByQuery(query)}
              onRetry={(jobId) => void retryPipeline(jobId)}
              onDelete={(jobId) => void deletePipeline(jobId)}
              onAdvance={(jobId, childId) => void advancePipeline(jobId, childId)}
              onSampleTranslation={(jobId, childId, providerProfileId, providerConfigId) =>
                void samplePipelineTranslation(jobId, childId, providerProfileId, providerConfigId)
              }
              onApplySampleProvider={(jobId, childId, providerProfileId, providerConfigId) =>
                void applyPipelineTranslationProvider(jobId, childId, providerProfileId, providerConfigId)
              }
              onExportDiagnostic={(jobId, profile) => void exportPipelineDiagnostic(jobId, profile)}
              onSaveCustomInstructions={(jobId, childId, customInstructions) =>
                void savePipelineCustomInstructions(jobId, childId, customInstructions)
              }
              onApproveGate={(jobId, childId, stageId) => void approvePipelineGate(jobId, childId, stageId)}
              onRouteOverride={(jobId, childId, routeItemId, routeOverride) =>
                void overridePipelineRoute(jobId, childId, routeItemId, routeOverride)
              }
              onOpenOutput={(jobId) => void openPipelineOutput(jobId)}
              routeOverrides={pipelineRouteOverrides}
              onRouteOverrideChange={changePipelineRouteOverride}
              onHandoff={(jobId, artifactPath) => void handoffPipelineMarkdown(jobId, artifactPath)}
            />
          )}

          {activeTab === "settings" && (
            <SettingsPage
              copy={copy}
              locale={locale}
              languageSetting={languageSetting}
              onLanguageChange={updateLanguageSetting}
              settings={settings}
              repoPath={repoPath}
              proxySettings={proxySettings}
              proxyBusy={proxyBusy}
              proxyTestResult={proxyTestResult}
              runtimeStatus={runtimeStatus}
              nodeModulesStatus={nodeModulesStatus}
              nodeModulesProgress={nodeModulesProgress}
              nodeModulesDownloadState={nodeModulesDownloadState}
              nodeModulesMessage={nodeModulesHudMessage}
              diagnosticLogSettings={diagnosticLogSettings}
              onUpdateSetting={updateSetting}
              onChooseRepo={() => void chooseRepo()}
              onProxyChange={updateProxySettingsDraft}
              onProxyTest={() => void doTestProxySettings()}
              onProxyAutoDetect={() => void doAutoDetectProxySettings(true, false)}
              onRuntimeRetry={retryRuntimePrepare}
              onNodeModulesToggle={(value) => void updateAutoInstallNodeModules(value)}
              onNodeModulesStop={() => void stopNodeModulesInstall(false)}
              onNodeModulesCancel={() => void stopNodeModulesInstall(true)}
              onExportLogs={() => void doExportLauncherLogs()}
            />
          )}

          {activeTab === "logs" && <LogsPage copy={copy} activities={activities} />}
        </section>
      </main>
      <ConfirmDialog dialog={confirmDialog} onCancel={() => resolveConfirmDialog(false)} onConfirm={() => resolveConfirmDialog(true)} />
    </div>
  );
}

function proxyFailureResult(message: string): ProxyTestResult {
  return {
    ok: false,
    message,
    elapsedMs: null,
    httpVersion: null,
    targetUrl: "",
  };
}

function detectedProxySettings(proxy: NetworkProxySettings): NetworkProxySettings {
  return {
    ...proxy,
    enabled: true,
    host: proxy.host || "127.0.0.1",
    port: proxy.port ?? 7890,
  };
}
