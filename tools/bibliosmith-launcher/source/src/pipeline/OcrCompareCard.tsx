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

/** Route kinds where choosing an OCR engine is still an open question. */
const OCR_ROUTE_KINDS = new Set(["remote_paddleocr", "mineru", "missing_credentials"]);

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
 * Whether comparing engines can still change anything for this book.
 *
 * The backend refuses a sample once extraction is running or done — by then the
 * engine has been chosen and the comparison would answer a closed question — so
 * the card follows the same rule rather than offering a button that errors.
 */
export function canCompareOcrEngines(unit: BookUnit): boolean {
  const route = unitRoute(unit);
  if (!route || !OCR_ROUTE_KINDS.has(route.routeKind)) return false;
  const extract = unit.child?.stages.find((stage) => stage.stageId === "extract");
  if (!extract) return false;
  return extract.status !== "running" && extract.status !== "completed";
}

function EnginePane({
  result,
  copy,
  picked,
  onPick,
}: {
  result: BookPipelineOcrSampleEngine;
  copy: PipelineCopy;
  picked: boolean;
  onPick: () => void;
}) {
  const failed = result.status === "failed";
  return (
    <button
      type="button"
      className={`pl-ocrpane${picked ? " picked" : ""}${failed ? " failed" : ""}`}
      // A failed engine is shown so its reason is visible, but it cannot be
      // chosen: there is no evidence it converts this book.
      disabled={failed}
      aria-pressed={picked}
      onClick={onPick}
    >
      <div className="pl-ocrpane-head">
        <h5>{ENGINE_LABEL[result.engine]}</h5>
        {picked && <span className="pl-ocrpane-tag">{copy.ocrComparePicked}</span>}
      </div>
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
          <pre className="pl-ocrpane-text">
            {result.markdownExcerpt.trim() || copy.ocrCompareEmpty}
          </pre>
        </>
      )}
    </button>
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
          <div className="pl-ocrpanes">
            {report.engines.map((result) => (
              <EnginePane
                key={result.engine}
                result={result}
                copy={copy}
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
