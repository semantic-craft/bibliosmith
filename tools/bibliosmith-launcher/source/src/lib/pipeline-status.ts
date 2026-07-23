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

export function translationHandoffReady(job: BookPipelineJob) {
  return job.children.some((child) =>
    child.stages.some((stage) => stage.stageId === "handoff" && stage.status === "completed")
    && child.currentStageId === "split"
    && child.status === "ready"
  );
}
