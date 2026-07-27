import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readBookPipelineArtifactExcerpt } from "../api";
import { BookDrawer, type BookDrawerProps } from "./BookDrawer";
import { pipelineCopy } from "./copy";
import type { BookUnit } from "./model";
import { MODEL_BRANDS } from "../pages/settings/modelCatalog";
import { BOOK_PIPELINE_DIAGNOSTIC_PROFILES } from "../types";
import { approvalRef, artifact, bookUnit, stage, unitSummary } from "../test/fixtures";

// The drawer reads an artifact excerpt and a sample report on mount. Both are
// desktop-only calls that reject in a browser, and api.ts answers a lot of other
// calls with preview fixtures — mocking the module keeps these assertions about
// the component's own contract rather than about the preview stub.
vi.mock("../api", () => ({
  readBookPipelineArtifactExcerpt: vi.fn(() => Promise.reject(new Error("desktop only"))),
  readBookPipelineTranslationSample: vi.fn(() => Promise.reject(new Error("desktop only"))),
}));

const copy = pipelineCopy("en");

function drawerProps(unit: BookUnit, over: Partial<BookDrawerProps> = {}): BookDrawerProps {
  return {
    copy,
    units: [unit],
    unit,
    busy: null,
    onSelect: vi.fn(),
    onClose: vi.fn(),
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
    onHandoff: vi.fn(),
    ...over,
  };
}

function renderDrawer(unit: BookUnit, over: Partial<BookDrawerProps> = {}) {
  const props = drawerProps(unit, over);
  const result = render(<BookDrawer {...props} />);
  const card = result.container.querySelector(".pl-gatecard");
  return { ...result, props, card };
}

/** A book parked at the translation gate, which is where the provider picker lives. */
function gateUnit(over: Parameters<typeof bookUnit>[0] = {}) {
  return bookUnit({
    status: "waiting_for_approval",
    stages: [
      stage("prepare", "completed"),
      stage("approve_translation", "waiting_for_approval", {
        unitSummary: unitSummary({ total: 4, completed: 4 }),
      }),
    ],
    ...over,
    jobOver: { translationMode: "fast", ...over.jobOver },
  });
}

function providerSelect(): HTMLSelectElement {
  return screen.getByLabelText(copy.sampleProvider) as HTMLSelectElement;
}

const optionValues = (select: HTMLSelectElement) =>
  Array.from(select.options).map((option) => option.value);

const CATALOG_SLOTS = MODEL_BRANDS.flatMap((brand) =>
  brand.slots.map((slot) => `${slot.profileId}:${slot.configId}`),
);

describe("BookDrawer provider picker", () => {
  // Regression: the options were one per brand while the registry carries eight
  // slots, so Qwen's and MiMo's second billing plan could not be picked here at
  // all — and before that, three brands were hard-coded against a catalog of six.
  it("offers every slot in the catalog, not just every brand", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "deepseek", translationConfigId: "deepseek-default" } }));

    expect(CATALOG_SLOTS.length).toBeGreaterThan(MODEL_BRANDS.length);
    expect(optionValues(providerSelect())).toEqual(CATALOG_SLOTS);
    expect(optionValues(providerSelect())).toContain("qwen:token-plan");
    expect(optionValues(providerSelect())).toContain("mimo:token-plan");
  });

  it("labels a two-plan brand's slots apart", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "deepseek", translationConfigId: "deepseek-default" } }));
    const labels = Array.from(providerSelect().options).map((option) => option.textContent);
    const qwen = MODEL_BRANDS.find((brand) => brand.profileId === "qwen")!;
    expect(labels).toContain(`${qwen.brand} · ${qwen.slots[0].label}`);
    expect(labels).toContain(`${qwen.brand} · ${qwen.slots[1].label}`);
    // A single-slot brand stays unqualified.
    expect(labels).toContain("DeepSeek");
  });

  it("selects the slot the job already carries", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "qwen", translationConfigId: "token-plan" } }));
    expect(providerSelect().value).toBe("qwen:token-plan");
  });

  // A job may predate a rename, or belong to the expert agent. Dropping such a
  // slot from the list would strand the picker on a value it has no option for.
  it("keeps a slot the catalog no longer lists selectable", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "retired-profile", translationConfigId: "old" } }));
    const select = providerSelect();
    expect(select.value).toBe("retired-profile:old");
    expect(optionValues(select)).toEqual(["retired-profile:old", ...CATALOG_SLOTS]);
  });

  it("does not add a duplicate option for a slot the catalog does list", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "qwen", translationConfigId: "payg" } }));
    const values = optionValues(providerSelect());
    expect(values.filter((value) => value === "qwen:payg")).toHaveLength(1);
  });

  it("falls back to the brand's first slot when the job carries no config", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "kimi", translationConfigId: "" } }));
    const kimi = MODEL_BRANDS.find((brand) => brand.profileId === "kimi")!;
    expect(providerSelect().value).toBe(`kimi:${kimi.slots[0].configId}`);
  });

  it("hides the sample controls for an expert-mode job", () => {
    const { card } = renderDrawer(gateUnit({ jobOver: { translationMode: "expert" } }));
    expect(card).not.toBeNull();
    expect(screen.queryByLabelText(copy.sampleProvider)).toBeNull();
  });
});

/**
 * Running a sample used to adopt its provider as the job's, so trying a model
 * out silently redirected the whole book. Sampling now leaves the job alone,
 * which means the gap between "what I sampled" and "what the book will run on"
 * has to be visible — the approval binding carries the job's slot while the
 * evidence in front of the user came from the sample's.
 */
describe("BookDrawer sample provider decoupling", () => {
  const onQwenPayg = { translationProfileId: "qwen", translationConfigId: "payg" };

  it("names the book's own model alongside the sample picker", () => {
    const { card } = renderDrawer(gateUnit({ jobOver: onQwenPayg }));
    const qwen = MODEL_BRANDS.find((brand) => brand.profileId === "qwen")!;
    // Scoped to the row: the same label is also one of the picker's options.
    const row = within(card as HTMLElement).getByText(copy.jobProvider).closest(".pl-evi-row");
    expect(row?.querySelector(".pl-v")?.textContent).toBe(`${qwen.brand} · ${qwen.slots[0].label}`);
  });

  it("says nothing while the picker still matches the book's model", () => {
    const { card } = renderDrawer(gateUnit({ jobOver: onQwenPayg }));
    expect(within(card as HTMLElement).queryByText(copy.sampleProviderDiffers)).toBeNull();
    expect(
      within(card as HTMLElement).queryByRole("button", { name: copy.applySampleProvider }),
    ).toBeNull();
  });

  it("warns and offers the adopt action once the picker moves off it", async () => {
    const user = userEvent.setup();
    const { card } = renderDrawer(gateUnit({ jobOver: onQwenPayg }));

    await user.selectOptions(providerSelect(), "kimi:kimi-default");

    expect(within(card as HTMLElement).getByText(copy.sampleProviderDiffers)).toBeTruthy();
    expect(
      within(card as HTMLElement).getByRole("button", { name: copy.applySampleProvider }),
    ).toBeTruthy();
  });

  it("keeps sampling and adopting as two separate calls", async () => {
    const user = userEvent.setup();
    const { card, props } = renderDrawer(gateUnit({ jobOver: onQwenPayg }));

    await user.selectOptions(providerSelect(), "kimi:kimi-default");
    await user.click(within(card as HTMLElement).getByRole("button", { name: copy.sampleRun }));
    expect(props.onSampleTranslation).toHaveBeenCalledWith("job-1", "child-1", "kimi", "kimi-default");
    expect(props.onApplySampleProvider).not.toHaveBeenCalled();

    await user.click(
      within(card as HTMLElement).getByRole("button", { name: copy.applySampleProvider }),
    );
    expect(props.onApplySampleProvider).toHaveBeenCalledWith("job-1", "child-1", "kimi", "kimi-default");
  });
});

describe("BookDrawer gate card", () => {
  function approveButton(card: Element) {
    return within(card as HTMLElement).getByRole("button", { name: copy.gate1Ok }) as HTMLButtonElement;
  }

  it("lets the user approve when every check passes", () => {
    const { card } = renderDrawer(gateUnit());
    expect(approveButton(card!).disabled).toBe(false);
    expect(within(card as HTMLElement).queryByText(copy.gateBlockedByChecks)).toBeNull();
  });

  it("disables approval while an upstream stage is still failed", () => {
    const unit = gateUnit({
      stages: [
        stage("extract", "failed"),
        stage("approve_translation", "waiting_for_approval"),
      ],
    });
    const { card } = renderDrawer(unit);
    expect(approveButton(card!).disabled).toBe(true);
    expect(within(card as HTMLElement).getByText(copy.gateBlockedByChecks)).toBeTruthy();
  });

  it("disables approval while the promotion gate's QA is unresolved", () => {
    const unit = bookUnit({
      status: "waiting_for_approval",
      stages: [
        stage("expert_qa", "completed", { unitSummary: unitSummary({ total: 4, completed: 3, failed: 1 }) }),
        stage("approve_promotion", "waiting_for_approval"),
      ],
    });
    const { container } = renderDrawer(unit);
    const card = container.querySelector(".pl-gatecard") as HTMLElement;
    const button = within(card).getByRole("button", { name: copy.gate2Ok }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
  });

  // An invalidated gate is not a "fix the checks" situation — the artifacts
  // moved underneath the approval, so the action is withdrawn entirely.
  it("withdraws the approval action when a bound hash no longer matches", () => {
    const unit = gateUnit({
      childOver: { artifacts: [artifact("chapter_source", { artifactId: "art-1", sha256: "ffff" })] },
      jobOver: {
        translationMode: "fast",
        approvalReferences: [approvalRef({ boundArtifactHashes: { "art-1": "aaaa" }, childJobId: "child-1" })],
      },
    });
    const { card } = renderDrawer(unit);
    expect(within(card as HTMLElement).queryByRole("button", { name: copy.gate1Ok })).toBeNull();
    expect(within(card as HTMLElement).getByText(copy.gateInvalidatedNote)).toBeTruthy();
  });

  // These three buttons shipped permanently disabled, titled "waits on a runner
  // command". The only way past a held book was to delete and re-queue it, which
  // for a collection took the whole batch.
  it("re-routes a held book in place instead of only offering disabled buttons", async () => {
    const user = userEvent.setup();
    const held = bookUnit({
      status: "blocked",
      stages: [stage("route", "blocked")],
      childOver: {
        route: [
          {
            id: "DIRTY",
            title: "A Book",
            sourceKind: "zotero_attachment",
            sourceRef: "zotero://DIRTY",
            routeKind: "blocked_dirty_text_layer",
            canRun: false,
            blockedReason: "Dirty text layer needs a decision.",
            summary: "Held for review",
          },
        ],
      },
      jobOver: { status: "blocked" },
    });
    const { props } = renderDrawer(held);

    const forcePaddle = screen.getByRole("button", { name: copy.blockedForcePaddle });
    expect(forcePaddle.hasAttribute("disabled")).toBe(false);
    await user.click(forcePaddle);

    expect(props.onRouteOverride).toHaveBeenCalledWith("job-1", "child-1", "DIRTY", "paddle");

    await user.click(screen.getByRole("button", { name: copy.blockedKeepMineru }));
    expect(props.onRouteOverride).toHaveBeenCalledWith("job-1", "child-1", "DIRTY", "mineru");
  });

  // Deleting used to remove the whole job, taking every other book in the batch
  // with it. It now drops just the book the user pointed at.
  it("drops only the book it was opened on", async () => {
    const user = userEvent.setup();
    const batch = bookUnit({ status: "completed" });
    batch.job.children = [batch.child!, { ...batch.child!, id: "child-2" }, { ...batch.child!, id: "child-3" }];

    const { container, props } = renderDrawer(batch);
    await user.click(within(container).getByRole("button", { name: copy.deleteBook }));
    await user.click(screen.getByRole("button", { name: copy.deleteBookConfirm }));

    expect(props.onDelete).toHaveBeenCalledWith("job-1", "child-1");
  });

  // A single-book job keeps the original wording, which was already accurate.
  it("keeps the single-book wording when the job holds one book", async () => {
    const user = userEvent.setup();
    const { container } = renderDrawer(bookUnit({ status: "completed" }));
    await user.click(within(container).getByRole("button", { name: copy.deleteBook }));

    expect(screen.getByText(copy.deleteBookConfirmHint)).toBeTruthy();
    expect(screen.getByRole("button", { name: copy.deleteBookConfirm })).toBeTruthy();
  });

  // The confirmation used to be cleared by an effect watching unit.key. It is
  // now local state that dies with the drawer, so the workbench keys the drawer
  // on the selected book; this pins that composition down.
  it("drops a pending delete confirmation when the drawer moves to another book", async () => {
    const user = userEvent.setup();
    const first = bookUnit({ status: "completed" });
    const second = bookUnit({ status: "completed", jobOver: { id: "job-2" } });
    const props = drawerProps(first, { units: [first, second] });

    const { container, rerender } = render(<BookDrawer key={first.key} {...props} />);
    await user.click(within(container).getByRole("button", { name: copy.deleteBook }));
    expect(screen.getByText(copy.deleteBookConfirmHint)).toBeTruthy();

    rerender(<BookDrawer key={second.key} {...props} unit={second} />);

    expect(screen.queryByText(copy.deleteBookConfirmHint)).toBeNull();
  });

  // The record command, its wrapper and its types all shipped; nothing called
  // them, so the one thing this feature asks a human for could not be given.
  it("records reader verification against the built EPUB", async () => {
    const user = userEvent.setup();
    const built = bookUnit({
      status: "completed",
      stages: [stage("validate_reading", "completed")],
      childOver: {
        artifacts: [artifact("reading_epub", { artifactId: "art-epub", sha256: "abcd1234" })],
        readerEvidence: [
          {
            reader: "Calibre",
            readerVersion: "8.4",
            artifactKind: "reading_epub",
            artifactSha256: "0000",
            conclusion: "passed",
            recordedAt: "2026-07-26T00:00:00Z",
            stale: true,
          },
        ],
      },
    });
    const { container, props } = renderDrawer(built);
    await user.click(within(container).getByRole("tab", { name: copy.tabArtifacts }));

    // An existing record is shown, and one taken against an older build says so.
    const row = screen.getByText(/Calibre 8\.4/).closest(".pl-evi-row");
    expect(row).toBeTruthy();
    expect(row!.textContent).toContain(copy.readerEvidenceStale);

    await user.type(screen.getByLabelText(copy.readerEvidenceName), "Apple Books");
    await user.type(screen.getByLabelText(copy.readerEvidenceVersion), "7.2");
    await user.click(screen.getByRole("button", { name: copy.readerEvidenceRecord }));

    expect(props.onRecordReaderEvidence).toHaveBeenCalledWith(
      "job-1",
      "child-1",
      "reading_epub",
      "Apple Books",
      "7.2",
      "passed",
    );
  });

  // Nothing to open means nothing to verify; the form must not invite a record
  // that the backend would only reject.
  it("cannot record reader verification before an EPUB is built", async () => {
    const user = userEvent.setup();
    const { container } = renderDrawer(bookUnit({ status: "running" }));
    await user.click(within(container).getByRole("tab", { name: copy.tabArtifacts }));

    expect(screen.getByText(copy.readerEvidenceNeedsBuild)).toBeTruthy();
    expect(screen.queryByRole("button", { name: copy.readerEvidenceRecord })).toBeNull();
  });

  it("reports the gate's approval to the caller with the focused child", async () => {
    const user = userEvent.setup();
    const { card, props } = renderDrawer(gateUnit());
    await user.click(approveButton(card!));
    expect(props.onApproveGate).toHaveBeenCalledWith("job-1", "child-1", "approve_translation");
  });

  it("shows the state card instead of a gate when nothing is pending", () => {
    const { container } = renderDrawer(
      bookUnit({ status: "running", stages: [stage("translate", "running")] }),
    );
    expect(container.querySelector(".pl-gatecard")).toBeNull();
  });
});

/**
 * The backend has had three redaction profiles, configured and covered by a
 * monotonic-disclosure test, since the diagnostic command landed — with no way
 * to reach any of them from the UI, so a user reporting a problem had only
 * screenshots.
 */
// The excerpt used to be blanked by an effect that ran a render after the
// artifact changed, so one frame showed the previous book's text. It now
// carries the artifact id it was read for and is filtered during render.
describe("BookDrawer gate sample preview", () => {
  function unitWithSample(artifactId: string, jobId: string) {
    return gateUnit({
      jobOver: { id: jobId },
      childOver: { artifacts: [artifact("chapter_source", { artifactId })] },
    });
  }

  it("shows the excerpt it read for the current artifact", async () => {
    vi.mocked(readBookPipelineArtifactExcerpt).mockResolvedValue({
      artifactId: "art-1",
      kind: "chapter_source",
      excerpt: "First chapter opening line",
      truncated: false,
    });

    renderDrawer(unitWithSample("art-1", "job-1"));

    expect(await screen.findByText(/First chapter opening line/)).toBeTruthy();
  });

  it("does not show one book's excerpt while the next book's is still loading", async () => {
    vi.mocked(readBookPipelineArtifactExcerpt).mockResolvedValue({
      artifactId: "art-1",
      kind: "chapter_source",
      excerpt: "First chapter opening line",
      truncated: false,
    });
    const first = unitWithSample("art-1", "job-1");
    const second = unitWithSample("art-2", "job-2");
    const props = drawerProps(first, { units: [first, second] });

    const { rerender } = render(<BookDrawer {...props} />);
    await screen.findByText(/First chapter opening line/);

    // A read that never settles: the previous excerpt must not stand in for it.
    vi.mocked(readBookPipelineArtifactExcerpt).mockReturnValue(new Promise(() => {}));
    rerender(<BookDrawer {...props} unit={second} />);

    expect(screen.queryByText(/First chapter opening line/)).toBeNull();
  });
});

describe("BookDrawer diagnostic export", () => {
  const section = () => screen.getByLabelText(copy.diagnosticTitle);
  const profileSelect = () => screen.getByLabelText(copy.diagnosticProfile) as HTMLSelectElement;

  it("offers every profile the backend accepts", () => {
    renderDrawer(gateUnit());
    expect(Array.from(profileSelect().options).map((option) => option.value)).toEqual([
      ...BOOK_PIPELINE_DIAGNOSTIC_PROFILES,
    ]);
  });

  // The default has to be the one that can be pasted anywhere without reading
  // it first: no artifact list, no paths, no error summaries.
  it("defaults to the public-issue profile", () => {
    renderDrawer(gateUnit());
    expect(profileSelect().value).toBe("public-issue");
    expect(within(section()).getByText(copy.diagnosticPublicIssueNote)).toBeTruthy();
  });

  it("says what each profile discloses as it is chosen", async () => {
    const user = userEvent.setup();
    renderDrawer(gateUnit());

    await user.selectOptions(profileSelect(), "redacted-support");
    expect(within(section()).getByText(copy.diagnosticRedactedSupportNote)).toBeTruthy();
    expect(within(section()).queryByText(copy.diagnosticPublicIssueNote)).toBeNull();

    await user.selectOptions(profileSelect(), "local-full");
    expect(within(section()).getByText(copy.diagnosticLocalFullNote)).toBeTruthy();
  });

  it("exports the chosen profile for this book", async () => {
    const user = userEvent.setup();
    const { props } = renderDrawer(gateUnit());

    await user.selectOptions(profileSelect(), "local-full");
    await user.click(within(section()).getByRole("button", { name: copy.diagnosticExport }));

    expect(props.onExportDiagnostic).toHaveBeenCalledWith("job-1", "local-full");
  });
});
