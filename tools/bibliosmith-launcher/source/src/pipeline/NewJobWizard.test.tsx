import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { NewJobWizard } from "./NewJobWizard";
import { pipelineCopy } from "./copy";
import { defaultPipelineDraft, type PipelineDraft, type RouteOverride } from "./model";
import type { BookPipelineRouteItem, ModelSlotView } from "../types";

const copy = pipelineCopy("en");

/**
 * The wizard is a controlled component: App owns the draft and the preview, and
 * `onDraftChange` there merges the patch *and* clears the preview, because any
 * draft change invalidates the routes the backend returned. This harness
 * reproduces exactly that contract — without the clear, the credential-chip
 * regression cannot happen and a test against it would pass either way.
 *
 * See App.tsx: `setPipelineDraft(d => ({...d, ...patch})); setPipelinePreview([])`.
 */
function routesFor(draft: PipelineDraft): BookPipelineRouteItem[] {
  const paddle = draft.hasPaddleocrCredentials;
  return [
    {
      id: "r1",
      title: "A Plain PDF",
      sourceKind: "zotero_attachment",
      sourceRef: "AAAA1111",
      routeKind: "direct_text",
      canRun: true,
      summary: "",
    },
    {
      id: "r2",
      title: "A Scanned PDF",
      sourceKind: "zotero_attachment",
      sourceRef: "BBBB2222",
      // What the backend does with the same flag: no credentials, no OCR route.
      routeKind: paddle ? "remote_paddleocr" : "missing_credentials",
      canRun: paddle,
      summary: "",
    },
  ];
}

function Harness({
  onPreview,
  modelSlots = [],
  draft: draftOverride,
}: {
  onPreview?: () => void;
  modelSlots?: ModelSlotView[];
  draft?: Partial<PipelineDraft>;
}) {
  const [draft, setDraft] = useState<PipelineDraft>({
    ...defaultPipelineDraft,
    sourceKind: "zotero_collection",
    zoteroSelector: "reading-queue",
    hasPaddleocrCredentials: false,
    ...draftOverride,
  });
  const [preview, setPreview] = useState<BookPipelineRouteItem[]>([]);
  const [routeOverrides, setRouteOverrides] = useState<Record<string, RouteOverride>>({});

  return (
    <NewJobWizard
      copy={copy}
      draft={draft}
      preview={preview}
      zoteroSources={[]}
      modelSlots={modelSlots}
      busy={null}
      onDraftChange={(patch) => {
        setDraft((current) => ({ ...current, ...patch }));
        setPreview([]);
      }}
      onPreview={() => {
        onPreview?.();
        setPreview(routesFor(draft));
      }}
      onQueueRun={async () => true}
      onChooseFolder={() => undefined}
      onChooseMarkdown={() => undefined}
      onDiscoverZotero={() => undefined}
      onSearchZotero={() => undefined}
      onClose={() => undefined}
      routeOverrides={routeOverrides}
      onRouteOverrideChange={(routeItemId, override) =>
        setRouteOverrides((current) => ({ ...current, [routeItemId]: override }))
      }
    />
  );
}

const routeRows = () => document.querySelectorAll(".pl-route-table tbody tr");
const nextToConfirm = () =>
  screen.getByRole("button", { name: copy.wizNextConfirm }) as HTMLButtonElement;

async function goToPreflight(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: copy.wizNextPreflight }));
}

describe("NewJobWizard preflight step", () => {
  it("previews the route on entering the step", async () => {
    const onPreview = vi.fn();
    const user = userEvent.setup();
    render(<Harness onPreview={onPreview} />);

    expect(onPreview).not.toHaveBeenCalled();
    await goToPreflight(user);

    expect(onPreview).toHaveBeenCalledTimes(1);
    expect(routeRows()).toHaveLength(2);
    expect(nextToConfirm().disabled).toBe(false);
  });

  // Regression: the preview effect was keyed on `step` alone. A credential chip
  // goes through onDraftChange, which clears the preview, so clicking one
  // emptied the route table and disabled "Next" with no way forward — a dead
  // end recoverable only by stepping back to the source step and in again.
  it("keeps the route table and Next alive after toggling a credential chip", async () => {
    const onPreview = vi.fn();
    const user = userEvent.setup();
    render(<Harness onPreview={onPreview} />);
    await goToPreflight(user);

    await user.click(screen.getByTitle(copy.paddleCreds));

    expect(onPreview).toHaveBeenCalledTimes(2);
    expect(routeRows()).toHaveLength(2);
    expect(nextToConfirm().disabled).toBe(false);
    expect(screen.queryByText(copy.noPreview)).toBeNull();
  });

  it("re-routes the affected book in both directions", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await goToPreflight(user);

    expect(screen.getByText(copy.routeMissingCredentials)).toBeTruthy();
    expect(screen.getAllByText(copy.preflightBlocked)).toHaveLength(1);

    await user.click(screen.getByTitle(copy.paddleCreds));
    expect(screen.getByText(copy.routeRemotePaddle)).toBeTruthy();
    expect(screen.getAllByText(copy.preflightReady)).toHaveLength(2);

    await user.click(screen.getByTitle(copy.paddleCreds));
    expect(screen.getByText(copy.routeMissingCredentials)).toBeTruthy();
    expect(screen.getAllByText(copy.preflightReady)).toHaveLength(1);
  });

  it("re-previews for the MinerU chip too", async () => {
    const onPreview = vi.fn();
    const user = userEvent.setup();
    render(<Harness onPreview={onPreview} />);
    await goToPreflight(user);

    await user.click(screen.getByTitle(copy.mineruCreds));

    expect(onPreview).toHaveBeenCalledTimes(2);
    expect(routeRows()).toHaveLength(2);
    expect(nextToConfirm().disabled).toBe(false);
  });

  it("carries the chip through to the confirm step", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await goToPreflight(user);

    await user.click(screen.getByTitle(copy.paddleCreds));
    await user.click(nextToConfirm());

    // Both books are launchable once the OCR route is available.
    expect(screen.getByText(copy.routesUnit(2))).toBeTruthy();
  });

  it("keeps a route override when the preview is re-run", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await goToPreflight(user);

    const overrides = screen.getAllByLabelText(copy.thOverride);
    await user.selectOptions(overrides[0], "keep");
    expect((overrides[0] as HTMLSelectElement).value).toBe("keep");

    await user.click(screen.getByTitle(copy.mineruCreds));
    expect((screen.getAllByLabelText(copy.thOverride)[0] as HTMLSelectElement).value).toBe("keep");
  });
});

describe("NewJobWizard source step", () => {
  it("holds Next until a source is chosen", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByText(copy.srcLocalPdfTitle));
    const next = screen.getByRole("button", { name: copy.wizNextPreflight }) as HTMLButtonElement;
    expect(next.disabled).toBe(true);
    expect(screen.getByText(copy.sourceMissing)).toBeTruthy();
  });
});

/**
 * The picker used to offer every slot with no hint about which had a key, so a
 * user could send a batch through a quarter-hour of OCR and only then hit
 * provider auth at the translate stage. The OCR side of the wizard has always
 * had this: real credential status from the backend, visible in the chips.
 */
describe("NewJobWizard provider credentials", () => {
  const translating = { mode: "convert_then_translate" as const, translationMode: "fast" as const };
  const slot = (profileId: string, configId: string, configured: boolean): ModelSlotView => ({
    profileId,
    configId,
    providerType: "openai-compatible",
    defaultModel: "m",
    configured,
  });
  // Only the OpenAI slot has a key; Kimi is the ticket's example of a provider a
  // user can pick without ever having entered one.
  const catalog = [
    slot("openai-compatible", "openai-default", true),
    slot("kimi", "kimi-default", false),
  ];
  const providerSelect = () => screen.getByLabelText(copy.provider) as HTMLSelectElement;
  const launchButton = () => screen.getByRole("button", { name: copy.wizLaunch }) as HTMLButtonElement;

  async function goToConfirm(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole("button", { name: copy.wizNextPreflight }));
    await user.click(nextToConfirm());
  }

  it("marks the slots the catalog has no key for", () => {
    render(<Harness draft={translating} modelSlots={catalog} />);

    const options = [...providerSelect().options];
    const kimi = options.find((option) => option.value === "kimi:kimi-default");
    const openai = options.find((option) => option.value === "openai-compatible:openai-default");
    expect(kimi?.text).toContain(copy.providerUnconfigured);
    expect(openai?.text).not.toContain(copy.providerUnconfigured);
    // Still selectable: Settings may have made an unconfigured slot the active
    // one, and disabling the current value would strand the picker.
    expect(kimi?.disabled).toBe(false);
  });

  it("holds the launch when the chosen provider has no key", async () => {
    const user = userEvent.setup();
    render(<Harness draft={translating} modelSlots={catalog} />);

    await user.selectOptions(providerSelect(), "kimi:kimi-default");
    await goToConfirm(user);

    expect(screen.getByText(copy.providerKeyMissing)).toBeTruthy();
    expect(launchButton().disabled).toBe(true);
  });

  it("launches normally once the chosen provider has a key", async () => {
    const user = userEvent.setup();
    render(<Harness draft={translating} modelSlots={catalog} />);

    await user.selectOptions(providerSelect(), "openai-compatible:openai-default");
    await goToConfirm(user);

    expect(screen.queryByText(copy.providerKeyMissing)).toBeNull();
    expect(launchButton().disabled).toBe(false);
  });

  // getModelCatalog is best-effort and App swallows its rejection. Treating the
  // resulting empty catalog as "nothing is configured" would hold back every job
  // on a transient read failure.
  it("says nothing when the catalog could not be read", async () => {
    const user = userEvent.setup();
    render(<Harness draft={translating} modelSlots={[]} />);

    expect(
      [...providerSelect().options].some((option) => option.text.includes(copy.providerUnconfigured)),
    ).toBe(false);

    await goToConfirm(user);
    expect(screen.queryByText(copy.providerKeyMissing)).toBeNull();
    expect(launchButton().disabled).toBe(false);
  });

  // Expert mode substitutes the "expert-agent" profile and a conversion-only job
  // never translates, so the picked slot is inert and must not hold the launch.
  it("ignores the picker for a job that will not use it", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        draft={{ ...translating, translationMode: "expert" }}
        modelSlots={[slot("openai-compatible", "openai-default", false)]}
      />,
    );

    await goToConfirm(user);
    expect(screen.queryByText(copy.providerKeyMissing)).toBeNull();
    expect(launchButton().disabled).toBe(false);
  });
});
