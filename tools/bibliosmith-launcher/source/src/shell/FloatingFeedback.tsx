import type { DownloadProgress } from "../types";
import type { Copy } from "../i18n";
import { formatPercent, progressWidth } from "../lib/format";
import type { DownloadHudState, FloatingToast } from "./types";

export function FloatingFeedback({
  toast,
  globalProgress,
  biblioSmithVisible,
  biblioSmithTitle,
  biblioSmithState,
  biblioSmithProgress,
  biblioSmithMessage,
  copy,
  onStopBiblioSmith,
  onCancelBiblioSmith,
  onRetryBiblioSmith,
  onCloseBiblioSmith,
}: {
  toast: FloatingToast | null;
  globalProgress: { percent: number; label: string } | null;
  biblioSmithVisible: boolean;
  biblioSmithTitle: string;
  biblioSmithState: DownloadHudState;
  biblioSmithProgress?: DownloadProgress | null;
  biblioSmithMessage: string;
  copy: Copy;
  onStopBiblioSmith: () => void;
  onCancelBiblioSmith: () => void;
  onRetryBiblioSmith: () => void;
  onCloseBiblioSmith: () => void;
}) {
  if (!toast && !globalProgress && !biblioSmithVisible) return null;
  const biblioSmithPercent = biblioSmithProgress?.percent ?? 0;
  const biblioSmithRunning = biblioSmithState === "downloading" || biblioSmithState === "cancelling";
  return (
    <div className="floating-feedback-layer" aria-live="polite">
      {toast && <div className={`floating-toast ${toast.tone}`}>{toast.message}</div>}
      {globalProgress && (
        <section className="floating-progress-card blue">
          <div className="floating-progress-header">
            <strong>{globalProgress.label}</strong>
            <span>{formatPercent(globalProgress.percent)}</span>
          </div>
          <div className="progress-bar">
            <span style={{ width: progressWidth(globalProgress.percent) }} />
          </div>
        </section>
      )}
      {biblioSmithVisible && (
        <TaskProgressCard
          accent="blue"
          title={biblioSmithTitle}
          state={biblioSmithState}
          percent={biblioSmithPercent}
          message={biblioSmithState === "cancelling" ? copy.working : biblioSmithMessage}
          running={biblioSmithRunning}
          copy={copy}
          onStop={onStopBiblioSmith}
          onCancel={onCancelBiblioSmith}
          onRetry={onRetryBiblioSmith}
          onClose={onCloseBiblioSmith}
        />
      )}
    </div>
  );
}

function TaskProgressCard({
  accent,
  title,
  state,
  percent,
  message,
  running,
  copy,
  onStop,
  onCancel,
  onRetry,
  onClose,
}: {
  accent: "blue" | "green";
  title: string;
  state: DownloadHudState;
  percent: number;
  message: string;
  running: boolean;
  copy: Copy;
  onStop: () => void;
  onCancel: () => void;
  onRetry: () => void;
  onClose: () => void;
}) {
  return (
    <section className={`floating-progress-card ${accent} ${state}`}>
      <div className="floating-progress-header">
        <strong>{title}</strong>
        <span>{formatPercent(percent)}</span>
      </div>
      <div className="progress-bar">
        <span style={{ width: progressWidth(percent) }} />
      </div>
      <div className="floating-progress-footer">
        <span>{message}</span>
        <div className="floating-progress-actions">
          {running ? (
            <>
              <button type="button" onClick={onStop} disabled={state === "cancelling"}>{copy.stopDownload}</button>
              <button type="button" onClick={onCancel} disabled={state === "cancelling"}>{copy.cancelDownload}</button>
            </>
          ) : (
            <>
              <button type="button" onClick={onRetry}>{copy.retry}</button>
              <button type="button" onClick={onClose}>{copy.close}</button>
            </>
          )}
        </div>
      </div>
    </section>
  );
}
