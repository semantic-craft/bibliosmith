import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BookDrawer, type BookDrawerProps } from "./BookDrawer";
import { pipelineCopy } from "./copy";
import type { BookUnit } from "./model";
import { MODEL_BRANDS } from "../pages/settings/modelCatalog";
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

function renderDrawer(unit: BookUnit, over: Partial<BookDrawerProps> = {}) {
  const props: BookDrawerProps = {
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
    onSaveCustomInstructions: vi.fn(),
    onApproveGate: vi.fn(),
    onOpenOutput: vi.fn(),
    onHandoff: vi.fn(),
    ...over,
  };
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

describe("BookDrawer provider picker", () => {
  // Regression: the three options were hard-coded while the catalog carried
  // six, so the drawer silently offered half the providers a user could
  // configure in Settings.
  it("offers every brand in the catalog", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "deepseek" } }));

    expect(MODEL_BRANDS.length).toBeGreaterThanOrEqual(6);
    expect(optionValues(providerSelect())).toEqual(MODEL_BRANDS.map((brand) => brand.profileId));
  });

  it("labels each option with its brand name", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "deepseek" } }));
    const labels = Array.from(providerSelect().options).map((option) => option.textContent);
    expect(labels).toEqual(MODEL_BRANDS.map((brand) => brand.brand));
  });

  it("selects the profile the job already carries", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "kimi" } }));
    expect(providerSelect().value).toBe("kimi");
  });

  // A job may predate a rename, or belong to the expert agent. Dropping such a
  // profile from the list would make the drawer silently rewrite it on open.
  it("keeps a profile the catalog no longer lists selectable", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "retired-profile" } }));
    const select = providerSelect();
    expect(select.value).toBe("retired-profile");
    expect(optionValues(select)).toEqual([
      "retired-profile",
      ...MODEL_BRANDS.map((brand) => brand.profileId),
    ]);
  });

  it("does not add a duplicate option for a profile the catalog does list", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "qwen" } }));
    const values = optionValues(providerSelect());
    expect(values.filter((value) => value === "qwen")).toHaveLength(1);
  });

  it("defaults the config box to the chosen brand's first slot", () => {
    renderDrawer(gateUnit({ jobOver: { translationProfileId: "kimi", translationConfigId: "" } }));
    const config = screen.getByLabelText(copy.sampleConfig) as HTMLInputElement;
    const kimi = MODEL_BRANDS.find((brand) => brand.profileId === "kimi");
    expect(config.value).toBe(kimi?.slots[0].configId);
  });

  it("hides the sample controls for an expert-mode job", () => {
    const { card } = renderDrawer(gateUnit({ jobOver: { translationMode: "expert" } }));
    expect(card).not.toBeNull();
    expect(screen.queryByLabelText(copy.sampleProvider)).toBeNull();
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
