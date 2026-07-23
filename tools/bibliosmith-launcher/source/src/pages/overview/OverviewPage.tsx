import { MoreHorizontal } from "lucide-react";
import type { ActivityItem, BookPipelineState } from "../../types";
import type { Copy } from "../../i18n";
import type { pipelineCopy } from "../../pipeline";
import { ActivityTable, PipelineSnapshotPanel, type ProductCardProps } from "../../components";
import "./overview.css";

export function OverviewPage({
  copy,
  pipelineCopy: bookPipelineCopy,
  projectStatusLine,
  biblioSmithCard,
  biblioSmithUpdateAvailable,
  visibleActivities,
  pipelineState,
  onViewLogs,
  onOpenPipeline,
  onGoUpdates,
}: {
  copy: Copy;
  pipelineCopy: ReturnType<typeof pipelineCopy>;
  projectStatusLine: string;
  biblioSmithCard: ProductCardProps;
  biblioSmithUpdateAvailable: boolean;
  visibleActivities: ActivityItem[];
  pipelineState: BookPipelineState;
  onViewLogs: () => void;
  onOpenPipeline: () => void;
  onGoUpdates: () => void;
}) {
  return (
    <>
      <header className="ov-head">
        <h1>{copy.overview}</h1>
        <span className="ov-head-status">{projectStatusLine}</span>
      </header>
      <section className="ov-products">
        <ProductRow copy={copy} card={biblioSmithCard} updateAvailable={biblioSmithUpdateAvailable} onGoUpdates={onGoUpdates} />
      </section>
      <section className="overview-bottom-grid">
        <ActivityTable copy={copy} activities={visibleActivities} onViewFullLog={onViewLogs} />
        <PipelineSnapshotPanel
          copy={bookPipelineCopy}
          state={pipelineState}
          onOpen={onOpenPipeline}
        />
      </section>
    </>
  );
}

function ProductRow({
  copy,
  card,
  updateAvailable,
  onGoUpdates,
}: {
  copy: Copy;
  card: ProductCardProps;
  updateAvailable: boolean;
  onGoUpdates: () => void;
}) {
  const Icon = card.icon;
  const PrimaryIcon = card.primaryIcon;
  const SecondaryIcon = card.secondaryIcon;
  return (
    <article className="ov-product-row">
      <div className={`ov-product-icon ${card.accent === "green" ? "green" : ""}`}>
        <Icon size={22} strokeWidth={2} />
      </div>
      <div className="ov-product-copy">
        <strong>{card.title}</strong>
        <span>{card.subtitle}</span>
      </div>
      <div className="ov-product-status">
        {updateAvailable ? (
          <button type="button" className="ov-chip link" onClick={onGoUpdates}>
            {copy.updateAvailable} →
          </button>
        ) : (
          <span className={`ov-chip ${card.statusTone}`}>{card.status}</span>
        )}
      </div>
      <div className="ov-product-actions">
        <button className="ov-btn primary" disabled={card.busy} onClick={card.onPrimary}>
          <PrimaryIcon size={15} />
          {card.busy ? card.busyText : card.primaryLabel}
        </button>
        <button className="ov-btn" disabled={card.secondaryDisabled || card.busy} onClick={card.onSecondary}>
          <SecondaryIcon size={15} />
          {card.secondaryLabel}
        </button>
        <button
          className="ov-btn icon"
          type="button"
          aria-label={card.moreLabel}
          title={card.moreLabel}
          disabled={card.moreDisabled || card.moreBusy}
          onClick={card.onMore}
        >
          <MoreHorizontal size={16} className={card.moreBusy ? "spin-icon" : undefined} />
        </button>
      </div>
    </article>
  );
}
