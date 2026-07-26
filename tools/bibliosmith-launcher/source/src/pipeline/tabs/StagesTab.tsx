import type { PipelineCopy } from "../copy";
import {
  GATE_STAGE_IDS,
  currentStage,
  focusStages,
  formatTime,
  stageLabel,
  type BookUnit,
} from "../model";
import type { TabProps } from "./tabProps";
import type { BookPipelineStage as BookStage } from "../../types";

function rowClass(status: string): string {
  switch (status) {
    case "completed":
      return "past";
    case "skipped":
      return "skipn";
    case "running":
      return "now";
    case "waiting_for_approval":
      return "gatewait";
    case "failed":
      return "failnode";
    case "blocked":
      return "blocknode";
    default:
      return "";
  }
}

// "retryable" used to be printed on every failure with nothing behind it. It is
// now whatever the runner's own retry budget says: a pending automatic retry, a
// remaining count, or the reason it stopped trying.
function retryMeta(stage: BookStage, copy: PipelineCopy): string {
  if (stage.giveUpReason) return copy.stageGaveUp(stage.giveUpReason);
  if (stage.nextRetryAt) return copy.stageRetryScheduled(stage.nextRetryAt);
  const remaining = Math.max(0, (stage.maxAttempts || 0) - stage.attempt);
  return remaining > 0 ? copy.stageRetriesLeft(remaining) : copy.stageRetryable;
}

function stageMeta(stage: BookStage, copy: PipelineCopy): { text: string; color?: string } {
  const { status, attempt } = stage;
  switch (status) {
    case "completed":
      return { text: `${copy.statusCompleted} · ${copy.attemptLabel(attempt)}` };
    case "skipped":
      return { text: copy.stageSkippedMeta };
    case "running":
      return { text: copy.stageRunningMeta, color: "var(--pl-running)" };
    case "waiting_for_approval":
      return { text: copy.stageWaitingYou, color: "var(--pl-approval)" };
    case "failed":
      return {
        text: `${copy.statusFailed} · ${copy.attemptLabel(attempt)} · ${retryMeta(stage, copy)}`,
        color: "var(--pl-failed)",
      };
    case "blocked":
      return { text: copy.stageBlockedMeta, color: "var(--pl-blocked)" };
    case "ready":
      return { text: copy.statusReady };
    default:
      return { text: copy.statusQueued };
  }
}

function StageEvidence({ unit, copy }: { unit: BookUnit; copy: PipelineCopy }) {
  const stage = currentStage(unit);
  if (!stage || stage.status === "completed" || stage.status === "skipped") return null;
  const errorText = stage.safeError?.summary || stage.error;
  const rows: { key: string; value: string }[] = [];
  if (errorText) rows.push({ key: copy.stageErrorLabel, value: errorText });
  if (stage.unitSummary && stage.unitSummary.total > 0) {
    rows.push({
      key: copy.stageUnitsLabel,
      value: `${stage.unitSummary.completed}/${stage.unitSummary.total}`,
    });
  }
  if (stage.artifactIds.length) rows.push({ key: copy.stageArtifactsLabel, value: String(stage.artifactIds.length) });
  if (Object.keys(stage.inputHashes).length) {
    rows.push({ key: copy.stageInputsLabel, value: String(Object.keys(stage.inputHashes).length) });
  }
  if (stage.startedAt) rows.push({ key: copy.stageStartedLabel, value: formatTime(stage.startedAt) });
  if (stage.finishedAt) rows.push({ key: copy.stageFinishedLabel, value: formatTime(stage.finishedAt) });
  if (!rows.length) return null;
  return (
    <div className="pl-card pl-vtl-open">
      {rows.map((row) => (
        <div className="pl-evi-row" key={row.key}>
          <span className="pl-k">{row.key}</span>
          <span className="pl-v">{row.value}</span>
        </div>
      ))}
    </div>
  );
}

export function StagesTab({ unit, copy }: TabProps) {
  const stages = focusStages(unit);
  const active = currentStage(unit);
  return (
    <div className="pl-vtl">
      {stages.map((stage) => {
        const cls = rowClass(stage.status);
        const meta = stageMeta(stage, copy);
        const isCurrent = active?.stageId === stage.stageId;
        return (
          <div
            key={stage.stageId}
            className={`pl-vtl-row ${cls}${GATE_STAGE_IDS.has(stage.stageId) ? " gate" : ""}`}
          >
            <div className="pl-vtl-node">
              <span>{cls === "past" ? "✓" : cls === "skipn" ? "–" : ""}</span>
            </div>
            <div className="pl-vtl-body">
              <div className="pl-vh">
                {stageLabel(stage.stageId, copy)}
                <span className="pl-vmeta" style={meta.color ? { color: meta.color } : undefined}>
                  {meta.text}
                </span>
              </div>
              {isCurrent && <StageEvidence unit={unit} copy={copy} />}
            </div>
          </div>
        );
      })}
    </div>
  );
}
