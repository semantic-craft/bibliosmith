import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readBookPipelineOcrSample } from "../api";
import { OcrCompareCard, canCompareOcrEngines } from "./OcrCompareCard";
import { pipelineCopy } from "./copy";
import type { BookUnit, PipelineBusy } from "./model";
import { artifact, bookUnit, routeItem, stage } from "../test/fixtures";
import type { BookPipelineOcrSampleReport } from "../types";

// The card reads its report on mount, which is a desktop-only call. Mocking the
// module keeps these assertions about the card's own contract rather than about
// api.ts's preview stubs.
vi.mock("../api", () => ({
  readBookPipelineOcrSample: vi.fn(() => Promise.reject(new Error("desktop only"))),
}));

const copy = pipelineCopy("en");
const readSample = vi.mocked(readBookPipelineOcrSample);

function report(over: Partial<BookPipelineOcrSampleReport> = {}): BookPipelineOcrSampleReport {
  return {
    schema: "ocr-sample-compare-report-v1",
    sourcePdfSha256: "0".repeat(64),
    totalPages: 40,
    sampledPages: [11, 21, 30],
    characterBudget: 4000,
    engines: [
      {
        engine: "paddleocr",
        status: "ok",
        markdownExcerpt: "# Paddle heading\n\nPaddle body.",
        characterCount: 30,
        pageCount: 3,
        elapsedMs: 4100,
        error: null,
      },
      {
        engine: "mineru",
        status: "ok",
        markdownExcerpt: "# MinerU heading\n\nMinerU body.",
        characterCount: 30,
        pageCount: 3,
        elapsedMs: 9200,
        error: null,
      },
    ],
    ...over,
  };
}

/**
 * A book still waiting to be converted through an OCR route.
 *
 * `source.path` carries a real storage path with the `#source_md5=` suffix
 * zotero_source_ref appends, because that is what a synced attachment actually
 * looks like — a fixture holding only an attachment key would have no PDF to
 * sample and so would not be sampleable at all.
 */
function sampleableUnit(over: { artifacts?: ReturnType<typeof artifact>[]; extract?: string } = {}): BookUnit {
  const unit = bookUnit({
    stages: [stage("route", "completed"), stage("extract", (over.extract ?? "pending") as never)],
  });
  const child = unit.child!;
  child.source.path = "/storage/ABCD1234/book.pdf#source_md5=d41d8cd98f00b204e9800998ecf8427e";
  child.route = [routeItem({ routeKind: "remote_paddleocr" })];
  child.artifacts = over.artifacts ?? [];
  return unit;
}

function renderCard(unit: BookUnit) {
  const onSampleOcr = vi.fn();
  const onRouteOverride = vi.fn();
  render(
    <OcrCompareCard
      unit={unit}
      copy={copy}
      busy={null}
      onSampleOcr={onSampleOcr}
      onRouteOverride={onRouteOverride}
    />,
  );
  return { onSampleOcr, onRouteOverride };
}

describe("canCompareOcrEngines", () => {
  it("is open while the book still needs converting", () => {
    expect(canCompareOcrEngines(sampleableUnit())).toBe(true);
  });

  it("closes once conversion has started or finished", () => {
    // The backend refuses a sample at that point, so offering the button would
    // only produce an error the user cannot act on.
    expect(canCompareOcrEngines(sampleableUnit({ extract: "running" }))).toBe(false);
    expect(canCompareOcrEngines(sampleableUnit({ extract: "completed" }))).toBe(false);
  });

  it("stays closed for a book that needs no OCR at all", () => {
    // A usable text layer means no engine to choose; sampling would spend on
    // pages nothing goes on to use.
    const unit = sampleableUnit();
    unit.child!.route = [routeItem({ routeKind: "direct_text" })];
    expect(canCompareOcrEngines(unit)).toBe(false);
  });

  it("is open for every route kind the backend lets an override reach", () => {
    // A dirty text layer is where the choice matters most — the book's own text
    // is unusable — and it is what the blocked-state buttons already offer
    // paddle/mineru for. missing_credentials is reachable too: one engine may
    // still be configured, and half a comparison decides the route.
    for (const routeKind of [
      "remote_paddleocr",
      "mineru",
      "missing_credentials",
      "blocked_dirty_text_layer",
    ]) {
      const unit = sampleableUnit();
      unit.child!.route = [routeItem({ routeKind })];
      expect(canCompareOcrEngines(unit), routeKind).toBe(true);
    }
  });

  it("stays closed when the attachment was never synced locally", () => {
    // A single Zotero attachment with no local file carries only a zotero://
    // reference. The backend rejects it with "not stored locally", and that is
    // knowable from state — not worth a paid round trip to discover.
    const unit = sampleableUnit();
    unit.child!.source.path = undefined;
    unit.child!.route = [
      routeItem({ routeKind: "missing_credentials", sourceRef: "zotero://attachment/ABCD1234" }),
    ];
    expect(canCompareOcrEngines(unit)).toBe(false);
  });

  it("accepts a storage path carrying its md5 fingerprint", () => {
    // zotero_source_ref appends #source_md5=<hex> to the real path; stripping
    // it by exact suffix is what the backend does, and `#` is legal in a name.
    const unit = sampleableUnit();
    unit.child!.source.path = undefined;
    unit.child!.route = [
      routeItem({ routeKind: "remote_paddleocr", sourceRef: "/books/draft#2.pdf#source_md5=abc" }),
    ];
    expect(canCompareOcrEngines(unit)).toBe(true);
  });

  it("stays closed when the only local file is not a PDF", () => {
    const unit = sampleableUnit();
    unit.child!.source.path = "/books/notes.epub";
    unit.child!.route = [
      routeItem({ routeKind: "remote_paddleocr", sourceRef: "/books/notes.epub" }),
    ];
    expect(canCompareOcrEngines(unit)).toBe(false);
  });

  it("stays closed for a unit with no child job", () => {
    // Collection parents carry no stages of their own.
    const unit = sampleableUnit();
    unit.child = null;
    expect(canCompareOcrEngines(unit)).toBe(false);
  });
});

describe("OcrCompareCard", () => {
  beforeEach(() => {
    readSample.mockReset();
    readSample.mockRejectedValue(new Error("desktop only"));
  });

  it("runs a comparison for the requested page count", async () => {
    const user = userEvent.setup();
    const { onSampleOcr } = renderCard(sampleableUnit());

    const pages = screen.getByRole("spinbutton");
    await user.clear(pages);
    await user.type(pages, "5");
    await user.click(screen.getByRole("button", { name: copy.ocrCompareRun }));

    expect(onSampleOcr).toHaveBeenCalledWith("job-1", "child-1", 5);
  });

  it("clamps the page count to what the backend accepts", async () => {
    const user = userEvent.setup();
    const { onSampleOcr } = renderCard(sampleableUnit());

    const pages = screen.getByRole("spinbutton");
    await user.clear(pages);
    await user.type(pages, "99");
    await user.click(screen.getByRole("button", { name: copy.ocrCompareRun }));

    // 10 is MAX_SAMPLE_PAGES on both sides; sending more would be rejected
    // after the user had already waited for the run.
    expect(onSampleOcr).toHaveBeenCalledWith("job-1", "child-1", 10);
  });

  it("shows both engines once a report is registered", async () => {
    readSample.mockResolvedValue(report());
    renderCard(sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] }));

    await waitFor(() => expect(screen.getByText("PaddleOCR")).toBeTruthy());
    expect(screen.getByText("MinerU")).toBeTruthy();
    expect(screen.getByText(/Paddle body\./)).toBeTruthy();
    expect(screen.getByText(/MinerU body\./)).toBeTruthy();
    // The pages the report names, so the user can see what was compared.
    expect(screen.getByText("11 · 21 · 30 / 40")).toBeTruthy();
  });

  it("writes the route override only for the engine the user picked", async () => {
    const user = userEvent.setup();
    readSample.mockResolvedValue(report());
    const { onRouteOverride } = renderCard(
      sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] }),
    );
    await waitFor(() => expect(screen.getByText("MinerU")).toBeTruthy());

    // Nothing is chosen yet, so there is nothing to confirm.
    const confirm = screen.getByRole("button", { name: copy.ocrComparePick });
    expect(confirm).toHaveProperty("disabled", true);

    await user.click(screen.getByText("MinerU"));
    await user.click(confirm);

    // "mineru" is the token apply_route_overrides maps to the mineru route.
    expect(onRouteOverride).toHaveBeenCalledWith("job-1", "child-1", "route-1", "mineru");
  });

  it("maps PaddleOCR to its own override token", async () => {
    const user = userEvent.setup();
    readSample.mockResolvedValue(report());
    const { onRouteOverride } = renderCard(
      sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] }),
    );
    await waitFor(() => expect(screen.getByText("PaddleOCR")).toBeTruthy());

    await user.click(screen.getByText("PaddleOCR"));
    await user.click(screen.getByRole("button", { name: copy.ocrComparePick }));

    // The backend token is "paddle", not the engine's own name — sending
    // "paddleocr" is silently ignored by apply_route_overrides.
    expect(onRouteOverride).toHaveBeenCalledWith("job-1", "child-1", "route-1", "paddle");
  });

  it("shows a failed engine's reason but refuses to route to it", async () => {
    const user = userEvent.setup();
    readSample.mockResolvedValue(
      report({
        engines: [
          {
            engine: "paddleocr",
            status: "failed",
            markdownExcerpt: "",
            characterCount: 0,
            pageCount: null,
            elapsedMs: 12,
            error: "BAIDU_PADDLEOCR_TOKEN is not configured",
          },
          report().engines[1],
        ],
      }),
    );
    const { onRouteOverride } = renderCard(
      sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] }),
    );
    await waitFor(() => expect(screen.getByText(/BAIDU_PADDLEOCR_TOKEN/)).toBeTruthy());

    await user.click(screen.getByText("PaddleOCR"));
    const confirm = screen.getByRole("button", { name: copy.ocrComparePick });
    // Half a comparison is still worth showing, but there is no evidence the
    // failed engine converts this book.
    expect(confirm).toHaveProperty("disabled", true);
    expect(onRouteOverride).not.toHaveBeenCalled();
  });

  it("does not show a report belonging to a previous comparison", async () => {
    readSample.mockResolvedValue(report());
    const unit = sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] });
    const { rerender } = render(
      <OcrCompareCard
        unit={unit}
        copy={copy}
        busy={null}
        onSampleOcr={vi.fn()}
        onRouteOverride={vi.fn()}
      />,
    );
    await waitFor(() => expect(screen.getByText("PaddleOCR")).toBeTruthy());

    // A re-sample registers a new digest. The old report must not survive into
    // the new one's render, which is what the version tag is for.
    const resampled = sampleableUnit({
      artifacts: [artifact("ocr_sample_report", { sha256: "def" })],
    });
    readSample.mockReturnValue(new Promise(() => undefined));
    rerender(
      <OcrCompareCard
        unit={resampled}
        copy={copy}
        busy={null}
        onSampleOcr={vi.fn()}
        onRouteOverride={vi.fn()}
      />,
    );
    expect(screen.queryByText("PaddleOCR")).toBeNull();
  });

  it("shows a report that arrives after mount, not only one present on mount", async () => {
    // The real journey is the opposite of every other test here: the card is
    // already open with no report, the user runs a comparison, and the new
    // artifact digest has to make the effect fetch again. Dropping `version`
    // from the dependency array leaves the card blank forever.
    readSample.mockResolvedValue(report());
    const props = {
      copy,
      busy: null as PipelineBusy,
      onSampleOcr: vi.fn(),
      onRouteOverride: vi.fn(),
    };
    const { rerender } = render(<OcrCompareCard unit={sampleableUnit()} {...props} />);
    expect(screen.queryByText("PaddleOCR")).toBeNull();

    rerender(
      <OcrCompareCard
        unit={sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] })}
        {...props}
      />,
    );
    await waitFor(() => expect(screen.getByText("PaddleOCR")).toBeTruthy());
  });

  it("drops the previous selection when a re-sample replaces the report", async () => {
    const user = userEvent.setup();
    readSample.mockResolvedValue(report());
    const props = {
      copy,
      busy: null as PipelineBusy,
      onSampleOcr: vi.fn(),
      onRouteOverride: vi.fn(),
    };
    const { rerender } = render(
      <OcrCompareCard
        unit={sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] })}
        {...props}
      />,
    );
    await waitFor(() => expect(screen.getByText("MinerU")).toBeTruthy());
    await user.click(screen.getByText("MinerU"));
    expect(screen.getByRole("button", { name: copy.ocrComparePick })).toHaveProperty(
      "disabled",
      false,
    );

    // A new comparison is evidence the old choice was never made against.
    readSample.mockResolvedValue(report({ totalPages: 88 }));
    rerender(
      <OcrCompareCard
        unit={sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "def" })] })}
        {...props}
      />,
    );
    await waitFor(() => expect(screen.getByText(/\/ 88/)).toBeTruthy());
    expect(screen.getByRole("button", { name: copy.ocrComparePick })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("reports each engine's cost in the units the label promises", async () => {
    readSample.mockResolvedValue(report());
    renderCard(sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] }));
    await waitFor(() => expect(screen.getByText("PaddleOCR")).toBeTruthy());

    // Seconds, not milliseconds: 4100ms next to a "s" label reads as an engine
    // that took over an hour.
    expect(screen.getByText(`4.1${copy.ocrCompareSeconds}`)).toBeTruthy();
    expect(screen.getByText(`9.2${copy.ocrCompareSeconds}`)).toBeTruthy();
    expect(screen.getAllByText(`30 ${copy.ocrCompareCharacters}`)).toHaveLength(2);
  });

  it("presents the engines as one mutually exclusive choice", async () => {
    const user = userEvent.setup();
    readSample.mockResolvedValue(report());
    renderCard(sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] }));
    await waitFor(() => expect(screen.getByText("PaddleOCR")).toBeTruthy());

    // Radios, not toggle buttons: exactly one engine converts the book, and the
    // excerpt must stay out of the control's accessible name.
    const radios = screen.getAllByRole("radio");
    expect(radios).toHaveLength(2);
    await user.click(screen.getByText("MinerU"));
    expect(radios[1]).toHaveProperty("checked", true);
    await user.click(screen.getByText("PaddleOCR"));
    expect(radios[0]).toHaveProperty("checked", true);
    expect(radios[1]).toHaveProperty("checked", false);
  });

  it("offers a re-sample once a comparison exists", async () => {
    readSample.mockResolvedValue(report());
    renderCard(sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] }));
    await waitFor(() => expect(screen.getByText("PaddleOCR")).toBeTruthy());
    expect(screen.getByRole("button", { name: copy.ocrCompareRetry })).toBeTruthy();
    expect(screen.queryByRole("button", { name: copy.ocrCompareRun })).toBeNull();
  });

  it("says so when an engine succeeded but returned nothing", async () => {
    // The backend accepts this: only both engines failing is an error, so an
    // empty pane must read as an answer rather than as a render bug.
    readSample.mockResolvedValue(
      report({
        engines: [
          { ...report().engines[0], markdownExcerpt: "   ", characterCount: 0 },
          report().engines[1],
        ],
      }),
    );
    renderCard(sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] }));
    await waitFor(() => expect(screen.getByText(copy.ocrCompareEmpty)).toBeTruthy());
  });

  it("will not write a route while another action is in flight", async () => {
    readSample.mockResolvedValue(report());
    render(
      <OcrCompareCard
        unit={sampleableUnit({ artifacts: [artifact("ocr_sample_report", { sha256: "abc" })] })}
        copy={copy}
        busy="sample"
        onSampleOcr={vi.fn()}
        onRouteOverride={vi.fn()}
      />,
    );
    await waitFor(() => expect(screen.getByText("PaddleOCR")).toBeTruthy());
    // A re-sample in flight is about to replace the evidence under the choice.
    expect(screen.getByRole("button", { name: copy.ocrComparePick })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("is inert while another pipeline action is running", () => {
    render(
      <OcrCompareCard
        unit={sampleableUnit()}
        copy={copy}
        busy="sample"
        onSampleOcr={vi.fn()}
        onRouteOverride={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: copy.ocrCompareRun })).toHaveProperty("disabled", true);
  });
});
