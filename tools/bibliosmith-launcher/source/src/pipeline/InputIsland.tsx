import { useEffect, useState } from "react";
import { FolderOpen, Search, Upload } from "lucide-react";
import type { BookPipelineRouteItem, BookPipelineSource, ModelSlotView } from "../types";
import { isTauriRuntime } from "../api";
import type { PipelineCopy } from "./copy";
import {
  providerCredentialMissing,
  routeKindLabel,
  routeTone,
  type PipelineBusy,
  type PipelineDraft,
  type RouteOverride,
} from "./model";

export type InputIslandProps = {
  copy: PipelineCopy;
  draft: PipelineDraft;
  preview: BookPipelineRouteItem[];
  zoteroSources: BookPipelineSource[];
  modelSlots: ModelSlotView[];
  busy: PipelineBusy;
  // "hero" is the empty shelf: the island is the whole page. "compact" is the
  // top strip above a populated shelf.
  variant: "hero" | "compact";
  // Recedes while the drawer holds the focus (Island.inactiveAlpha).
  dimmed?: boolean;
  onDraftChange: (patch: Partial<PipelineDraft>) => void;
  onPreview: () => void;
  onQueueRun: () => Promise<boolean>;
  onChooseFolder: () => void;
  onSearchZotero: (query: string) => void;
  routeOverrides: Record<string, RouteOverride>;
  onRouteOverrideChange: (routeItemId: string, override: RouteOverride) => void;
};

const ZOTERO_KINDS = new Set(["zotero_attachment", "zotero_collection", "zotero_filter"]);

function isZoteroDraft(draft: PipelineDraft): boolean {
  return ZOTERO_KINDS.has(draft.sourceKind);
}

export function hasSource(draft: PipelineDraft): boolean {
  if (isZoteroDraft(draft)) return Boolean(draft.zoteroSelector);
  return Boolean(draft.localPdfFolder);
}

/**
 * Identity of the thing the preflight is about, so the auto-preview effect
 * re-runs when the user swaps books rather than only when one first appears.
 * A plain "have I previewed yet" boolean would swallow the second choice.
 */
export function sourceIdentity(draft: PipelineDraft): string {
  return isZoteroDraft(draft)
    ? `${draft.sourceKind}:${draft.zoteroSelector}`
    : `local:${draft.localPdfFolder}`;
}

function basename(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut >= 0 ? path.slice(cut + 1) : path;
}

/**
 * Tauri hands over absolute paths and the frontend cannot stat them, so a path
 * ending in .pdf is taken to be a file and the batch becomes its folder — the
 * conversion wrapper only ever takes a directory. Nothing is hidden by that:
 * the preflight list below names every book the folder will run, before
 * anything is queued.
 */
export function droppedFolder(paths: string[]): string | null {
  const first = paths.find((path) => path.trim().length > 0);
  if (!first) return null;
  if (!/\.pdf$/i.test(first)) return first;
  const cut = Math.max(first.lastIndexOf("/"), first.lastIndexOf("\\"));
  return cut > 0 ? first.slice(0, cut) : null;
}

function executionRoutes(preview: BookPipelineRouteItem[]): BookPipelineRouteItem[] {
  return preview.filter((item) => item.routeKind !== "translation_handoff");
}

function ZoteroSearchBox({
  copy,
  busy,
  onSearch,
}: {
  copy: PipelineCopy;
  busy: PipelineBusy;
  onSearch: (query: string) => void;
}) {
  // Local state on purpose: the query is a title, not the selector the draft
  // carries, and writing every keystroke into the draft would corrupt the key
  // the preflight is running against.
  const [query, setQuery] = useState("");
  const submit = () => {
    const trimmed = query.trim();
    if (trimmed) onSearch(trimmed);
  };
  return (
    <div className="pl-island-search">
      <Search size={15} className={busy === "zotero" ? "spin-icon" : undefined} aria-hidden />
      <input
        type="text"
        aria-label={copy.zoteroTitleSearch}
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
      <button
        className="pl-btn sm"
        type="button"
        disabled={busy === "zotero" || !query.trim()}
        onClick={submit}
      >
        {copy.zoteroTitleSearchButton}
      </button>
    </div>
  );
}

function RoutePreflight({
  copy,
  preview,
  busy,
  routeOverrides,
  onRouteOverrideChange,
}: {
  copy: PipelineCopy;
  preview: BookPipelineRouteItem[];
  busy: PipelineBusy;
  routeOverrides: Record<string, RouteOverride>;
  onRouteOverrideChange: (routeItemId: string, override: RouteOverride) => void;
}) {
  const routes = executionRoutes(preview);
  if (!routes.length) {
    return (
      <p className="pl-island-note">{busy === "preview" ? copy.islandPreflightBusy : copy.noPreview}</p>
    );
  }
  return (
    <div className="pl-island-preflight">
      <table className="pl-route-table">
        <thead>
          <tr>
            <th>{copy.thBook}</th>
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
                <span className={`pl-chip ${routeTone(item.routeKind)}`}>
                  {routeKindLabel(item.routeKind, copy)}
                </span>
              </td>
              <td>
                <select
                  aria-label={copy.thOverride}
                  value={routeOverrides[item.id] ?? "auto"}
                  onChange={(event) =>
                    onRouteOverrideChange(item.id, event.target.value as RouteOverride)
                  }
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
                  <span className="pl-chip block" title={item.blockedReason ?? undefined}>
                    {copy.preflightBlocked}
                  </span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function InputIsland(props: InputIslandProps) {
  const {
    copy,
    draft,
    preview,
    zoteroSources,
    busy,
    variant,
    dimmed,
    onDraftChange,
    onPreview,
    onQueueRun,
    onChooseFolder,
    onSearchZotero,
    routeOverrides,
    onRouteOverrideChange,
  } = props;
  const [dragging, setDragging] = useState(false);
  const [launching, setLaunching] = useState(false);

  const identity = sourceIdentity(draft);
  const sourceChosen = hasSource(draft);

  // Preflight runs in place, so it has to follow the source rather than a
  // button. Keyed on the source identity and on the credential flags, because
  // App clears the preview on every draft change: without the credential keys a
  // key configured in Settings would blank the route list with nothing to
  // refill it.
  useEffect(() => {
    if (!sourceChosen) return;
    onPreview();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [identity, draft.hasPaddleocrCredentials, draft.hasMineruCredentials]);

  // Native drag-drop: Tauri suppresses the HTML5 events and reports paths on
  // its own channel. The listener is webview-wide, which suits an app whose
  // only drop target is "add this book".
  useEffect(() => {
    if (!isTauriRuntime()) return undefined;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === "over") {
            setDragging(true);
            return;
          }
          setDragging(false);
          if (event.payload.type !== "drop") return;
          const folder = droppedFolder(event.payload.paths);
          if (!folder) return;
          onDraftChange({
            sourceKind: "local_pdf_folder",
            localPdfFolder: folder,
            localPdfTitle: basename(folder) || "Local PDF folder",
          });
        }),
      )
      .then((unlisten) => {
        if (cancelled) unlisten();
        else dispose = unlisten;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      dispose?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const routes = executionRoutes(preview);
  const runnable = routes.filter((item) => item.canRun);
  const credentialMissing = providerCredentialMissing(draft, props.modelSlots);
  const launchDisabled =
    launching ||
    !sourceChosen ||
    credentialMissing ||
    runnable.length === 0 ||
    busy === "queue" ||
    busy === "run" ||
    busy === "preview";

  const launch = async () => {
    setLaunching(true);
    try {
      // Clearing the source on success is what collapses the island back to a
      // top strip: the book now lives on the shelf, and leaving the preflight
      // standing would invite queueing the same batch twice.
      if (await onQueueRun()) onDraftChange({ localPdfFolder: "", zoteroSelector: "" });
    } finally {
      setLaunching(false);
    }
  };

  const zoteroDraft = isZoteroDraft(draft);
  const chosenLabel = zoteroDraft ? draft.zoteroSelector : draft.localPdfFolder;
  // Nothing picked and nothing to pick from: the compact island has no reason
  // to be more than one line.
  const idle = !sourceChosen && zoteroSources.length === 0;

  return (
    <section
      className={`pl-island pl-input-island ${variant}${idle ? " idle" : ""}${dragging ? " dragging" : ""}${dimmed ? " dimmed" : ""}`}
      aria-label={copy.newJob}
    >
      <div className="pl-island-head">
        <Upload size={variant === "hero" ? 20 : 16} aria-hidden />
        <span className="pl-island-hint">
          {dragging ? copy.islandDropActive : copy.islandDropHint}
        </span>
        <button className="pl-btn sm" type="button" disabled={busy === "folder"} onClick={onChooseFolder}>
          <FolderOpen size={14} />
          {copy.chooseFolder}
        </button>
      </div>

      <ZoteroSearchBox copy={copy} busy={busy} onSearch={onSearchZotero} />

      {zoteroSources.length > 0 && (
        <div className="pl-island-results" role="listbox" aria-label={copy.islandZoteroResults}>
          {zoteroSources.map((source) => {
            const selector = source.selector || source.title || "";
            const picked = zoteroDraft && draft.zoteroSelector === selector;
            return (
              <button
                key={`${source.kind}:${selector}`}
                className={`pl-island-result${picked ? " picked" : ""}`}
                type="button"
                role="option"
                aria-selected={picked}
                onClick={() => onDraftChange({ sourceKind: source.kind, zoteroSelector: selector })}
              >
                {source.title || selector || source.kind}
              </button>
            );
          })}
        </div>
      )}

      {sourceChosen && (
        <div className="pl-island-chosen">
          <span className="pl-k">{zoteroDraft ? copy.batchSource : copy.islandDroppedFolder}</span>
          <span className="pl-v pl-mono">{chosenLabel}</span>
        </div>
      )}

      {sourceChosen && !zoteroDraft && <p className="pl-island-note">{copy.islandFolderBatchNote}</p>}

      {sourceChosen && (
        <RoutePreflight
          copy={copy}
          preview={preview}
          busy={busy}
          routeOverrides={routeOverrides}
          onRouteOverrideChange={onRouteOverrideChange}
        />
      )}

      <div className="pl-island-foot">
        <span className={`pl-chip ${draft.hasPaddleocrCredentials ? "ok" : "neutral"}`} title={copy.islandOcrHint}>
          {copy.paddleCreds} · {draft.hasPaddleocrCredentials ? copy.islandOcrReady : copy.islandOcrMissing}
        </span>
        <span className={`pl-chip ${draft.hasMineruCredentials ? "ok" : "neutral"}`} title={copy.islandOcrHint}>
          {copy.mineruCreds} · {draft.hasMineruCredentials ? copy.islandOcrReady : copy.islandOcrMissing}
        </span>
        <span className="pl-spacer" />
        <button className="pl-btn primary" type="button" disabled={launchDisabled} onClick={() => void launch()}>
          {runnable.length ? copy.islandEnqueue(runnable.length) : copy.islandEnqueueEmpty}
        </button>
      </div>

      {credentialMissing && <p className="pl-island-error">{copy.providerKeyMissing}</p>}
    </section>
  );
}
