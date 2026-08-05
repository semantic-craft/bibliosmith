import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OverviewTab } from "./OverviewTab";
import type { TabProps } from "./tabProps";
import { pipelineCopy } from "../copy";
import type { BookUnit } from "../model";
import { artifact, bookUnit, routeItem, stage } from "../../test/fixtures";
import {
  getBookPipelineStructureCorrectionDraft,
  saveBookPipelineStructureCorrection,
} from "../../api";

vi.mock("../../api", () => ({
  readBookPipelineOcrSample: vi.fn(() => Promise.reject(new Error("desktop only"))),
  getBookPipelineStructureCorrectionDraft: vi.fn(),
  saveBookPipelineStructureCorrection: vi.fn(),
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
  // A synced attachment carries a real storage path; without one there is no
  // PDF to sample and the card correctly stays hidden.
  unit.child!.source.path = "/storage/ABCD1234/book.pdf";
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
    unit.child!.source.path = "/storage/ABCD1234/book.pdf";
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

describe("OverviewTab · reading validation conclusions", () => {
  it("keeps package, structure and reader acceptance visibly separate", () => {
    const unit = bookUnit({
      status: "failed",
      stages: [stage("validate_reading", "failed")],
      childOver: {
        artifacts: [
          artifact("reading_epub"),
          artifact("epubcheck_report", {
            validation: { exists: true, nonempty: true, hashMatches: true, requiredChecksPassed: true },
          }),
          artifact("structural_readability_report", {
            validation: { exists: true, nonempty: true, hashMatches: true, requiredChecksPassed: false },
          }),
        ],
        readerEvidence: [],
      },
    });

    render(<OverviewTab {...tabProps(unit)} />);

    expect(screen.getByText(copy.artifactPackageValidity).parentElement?.textContent).toContain(
      copy.validationPassed,
    );
    expect(screen.getByText(copy.artifactStructuralReadability).parentElement?.textContent).toContain(
      copy.validationFailedLabel,
    );
    expect(screen.getByText(copy.artifactReaderAcceptance).parentElement?.textContent).toContain(
      copy.validationNotRecorded,
    );
  });
});

describe("OverviewTab · publication structure correction", () => {
  it("loads a source-bound draft, saves the correction, then retries split", async () => {
    const user = userEvent.setup();
    const onRetry = vi.fn();
    vi.mocked(getBookPipelineStructureCorrectionDraft).mockResolvedValue({
      schema: "publication-structure-correction-draft-v1",
      sourceMarkdownSha256: "a".repeat(64),
      publicationMapSha256: "b".repeat(64),
      anomalies: ["Internal extractor title escaped into the publication tree."],
      sections: [{ id: "section_001", title: "chapter_001", role: "bodymatter", kind: "chapter" }],
    });
    vi.mocked(saveBookPipelineStructureCorrection).mockResolvedValue({} as never);
    const unit = bookUnit({
      status: "failed",
      stages: [
        stage("route", "completed"),
        stage("extract", "completed"),
        stage("index", "completed"),
        stage("handoff", "completed"),
        stage("split", "failed"),
      ],
      childOver: { localProjectRoot: "/books/local/fixture" },
    });

    render(<OverviewTab {...tabProps(unit, { onRetry })} />);
    await user.click(screen.getByRole("button", { name: copy.structureCorrectionOpen }));
    expect(await screen.findByText(/Internal extractor title/)).toBeTruthy();
    await user.type(screen.getByLabelText(copy.structureCorrectionReason), "Extractor placeholder title.");
    const editor = screen.getByLabelText(copy.structureCorrectionSections);
    fireEvent.change(editor, {
      target: {
        value: JSON.stringify([
          { id: "section_001", title: "Title Page", role: "frontmatter", kind: "title_page" },
        ]),
      },
    });
    await user.click(screen.getByRole("button", { name: copy.structureCorrectionSave }));

    expect(saveBookPipelineStructureCorrection).toHaveBeenCalledWith(
      "job-1",
      "child-1",
      expect.objectContaining({
        reason: "Extractor placeholder title.",
        sections: [{ id: "section_001", title: "Title Page", role: "frontmatter", kind: "title_page" }],
      }),
    );
    expect(onRetry).toHaveBeenCalledWith("job-1");
  });
});
