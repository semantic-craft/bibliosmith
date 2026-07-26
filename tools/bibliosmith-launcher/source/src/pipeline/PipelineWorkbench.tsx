import { useEffect, useMemo, useState } from "react";
import { Plus } from "lucide-react";
import type {
  BookPipelineCustomInstructions,
  BookPipelineRouteItem,
  BookPipelineSource,
  BookPipelineState,
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
  onDelete: (jobId: string) => void;
  onAdvance: (jobId: string, childId: string) => void;
  onSampleTranslation: (jobId: string, childId: string, providerProfileId: string, providerConfigId: string) => void;
  onSaveCustomInstructions: (
    jobId: string,
    childId: string,
    customInstructions: BookPipelineCustomInstructions,
  ) => void;
  onApproveGate: (jobId: string, childId: string, stageId: "approve_translation" | "approve_promotion") => void;
  onOpenOutput: (jobId: string) => void;
  routeOverrides: Record<string, RouteOverride>;
  onRouteOverrideChange: (routeItemId: string, override: RouteOverride) => void;
  onHandoff: (jobId: string, artifactPath?: string | null) => void;
};

export function PipelineWorkbench(props: PipelineWorkbenchProps) {
  const { copy, state, busy } = props;
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [wizardOpen, setWizardOpen] = useState(false);

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
        <div className="pl-budget-pill" title={copy.ocrBudget}>
          <div className="pl-bl">
            <span>{copy.ocrBudget}</span>
            <b className="pl-num">—</b>
          </div>
        </div>
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
        <div className="pl-shelfwrap">
          <Shelf
            copy={copy}
            units={units}
            selectedKey={selected?.key ?? null}
            onSelect={setSelectedKey}
            onNewJob={() => setWizardOpen(true)}
          />
          {selected && (
            <BookDrawer
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
              onSaveCustomInstructions={props.onSaveCustomInstructions}
              onApproveGate={props.onApproveGate}
              onOpenOutput={props.onOpenOutput}
              onHandoff={props.onHandoff}
            />
          )}
        </div>
      )}
    </section>
  );
}
