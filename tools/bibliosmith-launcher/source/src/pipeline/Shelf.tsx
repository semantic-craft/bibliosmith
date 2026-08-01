import type { PipelineCopy } from "./copy";
import { stepSummaryCaption, type BookUnit } from "./model";
import { OperationProgressBar } from "./OperationProgress";

function coverHue(title: string): number {
  let hash = 0;
  for (let index = 0; index < title.length; index += 1) {
    hash = (hash * 31 + title.charCodeAt(index)) % 360;
  }
  return hash;
}

export function BookCover({ title, className }: { title: string; className?: string }) {
  const hue = coverHue(title);
  return (
    <div
      className={className ? `pl-cover ${className}` : "pl-cover"}
      style={{ background: `linear-gradient(155deg, hsl(${hue} 38% 46%), hsl(${hue} 44% 30%))` }}
    >
      <span>{title}</span>
    </div>
  );
}

function ribbonFor(unit: BookUnit, copy: PipelineCopy): { label: string; cls: string } | null {
  if (unit.status === "waiting_for_approval") return { label: copy.ribbonWaiting, cls: "wait" };
  if (unit.status === "failed" || unit.status === "partial" || unit.status === "blocked") {
    return { label: copy.ribbonProblem, cls: "bad" };
  }
  if (unit.status === "completed") return { label: copy.ribbonDone, cls: "done" };
  return null;
}

export function Shelf({
  copy,
  units,
  selectedKey,
  onSelect,
  dimmed,
}: {
  copy: PipelineCopy;
  units: BookUnit[];
  selectedKey: string | null;
  onSelect: (key: string) => void;
  dimmed?: boolean;
}) {
  return (
    <div className={dimmed ? "pl-shelf dimmed" : "pl-shelf"}>
      {units.map((unit) => {
        const ribbon = ribbonFor(unit, copy);
        return (
          <div
            key={unit.key}
            className={`pl-bookcard${unit.key === selectedKey ? " sel" : ""}`}
            role="button"
            tabIndex={0}
            onClick={() => onSelect(unit.key)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect(unit.key);
              }
            }}
          >
            <BookCover title={unit.title} className="shelf" />
            {ribbon && <span className={`pl-ribbon ${ribbon.cls}`}>{ribbon.label}</span>}
            <div className="pl-bmeta">
              <div className="pl-bt">{unit.title}</div>
              <div className="pl-bs">{stepSummaryCaption(unit, copy)}</div>
            </div>
            {unit.status === "running" ? (
              <OperationProgressBar unit={unit} copy={copy} compact />
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
