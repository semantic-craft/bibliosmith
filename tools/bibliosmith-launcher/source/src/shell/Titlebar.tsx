import { Settings } from "lucide-react";
import { LogoMark } from "./LogoMark";
import "./shell.css";

export function Titlebar({
  version,
  settingsLabel,
  settingsActive,
  updateBadgeLabel,
  onToggleSettings,
}: {
  version: string;
  settingsLabel: string;
  settingsActive: boolean;
  /**
   * Set while an update is waiting in Settings, and used as the dot's
   * accessible name so the badge carries the same sentence sighted users get
   * from the toast rather than being a decorative mark only they can see.
   */
  updateBadgeLabel: string | null;
  onToggleSettings: () => void;
}) {
  return (
    <header className="sh-toolbar">
      <div className="sh-toolbar-brand" data-tauri-drag-region>
        <LogoMark />
        <span>BiblioSmith</span>
        <span className="sh-toolbar-version">{version}</span>
      </div>
      <div className="sh-toolbar-drag" data-tauri-drag-region />
      <button
        className={`sh-settings-btn${settingsActive ? " active" : ""}`}
        type="button"
        title={updateBadgeLabel ? `${settingsLabel} · ${updateBadgeLabel}` : settingsLabel}
        aria-label={settingsLabel}
        aria-pressed={settingsActive}
        onClick={onToggleSettings}
      >
        <Settings size={16} />
        {updateBadgeLabel && <span className="sh-settings-badge" role="status">{updateBadgeLabel}</span>}
      </button>
    </header>
  );
}
