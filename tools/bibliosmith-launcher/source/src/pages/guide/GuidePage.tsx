import { useMemo, type MouseEvent } from "react";
import { ChevronLeft } from "lucide-react";
import type { ProjectDocument } from "../../types";
import type { Copy } from "../../i18n";
import { copyTextToClipboard, decodeCodePayload, renderMarkdownToHtml } from "../../lib/markdown";
import type { TutorialKind } from "../../shell";
import "./guide.css";

export function GuidePage({
  copy,
  kind,
  document,
  loading,
  canGoBack,
  repoReady,
  unavailableTitle,
  unavailableDescription,
  unavailableHelp,
  recoverLabel,
  onRecover,
  onChangeProject,
  onSelect,
  onBack,
  onOpenLink,
}: {
  copy: Copy;
  kind: TutorialKind;
  document: ProjectDocument | null;
  loading: boolean;
  canGoBack: boolean;
  repoReady: boolean;
  unavailableTitle: string;
  unavailableDescription: string;
  unavailableHelp: string;
  recoverLabel: string;
  onRecover: () => void;
  onChangeProject: () => void;
  onSelect: (kind: TutorialKind) => void;
  onBack: () => void;
  onOpenLink: (href: string) => void;
}) {
  const html = useMemo(() => renderMarkdownToHtml(document?.content ?? "", copy), [copy, document]);
  const handleClick = async (event: MouseEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement | null;
    const copyButton = target?.closest<HTMLButtonElement>("button[data-copy-code]");
    if (copyButton) {
      event.preventDefault();
      event.stopPropagation();
      const payload = copyButton.dataset.copyCode ?? "";
      const originalLabel = copyButton.textContent || copy.copyCode;
      try {
        await copyTextToClipboard(decodeCodePayload(payload));
        copyButton.textContent = copy.codeCopied;
        copyButton.classList.add("copied");
      } catch {
        copyButton.textContent = copy.codeCopyFailed;
        copyButton.classList.add("failed");
      }
      window.setTimeout(() => {
        copyButton.textContent = originalLabel;
        copyButton.classList.remove("copied", "failed");
      }, 1500);
      return;
    }
    const link = target?.closest("a");
    const href = link?.getAttribute("href");
    if (!href) return;
    if (href.startsWith("http://") || href.startsWith("https://")) {
      window.open(href, "_blank", "noopener,noreferrer");
      event.preventDefault();
      return;
    }
    if (href.startsWith("#")) return;
    event.preventDefault();
    onOpenLink(href);
  };

  return (
    <section className="gd-panel">
      <div className="gd-title-row">
        <div className="gd-heading-group">
          {canGoBack && (
            <button type="button" className="gd-back" onClick={onBack}>
              <ChevronLeft size={15} />
              <span>{copy.tutorialBack}</span>
            </button>
          )}
          <h2 className="gd-title">{copy.tutorialTitle}</h2>
        </div>
        <div className="gd-segment" role="tablist" aria-label={copy.tutorialTitle}>
          <button type="button" role="tab" aria-selected={kind === "readme"} className={kind === "readme" ? "active" : undefined} onClick={() => onSelect("readme")}>{copy.tutorialReadme}</button>
          <button type="button" role="tab" aria-selected={kind === "howto"} className={kind === "howto" ? "active" : undefined} onClick={() => onSelect("howto")}>{copy.tutorialHowTo}</button>
        </div>
      </div>
      <div className="gd-doc-meta">
        <strong>{document?.title || copy.tutorialTitle}</strong>
        <span>{copy.tutorialCurrentDocument}: {repoReady ? document?.path || copy.tutorialLoading : unavailableDescription}</span>
      </div>
      <div className="gd-scroll">
        {!repoReady ? (
          <div className="workspace-empty">
            <strong>{unavailableTitle}</strong>
            <p>{unavailableDescription}</p>
            <p>{unavailableHelp}</p>
            <div className="workspace-empty-actions">
              <button type="button" onClick={onRecover}>{recoverLabel}</button>
              {recoverLabel !== copy.changeProjectPath && (
                <button type="button" className="secondary" onClick={onChangeProject}>{copy.changeProjectPath}</button>
              )}
            </div>
          </div>
        ) : loading ? (
          <div className="table-empty">{copy.tutorialLoading}</div>
        ) : (
          <div className="markdown-body" onClick={handleClick} dangerouslySetInnerHTML={{ __html: html }} />
        )}
      </div>
    </section>
  );
}
