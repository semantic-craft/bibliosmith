import type { BookPipelineState } from "../types";
import type { pipelineCopy } from "../pipeline";
import { jobStatusTone } from "../lib/pipeline-status";
import { PanelHeading } from "./PanelHeading";

export function PipelineSnapshotPanel({
  copy,
  state,
  onOpen,
}: {
  copy: ReturnType<typeof pipelineCopy>;
  state: BookPipelineState;
  onOpen: () => void;
}) {
  const latest = state.jobs[0] ?? null;
  const artifactCount = state.jobs.reduce((total, job) => total + job.artifacts.length, 0);
  const routeCount = latest?.route.length ?? 0;
  return (
    <section className="data-panel pipeline-snapshot-panel">
      <div className="panel-title-row">
        <PanelHeading title={copy.overviewTitle} />
        <button className="panel-button" type="button" onClick={onOpen}>{copy.openPipeline}</button>
      </div>
      {latest ? (
        <div className="pipeline-snapshot-body">
          <div className="pipeline-snapshot-latest">
            <span>{copy.latestJob}</span>
            <strong>{latest.source.title || latest.source.path || latest.source.selector || latest.source.kind}</strong>
            <code>{latest.currentStep}</code>
            <span className={`status-pill ${jobStatusTone(latest.status)}`}>{latest.status}</span>
          </div>
          <div className="pipeline-snapshot-metrics">
            <div>
              <span>{copy.jobs}</span>
              <strong>{state.jobs.length}</strong>
            </div>
            <div>
              <span>{copy.routePreview}</span>
              <strong>{routeCount}</strong>
            </div>
            <div>
              <span>{copy.artifacts}</span>
              <strong>{artifactCount}</strong>
            </div>
          </div>
        </div>
      ) : (
        <button className="pipeline-snapshot-empty" type="button" onClick={onOpen}>
          <span>{copy.noJobs}</span>
          <strong>{copy.openPipeline}</strong>
        </button>
      )}
    </section>
  );
}
