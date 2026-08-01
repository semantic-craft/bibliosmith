import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { BookPipelineStage } from "../types";
import { job, childJob, stage } from "../test/fixtures";
import { pipelineCopy } from "./copy";
import { defaultPipelineDraft } from "./model";
import { PipelineWorkbench, type PipelineWorkbenchProps } from "./PipelineWorkbench";

vi.mock("../api", () => ({
  readBookPipelineArtifactExcerpt: vi.fn(() => Promise.reject(new Error("desktop only"))),
  readBookPipelineTranslationSample: vi.fn(() => Promise.reject(new Error("desktop only"))),
  // The input island only registers the native drag-drop listener under Tauri.
  isTauriRuntime: () => false,
}));

const copy = pipelineCopy("zh-CN");

function workbenchProps(
  stages: BookPipelineStage[] = [stage("route", "completed"), stage("extract", "running")],
): PipelineWorkbenchProps {
  const child = childJob({
    status: "running",
    stages,
  });
  const runningJob = job({ children: [child], status: "running" });
  return {
    copy,
    state: { schemaVersion: "1", revision: 1, jobs: [runningJob] },
    draft: defaultPipelineDraft,
    preview: [],
    zoteroSources: [],
    modelSlots: [],
    busy: null,
    onDraftChange: vi.fn(),
    onPreview: vi.fn(),
    onQueueRun: vi.fn(async () => true),
    onChooseFolder: vi.fn(),
    onSearchZotero: vi.fn(),
    onRetry: vi.fn(),
    onDelete: vi.fn(),
    onAdvance: vi.fn(),
    onSampleTranslation: vi.fn(),
    onApplySampleProvider: vi.fn(),
    onExportDiagnostic: vi.fn(),
    onSaveCustomInstructions: vi.fn(),
    onApproveGate: vi.fn(),
    onRouteOverride: vi.fn(),
    onRecordReaderEvidence: vi.fn(),
    onOpenOutput: vi.fn(),
    routeOverrides: {},
    onRouteOverrideChange: vi.fn(),
    onHandoff: vi.fn(),
  };
}

describe("PipelineWorkbench split view", () => {
  it("lets the reader widen the details pane from an accessible separator", async () => {
    const user = userEvent.setup();
    const { container } = render(<PipelineWorkbench {...workbenchProps()} />);

    await user.click(screen.getByRole("button", { name: /A Book.*整理文字/ }));
    const separator = screen.getByRole("separator", { name: copy.resizeDrawer });
    expect(separator.getAttribute("aria-valuenow")).toBe("50");

    fireEvent.keyDown(separator, { key: "ArrowLeft" });

    expect(separator.getAttribute("aria-valuenow")).toBe("55");
    expect(
      (container.querySelector(".pl-shelfwrap") as HTMLElement).style.getPropertyValue("--pl-drawer-width"),
    ).toBe("55%");
  });

  it("resizes the details pane by dragging the divider", async () => {
    const user = userEvent.setup();
    const { container } = render(<PipelineWorkbench {...workbenchProps()} />);

    await user.click(screen.getByRole("button", { name: /A Book.*整理文字/ }));
    const separator = screen.getByRole("separator", { name: copy.resizeDrawer });
    const splitView = container.querySelector(".pl-shelfwrap") as HTMLElement;
    vi.spyOn(splitView, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      top: 0,
      right: 1000,
      bottom: 700,
      left: 0,
      width: 1000,
      height: 700,
      toJSON: () => ({}),
    });
    Object.defineProperty(separator, "setPointerCapture", { value: vi.fn() });
    Object.defineProperty(separator, "releasePointerCapture", { value: vi.fn() });

    fireEvent.pointerDown(separator, { pointerId: 1, clientX: 500 });
    fireEvent.pointerMove(separator, { pointerId: 1, clientX: 400 });
    fireEvent.pointerUp(separator, { pointerId: 1, clientX: 400 });

    expect(separator.getAttribute("aria-valuenow")).toBe("60");
    expect(splitView.style.getPropertyValue("--pl-drawer-width")).toBe("60%");
  });

  it("states the completed text cleanup step on the shelf and in book details", async () => {
    const user = userEvent.setup();
    render(
      <PipelineWorkbench
        {...workbenchProps([
          stage("extract", "completed"),
          stage("index", "completed"),
          stage("handoff", "completed"),
          stage("split", "completed"),
          stage("prepare", "completed"),
          stage("approve_translation", "completed"),
          stage("translate", "running"),
          stage("expert_qa", "pending"),
        ])}
      />,
    );

    expect(screen.getAllByText("整理文字：已完成；翻译：进行中")).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: /A Book.*整理文字/ }));
    expect(screen.getAllByText("整理文字：已完成；翻译：进行中").length).toBeGreaterThanOrEqual(2);
  });

  it("shows live AI work with a real chapter progress bar", async () => {
    const user = userEvent.setup();
    const props = workbenchProps([
      stage("prepare", "completed"),
      stage("approve_translation", "completed"),
      stage("translate", "running"),
      stage("expert_qa", "pending"),
    ]);
    props.state.jobs[0].progress = {
      ...props.state.jobs[0].progress,
      operation: {
        stageId: "translate",
        scopeId: "child-1",
        completed: 37,
        total: 100,
        unitKind: "chapters",
        phase: "translating",
        activityAt: "2026-07-29T12:00:00Z",
      },
    };

    render(<PipelineWorkbench {...props} />);

    expect(screen.getByText("37 / 100 章")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: /A Book.*翻译/ }));
    const bars = screen.getAllByRole("progressbar", { name: "AI 正在翻译：37 / 100 章" });
    expect(bars).toHaveLength(2);
    const bar = bars[1];
    expect(bar.getAttribute("aria-valuenow")).toBe("37");
    expect(bar.getAttribute("aria-valuemax")).toBe("100");
  });
});
