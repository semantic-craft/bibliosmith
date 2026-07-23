import { FolderOpen } from "lucide-react";
import type { Copy } from "../../i18n";

export function ProjectPathPanel({ copy, path, onChange }: { copy: Copy; path: string; onChange: () => void }) {
  return (
    <div className="st-row">
      <div className="st-row-copy">
        <strong>{copy.projectPath}</strong>
        <code title={path}>{path}</code>
      </div>
      <button className="st-btn" type="button" onClick={onChange}>
        <FolderOpen size={14} />
        {copy.changeProjectPath}
      </button>
    </div>
  );
}
