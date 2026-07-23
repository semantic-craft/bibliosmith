import { RefreshCcw } from "lucide-react";
import type { CommitInfo } from "../../types";
import type { Copy } from "../../i18n";
import { CommitTable, type ProductCardProps } from "../../components";
import "./updates.css";

export function UpdatesPage({
  copy,
  biblioSmithCard,
  launcherVersion,
  launcherLatest,
  launcherBusy,
  onCheckLauncher,
  commits,
  displayedCommits,
  latestBiblioSmithVersion,
  showAllCommits,
  commitEmptyMessage,
  refreshInProgress,
  lastRefreshAt,
  onToggleShowAllCommits,
  onCheckAll,
}: {
  copy: Copy;
  biblioSmithCard: ProductCardProps;
  launcherVersion: string;
  launcherLatest: string;
  launcherBusy: boolean;
  onCheckLauncher: () => void;
  commits: CommitInfo[];
  displayedCommits: CommitInfo[];
  latestBiblioSmithVersion: string;
  showAllCommits: boolean;
  commitEmptyMessage: string;
  refreshInProgress: boolean;
  lastRefreshAt: string;
  onToggleShowAllCommits: () => void;
  onCheckAll: () => void;
}) {
  return (
    <>
      <header className="up-head">
        <h1>{copy.updateCenterTitle}</h1>
        {lastRefreshAt && <span className="up-last">{copy.lastCheckedAt(lastRefreshAt)}</span>}
        <div className="up-head-actions">
          <button className="up-btn primary" type="button" onClick={onCheckAll} disabled={refreshInProgress}>
            <RefreshCcw size={14} className={refreshInProgress ? "spin-icon" : undefined} />
            {refreshInProgress ? copy.working : copy.checkAllUpdates}
          </button>
        </div>
      </header>

      <section className="up-cards">
        <article className="up-card">
          <div className="up-card-head">
            <strong>BiblioSmith Launcher</strong>
            <span className="up-chip success">{copy.upToDate}</span>
          </div>
          <div className="up-versions">
            <div className="up-version-line">
              <span>{copy.currentVersion}</span>
              <strong>{launcherVersion}</strong>
            </div>
            <div className="up-version-line">
              <span>{copy.latestVersion}</span>
              <strong>{launcherLatest || launcherVersion}</strong>
            </div>
          </div>
          <p className="up-notes">{copy.localBuildNote}</p>
          <div className="up-card-actions">
            <button className="up-btn" type="button" onClick={onCheckLauncher} disabled={launcherBusy}>
              <RefreshCcw size={14} />
              {copy.checkUpdates}
            </button>
          </div>
        </article>

        <VersionCard copy={copy} card={biblioSmithCard} actionLabel={biblioSmithCard.moreLabel} actionBusy={Boolean(biblioSmithCard.moreBusy)} onAction={biblioSmithCard.onMore} />
      </section>

      <CommitTable
        copy={copy}
        commits={displayedCommits}
        totalCount={commits.length}
        latestVersion={latestBiblioSmithVersion}
        showAll={showAllCommits}
        emptyMessage={commitEmptyMessage}
        onToggleShowAll={onToggleShowAllCommits}
      />
    </>
  );
}

function VersionCard({
  copy,
  card,
  actionLabel,
  actionBusy,
  onAction,
}: {
  copy: Copy;
  card: ProductCardProps;
  actionLabel: string;
  actionBusy: boolean;
  onAction: () => void;
}) {
  return (
    <article className="up-card">
      <div className="up-card-head">
        <strong>{card.title}</strong>
        <span className={`up-chip ${card.statusTone}`}>{card.status}</span>
      </div>
      <div className="up-versions">
        <div className="up-version-line">
          <span>{copy.currentVersion}</span>
          <strong>{card.current}</strong>
        </div>
        <div className="up-version-line">
          <span>{copy.latestVersion}</span>
          <strong>{card.latest}</strong>
        </div>
        <div className="up-version-line">
          <span>{copy.latestUpdate}</span>
          <strong>{card.latestUpdated}</strong>
        </div>
      </div>
      <span />
      <div className="up-card-actions">
        <button className="up-btn" type="button" onClick={onAction} disabled={actionBusy}>
          <RefreshCcw size={14} className={actionBusy ? "spin-icon" : undefined} />
          {actionBusy ? card.busyText : actionLabel}
        </button>
      </div>
    </article>
  );
}
