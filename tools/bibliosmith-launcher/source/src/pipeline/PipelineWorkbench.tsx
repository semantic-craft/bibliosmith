import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { Plus } from "lucide-react";
import type {
  BookPipelineCustomInstructions,
  BookPipelineRouteItem,
  BookPipelineSource,
  BookPipelineState,
  BookPipelineDiagnosticProfile,
  ModelSlotView,
} from "../types";
import "./pipeline.css";
import type { PipelineCopy } from "./copy";
import { flattenBookUnits, type PipelineBusy, type PipelineDraft, type RouteOverride } from "./model";
import { Shelf } from "./Shelf";
import { BookDrawer } from "./BookDrawer";
import { NewJobWizard } from "./NewJobWizard";

export type PipelineWorkbenchProps = {
  copy: PipelineCopy;
  state: BookPipelineState;
  draft: PipelineDraft;
  preview: BookPipelineRouteItem[];
  zoteroSources: BookPipelineSource[];
  modelSlots: ModelSlotView[];
  busy: PipelineBusy;
  onDraftChange: (patch: Partial<PipelineDraft>) => void;
  onPreview: () => void;
  onQueueRun: () => Promise<boolean>;
  onChooseFolder: () => void;
  onChooseMarkdown: () => void;
  onDiscoverZotero: () => void;
  onSearchZotero: (query: string) => void;
  onRetry: (jobId: string) => void;
  onDelete: (jobId: string, childId?: string | null) => void;
  onAdvance: (jobId: string, childId: string) => void;
  onSampleTranslation: (jobId: string, childId: string, providerProfileId: string, providerConfigId: string) => void;
  onApplySampleProvider: (jobId: string, childId: string, providerProfileId: string, providerConfigId: string) => void;
  onExportDiagnostic: (jobId: string, profile: BookPipelineDiagnosticProfile) => void;
  onSaveCustomInstructions: (
    jobId: string,
    childId: string,
    customInstructions: BookPipelineCustomInstructions,
  ) => void;
  onApproveGate: (jobId: string, childId: string, stageId: "approve_translation" | "approve_promotion") => void;
  onRouteOverride: (jobId: string, childId: string, routeItemId: string, routeOverride: string) => void;
  onRecordReaderEvidence: (
    jobId: string,
    childId: string,
    artifactKind: string,
    reader: string,
    readerVersion: string,
    conclusion: string,
  ) => void;
  onOpenOutput: (jobId: string) => void;
  routeOverrides: Record<string, RouteOverride>;
  onRouteOverrideChange: (routeItemId: string, override: RouteOverride) => void;
  onHandoff: (jobId: string, artifactPath?: string | null) => void;
};

export function PipelineWorkbench(props: PipelineWorkbenchProps) {
  const { copy, state, busy } = props;
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [drawerPercent, setDrawerPercent] = useState(50);
  const splitViewRef = useRef<HTMLDivElement>(null);
  const resizeHandleRef = useRef<HTMLDivElement>(null);
  const draggedDrawerPercent = useRef(drawerPercent);
  const resizing = useRef(false);

  const applyDrawerPercent = (value: number) => {
    const next = Math.max(35, Math.min(70, Math.round(value)));
    draggedDrawerPercent.current = next;
    splitViewRef.current?.style.setProperty("--pl-drawer-width", `${next}%`);
    resizeHandleRef.current?.setAttribute("aria-valuenow", String(next));
    return next;
  };

  const units = useMemo(() => flattenBookUnits(state.jobs), [state.jobs]);
  const selected = units.find((unit) => unit.key === selectedKey) ?? null;

  // Esc closes the wizard first, then the drawer.
  useEffect(() => {
    if (!wizardOpen && !selectedKey) return undefined;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (wizardOpen) setWizardOpen(false);
      else setSelectedKey(null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [wizardOpen, selectedKey]);

  return (
    <section className="pl-panel">
      <div className="pl-topbar">
        <h1>{copy.inboxTitle}</h1>
        <span className="pl-spacer" />
        {!wizardOpen && (
          <button className="pl-btn primary" type="button" onClick={() => setWizardOpen(true)}>
            <Plus size={15} />
            {copy.newJob}
          </button>
        )}
      </div>

      {wizardOpen ? (
        <NewJobWizard
          copy={props.copy}
          draft={props.draft}
          preview={props.preview}
          zoteroSources={props.zoteroSources}
          modelSlots={props.modelSlots}
          busy={busy}
          onDraftChange={props.onDraftChange}
          onPreview={props.onPreview}
          onQueueRun={props.onQueueRun}
          onChooseFolder={props.onChooseFolder}
          onChooseMarkdown={props.onChooseMarkdown}
          onDiscoverZotero={props.onDiscoverZotero}
          onSearchZotero={props.onSearchZotero}
          onClose={() => setWizardOpen(false)}
          routeOverrides={props.routeOverrides}
          onRouteOverrideChange={props.onRouteOverrideChange}
        />
      ) : units.length === 0 ? (
        <div className="pl-empty">
          <h3>{copy.inboxEmptyTitle}</h3>
          <p>{copy.inboxEmptyBody}</p>
          <button className="pl-btn primary" type="button" onClick={() => setWizardOpen(true)}>
            <Plus size={15} />
            {copy.newJob}
          </button>
          <p className="pl-empty-formats">{copy.inboxEmptyFormats}</p>
        </div>
      ) : (
        <div
          ref={splitViewRef}
          className="pl-shelfwrap"
          style={{ "--pl-drawer-width": `${drawerPercent}%` } as CSSProperties}
        >
          <Shelf
            copy={copy}
            units={units}
            selectedKey={selected?.key ?? null}
            onSelect={setSelectedKey}
            onNewJob={() => setWizardOpen(true)}
          />
          {selected && (
            <div
              ref={resizeHandleRef}
              className="pl-resizer"
              role="separator"
              aria-label={copy.resizeDrawer}
              aria-orientation="vertical"
              aria-valuemin={35}
              aria-valuemax={70}
              aria-valuenow={drawerPercent}
              tabIndex={0}
              onPointerDown={(event) => {
                resizing.current = true;
                event.currentTarget.setPointerCapture(event.pointerId);
              }}
              onPointerMove={(event) => {
                if (!resizing.current || !splitViewRef.current) return;
                const bounds = splitViewRef.current.getBoundingClientRect();
                if (bounds.width <= 0) return;
                applyDrawerPercent(((bounds.right - event.clientX) / bounds.width) * 100);
              }}
              onPointerUp={(event) => {
                if (!resizing.current) return;
                resizing.current = false;
                event.currentTarget.releasePointerCapture(event.pointerId);
                setDrawerPercent(draggedDrawerPercent.current);
              }}
              onPointerCancel={() => {
                resizing.current = false;
                setDrawerPercent(draggedDrawerPercent.current);
              }}
              onKeyDown={(event) => {
                if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
                event.preventDefault();
                setDrawerPercent((current) => applyDrawerPercent(current + (event.key === "ArrowLeft" ? 5 : -5)));
              }}
            />
          )}
          {selected && (
            <BookDrawer
              // Remount per book: the drawer's local state (delete
              // confirmation, provider picker, drafts) belongs to one book.
              key={selected.key}
              copy={copy}
              units={units}
              unit={selected}
              busy={busy}
              onSelect={setSelectedKey}
              onClose={() => setSelectedKey(null)}
              onRetry={props.onRetry}
              onDelete={props.onDelete}
              onAdvance={props.onAdvance}
              onSampleTranslation={props.onSampleTranslation}
              onApplySampleProvider={props.onApplySampleProvider}
              onExportDiagnostic={props.onExportDiagnostic}
              onSaveCustomInstructions={props.onSaveCustomInstructions}
              onApproveGate={props.onApproveGate}
              onRouteOverride={props.onRouteOverride}
              onRecordReaderEvidence={props.onRecordReaderEvidence}
              onOpenOutput={props.onOpenOutput}
              onHandoff={props.onHandoff}
            />
          )}
        </div>
      )}
    </section>
  );
}
