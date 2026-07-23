import type { PipelineCopy } from "../copy";
import {
  approvalRecords,
  focusStages,
  formatTime,
  gateLabel,
  hashShort,
  pendingGates,
  type BookUnit,
  type GateView,
} from "../model";
import type { TabProps } from "./tabProps";

function EventStrip({ unit, gate, copy }: { unit: BookUnit; gate: GateView; copy: PipelineCopy }) {
  const stages = focusStages(unit);
  const index = stages.findIndex((stage) => stage.stageId === gate.stageId);
  const previous = index > 0 ? stages[index - 1] : null;
  const isTranslation = gate.stageId === "approve_translation";
  return (
    <div className="pl-estrip">
      <div className="pl-estep">
        <span className="pl-edot">✓</span>
        <div>
          <div className="pl-et">{isTranslation ? copy.eventPrereqTranslation : copy.eventPrereqPromotion}</div>
          <div className="pl-ett pl-num">{formatTime(previous?.finishedAt)}</div>
        </div>
      </div>
      <span className="pl-esep done" />
      <div className="pl-estep">
        <span className="pl-edot">✓</span>
        <div>
          <div className="pl-et">{copy.eventPacket}</div>
          <div className="pl-ett pl-num">{formatTime(gate.stage.startedAt)}</div>
        </div>
      </div>
      {gate.invalidated ? (
        <>
          <span className="pl-esep done" />
          <div className="pl-estep deadstep">
            <span className="pl-edot">✗</span>
            <div>
              <div className="pl-et">{copy.eventInvalidated}</div>
              <div className="pl-ett pl-num">{formatTime(unit.job.updatedAt)}</div>
            </div>
          </div>
          <span className="pl-esep" />
          <div className="pl-estep future">
            <span className="pl-edot" />
            <div>
              <div className="pl-et">{copy.eventDecision}</div>
              <div className="pl-ett">{copy.eventRepackage}</div>
            </div>
          </div>
        </>
      ) : (
        <>
          <span className="pl-esep done" />
          <div className="pl-estep now">
            <span className="pl-edot">…</span>
            <div>
              <div className="pl-et">{copy.eventDecision}</div>
              <div className="pl-ett">{copy.eventNow}</div>
            </div>
          </div>
          <span className="pl-esep" />
          <div className="pl-estep future">
            <span className="pl-edot" />
            <div>
              <div className="pl-et">{isTranslation ? copy.eventAfterTranslation : copy.eventAfterPromotion}</div>
              <div className="pl-ett">{copy.afterApproval}</div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function DiffGrid({ unit, gate, copy }: { unit: BookUnit; gate: GateView; copy: PipelineCopy }) {
  const isTranslation = gate.stageId === "approve_translation";
  const formats = (unit.job.outputFormats ?? []).map((format) => format.toUpperCase()).join(" / ");
  return (
    <div className="pl-diff-grid">
      <div className="pl-diff-col">
        <h5>{copy.beforeNow}</h5>
        {isTranslation ? (
          <>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.diffSourceText}</span>
              <span className="pl-v">{copy.diffSourceLocal}</span>
            </div>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.diffChapterUnits}</span>
              <span className="pl-v">{copy.diffUnitsReady}</span>
            </div>
          </>
        ) : (
          <>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.diffTranslated}</span>
              <span className="pl-v">{copy.diffTranslatedDraft}</span>
            </div>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.diffReading}</span>
              <span className="pl-v">{copy.diffReadingNone}</span>
            </div>
          </>
        )}
      </div>
      <div className="pl-diff-arrow">→</div>
      <div className="pl-diff-col after">
        <h5>{copy.afterWill}</h5>
        {isTranslation ? (
          <>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.diffSourceText}</span>
              <span className="pl-v">
                {copy.diffSourceSend}{unit.job.translationProfileId ? ` · ${unit.job.translationProfileId}` : ""}
              </span>
            </div>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.diffChapterUnits}</span>
              <span className="pl-v">{copy.diffUnitsStart}</span>
            </div>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.reversibility}</span>
              <span className="pl-v">{copy.irreversibleSend}</span>
            </div>
          </>
        ) : (
          <>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.diffTranslated}</span>
              <span className="pl-v">{copy.diffPromoteFinal}</span>
            </div>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.diffReading}</span>
              <span className="pl-v">{formats ? `${formats} · ${copy.diffReadingBuilt}` : copy.diffReadingBuilt}</span>
            </div>
            <div className="pl-diff-row">
              <span className="pl-k">{copy.reversibility}</span>
              <span className="pl-v">{copy.finalOnlyByApproval}</span>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function GateComposite({
  unit,
  gate,
  copy,
  busy,
  onOpenOutput,
  onApproveGate,
}: {
  unit: BookUnit;
  gate: GateView;
  copy: PipelineCopy;
  busy: TabProps["busy"];
  onOpenOutput: (jobId: string) => void;
  onApproveGate: TabProps["onApproveGate"];
}) {
  const passed = gate.checks.filter((check) => check.ok === true).length;
  const failed = gate.checks.filter((check) => check.ok === false).length;
  const scope = gate.stage.unitSummary
    ? `${gate.stage.unitSummary.completed}/${gate.stage.unitSummary.total}`
    : gate.boundEntries.length
      ? String(gate.boundEntries.length)
      : "—";
  return (
    <div>
      <div style={{ display: "flex", alignItems: "baseline", gap: 10 }}>
        <span className={`pl-gate-tag${gate.invalidated ? " dead" : ""}`}>
          {gate.invalidated ? `${copy.invalidPrefix} · ` : `${copy.humanGatePrefix} · `}
          {gateLabel(gate.stageId, copy)}
        </span>
        <span style={{ fontSize: 12, color: "var(--muted)" }}>
          {copy.scopeLabel} {scope}
        </span>
      </div>

      <EventStrip unit={unit} gate={gate} copy={copy} />

      <div className="pl-cklist">
        {gate.checks.map((check) => (
          <div key={check.title} className={`pl-ckrow${check.ok === false ? " fail" : check.ok === "unknown" ? " unknown" : ""}`}>
            <span className="pl-ckicon">{check.ok === true ? "✓" : check.ok === false ? "✗" : "—"}</span>
            <div>
              <b>{check.title}</b>
              <span>{check.detail}</span>
            </div>
          </div>
        ))}
      </div>

      <DiffGrid unit={unit} gate={gate} copy={copy} />

      {gate.invalidated ? (
        <div className="pl-appr-foot">
          <button className="pl-btn primary" type="button" disabled title={copy.runnerPendingNote}>
            {copy.regeneratePacket}
          </button>
          <span className="pl-summary" style={{ color: "var(--pl-failed)" }}>
            {failed} {copy.checksFailedNote}
          </span>
        </div>
      ) : (
        <div className="pl-appr-foot">
          <button
            className="pl-btn primary"
            type="button"
            disabled={busy === "gateApproval" || failed > 0 || !unit.child}
            onClick={() => unit.child && onApproveGate(unit.job.id, unit.child.id, gate.stageId)}
          >
            {copy.approve}
          </button>
          <button className="pl-btn danger-ghost" type="button" disabled title={copy.runnerPendingNote}>
            {copy.reject}
          </button>
          {unit.job.openTarget && (
            <button className="pl-btn quiet" type="button" onClick={() => onOpenOutput(unit.job.id)}>
              {copy.openPacket}
            </button>
          )}
          <span className="pl-summary pl-num">{copy.checksPassed(passed, gate.checks.length)}</span>
        </div>
      )}
      {gate.invalidated && <p className="pl-appr-note">{copy.approvalActionPending}</p>}
    </div>
  );
}

function ApprovalRecordsView({ unit, copy }: { unit: BookUnit; copy: PipelineCopy }) {
  const records = approvalRecords(unit);
  if (!records.length) return null;
  return (
    <div>
      <p className="pl-muted-note" style={{ marginBottom: 10 }}>{copy.approvedRecordsNote}</p>
      <div className="pl-cklist">
        {records.map((record) => {
          const bound = Object.entries(record.boundArtifactHashes)
            .map(([artifactId, hash]) => `${artifactId} ${hashShort(hash)}`)
            .join(" · ");
          return (
            <div key={record.approvalId} className={`pl-ckrow${record.decision === "rejected" ? " fail" : ""}`}>
              <span className="pl-ckicon">{record.decision === "rejected" ? "✗" : "✓"}</span>
              <div>
                <b>
                  {gateLabel(record.stageId, copy)} · {record.decision === "rejected" ? copy.decisionRejected : copy.decisionApproved}
                </b>
                <span>
                  {bound ? `${copy.boundWith} ${bound}` : record.approvalId}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function ApprovalTab({ unit, copy, busy, onOpenOutput, onApproveGate }: TabProps) {
  const gates = pendingGates(unit, copy);
  return (
    <div style={{ display: "grid", gap: 22 }}>
      {gates.length === 0 && approvalRecords(unit).length === 0 && (
        <p className="pl-muted-note">{copy.approvalEmpty}</p>
      )}
      {gates.map((gate) => (
        <GateComposite
          key={gate.stageId}
          unit={unit}
          gate={gate}
          copy={copy}
          busy={busy}
          onOpenOutput={onOpenOutput}
          onApproveGate={onApproveGate}
        />
      ))}
      <ApprovalRecordsView unit={unit} copy={copy} />
    </div>
  );
}
