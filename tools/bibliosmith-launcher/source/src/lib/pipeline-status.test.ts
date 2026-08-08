import { describe, expect, it } from "vitest";
import {
  jobStatusTone,
  pipelineJobActivelyRunning,
  pipelineJobOutcomeSucceeded,
  pipelineJobSucceeded,
  translationHandoffReady,
} from "./pipeline-status";
import { childJob, job, stage } from "../test/fixtures";

describe("pipelineJobSucceeded", () => {
  it("counts a partial run as a success", () => {
    expect(pipelineJobSucceeded("completed")).toBe(true);
    expect(pipelineJobSucceeded("partial")).toBe(true);
  });

  it("counts everything still open or broken as not a success", () => {
    for (const status of ["pending", "ready", "running", "waiting_for_approval", "blocked", "failed", "skipped"]) {
      expect(pipelineJobSucceeded(status), status).toBe(false);
    }
  });
});

describe("jobStatusTone", () => {
  it("maps each status family to its tone", () => {
    expect(jobStatusTone("completed")).toBe("success");
    expect(jobStatusTone("partial")).toBe("success");
    expect(jobStatusTone("failed")).toBe("warning");
    expect(jobStatusTone("blocked")).toBe("warning");
    expect(jobStatusTone("running")).toBe("blue");
    expect(jobStatusTone("ready")).toBe("blue");
    expect(jobStatusTone("waiting_for_approval")).toBe("blue");
    expect(jobStatusTone("pending")).toBe("muted");
    expect(jobStatusTone("some_future_status")).toBe("muted");
  });
});

describe("translationHandoffReady", () => {
  /** A child parked after handoff, waiting for the translation half to start. */
  const readyChild = () =>
    childJob({
      status: "ready",
      currentStageId: "split",
      stages: [stage("handoff", "completed"), stage("split", "ready")],
    });

  it("is true for a child parked at split with handoff done", () => {
    expect(translationHandoffReady(job({ children: [readyChild()] }))).toBe(true);
  });

  it("is false while handoff itself has not finished", () => {
    const child = childJob({
      status: "ready",
      currentStageId: "split",
      stages: [stage("handoff", "running")],
    });
    expect(translationHandoffReady(job({ children: [child] }))).toBe(false);
  });

  it("is false once the child has moved past split", () => {
    const child = childJob({
      status: "running",
      currentStageId: "translate",
      stages: [stage("handoff", "completed")],
    });
    expect(translationHandoffReady(job({ children: [child] }))).toBe(false);
  });

  it("is true when any one child of a collection is parked there", () => {
    const other = childJob({ id: "child-2", status: "running", currentStageId: "extract" });
    expect(translationHandoffReady(job({ children: [other, readyChild()] }))).toBe(true);
  });

  it("is false for a job with no children", () => {
    expect(translationHandoffReady(job({ children: [] }))).toBe(false);
  });
});

describe("pipelineJobOutcomeSucceeded", () => {
  it("accepts a plain completed job", () => {
    expect(pipelineJobOutcomeSucceeded(job({ status: "completed" }))).toBe(true);
  });

  // A conversion-only job that handed off is a success even though its own
  // status never reaches "completed".
  it("accepts a still-open job whose child is parked at the handoff", () => {
    const child = childJob({
      status: "ready",
      currentStageId: "split",
      stages: [stage("handoff", "completed")],
    });
    expect(pipelineJobOutcomeSucceeded(job({ status: "ready", children: [child] }))).toBe(true);
  });

  it("rejects a failed job with nothing handed off", () => {
    expect(pipelineJobOutcomeSucceeded(job({ status: "failed" }))).toBe(false);
  });
});

// Mirrors job_is_actively_running in book_pipeline.rs. The launcher's update
// install reads this: installing replaces the App bundle a running stage is
// executing its Python, Node and Chromium out of, so a job this call misses is
// a job the install interrupts.
describe("pipelineJobActivelyRunning", () => {
  it("accepts the two job statuses that mean running", () => {
    expect(pipelineJobActivelyRunning(job({ status: "running" }))).toBe(true);
    // handoff_running is a running state whose name does not say "running";
    // reading job.status === "running" alone answers no for it.
    expect(pipelineJobActivelyRunning(job({ status: "handoff_running" }))).toBe(true);
  });

  it("accepts a job whose own status has moved on but whose stage is executing", () => {
    const child = childJob({ status: "ready", stages: [stage("translate", "running")] });
    expect(pipelineJobActivelyRunning(job({ status: "ready", children: [child] }))).toBe(true);
  });

  it("ignores a running stage on a child that was removed from the shelf", () => {
    const child = childJob({
      status: "ready",
      stages: [stage("translate", "running")],
      removedAt: "2026-08-07T00:00:00Z",
    });
    expect(pipelineJobActivelyRunning(job({ status: "ready", children: [child] }))).toBe(false);
  });

  it("rejects jobs that are queued, waiting on a person, or finished", () => {
    for (const status of ["pending", "ready", "waiting_for_approval", "blocked", "failed", "completed", "skipped"]) {
      expect(pipelineJobActivelyRunning(job({ status, children: [] })), status).toBe(false);
    }
  });
});
