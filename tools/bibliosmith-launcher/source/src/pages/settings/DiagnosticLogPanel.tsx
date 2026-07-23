import { Download } from "lucide-react";
import type { DiagnosticLogSettings } from "../../types";
import type { Copy } from "../../i18n";
import { formatBytes } from "../../lib/format";
import { SettingToggle } from "../../components";

export function DiagnosticLogPanel({
  copy,
  settings,
  enabled,
  onToggle,
  onExport,
}: {
  copy: Copy;
  settings: DiagnosticLogSettings | null;
  enabled: boolean;
  onToggle: (value: boolean) => void;
  onExport: () => void;
}) {
  const maxSize = formatBytes(settings?.maxTotalBytes ?? 24 * 1024 * 1024);
  const logPath = settings?.logFile || "";
  return (
    <>
      <SettingToggle
        title={copy.saveLogsTitle}
        description={copy.saveLogsDescription(maxSize)}
        checked={enabled}
        onChange={onToggle}
      />
      <div className="st-row">
        <div className="st-row-copy">
          <strong>{copy.exportLogs}</strong>
          <span>{copy.exportLogsDescription}</span>
          {logPath && <code title={logPath}>{logPath}</code>}
        </div>
        <button className="st-btn" type="button" onClick={onExport}>
          <Download size={14} />
          {copy.exportLogs}
        </button>
      </div>
    </>
  );
}
