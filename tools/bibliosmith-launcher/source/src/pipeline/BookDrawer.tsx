import { useEffect, useState, type ReactElement } from "react";
import { ChevronLeft, ChevronRight, Download, FolderOpen, Trash2, X } from "lucide-react";
import { readBookPipelineArtifactExcerpt, readBookPipelineTranslationSample } from "../api";
import type {
  BookPipelineArtifact,
  BookPipelineCustomInstructions,
  BookPipelineDiagnosticProfile,
  BookPipelineTranslationSampleReport,
} from "../types";
import type { PipelineCopy } from "./copy";
import {
  activeStepIndex,
  allArtifacts,
  currentStage,
  firstMarkdownArtifact,
  fourStepStates,
  pendingGates,
  providerDefaultConfig,
  stepCaption,
  stepSummaryCaption,
  stepName,
  stepStatusCaption,
  sourceChangedRequiresRebuild,
  translationFailureSummary,
  unitAdvanceAction,
  unitRoute,
  type BookUnit,
  type GateView,
  type PipelineBusy,
  type StepState,
} from "./model";
import { MODEL_BRANDS, slotDisplayName, slotMeta } from "../pages/settings/modelCatalog";
import { BookCover } from "./Shelf";
import { OverviewTab } from "./tabs/OverviewTab";
import { StagesTab } from "./tabs/StagesTab";
import { ArtifactsTab } from "./tabs/ArtifactsTab";
import { ApprovalTab } from "./tabs/ApprovalTab";
import { LogsTab } from "./tabs/LogsTab";
import { OperationProgressBar } from "./OperationProgress";

// Identity tags. Each pairs a piece of local state with the thing it describes,
// so a render can tell a stale value from a current one without an effect.
type ProviderChoice = { basis: string; profileId: string; configId: string };
type SampleReportState = { version: string; report: BookPipelineTranslationSampleReport };
type SampleState = { id: string; text: string; truncated: boolean };

// A saved instruction pair identifies one editor instance; NUL cannot appear in
// either field, so it separates them unambiguously.
function customInstructionsKey(unit: BookUnit) {
  const saved = unit.child?.customInstructions;
  return `${saved?.translation ?? ""}\u0000${saved?.reflection ?? ""}`;
}

export type BookDrawerProps = {
  copy: PipelineCopy;
  units: BookUnit[];
  unit: BookUnit;
  busy: PipelineBusy;
  onSelect: (key: string) => void;
  onClose: () => void;
  onRetry: (jobId: string) => void;
  onDelete: (jobId: string, childId?: string | null) => void;
  onAdvance: (jobId: string, childId: string, invalidateDownstream?: boolean) => void;
  onSampleTranslation: (jobId: string, childId: string, providerProfileId: string, providerConfigId: string) => void;
  // Adopting a sampled slot as the book's own. Separate from sampling, because
  // sampling is "try one out" and this decides what the full run uses.
  onApplySampleProvider: (jobId: string, childId: string, providerProfileId: string, providerConfigId: string) => void;
  onExportDiagnostic: (jobId: string, profile: BookPipelineDiagnosticProfile) => void;
  onSaveCustomInstructions: (
    jobId: string,
    childId: string,
    customInstructions: BookPipelineCustomInstructions,
  ) => void;
  onApproveGate: (jobId: string, childId: string, stageId: "approve_translation" | "approve_promotion") => void;
  onRouteOverride: (jobId: string, childId: string, routeItemId: string, routeOverride: string) => void;
  onRecordReaderEvidence: (
    jobId: string,
    childId: string,
    artifactKind: string,
    reader: string,
    readerVersion: string,
    conclusion: string,
  ) => void;
  onOpenOutput: (jobId: string) => void;
  onHandoff: (jobId: string, artifactPath?: string | null) => void;
};

// The draft starts from what is saved and diverges as the user types, so a new
// saved value means a new editor: the caller keys this component on the saved
// pair instead of an effect copying it back into local state.
function CustomInstructionsEditor({ unit, copy, busy, onSaveCustomInstructions }: BookDrawerProps) {
  const savedTranslation = unit.child?.customInstructions?.translation ?? "";
  const savedReflection = unit.child?.customInstructions?.reflection ?? "";
  const [translation, setTranslation] = useState(savedTranslation);
  const [reflection, setReflection] = useState(savedReflection);

  const translationLength = Array.from(translation).length;
  const reflectionLength = Array.from(reflection).length;
  const tooLong = translationLength > 2000 || reflectionLength > 2000;
  const dirty = translation !== savedTranslation || reflection !== savedReflection;
  const saving = busy === "customInstructions";
  const disabled = Boolean(busy) || !unit.child || !dirty || tooLong;

  return (
    <section className="pl-custom-instructions">
      <div className="pl-ci-head">
        <div>
          <h3>{copy.customInstructionsTitle}</h3>
          <p>{copy.customInstructionsHelp}</p>
        </div>
      </div>
      <label className="pl-ci-field">
        <span>{copy.customTranslationLabel}</span>
        <textarea
          value={translation}
          placeholder={copy.customTranslationPlaceholder}
          onChange={(event) => setTranslation(event.currentTarget.value)}
        />
        <small className={translationLength > 2000 ? "over" : ""}>
          {copy.customInstructionsCount(translationLength)}
        </small>
      </label>
      <label className="pl-ci-field">
        <span>{copy.customReflectionLabel}</span>
        <textarea
          value={reflection}
          placeholder={copy.customReflectionPlaceholder}
          onChange={(event) => setReflection(event.currentTarget.value)}
        />
        <small className={reflectionLength > 2000 ? "over" : ""}>
          {copy.customInstructionsCount(reflectionLength)}
        </small>
      </label>
      {tooLong && <p className="pl-ci-error">{copy.customInstructionsTooLong}</p>}
      <div className="pl-ci-actions">
        <button
          className="pl-btn sm"
          type="button"
          disabled={disabled}
          onClick={() => {
            if (!unit.child) return;
            onSaveCustomInstructions(unit.job.id, unit.child.id, { translation, reflection });
          }}
        >
          {saving ? copy.savingCustomInstructions : copy.saveCustomInstructions}
        </button>
      </div>
    </section>
  );
}

const STEP_MARKS: Record<StepState, (index: number) => string> = {
  done: () => "✓",
  gate: () => "!",
  error: () => "✕",
  current: (index) => String(index + 1),
  todo: (index) => String(index + 1),
  none: () => "–",
};

function FourStepStrip({ unit, copy }: { unit: BookUnit; copy: PipelineCopy }) {
  const states = fourStepStates(unit);
  return (
    <>
      <div className="pl-steps4">
        {states.map((state, index) => (
          <div key={index} className={`pl-step4 ${state}`}>
            <div className="pl-s4c">{STEP_MARKS[state](index)}</div>
            <div className="pl-s4n">{stepName(index, copy)}</div>
            <div className="pl-s4status">{stepStatusCaption(unit, index, copy)}</div>
          </div>
        ))}
      </div>
      <div className="pl-s4cap">
        {activeStepIndex(states) === -1
          ? stepSummaryCaption(unit, copy)
          : `${copy.stepCurrentPrefix}${stepSummaryCaption(unit, copy)}`}
      </div>
    </>
  );
}

/**
 * Sample source per gate: the translation gate shows what is about to be sent
 * (a prepared/extracted chapter); the promotion gate shows what is about to be
 * finalized (a translated chapter). First registered, non-superseded match wins.
 */
const SAMPLE_KINDS: Record<"approve_translation" | "approve_promotion", string[]> = {
  approve_translation: ["chapter_source", "extraction_markdown", "markdown"],
  approve_promotion: ["chapter_translation", "translation_draft"],
};

function sampleArtifact(unit: BookUnit, stageId: GateView["stageId"]): BookPipelineArtifact | null {
  const artifacts = allArtifacts(unit).filter((artifact) => artifact.artifactId && !artifact.supersededBy);
  for (const kind of SAMPLE_KINDS[stageId]) {
    const hit = artifacts.find((artifact) => artifact.kind === kind);
    if (hit) return hit;
  }
  return null;
}

function translationSampleArtifact(unit: BookUnit): BookPipelineArtifact | null {
  return allArtifacts(unit).find(
    (artifact) => artifact.kind === "translation_sample_report" && !artifact.supersededBy,
  ) ?? null;
}

/** The 3-5 "take a look" confirmation card for a pending human gate. */
function GateCard({
  unit,
  gate,
  copy,
  busy,
  onSampleTranslation,
  onApplySampleProvider,
  onApproveGate,
  onOpenOutput,
}: {
  unit: BookUnit;
  gate: GateView;
  copy: PipelineCopy;
  busy: PipelineBusy;
  onSampleTranslation: BookDrawerProps["onSampleTranslation"];
  onApplySampleProvider: BookDrawerProps["onApplySampleProvider"];
  onApproveGate: BookDrawerProps["onApproveGate"];
  onOpenOutput: BookDrawerProps["onOpenOutput"];
}) {
  const isTranslation = gate.stageId === "approve_translation";
  const canRunProviderSample = isTranslation && unit.job.translationMode !== "expert";
  const failedChecks = gate.checks.filter((check) => check.ok === false).length;
  const scope = gate.stage.unitSummary?.total ? copy.gateScopeChapters(gate.stage.unitSummary.total) : "";
  const lead = isTranslation ? copy.gate1Lead(scope) : copy.gate2Lead(scope);
  const canOpen = Boolean(unit.job.openTarget);
  const childId = unit.child?.id ?? null;

  // What the full book will run on. The picker below starts here but is only the
  // sample's provider from then on — running a sample no longer adopts it.
  const jobProfile = unit.job.translationProfileId || "openai-compatible";
  const jobConfig = unit.job.translationConfigId || providerDefaultConfig(jobProfile);
  // The three pieces of state below each belong to one identity — the job's
  // provider, one sample report, one excerpt — so each carries that identity
  // and is filtered during render. They used to be reset by an effect, which
  // left one frame showing the previous book's values.
  const providerBasis = `${jobProfile}:${jobConfig}`;
  const [providerChoice, setProviderChoice] = useState<ProviderChoice | null>(null);
  const providerPicked = providerChoice?.basis === providerBasis ? providerChoice : null;
  const providerProfileId = providerPicked?.profileId ?? jobProfile;
  const providerConfigId = providerPicked?.configId ?? jobConfig;
  const providerDiffers = providerProfileId !== jobProfile || providerConfigId !== jobConfig;
  // A job stored before a slot was retired still carries it, and the engine
  // rejects an unknown (profile, config) pair outright — so approving would
  // bind the book to a translation that cannot start. The picker below already
  // keeps the retired value selectable and "apply" already writes a new one, so
  // the gap was only that nothing said the book had to be moved first.
  //
  // Gated on canRunProviderSample, not on the catalog alone: expert mode carries
  // the "expert-agent" profile, which the catalog never lists and which is not
  // retired, and the promotion gate does not run on this slot at all. Both would
  // otherwise be blocked here with no picker on screen to unblock them.
  const jobSlotRetired = canRunProviderSample && !slotMeta(jobProfile, jobConfig);

  const reportArtifact = canRunProviderSample ? translationSampleArtifact(unit) : null;
  const reportVersion = reportArtifact?.sha256 ?? reportArtifact?.artifactId ?? null;
  const [sampleReportState, setSampleReport] = useState<SampleReportState | null>(null);
  const sampleReport = sampleReportState?.version === reportVersion ? sampleReportState.report : null;
  useEffect(() => {
    if (!canRunProviderSample || !childId || !reportVersion) return undefined;
    let cancelled = false;
    readBookPipelineTranslationSample(unit.job.id, childId)
      .then((report) => {
        if (!cancelled) setSampleReport({ version: reportVersion, report });
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [canRunProviderSample, childId, reportVersion, unit.job.id]);

  // Best-effort sample of the content being confirmed; the card is fully
  // functional without one (preview mode, unreadable artifact, older jobs).
  const sampleId = sampleArtifact(unit, gate.stageId)?.artifactId ?? null;
  const [sampleState, setSample] = useState<SampleState | null>(null);
  const sample = sampleState?.id === sampleId ? sampleState : null;
  useEffect(() => {
    if (!sampleId) return undefined;
    let cancelled = false;
    readBookPipelineArtifactExcerpt(unit.job.id, sampleId)
      .then((result) => {
        if (!cancelled && result.excerpt.trim()) {
          setSample({ id: sampleId, text: result.excerpt, truncated: result.truncated });
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [unit.job.id, sampleId]);

  return (
    <div className="pl-gatecard">
      <div className="pl-ghead">⚑ {copy.gateCardTitle}</div>
      <div className="pl-gbody">
        <p className="pl-glead">{lead}</p>
        {canRunProviderSample && childId && (
          <section className="pl-scompare" aria-label={copy.sampleCompareTitle}>
            <div className="pl-scompare-head">
              <h4>{copy.sampleCompareTitle}</h4>
              <p>{copy.sampleCompareIntro}</p>
            </div>
            <div className="pl-scontrols">
              <label>
                {copy.sampleProvider}
                <select
                  value={`${providerProfileId}:${providerConfigId}`}
                  disabled={busy === "sample"}
                  onChange={(event) => {
                    const [profile, config] = event.target.value.split(":");
                    setProviderChoice({ basis: providerBasis, profileId: profile, configId: config });
                  }}
                >
                  {/* One option per slot, not per brand: Qwen and MiMo bill two
                      ways, and a brand-only list made their second plan
                      unreachable. A job may also carry a slot the catalog no
                      longer lists (renamed, or the expert agent); keep it
                      selectable so opening the drawer cannot strand the picker
                      on a value it has no option for. */}
                  {!slotMeta(providerProfileId, providerConfigId) && (
                    <option value={`${providerProfileId}:${providerConfigId}`}>
                      {slotDisplayName(providerProfileId, providerConfigId)}
                    </option>
                  )}
                  {MODEL_BRANDS.flatMap((brand) =>
                    brand.slots.map((slot) => (
                      <option
                        key={`${slot.profileId}:${slot.configId}`}
                        value={`${slot.profileId}:${slot.configId}`}
                      >
                        {slotDisplayName(slot.profileId, slot.configId)}
                      </option>
                    )),
                  )}
                </select>
              </label>
              <button
                className="pl-btn sm"
                type="button"
                disabled={busy !== null || !providerProfileId.trim() || !providerConfigId.trim()}
                onClick={() => onSampleTranslation(
                  unit.job.id,
                  childId,
                  providerProfileId.trim(),
                  providerConfigId.trim(),
                )}
              >
                {sampleReport ? copy.sampleRetry : copy.sampleRun}
              </button>
            </div>
            {/* Sampling no longer adopts its provider, so the gap has to be
                visible: the binding the user approves carries the job's slot,
                while the evidence in front of them came from the sample's. */}
            <div className="pl-evi-row">
              <span className="pl-k">{copy.jobProvider}</span>
              <span className="pl-v">{slotDisplayName(jobProfile, jobConfig)}</span>
              {providerDiffers && (
                <button
                  className="pl-btn sm"
                  type="button"
                  disabled={busy !== null}
                  onClick={() => onApplySampleProvider(
                    unit.job.id,
                    childId,
                    providerProfileId.trim(),
                    providerConfigId.trim(),
                  )}
                >
                  {copy.applySampleProvider}
                </button>
              )}
            </div>
            {providerDiffers && <p className="pl-wiz-error">{copy.sampleProviderDiffers}</p>}
            {sampleReport && (
              <div className="pl-sresults">
                {sampleReport.samples.map((entry) => (
                  <article className="pl-sresult" key={entry.chunkRef}>
                    <div className="pl-sresult-meta">
                      <code>{entry.chunkRef}</code>
                      <span className={`pl-degradation ${entry.degradation}`}>
                        {entry.degradation === "none"
                          ? copy.sampleDegradationNone
                          : entry.degradation === "aligned"
                            ? copy.sampleDegradationAligned
                            : copy.sampleDegradationSource}
                      </span>
                    </div>
                    <div className="pl-scolumns">
                      <div>
                        <h5>{copy.sampleSource}</h5>
                        <p>{entry.sourceExcerpt}</p>
                      </div>
                      <div>
                        <h5>{copy.sampleTranslation}</h5>
                        <p>{entry.translatedExcerpt}</p>
                      </div>
                    </div>
                  </article>
                ))}
              </div>
            )}
          </section>
        )}
        {!sampleReport && sample && (
          <div className="pl-gsample">
            <h5>{isTranslation ? copy.gateSampleSourceTitle : copy.gateSampleTranslationTitle}</h5>
            {sample.text}
            {sample.truncated ? "…" : ""}
          </div>
        )}
        <p className="pl-gprivacy">{isTranslation ? copy.gate1Privacy : copy.gate2Privacy}</p>
        {gate.invalidated ? (
          <p className="pl-gblocked">{copy.gateInvalidatedNote}</p>
        ) : (
          <div className="pl-gacts">
            <button
              className="pl-btn primary"
              type="button"
              disabled={
                busy === "gateApproval"
                || busy === "sample"
                || failedChecks > 0
                || jobSlotRetired
                || !unit.child
              }
              onClick={() => unit.child && onApproveGate(unit.job.id, unit.child.id, gate.stageId)}
            >
              {isTranslation ? copy.gate1Ok : copy.gate2Ok}
            </button>
            {canOpen && (
              <button className="pl-btn" type="button" disabled={busy === "open"} onClick={() => onOpenOutput(unit.job.id)}>
                {isTranslation ? copy.gate1Alt : copy.gate2Alt}
              </button>
            )}
          </div>
        )}
        {failedChecks > 0 && !gate.invalidated && <p className="pl-gblocked">{copy.gateBlockedByChecks}</p>}
        {jobSlotRetired && !gate.invalidated && (
          <p className="pl-gblocked">{copy.gateBlockedByRetiredProvider}</p>
        )}
      </div>
    </div>
  );
}

/** State card for books not sitting at a gate: error / blocked / advance / done / running. */
function StateCard(props: BookDrawerProps) {
  const { unit, copy, busy, onRetry, onAdvance, onOpenOutput, onHandoff } = props;
  const stage = currentStage(unit);
  const errorText =
    translationFailureSummary(stage, copy) ||
    stage?.safeError?.summary ||
    stage?.error ||
    unit.child?.lastError ||
    unit.job.lastError ||
    "";
  const advance = unitAdvanceAction(unit);

  if (sourceChangedRequiresRebuild(unit) && unit.child) {
    return (
      <div className="pl-hintcard blockc">
        <span>◈ {copy.sourceChangedBody}</span>
        <span className="pl-spacer" />
        <button
          className="pl-btn sm primary"
          type="button"
          disabled={busy === "advance"}
          onClick={() => onAdvance(unit.job.id, unit.child!.id, true)}
        >
          {copy.rebuildFromMineru}
        </button>
      </div>
    );
  }

  if (unit.status === "failed" || unit.status === "partial") {
    return (
      <div className="pl-hintcard errc">
        <span>✗ {errorText ? `${errorText}。` : ""}{copy.abRetryHint}</span>
        <span className="pl-spacer" />
        <button className="pl-btn sm" type="button" disabled={busy === "retry"} onClick={() => onRetry(unit.job.id)}>
          {copy.retryJob}
        </button>
      </div>
    );
  }

  if (unit.status === "blocked") {
    return (
      <div className="pl-hintcard blockc">
        <span>◈ {errorText || unitRoute(unit)?.blockedReason || unitRoute(unit)?.summary || copy.statusBlocked}</span>
        <span className="pl-spacer" />
        {advance && (
          <button
            className="pl-btn sm primary"
            type="button"
            disabled={busy === "advance"}
            onClick={() => onAdvance(unit.job.id, advance.childId)}
          >
            {copy.recheckHandoff}
          </button>
        )}
      </div>
    );
  }

  if (advance) {
    const label = advance.stageId === "translate" ? copy.runTranslation : copy.continueStage;
    return (
      <div className="pl-hintcard">
        <span>{stepSummaryCaption(unit, copy)}</span>
        <span className="pl-spacer" />
        <button
          className="pl-btn sm primary"
          type="button"
          disabled={busy === "advance"}
          onClick={() => onAdvance(unit.job.id, advance.childId)}
        >
          {label}
        </button>
      </div>
    );
  }

  const markdown = firstMarkdownArtifact(unit.job);
  if (unit.job.mode === "conversion_only" && unit.job.status === "completed" && markdown) {
    return (
      <div className="pl-hintcard">
        <span>{copy.handoffReadyHint}</span>
        <span className="pl-spacer" />
        <button
          className="pl-btn sm primary"
          type="button"
          disabled={busy === "handoff"}
          onClick={() => onHandoff(unit.job.id, markdown.path)}
        >
          {copy.handoff}
        </button>
      </div>
    );
  }

  if (unit.status === "completed") {
    return (
      <div className="pl-hintcard">
        <span>✓ {stepCaption(unit, copy)}</span>
        <span className="pl-spacer" />
        {unit.job.openTarget && (
          <button className="pl-btn sm primary" type="button" disabled={busy === "open"} onClick={() => onOpenOutput(unit.job.id)}>
            <FolderOpen size={14} />
            {unit.job.openTarget.actionLabel || copy.openOutput}
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="pl-running-stack">
      <OperationProgressBar unit={unit} copy={copy} />
      <div className="pl-hintcard">
        <span>{stepSummaryCaption(unit, copy)} · {copy.abNoAction}</span>
      </div>
    </div>
  );
}

type AdvTab = "overview" | "stages" | "artifacts" | "approval" | "logs";

function AdvancedDetails(props: BookDrawerProps) {
  const { unit, copy } = props;
  const [tab, setTab] = useState<AdvTab>("overview");
  const tabProps = {
    unit,
    copy,
    busy: props.busy,
    onRetry: props.onRetry,
    onAdvance: props.onAdvance,
    onApproveGate: props.onApproveGate,
    onRouteOverride: props.onRouteOverride,
    onRecordReaderEvidence: props.onRecordReaderEvidence,
    onOpenOutput: props.onOpenOutput,
    onHandoff: props.onHandoff,
    onGoApproval: () => setTab("approval"),
  };
  return (
    <details className="pl-adv">
      <summary>{copy.advancedDetails}</summary>
      <div className="pl-tabs" role="tablist">
        {(
          [
            ["overview", copy.tabOverview],
            ["stages", copy.tabStages],
            ["artifacts", copy.tabArtifacts],
            ["approval", copy.tabApproval],
            ["logs", copy.tabLogs],
          ] as [AdvTab, string][]
        ).map(([key, label]) => (
          <button
            key={key}
            className={`pl-tab${tab === key ? " on" : ""}`}
            type="button"
            role="tab"
            aria-selected={tab === key}
            onClick={() => setTab(key)}
          >
            {label}
          </button>
        ))}
      </div>
      <div className="pl-tabbody">
        {tab === "overview" && <OverviewTab {...tabProps} />}
        {tab === "stages" && <StagesTab {...tabProps} />}
        {tab === "artifacts" && <ArtifactsTab {...tabProps} />}
        {tab === "approval" && <ApprovalTab {...tabProps} />}
        {tab === "logs" && <LogsTab {...tabProps} />}
      </div>
      <DiagnosticExport
        copy={copy}
        busy={props.busy}
        jobId={unit.job.id}
        onExportDiagnostic={props.onExportDiagnostic}
      />
    </details>
  );
}

/**
 * The three redaction profiles have been configured and tested on the backend
 * since the diagnostic command landed, with nothing in the UI to reach them —
 * a user reporting a problem had only screenshots. Public-issue is the default
 * because it is the one that can be pasted anywhere without reading it first.
 */
function DiagnosticExport({
  copy,
  busy,
  jobId,
  onExportDiagnostic,
}: {
  copy: PipelineCopy;
  busy: PipelineBusy;
  jobId: string;
  onExportDiagnostic: BookDrawerProps["onExportDiagnostic"];
}) {
  const [profile, setProfile] = useState<BookPipelineDiagnosticProfile>("public-issue");
  const profiles: [BookPipelineDiagnosticProfile, string, string][] = [
    ["public-issue", copy.diagnosticPublicIssue, copy.diagnosticPublicIssueNote],
    ["redacted-support", copy.diagnosticRedactedSupport, copy.diagnosticRedactedSupportNote],
    ["local-full", copy.diagnosticLocalFull, copy.diagnosticLocalFullNote],
  ];
  const note = profiles.find(([key]) => key === profile)?.[2] ?? "";
  return (
    <section className="pl-diagnostic" aria-label={copy.diagnosticTitle}>
      <h4>{copy.diagnosticTitle}</h4>
      <p>{copy.diagnosticIntro}</p>
      <div className="pl-diagnostic-controls">
        <label>
          {copy.diagnosticProfile}
          <select
            value={profile}
            disabled={busy === "diagnostic"}
            onChange={(event) => setProfile(event.target.value as BookPipelineDiagnosticProfile)}
          >
            {profiles.map(([key, label]) => (
              <option key={key} value={key}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <button
          className="pl-btn sm"
          type="button"
          disabled={busy !== null}
          onClick={() => onExportDiagnostic(jobId, profile)}
        >
          <Download size={14} />
          {copy.diagnosticExport}
        </button>
      </div>
      <p className="pl-diagnostic-note">{note}</p>
    </section>
  );
}

function actionBar(props: BookDrawerProps): { hint: string; button: ReactElement | null } {
  const { unit, copy, busy } = props;
  const gate = pendingGates(unit, copy)[0];
  if (sourceChangedRequiresRebuild(unit)) {
    return { hint: copy.sourceChangedTitle, button: null };
  }
  if (gate && !gate.invalidated) {
    const isTranslation = gate.stageId === "approve_translation";
    const failedChecks = gate.checks.filter((check) => check.ok === false).length;
    return {
      hint: `${copy.abGatePrefix}${isTranslation ? copy.capGateTranslation : copy.capGatePromotion}`,
      button: (
        <button
          className="pl-btn primary"
          type="button"
          disabled={busy === "gateApproval" || busy === "sample" || failedChecks > 0 || !unit.child}
          onClick={() => unit.child && props.onApproveGate(unit.job.id, unit.child.id, gate.stageId)}
        >
          {isTranslation ? copy.gate1Ok : copy.gate2Ok}
        </button>
      ),
    };
  }
  if (unit.status === "failed" || unit.status === "partial") {
    return {
      hint: stepSummaryCaption(unit, copy),
      button: (
        <button className="pl-btn" type="button" disabled={busy === "retry"} onClick={() => props.onRetry(unit.job.id)}>
          {copy.retryJob}
        </button>
      ),
    };
  }
  if (unitAdvanceAction(unit)) {
    return { hint: copy.abAdvanceRequired, button: null };
  }
  if (unit.status === "completed" && unit.job.openTarget) {
    return {
      hint: copy.capAllDone,
      button: (
        <button className="pl-btn primary" type="button" disabled={busy === "open"} onClick={() => props.onOpenOutput(unit.job.id)}>
          {unit.job.openTarget.actionLabel || copy.openOutput}
        </button>
      ),
    };
  }
  return { hint: `${stepSummaryCaption(unit, copy)} · ${copy.abNoAction}`, button: null };
}

export function BookDrawer(props: BookDrawerProps) {
  const { copy, units, unit, busy, onSelect, onClose, onDelete } = props;
  // Delete now drops just this book; only the last one left takes the job with
  // it, so the batch wording is gone and the confirmation is true again.
  const index = units.findIndex((candidate) => candidate.key === unit.key);
  const step = (offset: number) => {
    if (!units.length) return;
    const next = units[(index + offset + units.length) % units.length];
    onSelect(next.key);
  };
  // `confirmingDelete` resets with the drawer: PipelineWorkbench keys this
  // component on the selected book, so switching books remounts it.
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const gates = pendingGates(unit, copy);
  const bar = actionBar(props);
  return (
    <>
      <div className="pl-scrim" onClick={onClose} />
      <aside className="pl-drawer" aria-label={unit.title}>
        <div className="pl-dhead">
          <span className="pl-dord pl-num">{index + 1} / {units.length}</span>
          <div className="pl-dnav">
            <button type="button" aria-label={copy.deleteBook} title={copy.deleteBook} onClick={() => setConfirmingDelete(true)}>
              <Trash2 size={15} />
            </button>
            <button type="button" aria-label={copy.drawerPrev} onClick={() => step(-1)}>
              <ChevronLeft size={15} />
            </button>
            <button type="button" aria-label={copy.drawerNext} onClick={() => step(1)}>
              <ChevronRight size={15} />
            </button>
            <button type="button" aria-label={copy.drawerClose} onClick={onClose}>
              <X size={15} />
            </button>
          </div>
        </div>
        <div className="pl-dbody">
          {confirmingDelete && (
            <div className="pl-hintcard errc">
              <span>{copy.deleteBookConfirmHint}</span>
              <span className="pl-spacer" />
              <button
                className="pl-btn sm danger-ghost"
                type="button"
                disabled={busy === "delete"}
                onClick={() => onDelete(unit.job.id, unit.child?.id ?? null)}
              >
                {copy.deleteBookConfirm}
              </button>
              <button
                className="pl-btn sm quiet"
                type="button"
                disabled={busy === "delete"}
                onClick={() => setConfirmingDelete(false)}
              >
                {copy.deleteBookCancel}
              </button>
            </div>
          )}
          <div className="pl-dbook">
            <BookCover title={unit.title} className="drawer" />
            <div>
              <h2>{unit.title}</h2>
              <div className="pl-dnow">{stepSummaryCaption(unit, copy)}</div>
            </div>
          </div>
          <FourStepStrip unit={unit} copy={copy} />
          {unit.job.mode !== "conversion_only" && unit.job.translationMode === "fast" && (
            <CustomInstructionsEditor key={customInstructionsKey(unit)} {...props} />
          )}
          {gates.length > 0 ? (
            gates.map((gate) => (
              <GateCard
                key={gate.stageId}
                unit={unit}
                gate={gate}
                copy={copy}
                busy={props.busy}
                onSampleTranslation={props.onSampleTranslation}
                onApplySampleProvider={props.onApplySampleProvider}
                onApproveGate={props.onApproveGate}
                onOpenOutput={props.onOpenOutput}
              />
            ))
          ) : (
            <StateCard {...props} />
          )}
          <AdvancedDetails {...props} />
        </div>
        <div className="pl-dfoot">
          <span className="pl-dhint">{bar.hint}</span>
          {bar.button}
        </div>
      </aside>
    </>
  );
}
