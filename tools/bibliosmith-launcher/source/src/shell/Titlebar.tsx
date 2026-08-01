import { Settings } from "lucide-react";
import { LogoMark } from "./LogoMark";
import "./shell.css";

export function Titlebar({
  version,
  settingsLabel,
  settingsActive,
  onToggleSettings,
}: {
  version: string;
  settingsLabel: string;
  settingsActive: boolean;
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
        title={settingsLabel}
        aria-label={settingsLabel}
        aria-pressed={settingsActive}
        onClick={onToggleSettings}
      >
        <Settings size={16} />
      </button>
    </header>
  );
}
