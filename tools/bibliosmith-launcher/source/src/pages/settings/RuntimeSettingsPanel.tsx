import { RefreshCcw } from "lucide-react";
import type { RuntimeStatus } from "../../types";
import type { Copy } from "../../i18n";

export function RuntimeSettingsPanel({
  copy,
  status,
  onRetry,
}: {
  copy: Copy;
  status: RuntimeStatus | null;
  onRetry: () => void;
}) {
  const ready = Boolean(status?.ready);
  const running = Boolean(status?.running);
  const text = ready ? copy.runtimeStatusReady : copy.runtimeStatusMissing;
  return (
    <div className="st-row">
      <div className="st-row-copy">
        <div className="st-title-line">
          <strong>{copy.runtimeStatusTitle}</strong>
          <span className={`st-pill ${ready ? "success" : running ? "working" : "error"}`}>
            {text}
          </span>
        </div>
        <span>{copy.runtimeStatusDescription}</span>
        <div className="st-code-lines">
          <code>Python {status?.python.version ?? "3.12"}: {status?.python.path || status?.python.message || "-"}</code>
          <code>Java {status?.java.version ?? "17"}: {status?.java.path || status?.java.message || "-"}</code>
        </div>
      </div>
      {!ready && (
        <button className="st-btn" type="button" onClick={onRetry} disabled={running}>
          <RefreshCcw size={14} />
          {copy.runtimeBootstrapRetry}
        </button>
      )}
    </div>
  );
}
