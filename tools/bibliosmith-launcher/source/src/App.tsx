import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import {
  BOOK_PIPELINE_STATE_SCHEMA_VERSION,
  chooseBookPipelinePdfFolder,
  advanceBookPipelineJob,
  setBookPipelineRouteOverride,
  approveBookPipelineGate,
  removeBooksFromShelf,
  migrateBookPipelineProject,
  discoverBookPipelineZoteroSources,
  getBookPipelineState,
  getModelCatalog,
  getOcrCredentialsStatus,
  getRuntimeStatus,
  handoffBookPipelineMarkdown,
  listenRuntimeProgress,
  openBookPipelineOutput,
  previewBookPipelineRoute,
  queueBookPipelineJob,
  recordFrontendActivity,
  retryBookPipelineJob,
  runBookPipelineOcrSample,
  runBookPipelineTranslationSample,
  setBookPipelineTranslationProvider,
  runBookPipelineJob,
  copyTranslationPromptPack,
  deleteTranslationPromptPack,
  getTranslationPromptPackDefault,
  listTranslationPromptPacks,
  previewBookTranslationPrompt,
  saveTranslationPromptPackRevision,
  selectBookTranslationPromptPack,
  setTranslationPromptPackDefault,
  startRuntimePrepare,
} from "./api";
import {
  ActivityItem,
  BookPipelineJob,
  BookPipelinePreviewConfig,
  BookPipelineRouteItem,
  BookPipelineSource,
  BookPipelineShelfSelection,
  BookPipelineState,
  LauncherSettings,
  ModelSlotView,
  TranslationPromptPackCatalog,
  TranslationPromptPackReference,
  TranslationPromptPackRevisionDraft,
} from "./types";
import { copies, detectLocale, type LanguageSetting, type Locale } from "./i18n";
import { SettingsOverlay } from "./pages/settings";
import { pipelineJobOutcomeSucceeded, translationHandoffReady } from "./lib/pipeline-status";
import { FloatingFeedback, Titlebar, type FloatingToast, type ToastTone } from "./shell";
import { WorkspaceSetupGate } from "./startup/WorkspaceSetupGate";
import launcherVersionManifest from "../launcher-version.json";
import {
  PipelineWorkbench,
  defaultPipelineDraft,
  effectivePipelineMode,
  isTrackOnlyDraftPatch,
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
  saveLogsToLocal: true,
};

function upsertPipelineJob(state: BookPipelineState, job: BookPipelineJob): BookPipelineState {
  const existing = state.jobs.filter((item) => item.id !== job.id);
  return { ...state, revision: state.revision + 1, jobs: [job, ...existing] };
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
  const languageSetting = loadLanguageSetting();
  const locale = languageSetting === "system" ? detectLocale() : languageSetting;
  return (
    <WorkspaceSetupGate locale={locale} version={LAUNCHER_VERSION}>
      <LauncherApp />
    </WorkspaceSetupGate>
  );
}

function LauncherApp() {
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
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settings, setSettings] = useState<LauncherSettings>(loadSettings);
  const [floatingToast, setFloatingToast] = useState<FloatingToast | null>(null);
  const [pipelineState, setPipelineState] = useState<BookPipelineState>({
    schemaVersion: BOOK_PIPELINE_STATE_SCHEMA_VERSION,
    revision: 0,
    jobs: [],
  });
  const [promptPackCatalog, setPromptPackCatalog] = useState<TranslationPromptPackCatalog | null>(null);
  const [promptPackDefaults, setPromptPackDefaults] = useState<Record<"programmatic" | "expert-agent", TranslationPromptPackReference | null>>({
    programmatic: null,
    "expert-agent": null,
  });
  const [promptPackBusy, setPromptPackBusy] = useState(false);
  const [pipelineDraft, setPipelineDraft] = useState<PipelineDraft>(defaultPipelineDraft);
  // Slots keep their `configured` flag so the wizard can say which providers have
  // a key. Stays empty when the catalog cannot be read, which the wizard reads as
  // "unknown" rather than "none configured".
  const [modelSlots, setModelSlots] = useState<ModelSlotView[]>([]);
  const refreshPromptPacks = useCallback(async () => {
    const [catalog, programmatic, expert] = await Promise.all([
      listTranslationPromptPacks(),
      getTranslationPromptPackDefault("programmatic"),
      getTranslationPromptPackDefault("expert-agent"),
    ]);
    setPromptPackCatalog(catalog);
    setPromptPackDefaults({ programmatic, "expert-agent": expert });
  }, []);
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
  // What is actually configured in Keychain drives the input
  // island's OCR chips and, through the preview config, the routes the backend
  // hands back.
  const refreshOcrCredentialStatus = useCallback(() => {
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
  useEffect(() => refreshOcrCredentialStatus(), [refreshOcrCredentialStatus]);
  // Settings is the only place OCR keys can be entered, so closing it is the
  // moment the chips (and the preflight they feed) can be stale.
  const closeSettings = useCallback(() => {
    setSettingsOpen(false);
    refreshOcrCredentialStatus();
  }, [refreshOcrCredentialStatus]);
  const [pipelinePreview, setPipelinePreview] = useState<BookPipelineRouteItem[]>([]);
  const [pipelineRouteOverrides, setPipelineRouteOverrides] = useState<Record<string, RouteOverride>>({});
  const [pipelineZoteroSources, setPipelineZoteroSources] = useState<BookPipelineSource[]>([]);
  const [pipelineBusy, setPipelineBusy] = useState<PipelineBusy>("loading");
  const runtimePrepareStartedRef = useRef(false);
  const startupInitializedRef = useRef(false);
  const floatingToastTimer = useRef<number | null>(null);

  // Activity lines go straight to the backend log; the in-app activity feed
  // retired together with the logs page. The floating toast is the visible half.
  const addActivity = useCallback((level: ActivityItem["level"], message: string) => {
    void recordFrontendActivity(level, message).catch(() => undefined);
  }, []);

  useEffect(() => {
    void Promise.all([
      listTranslationPromptPacks(),
      getTranslationPromptPackDefault("programmatic"),
      getTranslationPromptPackDefault("expert-agent"),
    ])
      .then(([catalog, programmatic, expert]) => {
        setPromptPackCatalog(catalog);
        setPromptPackDefaults({ programmatic, "expert-agent": expert });
      })
      .catch((error) => {
        addActivity("error", `Translation prompt packs unavailable: ${String(error)}`);
      });
  }, [addActivity]);

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

  // Two shapes only: the folder the island was given, or the Zotero item picked
  // from a title search. The fake, external-adapter and Markdown source kinds
  // stay in the backend contract but no longer have a way in from the UI.
  const buildPipelineSource = useCallback((): BookPipelineSource => {
    if (pipelineDraft.sourceKind === "local_pdf_folder") {
      return {
        kind: "local_pdf_folder",
        title: pipelineDraft.localPdfTitle || "Local PDF folder",
        path: pipelineDraft.localPdfFolder,
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

  // `configOverride` lets the override click preview the map it is about to
  // commit; without it the preview would run against the pre-click config.
  const previewPipeline = useCallback(async (configOverride?: BookPipelinePreviewConfig) => {
    setPipelineBusy("preview");
    try {
      const route = await previewBookPipelineRoute(buildPipelineSource(), pipelineDraft.mode, configOverride ?? pipelineConfig);
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

  // Re-previewing belongs to the click, not to an effect watching the override
  // map: the route chips and the launch counts must show the backend's
  // decision rather than a client-side guess, and the click already knows the
  // map it is about to commit.
  const changePipelineRouteOverride = useCallback((routeItemId: string, override: RouteOverride) => {
    const next = { ...pipelineRouteOverrides };
    if (override === "auto") delete next[routeItemId];
    else next[routeItemId] = override;
    setPipelineRouteOverrides(next);
    if (pipelinePreview.length > 0) {
      void previewPipeline({ ...pipelineConfig, routeOverrides: next });
    }
  }, [pipelineConfig, pipelinePreview.length, pipelineRouteOverrides, previewPipeline]);

  const queueAndRunPipeline = useCallback(async (): Promise<boolean> => {
    setPipelineBusy("queue");
    try {
      const source = buildPipelineSource();
      const executor = pipelineDraft.translationMode === "expert" ? "expert-agent" : "programmatic";
      const promptPackReference = promptPackDefaults[executor];
      if (!promptPackReference || !promptPackCatalog) {
        throw new Error("翻译提示词方案尚未加载，请稍后重试。");
      }
      const promptPackRevision = promptPackCatalog.packs
        .find((pack) => pack.packId === promptPackReference.packId)
        ?.revisions.find((revision) => revision.revisionId === promptPackReference.revisionId && revision.contentSha256 === promptPackReference.contentSha256);
      if (!promptPackRevision || promptPackRevision.executor !== executor) {
        throw new Error("默认翻译提示词方案与当前执行方式不兼容。");
      }
      const secondPassEnabled = promptPackRevision.stages.some((stage) => stage.stageId === "reflect")
        && promptPackRevision.stages.some((stage) => stage.stageId === "improve");
      // Fast routes use the versioned provider registry IDs introduced by #60.
      // Every job now translates, so the flags no longer have to be masked off
      // for a conversion-only run.
      const translationIntent =
        pipelineDraft.translationMode === "expert"
          ? { translationMode: "expert" as const, profileId: "expert-agent", configId: "default", skillIds: promptPackRevision.requiredSkillIds ?? ["expert-translation-quality"], promptPackReference, secondPassEnabled: false, textCleanup: false, digestMode: pipelineDraft.digestMode, outputFormats: pipelineDraft.outputFormats }
          : { translationMode: "fast" as const, profileId: pipelineDraft.providerProfileId, configId: pipelineDraft.providerConfigId, skillIds: [], promptPackReference, secondPassEnabled, textCleanup: pipelineDraft.textCleanup, digestMode: pipelineDraft.digestMode, outputFormats: pipelineDraft.outputFormats };
      // Not `pipelineDraft.mode` directly: the layout-preserving track is only
      // eligible for a single text PDF, and the draft can still be carrying that
      // choice from a book the user has since swapped away from.
      const mode = effectivePipelineMode(pipelineDraft.mode, pipelinePreview);
      const queued = await queueBookPipelineJob(source, mode, translationIntent, pipelineConfig);
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
    // pipelinePreview is here for the same reason: the effective mode is decided
    // from the current preflight, and a stale one would queue the layout track
    // for a book whose route no longer allows it.
  }, [addActivity, buildPipelineSource, pipelineConfig, pipelineDraft.digestMode, pipelineDraft.mode, pipelineDraft.outputFormats, pipelineDraft.providerConfigId, pipelineDraft.providerProfileId, pipelineDraft.textCleanup, pipelineDraft.translationMode, pipelinePreview, promptPackCatalog, promptPackDefaults, showFloatingToast]);

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

  const removePipelineBooks = useCallback(async (selections: BookPipelineShelfSelection[]) => {
    setPipelineBusy("delete");
    try {
      const state = await removeBooksFromShelf(selections);
      setPipelineState(state);
      addActivity("success", `Removed ${selections.length} book(s) from the shelf`);
      showFloatingToast(bookPipelineCopy.deleteBookDone, "success");
      return true;
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
      return false;
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, bookPipelineCopy.deleteBookDone, showFloatingToast]);

  const migratePipelineProject = useCallback(async (jobId: string, childId?: string | null) => {
    setPipelineBusy("migrate");
    try {
      const state = await migrateBookPipelineProject(jobId, childId);
      setPipelineState(state);
      addActivity("success", `Book Pipeline project migrated: ${jobId}`);
      showFloatingToast(bookPipelineCopy.projectMigrationDone, "success");
      return true;
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
      return false;
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, bookPipelineCopy.projectMigrationDone, showFloatingToast]);

  const advancePipeline = useCallback(async (jobId: string, childId: string, invalidateDownstream = false) => {
    setPipelineBusy("advance");
    try {
      const job = await advanceBookPipelineJob(jobId, childId, invalidateDownstream);
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

  const selectPipelinePromptPack = useCallback(async (
    jobId: string,
    childId: string,
    promptPackReference: TranslationPromptPackReference,
  ) => {
    setPipelineBusy("promptPack");
    try {
      const job = await selectBookTranslationPromptPack(jobId, childId, promptPackReference);
      setPipelineState((current) => upsertPipelineJob(current, job));
      addActivity("success", `Book Pipeline prompt pack selected: ${promptPackReference.packId}@${promptPackReference.revisionId}`);
      showFloatingToast("已切换本书翻译提示词方案，旧审批已失效", "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, showFloatingToast]);

  const previewPipelinePrompt = useCallback(async (jobId: string, childId: string) => {
    return previewBookTranslationPrompt(jobId, childId);
  }, []);

  const mutatePromptPacks = useCallback(async (operation: () => Promise<unknown>, success: string) => {
    setPromptPackBusy(true);
    try {
      await operation();
      await refreshPromptPacks();
      showFloatingToast(success, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
      throw error;
    } finally {
      setPromptPackBusy(false);
    }
  }, [addActivity, refreshPromptPacks, showFloatingToast]);

  const copyPromptPack = useCallback((source: TranslationPromptPackReference, displayName: string) =>
    mutatePromptPacks(() => copyTranslationPromptPack(source, displayName), "已创建可编辑的本地方案"), [mutatePromptPacks]);
  const savePromptPackRevision = useCallback((draft: TranslationPromptPackRevisionDraft) =>
    mutatePromptPacks(() => saveTranslationPromptPackRevision(draft), "新版本已保存；现有任务不会自动升级"), [mutatePromptPacks]);
  const deletePromptPack = useCallback((packId: string) =>
    mutatePromptPacks(() => deleteTranslationPromptPack(packId), "本地方案已删除；历史任务仍可解析原版本"), [mutatePromptPacks]);
  const setPromptPackDefault = useCallback((executor: "programmatic" | "expert-agent", value: TranslationPromptPackReference) =>
    mutatePromptPacks(() => setTranslationPromptPackDefault(executor, value), "默认翻译提示词方案已更新"), [mutatePromptPacks]);

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

  // Run both OCR engines over the same sampled pages. Like the translation
  // sample it adopts nothing: the user picks a side afterwards, which goes
  // through the ordinary route override.
  const samplePipelineOcr = useCallback(async (jobId: string, childId: string, samplePages: number) => {
    setPipelineBusy("sample");
    try {
      const job = await runBookPipelineOcrSample(jobId, childId, samplePages);
      setPipelineState((current) => upsertPipelineJob(current, job));
      // "requested", not "sampled": a book with fewer interior pages than asked
      // for yields fewer, and the count that was actually compared is in the
      // job's own log summary. Claiming ten pages here when four were sampled
      // would misstate what the run spent.
      addActivity("success", `OCR sample ready; requested up to ${samplePages} page(s)`);
      showFloatingToast(bookPipelineCopy.ocrCompareReady, "success");
    } catch (error) {
      const message = String(error);
      addActivity("error", message);
      showFloatingToast(message, "error");
    } finally {
      setPipelineBusy(null);
    }
  }, [addActivity, bookPipelineCopy.ocrCompareReady, showFloatingToast]);

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

  // The request is built from the typed query rather than from the draft, so a
  // search never races the selector the draft is carrying.
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
      // Preselect the top hit so a single-match search is one action, and leave
      // the draft alone when nothing matched — falling back to the "query=…"
      // selector used to hand the preflight a source that cannot resolve.
      const best = result.sources[0];
      if (best) {
        setPipelineDraft((draft) => ({
          ...draft,
          sourceKind: best.kind,
          zoteroSelector: best.selector || best.title || draft.zoteroSelector,
        }));
      }
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

  const updateSetting = useCallback(
    async (key: keyof LauncherSettings, value: boolean) => {
      const next = { ...settings, [key]: value };
      setSettings(next);
      saveSettings(next);
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
    [addActivity, copy, settings],
  );

  useEffect(() => {
    const unlistenRuntime = listenRuntimeProgress((progress) => {
      if (progress.state === "failed") {
        const message = progress.message || copy.runtimeBootstrapFailed;
        addActivity("warning", message);
        showFloatingToast(message, "warning");
      }
    });
    if (!runtimePrepareStartedRef.current) {
      runtimePrepareStartedRef.current = true;
      void getRuntimeStatus()
        .then((status) => {
          if (status.ready) return undefined;
          return startRuntimePrepare().then(() => undefined);
        })
        .catch((error) => addActivity("warning", copy.runtimePrepareFailed(String(error))));
    }
    return () => {
      unlistenRuntime.then((fn) => fn()).catch(() => undefined);
    };
  }, [addActivity, copy, showFloatingToast]);

  useEffect(() => {
    if (startupInitializedRef.current) return;
    startupInitializedRef.current = true;
    void recordFrontendActivity("info", "frontend startup initialization begin").catch(() => undefined);
    isEnabled()
      .then((enabled) => {
        setSettings((old) => {
          const next = { ...old, autoStart: enabled };
          saveSettings(next);
          return next;
        });
      })
      .catch(() => undefined);
  }, []);

  // The mount load reads its state in the promise continuation instead of
  // calling the shared refreshers, whose first statement flips a busy flag
  // synchronously. `pipelineBusy` starts at "loading" for the same reason.
  useEffect(() => {
    let cancelled = false;
    void getBookPipelineState()
      .then((next) => {
        if (!cancelled) setPipelineState(next);
      })
      .catch((error: unknown) => {
        if (!cancelled) addActivity("warning", String(error));
      })
      .finally(() => {
        if (!cancelled) setPipelineBusy((current) => (current === "loading" ? null : current));
      });
    return () => {
      cancelled = true;
    };
  }, [addActivity]);

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
    return () => {
      if (floatingToastTimer.current) {
        window.clearTimeout(floatingToastTimer.current);
      }
    };
  }, []);

  return (
    <div className="launcher-frame">
      <Titlebar
        version={LAUNCHER_VERSION}
        settingsLabel={copy.settingsTitle}
        settingsActive={settingsOpen}
        onToggleSettings={() => (settingsOpen ? closeSettings() : setSettingsOpen(true))}
      />

      <main className="app-shell">
        <section className={settingsOpen ? "workspace settings-open" : "workspace"}>
          <FloatingFeedback toast={floatingToast} />

          {(
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
                // A draft change invalidates the routes the backend returned,
                // with one exception: the track choice does not feed the
                // preflight at all. Clearing on it emptied the route table and
                // took the choice itself away with it, because the auto-preview
                // effect is keyed on the source identity and never re-fired.
                if (!isTrackOnlyDraftPatch(patch)) setPipelinePreview([]);
              }}
              onPreview={() => void previewPipeline()}
              onQueueRun={queueAndRunPipeline}
              onChooseFolder={() => void choosePipelinePdfFolder()}
              onSearchZotero={(query) => void discoverZoteroByQuery(query)}
              onRetry={(jobId) => void retryPipeline(jobId)}
              onRemoveBooks={removePipelineBooks}
              onMigrateProject={migratePipelineProject}
              onAdvance={(jobId, childId, invalidateDownstream) =>
                void advancePipeline(jobId, childId, invalidateDownstream)
              }
              onSampleTranslation={(jobId, childId, providerProfileId, providerConfigId) =>
                void samplePipelineTranslation(jobId, childId, providerProfileId, providerConfigId)
              }
              onApplySampleProvider={(jobId, childId, providerProfileId, providerConfigId) =>
                void applyPipelineTranslationProvider(jobId, childId, providerProfileId, providerConfigId)
              }
              promptPackCatalog={promptPackCatalog}
              onSelectPromptPack={(jobId, childId, promptPackReference) =>
                void selectPipelinePromptPack(jobId, childId, promptPackReference)
              }
              onPreviewPrompt={previewPipelinePrompt}
              onApproveGate={(jobId, childId, stageId) => void approvePipelineGate(jobId, childId, stageId)}
              onRouteOverride={(jobId, childId, routeItemId, routeOverride) =>
                void overridePipelineRoute(jobId, childId, routeItemId, routeOverride)
              }
              onSampleOcr={(jobId, childId, samplePages) =>
                void samplePipelineOcr(jobId, childId, samplePages)
              }
              onOpenOutput={(jobId) => void openPipelineOutput(jobId)}
              routeOverrides={pipelineRouteOverrides}
              onRouteOverrideChange={changePipelineRouteOverride}
              onHandoff={(jobId, artifactPath) => void handoffPipelineMarkdown(jobId, artifactPath)}
            />
          )}

          {settingsOpen && (
            <SettingsOverlay
              copy={copy}
              locale={locale}
              languageSetting={languageSetting}
              onLanguageChange={updateLanguageSetting}
              settings={settings}
              onUpdateSetting={updateSetting}
              promptPackCatalog={promptPackCatalog}
              promptPackDefaults={promptPackDefaults}
              promptPackBusy={promptPackBusy}
              onCopyPromptPack={copyPromptPack}
              onSavePromptPackRevision={savePromptPackRevision}
              onDeletePromptPack={deletePromptPack}
              onSetPromptPackDefault={setPromptPackDefault}
              onClose={closeSettings}
            />
          )}
        </section>
      </main>
    </div>
  );
}
