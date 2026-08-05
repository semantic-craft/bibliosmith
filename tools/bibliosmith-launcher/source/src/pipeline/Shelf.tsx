import type { PipelineCopy } from "./copy";
import { phaseSummaryCaption, type BookUnit } from "./model";
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
  manageMode = false,
  selectedKeys = new Set<string>(),
  disabledKeys = new Set<string>(),
  onToggle,
}: {
  copy: PipelineCopy;
  units: BookUnit[];
  selectedKey: string | null;
  onSelect: (key: string) => void;
  dimmed?: boolean;
  manageMode?: boolean;
  selectedKeys?: Set<string>;
  disabledKeys?: Set<string>;
  onToggle?: (key: string) => void;
}) {
  return (
    <div className={`pl-shelf${dimmed ? " dimmed" : ""}${manageMode ? " managing" : ""}`}>
      {units.map((unit) => {
        const ribbon = ribbonFor(unit, copy);
        const disabled = disabledKeys.has(unit.key);
        const selected = selectedKeys.has(unit.key);
        return (
          <div
            key={unit.key}
            className={`pl-bookcard${unit.key === selectedKey || selected ? " sel" : ""}${disabled ? " disabled" : ""}`}
            role={manageMode ? "checkbox" : "button"}
            aria-checked={manageMode ? selected : undefined}
            aria-disabled={manageMode && disabled ? true : undefined}
            aria-label={manageMode ? `${unit.title}${disabled ? ` · ${copy.runningBookCannotRemove}` : ""}` : undefined}
            tabIndex={manageMode && disabled ? -1 : 0}
            onClick={() => {
              if (manageMode) {
                if (!disabled) onToggle?.(unit.key);
              } else {
                onSelect(unit.key);
              }
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                if (manageMode) {
                  if (!disabled) onToggle?.(unit.key);
                } else {
                  onSelect(unit.key);
                }
              }
            }}
          >
            <BookCover title={unit.title} className="shelf" />
            {manageMode && <span className="pl-selectmark" aria-hidden="true">{selected ? "✓" : ""}</span>}
            {manageMode && disabled && <span className="pl-disabled-label">{copy.runningBookCannotRemove}</span>}
            {ribbon && <span className={`pl-ribbon ${ribbon.cls}`}>{ribbon.label}</span>}
            <div className="pl-bmeta">
              <div className="pl-bt">{unit.title}</div>
              <div className="pl-bs">{phaseSummaryCaption(unit, copy)}</div>
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
