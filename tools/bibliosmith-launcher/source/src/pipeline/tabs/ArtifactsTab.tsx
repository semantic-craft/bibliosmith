import { allArtifacts, hashShort, stageLabel } from "../model";
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

export function ArtifactsTab({ unit, copy }: TabProps) {
  const artifacts = allArtifacts(unit);
  if (!artifacts.length) {
    return <p className="pl-muted-note">{copy.artifactsEmpty}</p>;
  }
  return (
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
  );
}
