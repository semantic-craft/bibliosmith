import type { PipelineCopy } from "../copy";
import {
  GATE_STAGE_IDS,
  currentStage,
  firstMarkdownArtifact,
  focusStages,
  gateLabel,
  hashShort,
  pendingGates,
  routeKindLabel,
  stageLabel,
  statusLabel,
  sourceChangedRequiresRebuild,
  translationFailureSummary,
  unitAdvanceAction,
  unitRoute,
  allArtifacts,
  type BookUnit,
} from "../model";
import type { TabProps } from "./tabProps";
import { OcrCompareCard, canCompareOcrEngines } from "../OcrCompareCard";

function railNodeClass(status: string): string {
  switch (status) {
    case "completed":
      return "done";
    case "skipped":
      return "skipn";
    case "running":
      return "run";
    case "waiting_for_approval":
      return "waitg";
    case "failed":
      return "failn";
    case "blocked":
      return "blockn";
    default:
      return "";
  }
}

function StageRail({ unit, copy }: { unit: BookUnit; copy: PipelineCopy }) {
  const stages = focusStages(unit);
  return (
    <div className="pl-stage-rail">
      {stages.map((stage) => {
        const cls = railNodeClass(stage.status);
        const gate = GATE_STAGE_IDS.has(stage.stageId);
        return (
          <div key={stage.stageId} className={`pl-srail-node ${cls}${gate ? " gate" : ""}`}>
            <div className="pl-snode">
              <span>{cls === "done" ? "✓" : cls === "skipn" ? "–" : ""}</span>
            </div>
            <div className="pl-slab">{stageLabel(stage.stageId, copy)}</div>
          </div>
        );
      })}
    </div>
  );
}

function ActionCard({ unit, copy, busy, onRetry, onAdvance, onHandoff, onGoApproval, onRouteOverride }: TabProps) {
  const stage = currentStage(unit);
  const errorText =
    translationFailureSummary(stage, copy) ||
    stage?.safeError?.summary ||
    stage?.error ||
    unit.child?.lastError ||
    unit.job.lastError ||
    "";
  const advance = unitAdvanceAction(unit);

  if (sourceChangedRequiresRebuild(unit)) {
    return (
      <div className="pl-hintcard blockc">
        <span>◈ {copy.sourceChangedBody}</span>
      </div>
    );
  }

  if (unit.status === "waiting_for_approval" || pendingGates(unit, copy).length > 0) {
    return (
      <div className="pl-hintcard">
        <span>{copy.waitingHintPrefix}{stage ? gateLabel(stage.stageId, copy) : copy.statusWaiting}。</span>
        <span className="pl-spacer" />
        <button className="pl-btn sm primary" type="button" onClick={onGoApproval}>
          {copy.goApprovalTab}
        </button>
      </div>
    );
  }

  if (unit.status === "failed" || unit.status === "partial") {
    return (
      <div className="pl-hintcard errc">
        <span>✗ {errorText ? `${errorText}。` : ""}{copy.failedHintRetryable}</span>
        <span className="pl-spacer" />
        <button className="pl-btn sm" type="button" disabled={busy === "retry"} onClick={() => onRetry(unit.job.id)}>
          {copy.retryJob}
        </button>
      </div>
    );
  }

  if (unit.status === "blocked") {
    // The three choices the wizard's preflight step already offers, now usable
    // after the fact instead of only before queueing.
    const route = unitRoute(unit);
    const childId = advance?.childId ?? unit.child?.id ?? "";
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
        {route && (
          <>
            <button
              className="pl-btn sm"
              type="button"
              disabled={busy === "routeOverride"}
              onClick={() => onRouteOverride(unit.job.id, childId, route.id, "mineru")}
            >
              {copy.blockedKeepMineru}
            </button>
            <button
              className="pl-btn sm"
              type="button"
              disabled={busy === "routeOverride"}
              onClick={() => onRouteOverride(unit.job.id, childId, route.id, "paddle")}
            >
              {copy.blockedForcePaddle}
            </button>
            <button
              className="pl-btn sm quiet"
              type="button"
              disabled={busy === "routeOverride"}
              onClick={() => onRouteOverride(unit.job.id, childId, route.id, "auto")}
            >
              {copy.blockedDefer}
            </button>
          </>
        )}
      </div>
    );
  }

  if (advance) {
    const label = advance.stageId === "translate" ? copy.runTranslation : copy.continueStage;
    return (
      <div className="pl-hintcard">
        <span>
          {stageLabel(advance.stageId, copy)} · {statusLabel(advance.stageStatus, copy)}
        </span>
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

  // conversion_only jobs stop after extraction; the manual translation handoff
  // remains the only way to push them into a local reading project.
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

  return null;
}

function EvidenceCard({ unit, copy }: { unit: BookUnit; copy: PipelineCopy }) {
  const route = unitRoute(unit);
  const source = unit.child?.source ?? unit.job.source;
  const fingerprint = allArtifacts(unit).find((artifact) => artifact.sourceRefs?.sourceRefSha256)?.sourceRefs
    ?.sourceRefSha256;
  return (
    <div className="pl-card">
      <h4 className="pl-card-title">{copy.evidenceTitle}</h4>
      <div className="pl-evi-row">
        <span className="pl-k">{copy.evTextLayer}</span>
        <span className="pl-v">{route?.blockedReason || route?.summary || "—"}</span>
      </div>
      <div className="pl-evi-row">
        <span className="pl-k">{copy.evRoute}</span>
        <span className="pl-v">{route ? routeKindLabel(route.routeKind, copy) : "—"}</span>
      </div>
      <div className="pl-evi-row">
        <span className="pl-k">{copy.evSourceKind}</span>
        <span className="pl-v">{source.kind.replaceAll("_", " ")}</span>
      </div>
      <div className="pl-evi-row">
        <span className="pl-k">{copy.evFingerprint}</span>
        <span className="pl-v">
          <code className="pl-mono">{hashShort(fingerprint)}</code>
        </span>
      </div>
    </div>
  );
}

function ArtifactDigestCard({ unit, copy }: { unit: BookUnit; copy: PipelineCopy }) {
  const artifacts = allArtifacts(unit);
  const kinds = new Set(artifacts.map((artifact) => artifact.kind));
  const stage = currentStage(unit);
  const extraction = kinds.has("extraction_markdown") || kinds.has("markdown")
    ? copy.artifactPresent
    : stage?.stageId === "extract" && stage.status === "running"
      ? copy.artifactGenerating
      : "—";
  const zoteroChild = artifacts.some(
    (artifact) => artifact.zoteroKey || artifact.sourceRefs?.markdownAttachmentKey,
  )
    ? copy.artifactAttached
    : "—";
  const sourceMap = kinds.has("source_map") ? copy.artifactPresent : "—";
  const readingKinds = artifacts.filter((artifact) => artifact.kind.startsWith("reading_"));
  const reading = readingKinds.length
    ? `${readingKinds.map((artifact) => artifact.kind.replace("reading_", "").toUpperCase()).join(" / ")} ✓`
    : "—";
  return (
    <div className="pl-card">
      <h4 className="pl-card-title">{copy.artifactDigestTitle}</h4>
      <div className="pl-evi-row">
        <span className="pl-k">{copy.artifactExtractionMarkdown}</span>
        <span className="pl-v">{extraction}</span>
      </div>
      <div className="pl-evi-row">
        <span className="pl-k">{copy.artifactZoteroAttachment}</span>
        <span className="pl-v">{zoteroChild}</span>
      </div>
      <div className="pl-evi-row">
        <span className="pl-k">{copy.artifactSourceMap}</span>
        <span className="pl-v">{sourceMap}</span>
      </div>
      <div className="pl-evi-row">
        <span className="pl-k">{copy.artifactReading}</span>
        <span className="pl-v">{reading}</span>
      </div>
    </div>
  );
}

export function OverviewTab(props: TabProps) {
  const { unit, copy, busy, onSampleOcr, onRouteOverride } = props;
  return (
    <>
      <StageRail unit={unit} copy={copy} />
      <ActionCard {...props} />
      {/* Only while the engine is still an open question — the backend refuses
          a sample once extraction has started, so an always-visible card would
          offer a button that errors. */}
      {canCompareOcrEngines(unit) && (
        <OcrCompareCard
          unit={unit}
          copy={copy}
          busy={busy}
          onSampleOcr={onSampleOcr}
          onRouteOverride={onRouteOverride}
        />
      )}
      <div className="pl-det-cards">
        <EvidenceCard unit={unit} copy={copy} />
        <ArtifactDigestCard unit={unit} copy={copy} />
      </div>
    </>
  );
}
