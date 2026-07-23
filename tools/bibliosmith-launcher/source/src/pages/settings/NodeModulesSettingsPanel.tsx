import type { DownloadProgress, NodeModulesStatus } from "../../types";
import type { Copy } from "../../i18n";
import { formatPercent, progressWidth } from "../../lib/format";
import type { DownloadHudState } from "../../shell";

export function NodeModulesSettingsPanel({
  copy,
  status,
  progress,
  state,
  message,
  onToggle,
  onStop,
  onCancel,
}: {
  copy: Copy;
  status: NodeModulesStatus | null;
  progress: DownloadProgress | null;
  state: DownloadHudState;
  message: string;
  onToggle: (value: boolean) => void;
  onStop: () => void;
  onCancel: () => void;
}) {
  const running = state === "downloading" || state === "cancelling" || Boolean(status?.running);
  const ready = Boolean(status?.ready);
  const autoInstall = status?.autoInstall ?? true;
  const failedOrStopped = state === "failed" || state === "stopped";
  const statusText = !autoInstall
    ? copy.nodeModulesDisabled
    : failedOrStopped
      ? copy.nodeModulesRetryHint
      : running
    ? copy.nodeModulesInstalling
    : ready
      ? copy.nodeModulesReady
      : status?.repoReady
        ? copy.nodeModulesMissing
        : copy.nodeModulesNotReady;
  const percent = progress?.percent ?? 0;
  const showProgress = running || state === "failed" || state === "stopped";
  return (
    <>
      <label className="st-row">
        <div className="st-row-copy">
          <div className="st-title-line">
            <strong>{copy.nodeModulesAutoInstallTitle}</strong>
            <span className={`st-pill ${ready ? "success" : failedOrStopped ? "error" : running ? "working" : "muted"}`}>
              {statusText}
            </span>
          </div>
          <span>{copy.nodeModulesAutoInstallDescription}</span>
        </div>
        <span className="st-switch">
          <input
            type="checkbox"
            role="switch"
            aria-checked={autoInstall}
            checked={autoInstall}
            onChange={(event) => onToggle(event.target.checked)}
          />
        </span>
      </label>
      {showProgress && (
        <div className="st-block">
          <div className="st-progress">
            <div className="floating-progress-header">
              <strong>{copy.nodeModulesInstalling}</strong>
              <span>{formatPercent(percent)}</span>
            </div>
            <div className="progress-bar">
              <span style={{ width: progressWidth(percent) }} />
            </div>
            <div className="st-progress-detail">{message}</div>
            {running && (
              <div className="st-progress-actions">
                <button className="st-btn" type="button" onClick={(event) => { event.preventDefault(); onStop(); }} disabled={state === "cancelling"}>{copy.stopDownload}</button>
                <button className="st-btn" type="button" onClick={(event) => { event.preventDefault(); onCancel(); }} disabled={state === "cancelling"}>{copy.cancelDownload}</button>
              </div>
            )}
          </div>
        </div>
      )}
    </>
  );
}
