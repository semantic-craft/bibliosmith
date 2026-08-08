import type { BookPipelineJob } from "../types";

export function jobStatusTone(status: string) {
  if (pipelineJobSucceeded(status)) return "success";
  if (status === "failed" || status === "blocked" || status === "partial") return "warning";
  if (status === "running" || status === "ready" || status === "waiting_for_approval") return "blue";
  return "muted";
}

export function pipelineJobSucceeded(status: string) {
  return status === "completed" || status === "partial";
}

export function pipelineJobOutcomeSucceeded(job: BookPipelineJob) {
  return pipelineJobSucceeded(job.status) || translationHandoffReady(job);
}

/**
 * Mirrors job_is_actively_running in book_pipeline.rs, the three conditions and
 * the removed-child exclusion included. A job whose own status has moved on can
 * still have a stage executing under it, and handoff_running is a running state
 * that does not say "running", so `job.status === "running"` answers no for work
 * that is very much in flight.
 *
 * The update install reads this: replacing the App bundle takes the Python,
 * Node and Chromium a running stage is executing with it.
 */
export function pipelineJobActivelyRunning(job: BookPipelineJob) {
  if (job.status === "running" || job.status === "handoff_running") return true;
  return job.children.some((child) =>
    !child.removedAt && child.stages.some((stage) => stage.status === "running")
  );
}

export function translationHandoffReady(job: BookPipelineJob) {
  return job.children.some((child) =>
    child.stages.some((stage) => stage.stageId === "handoff" && stage.status === "completed")
    && child.currentStageId === "split"
    && child.status === "ready"
  );
}
