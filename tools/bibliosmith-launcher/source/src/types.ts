export type LauncherState = {
  repoRoot: string;
  repoReady: boolean;
  repoStatus: string;
  branch: string;
  localCommit: string;
  localCommitShort: string;
  remoteUrl: string;
  dirty: boolean;
  proxyConfigured: boolean;
  platform: string;
};

export type CommitInfo = {
  hash: string;
  date: string;
  title: string;
  summary: string;
  fullMessage: string;
};

export type BiblioSmithUpdateInfo = {
  repoRoot: string;
  currentCommit: string;
  remoteRef: string;
  behindCount: number;
  aheadCount: number;
  hasUpdate: boolean;
  commits: CommitInfo[];
};

export type ActionResult = {
  ok: boolean;
  message: string;
  repoRoot?: string | null;
  requiresDownload?: boolean | null;
};

export type ProjectDocument = {
  kind: string;
  path: string;
  title: string;
  content: string;
};

export type DownloadProgress = {
  percent: number;
  downloadedBytes: number;
  totalBytes: number;
  message?: string | null;
  state?: "downloading" | "success" | "failed" | "stopped" | null;
};

export type DiagnosticLogSettings = {
  saveLogs: boolean;
  logDir: string;
  logFile: string;
  maxBytes: number;
  backupCount: number;
  maxTotalBytes: number;
};

export type NetworkProxySettings = {
  enabled: boolean;
  scheme: "http" | "https" | "socks5" | "socks5h";
  host: string;
  port: number | null;
};

export type ProxyTestResult = {
  ok: boolean;
  message: string;
  elapsedMs?: number | null;
  httpVersion?: string | null;
  targetUrl: string;
};

export type ProxyAutoDetectResult = {
  detected: boolean;
  proxy?: NetworkProxySettings | null;
  test?: ProxyTestResult | null;
  message: string;
};

export type NodeModulesStatus = {
  ready: boolean;
  running: boolean;
  autoInstall: boolean;
  repoReady: boolean;
  booksDir: string;
  nodeModulesDir: string;
};

export type RuntimeToolStatus = {
  ready: boolean;
  privateReady: boolean;
  version: string;
  source?: string | null;
  path?: string | null;
  message: string;
};

export type RuntimeStatus = {
  ready: boolean;
  privateReady: boolean;
  running: boolean;
  runtimeRoot: string;
  python: RuntimeToolStatus;
  java: RuntimeToolStatus;
};

export type LauncherSettings = {
  autoStart: boolean;
  saveLogsToLocal: boolean;
};

export type ActivityItem = {
  id: string;
  time: string;
  level: "info" | "success" | "warning" | "error";
  message: string;
};

export type BookPipelineSource = {
  kind: "fake" | "local_pdf_folder" | "markdown_source" | "external_adapter" | "zotero_attachment" | "zotero_collection" | "zotero_filter";
  title?: string | null;
  path?: string | null;
  selector?: string | null;
  runnerBehavior?: "succeed" | "fail_once" | "always_fail" | null;
  adapterCommand?: string | null;
  fakeZoteroItems?: FakeZoteroItem[] | null;
};

export type FakeZoteroItem = {
  key: string;
  title: string;
  attachmentPath?: string | null;
  hasTextLayer: boolean;
  dirtyTextLayer: boolean;
  scanned: boolean;
  alreadyConverted: boolean;
  preferMineru: boolean;
};

export type BookPipelineRouteItem = {
  id: string;
  title: string;
  sourceKind: string;
  sourceRef: string;
  routeKind: string;
  canRun: boolean;
  blockedReason?: string | null;
  summary: string;
  /** Set when routeKind came from a user override rather than auto-routing. */
  routeOverride?: string | null;
};

export type BookPipelineArtifact = {
  artifactId?: string;
  kind: string;
  path: string;
  sha256?: string | null;
  sizeBytes?: number | null;
  producer?: {
    childJobId?: string | null;
    stageId: string;
    unitId?: string | null;
    attempt: number;
  };
  inputHashes?: Record<string, string>;
  sourceRefs?: {
    collectionKey?: string | null;
    parentItemKey?: string | null;
    pdfAttachmentKey?: string | null;
    markdownAttachmentKey?: string | null;
    sourceRefSha256: string;
  };
  privacy?: "private_text" | "private_metadata" | "redacted_diagnostic" | string;
  validation?: {
    exists: boolean;
    nonempty: boolean;
    hashMatches: boolean;
    requiredChecksPassed?: boolean | null;
  };
  createdAt?: string;
  supersededBy?: string | null;
  zoteroKey?: string | null;
};

export type BookPipelineArtifactExcerpt = {
  artifactId: string;
  kind: string;
  excerpt: string;
  truncated: boolean;
};

export type BookPipelineTranslationSample = {
  chunkRef: string;
  sourceExcerpt: string;
  translatedExcerpt: string;
  degradation: "none" | "aligned" | "source";
};

export type BookPipelineTranslationSampleReport = {
  schema: "translation-engine-sample-report-v1";
  samples: BookPipelineTranslationSample[];
};

export type BookPipelineCollectionItem = {
  id: string;
  title: string;
  routeKind: string;
  status: string;
  lastError?: string | null;
  artifacts: BookPipelineArtifact[];
  attempts: number;
};

export type BookPipelineUnitSummary = {
  total: number;
  pending: number;
  ready: number;
  running: number;
  blocked: number;
  failed: number;
  completed: number;
  skipped: number;
};

export type BookPipelineStage = {
  stageId: string;
  status: "pending" | "ready" | "running" | "waiting_for_approval" | "blocked" | "failed" | "completed" | "skipped" | string;
  attempt: number;
  error?: string | null;
  safeError?: {
    code: string;
    summary: string;
    retryable: boolean;
    attempt: number;
    stageId: string;
    unitId?: string | null;
    timestamp: string;
    diagnosticArtifactId?: string | null;
  } | null;
  contractVersion: string;
  startedAt?: string | null;
  finishedAt?: string | null;
  inputHashes: Record<string, string>;
  artifactIds: string[];
  unitSummary?: BookPipelineUnitSummary | null;
  approvalId?: string | null;
  executionOwner?: string | null;
  maxAttempts?: number;
  retryBackoffSeconds?: number[];
  giveUpReason?: string | null;
  nextRetryAt?: string | null;
};

export type BookPipelineChildJob = {
  id: string;
  parentJobId: string;
  status: string;
  currentStageId: string;
  source: BookPipelineSource;
  route: BookPipelineRouteItem[];
  stages: BookPipelineStage[];
  artifacts: BookPipelineArtifact[];
  attempts: number;
  lastError?: string | null;
  localProjectRoot?: string | null;
  customInstructions?: BookPipelineCustomInstructions | null;
  readerEvidence?: BookPipelineReaderEvidence[];
  removedAt?: string | null;
};

/// A person opened the built book in a real reader and said what happened. The
/// backend fills `stale` by comparing the digest against the artifact as built,
/// so evidence taken against an older build is shown, not silently trusted.
export type BookPipelineReaderEvidence = {
  reader: string;
  readerVersion: string;
  artifactKind: string;
  artifactSha256: string;
  conclusion: string;
  recordedAt: string;
  stale?: boolean;
};

export const READER_EVIDENCE_ARTIFACT_KINDS = ["reading_epub", "reading_bilingual_epub"] as const;
export const READER_EVIDENCE_CONCLUSIONS = ["passed", "failed"] as const;

export type BookPipelineNavigationTarget = {
  targetId: string;
  kind: string;
  path: string;
  allowedRoot: string;
  artifactId?: string | null;
};

export type BookPipelineOpenTarget = {
  targetId: string;
  kind: string;
  actionLabel: string;
};

export type BookPipelineMembership = {
  revision: number;
  frozenAt?: string | null;
  discoveryStageId: string;
  childJobIds: string[];
};

export type BookPipelineStatusSummary = {
  total: number;
  pending: number;
  ready: number;
  running: number;
  waitingForApproval: number;
  blocked: number;
  failed: number;
  completed: number;
  skipped: number;
};

export type BookPipelineProgress = {
  stageTotal: number;
  stageCompleted: number;
  percent: number;
  activeStageId: string;
  unitSummary?: BookPipelineUnitSummary | null;
  retryAttemptsRemaining?: number;
  nextRetryAt?: string | null;
  giveUpReason?: string | null;
};

export type BookPipelineNotificationDelivery = {
  eventId: string;
  status: string;
  deliveryStatus: "delivered" | "failed" | string;
  attempts: number;
  deliveredAt?: string | null;
  safeError?: string | null;
};

export type BookPipelineApprovalReference = {
  approvalId: string;
  gateId: string;
  childJobId: string;
  stageId: string;
  decision: string;
  boundArtifactHashes: Record<string, string>;
};

export type BookPipelineOutputFormat = "md" | "html" | "epub" | "bilingual";

export type BookPipelineCustomInstructions = {
  translation?: string | null;
  reflection?: string | null;
};

export type BookPipelineTranslationIntent = {
  translationMode: "fast" | "expert";
  profileId: string;
  configId: string;
  skillIds: string[];
  secondPassEnabled: boolean;
  textCleanup: boolean;
  digestMode: boolean;
  outputFormats: BookPipelineOutputFormat[];
};

export type BookPipelineJob = {
  schemaVersion: string;
  id: string;
  kind: "single" | "collection" | string;
  mode: string;
  translationMode?: string;
  translationProfileId?: string;
  translationConfigId?: string;
  translationSkillIds?: string[];
  secondPassEnabled?: boolean;
  textCleanup?: boolean;
  digestMode?: boolean;
  outputFormats?: BookPipelineOutputFormat[];
  source: BookPipelineSource;
  route: BookPipelineRouteItem[];
  status: "pending" | "ready" | "running" | "waiting_for_approval" | "completed" | "partial" | "failed" | "blocked" | "skipped" | string;
  currentStageId: string;
  currentStep: string;
  lastError?: string | null;
  logSummary: string[];
  artifacts: BookPipelineArtifact[];
  collectionItems?: BookPipelineCollectionItem[];
  outputDir?: string | null;
  attempts: number;
  stages: BookPipelineStage[];
  children: BookPipelineChildJob[];
  membership?: BookPipelineMembership | null;
  summary: BookPipelineStatusSummary;
  progress: BookPipelineProgress;
  notificationDeliveries: BookPipelineNotificationDelivery[];
  approvalReferences: BookPipelineApprovalReference[];
  navigationTargets?: BookPipelineNavigationTarget[];
  openTarget?: BookPipelineOpenTarget | null;
  createdAt: string;
  updatedAt: string;
};

export type BookPipelineState = {
  schemaVersion: string;
  revision: number;
  jobs: BookPipelineJob[];
};

export type BookPipelineZoteroDiscoveryResult = {
  sources: BookPipelineSource[];
  logSummary: string[];
};

export type BookPipelinePreviewConfig = {
  hasPaddleocrCredentials: boolean;
  hasMineruCredentials: boolean;
  /** Per-route-item conversion overrides, keyed by route item id. */
  routeOverrides?: Record<string, string>;
};

export type BookPipelineActionResult = {
  ok: boolean;
  message: string;
  path?: string | null;
};

// Redaction profiles the backend accepts for a diagnostic bundle, in increasing
// order of disclosure. build_book_pipeline_diagnostic rejects anything else.
export const BOOK_PIPELINE_DIAGNOSTIC_PROFILES = [
  "public-issue",
  "redacted-support",
  "local-full",
] as const;

export type BookPipelineDiagnosticProfile = (typeof BOOK_PIPELINE_DIAGNOSTIC_PROFILES)[number];

export type BookPipelineCleanupEvidence = {
  kind: string;
  ok: boolean;
  detail: string;
  path?: string | null;
  zoteroKey?: string | null;
};

export type BookPipelineCleanupCandidate = {
  id: string;
  jobId: string;
  title: string;
  sourceKind: string;
  sourceRef: string;
  sourcePath?: string | null;
  sourcePdfKey?: string | null;
  markdownPath?: string | null;
  localOutputPath?: string | null;
  zoteroChildAttachmentKey?: string | null;
  checks: BookPipelineCleanupEvidence[];
  canApprove: boolean;
};

export type BookPipelineCleanupPreview = {
  candidates: BookPipelineCleanupCandidate[];
  logSummary: string[];
};

export type ActiveModel = {
  profileId: string;
  configId: string;
  model: string;
};

export type ModelSlotView = {
  profileId: string;
  configId: string;
  providerType: string;
  defaultModel: string;
  configured: boolean;
};

export type ModelCatalog = {
  slots: ModelSlotView[];
  active: ActiveModel | null;
};

export type ModelConnectionResult = {
  ok: boolean;
  message: string;
};

export type EmbeddingStatus = {
  backend: string;
  configured: boolean;
};

export type EmbeddingConnectionResult = {
  ok: boolean;
  message: string;
};

export type OcrServiceStatus = {
  configured: boolean;
  source: "keychain" | "env" | null;
};

export type OcrCredentialsStatus = {
  paddleocr: OcrServiceStatus;
  mineru: OcrServiceStatus;
};

export type OcrConnectionResult = {
  ok: boolean;
  message: string;
};
