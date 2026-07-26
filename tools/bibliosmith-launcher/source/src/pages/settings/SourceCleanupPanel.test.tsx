import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SourceCleanupPanel } from "./SourceCleanupPanel";
import type { BookPipelineCleanupCandidate } from "../../types";

const previewBookPipelineCleanup = vi.fn();
const approveBookPipelineCleanup = vi.fn();

vi.mock("../../api", () => ({
  previewBookPipelineCleanup: (...args: unknown[]) => previewBookPipelineCleanup(...args),
  approveBookPipelineCleanup: (...args: unknown[]) => approveBookPipelineCleanup(...args),
}));

function candidate(over: Partial<BookPipelineCleanupCandidate> = {}): BookPipelineCleanupCandidate {
  return {
    id: "cleanup-job-1",
    jobId: "job-1",
    title: "A Book",
    sourceKind: "zotero_attachment",
    sourceRef: "PDFKEY1",
    sourcePath: "/library/storage/PDFKEY1/book.pdf",
    sourcePdfKey: "PDFKEY1",
    markdownPath: "/out/book.md",
    localOutputPath: "/out",
    zoteroChildAttachmentKey: "MDKEY1",
    checks: [
      { kind: "markdown_output", ok: true, detail: "ok" },
      { kind: "validated_reading", ok: true, detail: "ok" },
    ],
    canApprove: true,
    ...over,
  };
}

describe("SourceCleanupPanel", () => {
  beforeEach(() => {
    previewBookPipelineCleanup.mockReset();
    approveBookPipelineCleanup.mockReset();
  });

  // The commands, their wrappers and their types all shipped; no component ever
  // called them, so the approval the design asked the user for was unreachable.
  it("lists candidates and records an approval", async () => {
    const user = userEvent.setup();
    previewBookPipelineCleanup.mockResolvedValue({ candidates: [candidate()], logSummary: [] });
    approveBookPipelineCleanup.mockResolvedValue({ ok: true, message: "Cleanup approval recorded." });

    render(<SourceCleanupPanel locale="en" />);

    await waitFor(() => expect(screen.getByText("A Book")).toBeTruthy());
    expect(screen.getByText(/book\.pdf/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Approve cleanup" }));

    expect(approveBookPipelineCleanup).toHaveBeenCalledWith("cleanup-job-1", true);
    await waitFor(() => expect(screen.getByText("Cleanup approval recorded.")).toBeTruthy());
  });

  it("cannot approve a candidate whose evidence is incomplete", async () => {
    previewBookPipelineCleanup.mockResolvedValue({
      candidates: [
        candidate({
          canApprove: false,
          checks: [{ kind: "validated_reading", ok: false, detail: "not validated" }],
        }),
      ],
      logSummary: [],
    });

    render(<SourceCleanupPanel locale="en" />);

    await waitFor(() => expect(screen.getByText("A Book")).toBeTruthy());
    expect(screen.getByRole("button", { name: "Approve cleanup" }).hasAttribute("disabled")).toBe(true);
    expect(approveBookPipelineCleanup).not.toHaveBeenCalled();
  });
});
