import { BookOpen, FileText, FolderOpen, Home, RefreshCcw, Settings } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { Copy } from "../i18n";
import { LogoMark } from "./LogoMark";
import type { TabId } from "./types";

export function Sidebar({
  copy,
  pipelineNavLabel,
  version,
  activeTab,
  pipelineLoading,
  updateAvailable,
  onSelectTab,
}: {
  copy: Copy;
  pipelineNavLabel: string;
  version: string;
  activeTab: TabId;
  pipelineLoading: boolean;
  updateAvailable: boolean;
  onSelectTab: (tab: TabId) => void;
}) {
  return (
    <aside className="sh-sidebar">
      <div className="sh-side-brand">
        <LogoMark />
        <span>BiblioSmith</span>
      </div>

      <nav className="sh-nav">
        <NavButton icon={Home} label={copy.overview} active={activeTab === "overview"} onClick={() => onSelectTab("overview")} />
        <NavButton icon={RefreshCcw} label={copy.updates} active={activeTab === "updates"} badge={updateAvailable} onClick={() => onSelectTab("updates")} />
        <NavButton icon={FolderOpen} label={pipelineNavLabel} active={activeTab === "pipeline"} working={pipelineLoading} onClick={() => onSelectTab("pipeline")} />
        <NavButton icon={BookOpen} label={copy.tutorial} active={activeTab === "tutorial"} onClick={() => onSelectTab("tutorial")} />
        <NavButton icon={Settings} label={copy.settings} active={activeTab === "settings"} onClick={() => onSelectTab("settings")} />
        <NavButton icon={FileText} label={copy.logs} active={activeTab === "logs"} onClick={() => onSelectTab("logs")} />
      </nav>

      <div className="sh-side-footer">
        <strong>{version}</strong>
        <span>{copy.mission}</span>
      </div>
    </aside>
  );
}

function NavButton({ icon: Icon, label, active, working, badge, onClick }: { icon: LucideIcon; label: string; active: boolean; working?: boolean; badge?: boolean; onClick: () => void }) {
  return (
    <button className={active ? "sh-nav-item active" : "sh-nav-item"} onClick={onClick}>
      <Icon size={16} strokeWidth={1.8} className={working ? "spin-icon" : undefined} />
      <span>{label}</span>
      {badge && <span className="sh-nav-dot" aria-hidden="true" />}
    </button>
  );
}
