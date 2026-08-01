import { useEffect, useState } from "react";
import { readBookPipelineOcrSample } from "../api";
import type { BookPipelineOcrSampleEngine, BookPipelineOcrSampleReport } from "../types";
import type { PipelineCopy } from "./copy";
import { allArtifacts, unitRoute, type BookUnit, type PipelineBusy } from "./model";

/** Engine name to the route-override token apply_route_overrides accepts. */
const ROUTE_TOKEN: Record<BookPipelineOcrSampleEngine["engine"], string> = {
  paddleocr: "paddle",
  mineru: "mineru",
};

const ENGINE_LABEL: Record<BookPipelineOcrSampleEngine["engine"], string> = {
  paddleocr: "PaddleOCR",
  mineru: "MinerU",
};

/**
 * Route kinds where choosing an OCR engine is still an open question.
 *
 * All four are in OVERRIDABLE_ROUTE_KINDS on the Rust side, so a pick here can
 * actually be written. `blocked_dirty_text_layer` belongs precisely because it
 * is the case where the choice matters most — the book's own text layer is
 * unusable and an engine has to be picked — and it is what the blocked-state
 * buttons already offer paddle/mineru for. `direct_text` is deliberately absent:
 * that book has a good text layer and needs no OCR, so comparing engines would
 * spend on pages nothing will use.
 */
const OCR_ROUTE_KINDS = new Set([
  "remote_paddleocr",
  "mineru",
  "missing_credentials",
  "blocked_dirty_text_layer",
]);

const DEFAULT_SAMPLE_PAGES = 3;
const MAX_SAMPLE_PAGES = 10;

// The report is read by artifact digest, so a stale render can be told from a
// current one without an effect resetting state between books.
type ReportState = { version: string; report: BookPipelineOcrSampleReport };

function ocrSampleArtifact(unit: BookUnit) {
  return (
    allArtifacts(unit).find(
      (artifact) => artifact.kind === "ocr_sample_report" && !artifact.supersededBy,
    ) ?? null
  );
}

/**
 * The local file a source reference points at, or null when it names none.
 *
 * Mirrors ocr_sample_local_path in book_pipeline.rs: zotero_source_ref appends
 * `#source_md5=<fingerprint>` to the attachment's storage path, and falls back
 * to a `zotero://attachment/<key>` URI when Zotero reported no path at all.
 * Stripped by exact suffix rather than by splitting on `#`, because `#` is legal
 * in a filename.
 */
function localPdfPath(sourceRef: string | null | undefined): string | null {
  if (!sourceRef) return null;
  const marker = sourceRef.lastIndexOf("#source_md5=");
  const path = (marker < 0 ? sourceRef : sourceRef.slice(0, marker)).trim();
  if (!path || path.startsWith("zotero://")) return null;
  return /\.pdf$/i.test(path) ? path : null;
}

/**
 * Whether comparing engines can still change anything for this book.
 *
 * Two conditions, both mirroring what the backend enforces so the card never
 * offers a button that errors:
 *
 * - Extraction has not started. The backend refuses a sample once it is running
 *   or done, because by then the engine has been chosen and the comparison
 *   would answer a closed question.
 * - There is a local PDF to sample. A Zotero attachment whose file was never
 *   synced carries only a `zotero://` reference, which the backend rejects with
 *   "not stored locally" — knowable from state, so it is not worth a paid
 *   round trip to discover.
 */
export function canCompareOcrEngines(unit: BookUnit): boolean {
  const route = unitRoute(unit);
  if (!route || !OCR_ROUTE_KINDS.has(route.routeKind)) return false;
  const child = unit.child;
  const extract = child?.stages.find((stage) => stage.stageId === "extract");
  if (!extract) return false;
  if (extract.status === "running" || extract.status === "completed") return false;
  // Same references, same order, as ocr_sample_source_pdf walks.
  return [child?.source.path, ...(child?.route ?? []).map((item) => item.sourceRef)].some(
    (reference) => localPdfPath(reference) !== null,
  );
}

/**
 * One engine's pane.
 *
 * The choice is a real radio rather than a clickable card: the two engines are
 * mutually exclusive, which is what a radio group means, and it keeps the
 * excerpt out of the control's accessible name. Wrapping the whole pane in a
 * button made the excerpt — up to 4000 characters of the book — the button's
 * name, and ARIA's "children presentational" rule erased the engine heading
 * that names it.
 */
function EnginePane({
  result,
  copy,
  groupName,
  picked,
  onPick,
}: {
  result: BookPipelineOcrSampleEngine;
  copy: PipelineCopy;
  groupName: string;
  picked: boolean;
  onPick: () => void;
}) {
  const failed = result.status === "failed";
  return (
    <div className={`pl-ocrpane${picked ? " picked" : ""}${failed ? " failed" : ""}`}>
      <label className="pl-ocrpane-head">
        <input
          type="radio"
          name={groupName}
          checked={picked}
          // A failed engine is shown so its reason is visible, but it cannot be
          // chosen: there is no evidence it converts this book.
          disabled={failed}
          onChange={onPick}
        />
        <span className="pl-ocrpane-name">{ENGINE_LABEL[result.engine]}</span>
      </label>
      {failed ? (
        <p className="pl-ocrpane-error">
          {copy.ocrCompareFailed}
          {result.error ? `: ${result.error}` : ""}
        </p>
      ) : (
        <>
          <div className="pl-ocrpane-meta">
            <span>
              {result.characterCount} {copy.ocrCompareCharacters}
            </span>
            <span>
              {(result.elapsedMs / 1000).toFixed(1)}
              {copy.ocrCompareSeconds}
            </span>
          </div>
          {/* Focusable so the excerpt can be scrolled from the keyboard; it
              overflows for any real page of text. dir=auto because OCR output
              is whatever language the book is in. */}
          <pre className="pl-ocrpane-text" tabIndex={0} dir="auto">
            {result.markdownExcerpt.trim() || copy.ocrCompareEmpty}
          </pre>
        </>
      )}
    </div>
  );
}

export function OcrCompareCard({
  unit,
  copy,
  busy,
  onSampleOcr,
  onRouteOverride,
}: {
  unit: BookUnit;
  copy: PipelineCopy;
  busy: PipelineBusy;
  onSampleOcr: (jobId: string, childId: string, samplePages: number) => void;
  onRouteOverride: (jobId: string, childId: string, routeItemId: string, routeOverride: string) => void;
}) {
  const route = unitRoute(unit);
  const childId = unit.child?.id ?? null;
  const artifact = ocrSampleArtifact(unit);
  const version = artifact?.sha256 ?? artifact?.artifactId ?? null;
  const [reportState, setReportState] = useState<ReportState | null>(null);
  const report = reportState?.version === version ? reportState.report : null;
  // Held as text so the field can be cleared and retyped. Clamping on every
  // keystroke instead would rewrite an empty box to 1, and typing "5" into it
  // would land on 15 — the user has to delete a digit to enter a number.
  const [pagesText, setPagesText] = useState(String(DEFAULT_SAMPLE_PAGES));
  const typed = Math.round(Number(pagesText));
  const pages = Number.isFinite(typed) && typed >= 1
    ? Math.min(typed, MAX_SAMPLE_PAGES)
    : DEFAULT_SAMPLE_PAGES;
  // Keyed on the report it belongs to, so a fresh comparison does not inherit
  // the previous one's selection.
  const [choice, setChoice] = useState<{ version: string; engine: string } | null>(null);
  const picked = choice?.version === version ? choice.engine : null;

  useEffect(() => {
    if (!childId || !version) return undefined;
    let cancelled = false;
    readBookPipelineOcrSample(unit.job.id, childId)
      .then((next) => {
        if (!cancelled) setReportState({ version, report: next });
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [childId, version, unit.job.id]);

  if (!childId || !route) return null;

  const running = busy === "sample";
  return (
    <div className="pl-card pl-ocrcard">
      <h4 className="pl-card-title">{copy.ocrCompareTitle}</h4>
      <p className="pl-ocrcard-lead">{copy.ocrCompareLead}</p>
      <div className="pl-ocrcard-controls">
        <label>
          <span>{copy.ocrComparePages}</span>
          <input
            type="number"
            min={1}
            max={MAX_SAMPLE_PAGES}
            value={pagesText}
            disabled={running}
            onChange={(event) => setPagesText(event.target.value)}
            // Snap back to what will actually be sent, so the box never keeps
            // showing a number the run would not use.
            onBlur={() => setPagesText(String(pages))}
          />
        </label>
        <button
          className="pl-btn sm"
          type="button"
          disabled={busy !== null}
          onClick={() => onSampleOcr(unit.job.id, childId, pages)}
        >
          {report ? copy.ocrCompareRetry : copy.ocrCompareRun}
        </button>
      </div>
      {report && (
        <>
          <div className="pl-evi-row">
            <span className="pl-k">{copy.ocrCompareSampledPages}</span>
            <span className="pl-v">
              {report.sampledPages.join(" · ")} / {report.totalPages}
            </span>
          </div>
          {/* A native radio group: same name, so exactly one engine is chosen
              and screen readers announce it as "1 of 2". Scoped to this child
              so two open books never share a group. */}
          <div className="pl-ocrpanes" role="radiogroup" aria-label={copy.ocrCompareTitle}>
            {report.engines.map((result) => (
              <EnginePane
                key={result.engine}
                result={result}
                copy={copy}
                groupName={`ocr-engine-${childId}`}
                picked={picked === result.engine}
                onPick={() => version && setChoice({ version, engine: result.engine })}
              />
            ))}
          </div>
          <div className="pl-ocrcard-actions">
            <button
              className="pl-btn sm primary"
              type="button"
              disabled={busy !== null || !picked}
              onClick={() => {
                if (!picked) return;
                onRouteOverride(
                  unit.job.id,
                  childId,
                  route.id,
                  ROUTE_TOKEN[picked as BookPipelineOcrSampleEngine["engine"]],
                );
              }}
            >
              {copy.ocrComparePick}
            </button>
          </div>
        </>
      )}
      <p className="pl-gprivacy">{copy.ocrCompareNote}</p>
    </div>
  );
}
