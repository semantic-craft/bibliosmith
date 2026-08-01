import type { PipelineCopy } from "./copy";
import {
  currentStage,
  operationPhaseLabel,
  operationUnitLabel,
  unitOperationProgress,
  type BookUnit,
} from "./model";

export function OperationProgressBar({
  unit,
  copy,
  compact = false,
}: {
  unit: BookUnit;
  copy: PipelineCopy;
  compact?: boolean;
}) {
  const stage = currentStage(unit);
  if (stage?.status !== "running") return null;
  const operation = unitOperationProgress(unit);
  const phase = operation
    ? operationPhaseLabel(operation.phase, copy)
    : stage.stageId === "translate"
      ? copy.progressTranslating
      : copy.progressWorking;
  const total = operation?.total && operation.total > 0 ? operation.total : null;
  const completed = total ? Math.min(operation?.completed ?? 0, total) : 0;
  const count = total && operation
    ? copy.progressCount(completed, total, operationUnitLabel(operation.unitKind, copy))
    : null;
  const percent = total ? Math.round((completed / total) * 100) : null;
  const ariaLabel = copy.progressAria(phase, count ?? undefined);

  return (
    <div className={`pl-live-progress${compact ? " compact" : ""}`}>
      <div className="pl-live-head">
        <span className="pl-live-phase"><i aria-hidden="true" />{phase}</span>
        {count && <span className="pl-live-count">{count}</span>}
      </div>
      <div
        className={`pl-prog pl-live-bar${percent === null ? " indeterminate" : ""}`}
        role="progressbar"
        aria-label={ariaLabel}
        aria-valuemin={percent === null ? undefined : 0}
        aria-valuemax={total ?? undefined}
        aria-valuenow={total ? completed : undefined}
      >
        <i style={percent === null ? undefined : { width: `${percent}%` }} />
      </div>
      {!compact && <div className="pl-live-note">{copy.progressHeartbeat}</div>}
    </div>
  );
}
