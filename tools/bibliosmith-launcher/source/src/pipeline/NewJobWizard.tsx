import { useEffect, useState } from "react";
import { FileText, FolderOpen, Library, RefreshCcw, Search } from "lucide-react";
import type { BookPipelineRouteItem, BookPipelineSource, ModelSlotView } from "../types";
import type { PipelineCopy } from "./copy";
import {
  configuredSlotKeys,
  providerCredentialMissing,
  providerSelectionApplies,
  routeKindLabel,
  routeTone,
  type PipelineBusy,
  type PipelineDraft,
  type RouteOverride,
} from "./model";
import { MODEL_BRANDS, slotDisplayName } from "../pages/settings/modelCatalog";

type WizardProps = {
  copy: PipelineCopy;
  draft: PipelineDraft;
  preview: BookPipelineRouteItem[];
  zoteroSources: BookPipelineSource[];
  // What the backend reports about each provider slot, including whether it has
  // a stored key. Empty when the catalog could not be read.
  modelSlots: ModelSlotView[];
  busy: PipelineBusy;
  onDraftChange: (patch: Partial<PipelineDraft>) => void;
  onPreview: () => void;
  onQueueRun: () => Promise<boolean>;
  onChooseFolder: () => void;
  onChooseMarkdown: () => void;
  onDiscoverZotero: () => void;
  onSearchZotero: (query: string) => void;
  onClose: () => void;
  routeOverrides: Record<string, RouteOverride>;
  onRouteOverrideChange: (routeItemId: string, override: RouteOverride) => void;
};

const ZOTERO_KINDS = new Set(["zotero_attachment", "zotero_collection", "zotero_filter"]);

function sourceMissing(draft: PipelineDraft): boolean {
  if (draft.sourceKind === "local_pdf_folder") return !draft.localPdfFolder;
  if (draft.sourceKind === "markdown_source") return !draft.markdownPath;
  if (ZOTERO_KINDS.has(draft.sourceKind)) return !draft.zoteroSelector;
  return false;
}

function executionRoutes(preview: BookPipelineRouteItem[]): BookPipelineRouteItem[] {
  return preview.filter((item) => item.routeKind !== "translation_handoff");
}

function StepIndicator({ copy, step }: { copy: PipelineCopy; step: number }) {
  const steps = [copy.wizStepSource, copy.wizStepPreflight, copy.wizStepConfirm];
  return (
    <div className="pl-wsteps">
      {steps.map((label, index) => {
        const number = index + 1;
        const cls = step > number ? "done" : step === number ? "cur" : "";
        return (
          <div key={label} style={{ display: "contents" }}>
            <div className={`pl-wstep ${cls}`}>
              <span className="pl-wnum pl-num">{step > number ? "✓" : number}</span>
              <span className="pl-wlab">{label}</span>
            </div>
            {number < steps.length && <div className={`pl-wsep${step > number ? " done" : ""}`} />}
          </div>
        );
      })}
    </div>
  );
}

/**
 * A book's title/author/year, not its Zotero key. Local state, not draft
 * state: typing here must not overwrite draft.zoteroSelector on every
 * keystroke, since that field is also the exact-key box below it (an
 * attachment key or collection key), and an in-progress query would corrupt
 * whatever key is sitting there. Submitting builds the request from this
 * text directly rather than through draft state, so there is nothing to race.
 */
function ZoteroTitleSearch({ copy, busy, onSearch }: { copy: PipelineCopy; busy: PipelineBusy; onSearch: (query: string) => void }) {
  const [query, setQuery] = useState("");
  const submit = () => {
    const trimmed = query.trim();
    if (trimmed) onSearch(trimmed);
  };
  return (
    <div className="pl-evi-row">
      <span className="pl-k">{copy.zoteroTitleSearch}</span>
      <input
        type="text"
        placeholder={copy.zoteroTitleSearchPlaceholder}
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            submit();
          }
        }}
      />
      <button className="pl-btn sm" type="button" disabled={busy === "zotero" || !query.trim()} onClick={submit}>
        <Search size={14} className={busy === "zotero" ? "spin-icon" : undefined} />
        {copy.zoteroTitleSearchButton}
      </button>
    </div>
  );
}

function SourceStep({ copy, draft, busy, modelSlots, onDraftChange, onChooseFolder, onChooseMarkdown, onDiscoverZotero, onSearchZotero, zoteroSources }: WizardProps) {
  // Options stay selectable when they have no key: the slot Settings made active
  // may itself be unconfigured, and disabling the current value would strand the
  // picker. The label says so instead, and the confirm step holds the launch.
  const configured = configuredSlotKeys(modelSlots);
  const cards = [
    { kind: "local_pdf_folder" as const, icon: <FolderOpen size={28} />, title: copy.srcLocalPdfTitle, desc: copy.srcLocalPdfDesc },
    { kind: "zotero_collection" as const, icon: <Library size={28} />, title: copy.srcZoteroTitle, desc: copy.srcZoteroDesc },
    { kind: "markdown_source" as const, icon: <FileText size={28} />, title: copy.srcMarkdownTitle, desc: copy.srcMarkdownDesc },
  ];
  const isZotero = ZOTERO_KINDS.has(draft.sourceKind);
  const selectedZoteroValue = `${draft.sourceKind}:${draft.zoteroSelector}`;
  // The backend rejects digestMode without epub outright (book_pipeline.rs,
  // "requires epub in outputFormats when digestMode is enabled"), so the box is
  // gated on the same condition rather than on a second rule invented here.
  const epubSelected = draft.outputFormats.includes("epub");
  return (
    <>
      <div className="pl-src-cards">
        {cards.map((card) => {
          const active = card.kind === "zotero_collection" ? isZotero : draft.sourceKind === card.kind;
          return (
            <div
              key={card.kind}
              className={`pl-card pl-src-card${active ? " on" : ""}`}
              role="button"
              tabIndex={0}
              onClick={() => {
                if (card.kind === "local_pdf_folder") {
                  if (draft.sourceKind !== "local_pdf_folder") onDraftChange({ sourceKind: "local_pdf_folder" });
                  if (!draft.localPdfFolder) onChooseFolder();
                } else if (card.kind === "markdown_source") {
                  if (draft.sourceKind !== "markdown_source") onDraftChange({ sourceKind: "markdown_source" });
                  if (!draft.markdownPath) onChooseMarkdown();
                } else {
                  onDraftChange({ sourceKind: "zotero_collection" });
                }
              }}
            >
              <div className="pl-sg">{card.icon}</div>
              <b>{card.title}</b>
              <span>{card.desc}</span>
            </div>
          );
        })}
      </div>

      {draft.sourceKind === "local_pdf_folder" && (
        <div className="pl-card pl-src-config">
          <div className="pl-evi-row">
            <span className="pl-k">{copy.selectedFolder}</span>
            <span className="pl-v pl-mono">{draft.localPdfFolder || copy.missingFolder}</span>
            <button className="pl-btn sm" type="button" disabled={busy === "folder"} onClick={onChooseFolder}>
              <FolderOpen size={14} />
              {copy.chooseFolder}
            </button>
          </div>
        </div>
      )}

      {draft.sourceKind === "markdown_source" && (
        <div className="pl-card pl-src-config">
          <div className="pl-evi-row">
            <span className="pl-k">{copy.selectedMarkdown}</span>
            <span className="pl-v pl-mono">{draft.markdownPath || copy.missingMarkdown}</span>
            <button className="pl-btn sm" type="button" disabled={busy === "markdown"} onClick={onChooseMarkdown}>
              <FileText size={14} />
              {copy.chooseMarkdown}
            </button>
          </div>
        </div>
      )}

      {isZotero && (
        <div className="pl-card pl-src-config">
          <ZoteroTitleSearch copy={copy} busy={busy} onSearch={onSearchZotero} />
          <div className="pl-evi-row">
            <span className="pl-k">{copy.selector}</span>
            <input
              type="text"
              value={draft.zoteroSelector}
              onChange={(event) => onDraftChange({ zoteroSelector: event.target.value })}
            />
            <button className="pl-btn sm" type="button" disabled={busy === "zotero"} onClick={onDiscoverZotero}>
              <RefreshCcw size={14} className={busy === "zotero" ? "spin-icon" : undefined} />
              {copy.discoverZotero}
            </button>
          </div>
          {zoteroSources.length > 0 && (
            <div className="pl-evi-row">
              <span className="pl-k">{copy.discoveredSources}</span>
              <select
                value={selectedZoteroValue}
                onChange={(event) => {
                  const source = zoteroSources.find((item) => `${item.kind}:${item.selector || ""}` === event.target.value);
                  if (!source) return;
                  onDraftChange({ sourceKind: source.kind, zoteroSelector: source.selector || source.title || "" });
                }}
              >
                {zoteroSources.map((source) => (
                  <option key={`${source.kind}:${source.selector || source.title || ""}`} value={`${source.kind}:${source.selector || ""}`}>
                    {source.title || source.selector || source.kind}
                  </option>
                ))}
              </select>
            </div>
          )}
        </div>
      )}

      <div className="pl-card pl-intent-row">
        <label>
          {copy.intentMode}
          <select
            value={draft.mode}
            onChange={(event) => onDraftChange({ mode: event.target.value as PipelineDraft["mode"] })}
          >
            <option value="convert_then_translate">{copy.convertThenTranslate}</option>
            <option value="conversion_only">{copy.conversionOnly}</option>
            <option value="translate_only">{copy.translateOnly}</option>
          </select>
        </label>
        {draft.mode !== "conversion_only" && (
          <label>
            {copy.intentTier}
            <select
              value={draft.translationMode}
              onChange={(event) => onDraftChange({ translationMode: event.target.value as PipelineDraft["translationMode"] })}
            >
              <option value="expert">{copy.tierExpert}</option>
              <option value="fast">{copy.tierFast}</option>
            </select>
          </label>
        )}
        {draft.mode !== "conversion_only" && draft.translationMode === "fast" && (
          <label>
            {copy.provider}
            <select
              value={`${draft.providerProfileId}:${draft.providerConfigId}`}
              onChange={(event) => {
                const [profileId, configId] = event.target.value.split(":");
                onDraftChange({ providerProfileId: profileId, providerConfigId: configId });
              }}
            >
              {MODEL_BRANDS.flatMap((brand) =>
                brand.slots.map((slot) => {
                  const key = `${slot.profileId}:${slot.configId}`;
                  const name = slotDisplayName(slot.profileId, slot.configId);
                  // No catalog means the read failed, not that every slot is
                  // unconfigured — say nothing rather than mislabel all eight.
                  const unconfigured = modelSlots.length > 0 && !configured.has(key);
                  return (
                    <option key={key} value={key}>
                      {unconfigured ? `${name} · ${copy.providerUnconfigured}` : name}
                    </option>
                  );
                }),
              )}
            </select>
          </label>
        )}
        {draft.mode !== "conversion_only" && draft.translationMode === "fast" && (
          <label>
            <input
              type="checkbox"
              checked={draft.textCleanup}
              onChange={(event) => onDraftChange({ textCleanup: event.target.checked })}
            />
            {copy.textCleanup}
          </label>
        )}
        {/* secondPassEnabled was wired end to end but had no control anywhere, so
            every job ran the reflection pass at its default and nobody could
            decline the extra spend. Expert mode drives its own passes and forces
            this off in the intent, so the box only applies to fast mode. */}
        {draft.mode !== "conversion_only" && draft.translationMode === "fast" && (
          <label>
            <input
              type="checkbox"
              checked={draft.secondPassEnabled}
              onChange={(event) => onDraftChange({ secondPassEnabled: event.target.checked })}
            />
            {copy.secondPass}
          </label>
        )}
        {draft.mode !== "conversion_only" && (
          <span className="pl-format-checks">
            {copy.intentOutput}
            {([
              ["md", copy.outputFormatMd],
              ["html", copy.outputFormatHtml],
              ["epub", copy.outputFormatEpub],
              ["bilingual", copy.outputFormatBilingual],
            ] as const).map(([format, label]) => {
              const checked = draft.outputFormats.includes(format);
              return (
                <label key={format}>
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={checked && draft.outputFormats.length === 1}
                    onChange={(event) => {
                      const outputFormats = event.target.checked
                        ? [...draft.outputFormats, format]
                        : draft.outputFormats.filter((candidate) => candidate !== format);
                      onDraftChange({
                        outputFormats,
                        digestMode: format === "epub" && !event.target.checked ? false : draft.digestMode,
                      });
                    }}
                  />
                  {label}
                </label>
              );
            })}
            <label title={epubSelected ? undefined : copy.outputFormatDigestRequiresEpub}>
              <input
                type="checkbox"
                checked={draft.digestMode}
                disabled={!epubSelected}
                onChange={(event) => onDraftChange({ digestMode: event.target.checked })}
              />
              {copy.outputFormatDigest}
            </label>
          </span>
        )}
      </div>
    </>
  );
}

function PreflightStep({ copy, draft, preview, busy, onDraftChange, routeOverrides, onRouteOverrideChange }: WizardProps) {
  const routes = executionRoutes(preview);
  const isZotero = ZOTERO_KINDS.has(draft.sourceKind);
  return (
    <>
      <div className="pl-preflight">
        {isZotero && (
          <span className="pl-pf" style={{ cursor: "default" }}>
            {preview.length ? "✓" : "…"} {copy.preflightZotero}
          </span>
        )}
        <button
          className={`pl-pf${draft.hasPaddleocrCredentials ? "" : " off"}`}
          type="button"
          title={copy.paddleCreds}
          onClick={() => onDraftChange({ hasPaddleocrCredentials: !draft.hasPaddleocrCredentials })}
        >
          {draft.hasPaddleocrCredentials ? "✓" : "✗"} {copy.paddleCreds}
        </button>
        <button
          className={`pl-pf${draft.hasMineruCredentials ? "" : " off"}`}
          type="button"
          title={copy.mineruCreds}
          onClick={() => onDraftChange({ hasMineruCredentials: !draft.hasMineruCredentials })}
        >
          {draft.hasMineruCredentials ? "✓" : "✗"} {copy.mineruCreds}
        </button>
      </div>

      <div className="pl-card pl-route-wrap">
        <table className="pl-route-table">
          <thead>
            <tr>
              <th>{copy.thBook}{routes.length ? `（${copy.routesUnit(routes.length)}）` : ""}</th>
              <th>{copy.thAutoRoute}</th>
              <th>{copy.thOverride}</th>
              <th>{copy.thPreflight}</th>
            </tr>
          </thead>
          <tbody>
            {routes.map((item) => (
              <tr key={item.id}>
                <td>
                  <div className="pl-rt-t">{item.title}</div>
                  <div className="pl-rt-s pl-mono">{item.sourceRef}</div>
                </td>
                <td>
                  <span className={`pl-chip ${routeTone(item.routeKind)}`}>{routeKindLabel(item.routeKind, copy)}</span>
                </td>
                <td>
                  <select
                    aria-label={copy.thOverride}
                    value={routeOverrides[item.id] ?? "auto"}
                    onChange={(event) => onRouteOverrideChange(item.id, event.target.value as RouteOverride)}
                  >
                    <option value="auto">{copy.overrideAuto}</option>
                    <option value="direct">{copy.overrideForceDirect}</option>
                    <option value="paddle">{copy.overrideForcePaddle}</option>
                    <option value="mineru">{copy.overrideForceMineru}</option>
                    <option value="keep">{copy.overrideKeep}</option>
                  </select>
                </td>
                <td>
                  {item.canRun ? (
                    <span className="pl-chip ok">{copy.preflightReady}</span>
                  ) : item.routeKind === "already_converted" ? (
                    <span className="pl-chip neutral">{copy.preflightSkip}</span>
                  ) : (
                    <span className="pl-chip block" title={item.blockedReason ?? undefined}>{copy.preflightBlocked}</span>
                  )}
                </td>
              </tr>
            ))}
            {!routes.length && (
              <tr>
                <td colSpan={4}>
                  <span className="pl-muted-note">{busy === "preview" ? "…" : copy.noPreview}</span>
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </>
  );
}

function ConfirmStep({ copy, draft, preview, modelSlots }: WizardProps) {
  const routes = executionRoutes(preview);
  const credentialMissing = providerCredentialMissing(draft, modelSlots);
  const runnable = routes.filter((item) => item.canRun);
  const held = routes.filter((item) => !item.canRun && item.routeKind !== "already_converted");
  const skipped = routes.filter((item) => item.routeKind === "already_converted");
  const sourceDescription =
    draft.sourceKind === "local_pdf_folder"
      ? draft.localPdfFolder
      : draft.sourceKind === "markdown_source"
        ? draft.markdownPath
        : draft.zoteroSelector;
  const isCollection = draft.sourceKind === "zotero_collection" || draft.sourceKind === "zotero_filter";
  const modeLabel =
    draft.mode === "conversion_only"
      ? copy.conversionOnly
      : draft.mode === "translate_only"
        ? copy.translateOnly
        : copy.convertThenTranslate;
  const tierLabel = draft.mode === "conversion_only" ? null : draft.translationMode === "expert" ? copy.tierExpert : copy.tierFast;
  return (
    <div className="pl-confirm-grid">
      <div className="pl-card">
        <h4 className="pl-card-title">{copy.confirmBatch}</h4>
        <div className="pl-evi-row">
          <span className="pl-k">{copy.batchSource}</span>
          <span className="pl-v pl-mono">{sourceDescription || "—"}</span>
        </div>
        <div className="pl-evi-row">
          <span className="pl-k">{copy.batchLaunch}</span>
          <span className="pl-v pl-num">{copy.routesUnit(runnable.length)}</span>
        </div>
        {held.length > 0 && (
          <div className="pl-evi-row">
            <span className="pl-k">{copy.batchHeld}</span>
            <span className="pl-v pl-num">{copy.routesUnit(held.length)} · {copy.batchHeldNote}</span>
          </div>
        )}
        {skipped.length > 0 && (
          <div className="pl-evi-row">
            <span className="pl-k">{copy.preflightSkip}</span>
            <span className="pl-v pl-num">{copy.routesUnit(skipped.length)}</span>
          </div>
        )}
      </div>
      <div className="pl-card">
        <h4 className="pl-card-title">{copy.confirmIntent}</h4>
        <div className="pl-evi-row">
          <span className="pl-k">{copy.intentMode}</span>
          <span className="pl-v">{tierLabel ? `${modeLabel} · ${tierLabel}` : modeLabel}</span>
        </div>
        {draft.mode !== "conversion_only" && (
          <div className="pl-evi-row">
            <span className="pl-k">{copy.intentOutput}</span>
            <span className="pl-v">{draft.outputFormats.map((format) => format.toUpperCase()).join(" · ")}</span>
          </div>
        )}
        {providerSelectionApplies(draft) && (
          <div className="pl-evi-row">
            <span className="pl-k">{copy.provider}</span>
            <span className="pl-v">
              {slotDisplayName(draft.providerProfileId, draft.providerConfigId)}
              {credentialMissing ? ` · ${copy.providerUnconfigured}` : ""}
            </span>
          </div>
        )}
        <div className="pl-evi-row">
          <span className="pl-k">{copy.intentTargetLang}</span>
          <span className="pl-v">zh-Hans</span>
        </div>
      </div>
      {credentialMissing && (
        <p className="pl-wiz-error pl-span2">{copy.providerKeyMissing}</p>
      )}
      <div className="pl-card pl-span2">
        <h4 className="pl-card-title">{copy.confirmStructure}</h4>
        <div className="pl-evi-row">
          <span className="pl-k">{isCollection ? copy.structureParentCollection : copy.structureParentSingle}</span>
          <span className="pl-v">{isCollection ? copy.structureParentNote : "—"}</span>
        </div>
        <div className="pl-evi-row">
          <span className="pl-k">{copy.structureChildren(routes.length)}</span>
          <span className="pl-v">{copy.structureChildrenNote}</span>
        </div>
        <div className="pl-evi-row">
          <span className="pl-k">{draft.mode === "conversion_only" ? "—" : copy.structureGates}</span>
          <span className="pl-v">{draft.mode === "conversion_only" ? copy.structureNoGates : copy.structureGatesNote}</span>
        </div>
      </div>
    </div>
  );
}

export function NewJobWizard(props: WizardProps) {
  const { copy, draft, preview, busy, onPreview, onQueueRun, onClose } = props;
  const [step, setStep] = useState(1);
  const [launching, setLaunching] = useState(false);

  // Entering the preflight step refreshes the route preview, and so does
  // changing anything this step feeds into it. The credential chips below are
  // the only such control, and every draft change clears the preview upstream,
  // so without re-running here a chip would empty the route table and leave
  // "Next" disabled with no way back except stepping out of the step and in
  // again. Keyed on the committed draft, so the request carries the new value
  // rather than racing it.
  useEffect(() => {
    if (step === 2) onPreview();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, draft.hasPaddleocrCredentials, draft.hasMineruCredentials]);

  const missing = sourceMissing(draft);
  const nextDisabled = missing || busy === "folder" || busy === "markdown" || busy === "zotero";
  const previewBusy = busy === "preview";
  // Holding the launch is the whole point of the ticket: without it the batch
  // runs OCR for as long as it takes and only then fails on provider auth.
  const credentialMissing = providerCredentialMissing(draft, props.modelSlots);

  const launch = async () => {
    setLaunching(true);
    try {
      const ok = await onQueueRun();
      if (ok) onClose();
    } finally {
      setLaunching(false);
    }
  };

  return (
    <div className="pl-takeover">
      <div className="pl-takeover-inner">
        <h2>{copy.wizardTitle}</h2>
        <p className="pl-tsub">{copy.wizardSub}</p>
        <StepIndicator copy={copy} step={step} />

        {step === 1 && <SourceStep {...props} />}
        {step === 2 && <PreflightStep {...props} />}
        {step === 3 && <ConfirmStep {...props} />}

        {missing && <p className="pl-wiz-error">{copy.sourceMissing}</p>}

        <div className="pl-wiz-foot">
          {step === 1 ? (
            <button className="pl-btn" type="button" onClick={onClose}>{copy.wizCancel}</button>
          ) : (
            <button className="pl-btn" type="button" onClick={() => setStep(step - 1)} disabled={launching}>
              {copy.wizBack}
            </button>
          )}
          {step === 1 && (
            <button className="pl-btn primary" type="button" disabled={nextDisabled} onClick={() => setStep(2)}>
              {copy.wizNextPreflight}
            </button>
          )}
          {step === 2 && (
            <button
              className="pl-btn primary"
              type="button"
              disabled={previewBusy || preview.length === 0}
              onClick={() => setStep(3)}
            >
              {copy.wizNextConfirm}
            </button>
          )}
          {step === 3 && (
            <button
              className="pl-btn primary"
              type="button"
              disabled={launching || credentialMissing || busy === "queue" || busy === "run"}
              onClick={() => void launch()}
            >
              {copy.wizLaunch}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
