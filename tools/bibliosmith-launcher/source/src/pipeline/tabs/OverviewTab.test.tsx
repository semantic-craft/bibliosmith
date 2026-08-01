import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OverviewTab } from "./OverviewTab";
import type { TabProps } from "./tabProps";
import { pipelineCopy } from "../copy";
import type { BookUnit } from "../model";
import { bookUnit, routeItem, stage } from "../../test/fixtures";

vi.mock("../../api", () => ({
  readBookPipelineOcrSample: vi.fn(() => Promise.reject(new Error("desktop only"))),
}));

const copy = pipelineCopy("en");

function tabProps(unit: BookUnit, over: Partial<TabProps> = {}): TabProps {
  return {
    unit,
    copy,
    busy: null,
    onRetry: vi.fn(),
    onAdvance: vi.fn(),
    onApproveGate: vi.fn(),
    onOpenOutput: vi.fn(),
    onHandoff: vi.fn(),
    onRouteOverride: vi.fn(),
    onSampleOcr: vi.fn(),
    onGoApproval: vi.fn(),
    ...over,
  };
}

function unitAwaitingOcr(routeKind = "remote_paddleocr"): BookUnit {
  const unit = bookUnit({ stages: [stage("route", "completed"), stage("extract", "pending")] });
  unit.child!.route = [routeItem({ routeKind })];
  return unit;
}

describe("OverviewTab · OCR comparison wiring", () => {
  it("offers the comparison for a book still waiting on an engine", () => {
    render(<OverviewTab {...tabProps(unitAwaitingOcr())} />);
    expect(screen.getByText(copy.ocrCompareTitle)).toBeTruthy();
  });

  it("hides it once conversion is under way", () => {
    const unit = bookUnit({ stages: [stage("route", "completed"), stage("extract", "completed")] });
    unit.child!.route = [routeItem({ routeKind: "remote_paddleocr" })];
    render(<OverviewTab {...tabProps(unit)} />);
    expect(screen.queryByText(copy.ocrCompareTitle)).toBeNull();
  });

  it("hides it for a book whose text layer is already usable", () => {
    render(<OverviewTab {...tabProps(unitAwaitingOcr("direct_text"))} />);
    expect(screen.queryByText(copy.ocrCompareTitle)).toBeNull();
  });

  /**
   * The card's own suite proves it calls onSampleOcr correctly, but every
   * argument here is (string, string, number): swapping jobId and childId
   * anywhere along OverviewTab -> BookDrawer -> PipelineWorkbench -> App type
   * checks cleanly and fails only against a real backend. This pins the one
   * hop the card cannot see.
   */
  it("threads the job and child through in the order the command expects", async () => {
    const user = userEvent.setup();
    const onSampleOcr = vi.fn();
    render(<OverviewTab {...tabProps(unitAwaitingOcr(), { onSampleOcr })} />);

    await user.click(screen.getByRole("button", { name: copy.ocrCompareRun }));

    expect(onSampleOcr).toHaveBeenCalledWith("job-1", "child-1", 3);
  });
});
