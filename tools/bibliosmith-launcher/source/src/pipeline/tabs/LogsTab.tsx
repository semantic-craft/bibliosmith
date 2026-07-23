import type { TabProps } from "./tabProps";

export function LogsTab({ unit, copy }: TabProps) {
  const lines = [...unit.job.logSummary].reverse();
  const errors = [unit.child?.lastError, unit.job.lastError].filter(
    (value, index, array): value is string => Boolean(value) && array.indexOf(value) === index,
  );
  return (
    <div>
      {errors.map((error) => (
        <div key={error} className="pl-feed-line err">
          <span className="pl-fm">{error}</span>
        </div>
      ))}
      {lines.map((line, index) => (
        <div key={`${index}-${line}`} className="pl-feed-line">
          <span className="pl-fm">{line}</span>
        </div>
      ))}
      {!lines.length && !errors.length && <p className="pl-muted-note">{copy.logsEmpty}</p>}
      <p className="pl-log-note">{copy.logRedactionNote}</p>
    </div>
  );
}
