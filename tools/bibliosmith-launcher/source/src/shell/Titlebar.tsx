import { CheckCircle2, ChevronDown, Globe2, Power } from "lucide-react";
import type { Copy } from "../i18n";
import { LogoMark } from "./LogoMark";
import "./shell.css";

export function Titlebar({
  copy,
  version,
  proxyConfigured,
  autoStart,
  projectReady,
  projectStatusValue,
  quickActionsOpen,
  onToggleQuickActions,
  onSelectRepo,
  onOpenRepo,
  onOpenBooks,
}: {
  copy: Copy;
  version: string;
  proxyConfigured: boolean;
  autoStart: boolean;
  projectReady: boolean;
  projectStatusValue: string;
  quickActionsOpen: boolean;
  onToggleQuickActions: () => void;
  onSelectRepo: () => void;
  onOpenRepo: () => void;
  onOpenBooks: () => void;
}) {
  return (
    <header className="sh-toolbar">
      <div className="sh-toolbar-brand" data-tauri-drag-region>
        <LogoMark />
        <span>BiblioSmith Launcher</span>
        <span className="sh-toolbar-version">{version}</span>
      </div>
      <div className="sh-toolbar-drag" data-tauri-drag-region />
      <div className="sh-toolbar-status">
        <span className={`sh-chip ${projectReady ? "ok" : "info"}`} title={copy.projectStatus}>
          <CheckCircle2 size={12} />
          {projectReady ? copy.running : projectStatusValue}
        </span>
        <span className={`sh-chip ${proxyConfigured ? "info" : "ok"}`} title={copy.networkProxy}>
          <Globe2 size={12} />
          {proxyConfigured ? copy.proxied : copy.direct}
        </span>
        <span className={`sh-chip ${autoStart ? "ok" : ""}`} title={copy.startup}>
          <Power size={12} />
          {autoStart ? copy.enabled : copy.disabled}
        </span>
      </div>
      <div className="sh-quick-wrap">
        <button className="sh-quick-btn" type="button" onClick={onToggleQuickActions} aria-expanded={quickActionsOpen}>
          {copy.quickActions}
          <ChevronDown size={14} />
        </button>
        {quickActionsOpen && (
          <div className="sh-quick-menu">
            <button type="button" onClick={onSelectRepo}>{copy.selectRepo}</button>
            <button type="button" onClick={onOpenRepo}>{copy.viewProject}</button>
            <button type="button" disabled={!projectReady} onClick={onOpenBooks}>{copy.openBooks}</button>
          </div>
        )}
      </div>
    </header>
  );
}
