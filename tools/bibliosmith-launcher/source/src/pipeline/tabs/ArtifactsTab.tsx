import { useState } from "react";
import { allArtifacts, hashShort, stageLabel } from "../model";
import { READER_EVIDENCE_ARTIFACT_KINDS, READER_EVIDENCE_CONCLUSIONS } from "../../types";
import type { PipelineCopy } from "../copy";
import type { BookUnit } from "../model";
import type { TabProps } from "./tabProps";

function validationMark(validation?: {
  exists: boolean;
  nonempty: boolean;
  hashMatches: boolean;
  requiredChecksPassed?: boolean | null;
} | null): "ok" | "failed" | "unknown" {
  if (!validation) return "unknown";
  const checks = [validation.exists, validation.nonempty, validation.hashMatches];
  if (validation.requiredChecksPassed !== null && validation.requiredChecksPassed !== undefined) {
    checks.push(validation.requiredChecksPassed);
  }
  if (checks.every(Boolean)) return "ok";
  return "failed";
}

export function ArtifactsTab(props: TabProps) {
  const { unit, copy } = props;
  const artifacts = allArtifacts(unit);
  const readerEvidence = (
    <ReaderEvidence
      unit={unit}
      copy={copy}
      busy={props.busy}
      onRecordReaderEvidence={props.onRecordReaderEvidence}
    />
  );
  if (!artifacts.length) {
    return (
      <>
        <p className="pl-muted-note">{copy.artifactsEmpty}</p>
        {readerEvidence}
      </>
    );
  }
  return (
    <>
    <div className="pl-card pl-art-wrap">
      <table className="pl-art-table">
        <thead>
          <tr>
            <th>{copy.thArtifact}</th>
            <th>{copy.thPath}</th>
            <th>{copy.thSha}</th>
            <th>{copy.thValidation}</th>
            <th>{copy.thProducer}</th>
          </tr>
        </thead>
        <tbody>
          {artifacts.map((artifact, index) => {
            const mark = validationMark(artifact.validation);
            return (
              <tr key={artifact.artifactId ?? `${artifact.kind}-${index}`}>
                <td className="pl-an">{artifact.kind}</td>
                <td className="pl-ap" title={artifact.path}>{artifact.path}</td>
                <td className="pl-ap">{hashShort(artifact.sha256)}</td>
                <td>
                  {mark === "ok" ? copy.validationOk : mark === "failed" ? copy.validationFailed : copy.validationUnknown}
                </td>
                <td>
                  {artifact.producer
                    ? `${stageLabel(artifact.producer.stageId, copy)} · ${copy.attemptLabel(artifact.producer.attempt)}`
                    : "—"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
    {readerEvidence}
    </>
  );
}


// The recording command has existed since the evidence landed; nothing called
// it, so the one thing this feature asks a human for could not be given.
function ReaderEvidence({
  unit,
  copy,
  busy,
  onRecordReaderEvidence,
}: {
  unit: BookUnit;
  copy: PipelineCopy;
  busy: TabProps["busy"];
  onRecordReaderEvidence: TabProps["onRecordReaderEvidence"];
}) {
  const [reader, setReader] = useState("");
  const [version, setVersion] = useState("");
  const [artifactKind, setArtifactKind] = useState<string>(READER_EVIDENCE_ARTIFACT_KINDS[0]);
  const [conclusion, setConclusion] = useState<string>(READER_EVIDENCE_CONCLUSIONS[0]);

  const childId = unit.child?.id ?? "";
  const evidence = unit.child?.readerEvidence ?? [];
  // Only artifacts a person can actually open, and only ones this book has built.
  const built = READER_EVIDENCE_ARTIFACT_KINDS.filter((kind) =>
    allArtifacts(unit).some((artifact) => artifact.kind === kind),
  );
  const pending = busy === "readerEvidence";
  const canRecord =
    Boolean(childId) && built.length > 0 && reader.trim() !== "" && version.trim() !== "" && !pending;

  return (
    <div className="pl-card pl-reader-evi">
      <strong>{copy.readerEvidenceTitle}</strong>
      <p className="pl-muted-note">{copy.readerEvidenceHint}</p>
      {evidence.length === 0 && <p className="pl-muted-note">{copy.readerEvidenceEmpty}</p>}
      {evidence.map((record) => (
        <div className="pl-evi-row" key={`${record.reader}-${record.artifactKind}`}>
          <span className="pl-k">
            {record.reader} {record.readerVersion}
          </span>
          <span className="pl-v">
            {record.artifactKind} ·{" "}
            {record.conclusion === "passed" ? copy.readerEvidencePassed : copy.readerEvidenceFailed} ·{" "}
            {hashShort(record.artifactSha256)}
            {record.stale ? ` · ${copy.readerEvidenceStale}` : ""}
          </span>
        </div>
      ))}
      {built.length === 0 ? (
        <p className="pl-muted-note">{copy.readerEvidenceNeedsBuild}</p>
      ) : (
        <div className="pl-evi-form">
          <label>
            {copy.readerEvidenceName}
            <input value={reader} onChange={(event) => setReader(event.target.value)} />
          </label>
          <label>
            {copy.readerEvidenceVersion}
            <input value={version} onChange={(event) => setVersion(event.target.value)} />
          </label>
          <label>
            {copy.readerEvidenceArtifact}
            <select value={artifactKind} onChange={(event) => setArtifactKind(event.target.value)}>
              {built.map((kind) => (
                <option key={kind} value={kind}>
                  {kind}
                </option>
              ))}
            </select>
          </label>
          <label>
            {copy.readerEvidenceConclusion}
            <select value={conclusion} onChange={(event) => setConclusion(event.target.value)}>
              {READER_EVIDENCE_CONCLUSIONS.map((value) => (
                <option key={value} value={value}>
                  {value === "passed" ? copy.readerEvidencePassed : copy.readerEvidenceFailed}
                </option>
              ))}
            </select>
          </label>
          <button
            className="pl-btn sm"
            type="button"
            disabled={!canRecord}
            onClick={() =>
              onRecordReaderEvidence(unit.job.id, childId, artifactKind, reader.trim(), version.trim(), conclusion)
            }
          >
            {copy.readerEvidenceRecord}
          </button>
        </div>
      )}
    </div>
  );
}
