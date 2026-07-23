import type { DownloadProgress } from "../types";
import type { Copy } from "../i18n";
import { formatPercent, progressWidth } from "../lib/format";
import { LogoMark } from "./LogoMark";
import type { RuntimeBootstrapState } from "./types";

export function RuntimeBootstrapScreen({
  copy,
  state,
  progress,
  message,
  onRetry,
  onContinue,
}: {
  copy: Copy;
  state: RuntimeBootstrapState;
  progress: DownloadProgress | null;
  message: string;
  onRetry: () => void;
  onContinue: () => void;
}) {
  const percent = progress?.percent ?? (state === "checking" ? 0.01 : state === "ready" ? 100 : 1);
  const isFailed = state === "failed";
  return (
    <div className="runtime-bootstrap-shell">
      <section className={`runtime-bootstrap-card ${state}`}>
        <LogoMark large />
        <div>
          <p className="runtime-bootstrap-kicker">BiblioSmith Launcher</p>
          <h1>{copy.runtimeBootstrapTitle}</h1>
          <p className="runtime-bootstrap-description">{copy.runtimeBootstrapDescription}</p>
        </div>
        <div className="runtime-bootstrap-progress">
          <div className="floating-progress-header">
            <strong>
              {state === "checking"
                ? copy.runtimeBootstrapChecking
                : state === "ready"
                  ? copy.runtimeBootstrapReady
                  : isFailed
                    ? copy.runtimeBootstrapFailed
                    : copy.runtimeBootstrapPreparing}
            </strong>
            <span>{formatPercent(percent)}</span>
          </div>
          <div className="progress-bar">
            <span style={{ width: progressWidth(percent) }} />
          </div>
          <div className="runtime-bootstrap-message">{message}</div>
        </div>
        {state !== "ready" && (
          <div className="runtime-bootstrap-actions">
            <button type="button" onClick={onContinue}>{copy.runtimeBootstrapContinue}</button>
            {isFailed && (
              <button type="button" className="primary" onClick={onRetry}>{copy.runtimeBootstrapRetry}</button>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
