import { useCallback, useState } from "react";
import type { CommitInfo } from "../types";
import type { Copy } from "../i18n";
import { PanelHeading } from "./PanelHeading";
import { RowIcon } from "./RowIcon";

export function CommitTable({
  copy,
  commits,
  totalCount,
  latestVersion,
  showAll,
  emptyMessage,
  onToggleShowAll,
}: {
  copy: Copy;
  commits: CommitInfo[];
  totalCount: number;
  latestVersion: string;
  showAll: boolean;
  emptyMessage?: string;
  onToggleShowAll: () => void;
}) {
  const [hoverTooltip, setHoverTooltip] = useState<{ text: string; left: number; top: number } | null>(null);

  const showTooltip = useCallback((target: HTMLElement, text: string) => {
    const rect = target.getBoundingClientRect();
    const margin = 18;
    const tooltipWidth = Math.min(720, Math.max(320, window.innerWidth - margin * 2));
    const left = Math.min(
      Math.max(rect.left, margin),
      Math.max(margin, window.innerWidth - tooltipWidth - margin),
    );
    const belowTop = rect.bottom + 10;
    const top = belowTop < window.innerHeight - 120 ? belowTop : Math.max(margin, rect.top - 220);
    setHoverTooltip({ text, left, top });
  }, []);

  const hideTooltip = useCallback(() => {
    setHoverTooltip(null);
  }, []);

  return (
    <section className={showAll ? "data-panel commit-panel expanded" : "data-panel commit-panel"}>
      <div className="panel-title-row">
        <PanelHeading title={copy.updateContent} />
        <div className="panel-actions">
          <span>{copy.updateTo} {latestVersion}</span>
          {totalCount > 1 && (
            <button type="button" onClick={onToggleShowAll}>
              {showAll ? copy.showLatestOnly : copy.viewAllUpdates}
            </button>
          )}
        </div>
      </div>
      <div className="table-wrap commit-table-wrap">
        {commits.length ? (
          <table className="data-table commit-table">
            <thead>
              <tr>
                <th>{copy.date}</th>
                <th>{copy.commit}</th>
                <th>{copy.title}</th>
                <th>{copy.summary}</th>
              </tr>
            </thead>
            <tbody>
              {commits.map((commit, index) => {
                const tooltipText = formatCommitTooltip(copy, commit);
                return (
                  <tr key={`${commit.hash}-${commit.date}`}>
                    <td><RowIcon index={index} />{commit.date.slice(0, 16).replace("T", " ")}</td>
                    <td><code>{commit.hash}</code></td>
                    <td>{commit.title}</td>
                    <td
                      className="commit-summary-cell"
                      data-tooltip={tooltipText}
                      aria-label={tooltipText}
                      onMouseEnter={(event) => showTooltip(event.currentTarget, tooltipText)}
                      onMouseLeave={hideTooltip}
                      onFocus={(event) => showTooltip(event.currentTarget, tooltipText)}
                      onBlur={hideTooltip}
                      tabIndex={0}
                    >
                      {commit.summary || copy.noCommits}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        ) : (
          <div className="table-empty">{emptyMessage ?? copy.noCommits}</div>
        )}
      </div>
      {hoverTooltip && (
        <div
          className="commit-hover-tooltip"
          role="tooltip"
          style={{ left: hoverTooltip.left, top: hoverTooltip.top }}
        >
          {hoverTooltip.text}
        </div>
      )}
    </section>
  );
}

function formatCommitTooltip(copy: Copy, commit: CommitInfo) {
  const date = commit.date.slice(0, 16).replace("T", " ");
  const details = commit.fullMessage?.trim() || commit.summary || copy.noCommits;
  return [
    `${copy.date}: ${date}`,
    `${copy.commit}: ${commit.hash}`,
    `${copy.title}: ${commit.title}`,
    "",
    details,
  ].join("\n");
}
