import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { InputIsland, droppedFolder, sourceIdentity } from "./InputIsland";
import { pipelineCopy } from "./copy";
import {
  defaultPipelineDraft,
  isTrackOnlyDraftPatch,
  type PipelineDraft,
  type RouteOverride,
} from "./model";
import type { BookPipelineRouteItem, BookPipelineSource, ModelSlotView } from "../types";

const copy = pipelineCopy("en");

/**
 * The island is a controlled component: App owns the draft and the preview, and
 * its `onDraftChange` merges the patch *and* clears the preview, because any
 * draft change invalidates the routes the backend returned. The harness
 * reproduces that exactly — without the clear, the auto-preflight regressions
 * below cannot happen and a test against them would pass either way.
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
  onQueueRun,
  onSearchZotero,
  modelSlots = [],
  zoteroSources = [],
  draft: draftOverride,
  routes = routesFor,
}: {
  onPreview?: () => void;
  onQueueRun?: () => Promise<boolean>;
  onSearchZotero?: (query: string) => void;
  modelSlots?: ModelSlotView[];
  zoteroSources?: BookPipelineSource[];
  draft?: Partial<PipelineDraft>;
  routes?: (draft: PipelineDraft) => BookPipelineRouteItem[];
}) {
  const [draft, setDraft] = useState<PipelineDraft>({
    ...defaultPipelineDraft,
    sourceKind: "zotero_attachment",
    zoteroSelector: "AAAA1111",
    hasPaddleocrCredentials: false,
    ...draftOverride,
  });
  const [preview, setPreview] = useState<BookPipelineRouteItem[]>([]);
  const [routeOverrides, setRouteOverrides] = useState<Record<string, RouteOverride>>({});

  return (
    <InputIsland
      copy={copy}
      draft={draft}
      preview={preview}
      zoteroSources={zoteroSources}
      modelSlots={modelSlots}
      busy={null}
      variant="compact"
      onDraftChange={(patch) => {
        setDraft((current) => ({ ...current, ...patch }));
        // Mirrors App exactly, including the track-only exemption: a mode patch
        // must not invalidate the preflight, or the choice takes itself off
        // screen on the first click.
        if (!isTrackOnlyDraftPatch(patch)) setPreview([]);
      }}
      onPreview={() => {
        onPreview?.();
        setPreview(routes(draft));
      }}
      onQueueRun={onQueueRun ?? (async () => true)}
      onChooseFolder={() => undefined}
      onSearchZotero={onSearchZotero ?? (() => undefined)}
      routeOverrides={routeOverrides}
      onRouteOverrideChange={(routeItemId, override) =>
        setRouteOverrides((current) => ({ ...current, [routeItemId]: override }))
      }
    />
  );
}

const routeRows = () => document.querySelectorAll(".pl-route-table tbody tr");
const startButton = () =>
  screen.getByRole("button", { name: /^Start/ }) as HTMLButtonElement;

describe("InputIsland preflight", () => {
  it("previews the route in place as soon as a source is present", () => {
    const onPreview = vi.fn();
    render(<Harness onPreview={onPreview} />);

    expect(onPreview).toHaveBeenCalledTimes(1);
    expect(routeRows()).toHaveLength(2);
  });

  it("holds the preflight and the launch until a source is chosen", () => {
    const onPreview = vi.fn();
    render(<Harness onPreview={onPreview} draft={{ zoteroSelector: "" }} />);

    expect(onPreview).not.toHaveBeenCalled();
    expect(routeRows()).toHaveLength(0);
    expect(startButton().disabled).toBe(true);
  });

  // Regression guard carried over from the wizard: App clears the preview on
  // every draft change, so a credential flip has to re-run the preflight or the
  // route list empties with nothing to refill it.
  it("re-previews when the OCR credential flags change", () => {
    const onPreview = vi.fn();
    const { rerender } = render(<Harness onPreview={onPreview} />);
    expect(onPreview).toHaveBeenCalledTimes(1);

    rerender(<Harness onPreview={onPreview} draft={{ hasPaddleocrCredentials: true }} />);

    expect(routeRows()).toHaveLength(2);
    expect(screen.queryByText(copy.noPreview)).toBeNull();
  });

  // A boolean "already previewed" guard would swallow this: the second book
  // never gets a preflight and the user launches against the first one's routes.
  it("re-previews when the chosen book changes", async () => {
    const onPreview = vi.fn();
    const user = userEvent.setup();
    render(
      <Harness
        onPreview={onPreview}
        zoteroSources={[
          { kind: "zotero_attachment", title: "First", selector: "AAAA1111" },
          { kind: "zotero_attachment", title: "Second", selector: "BBBB2222" },
        ]}
      />,
    );
    expect(onPreview).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("option", { name: "Second" }));

    expect(onPreview).toHaveBeenCalledTimes(2);
    expect(routeRows()).toHaveLength(2);
  });

  it("keeps a route override when the preview is re-run", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const overrides = screen.getAllByLabelText(copy.thOverride);
    await user.selectOptions(overrides[0], "keep");

    expect((screen.getAllByLabelText(copy.thOverride)[0] as HTMLSelectElement).value).toBe("keep");
  });
});

describe("InputIsland launch", () => {
  it("counts only the runnable books and queues them", async () => {
    const onQueueRun = vi.fn(async () => true);
    const user = userEvent.setup();
    render(<Harness onQueueRun={onQueueRun} />);

    // One of the two fixture routes is blocked without PaddleOCR credentials.
    expect(screen.getByRole("button", { name: copy.islandEnqueue(1) })).toBeTruthy();

    await user.click(startButton());
    expect(onQueueRun).toHaveBeenCalledTimes(1);
  });

  it("holds the launch when nothing in the batch can run", () => {
    render(<Harness draft={{ zoteroSelector: "" }} />);
    expect(startButton().disabled).toBe(true);
  });

  // Carried over from the wizard: without this the batch runs OCR to completion
  // and only then dies on provider auth at the translate stage.
  it("holds the launch when the chosen provider has no key", () => {
    render(
      <Harness
        draft={{ hasPaddleocrCredentials: true, providerProfileId: "kimi", providerConfigId: "kimi-default" }}
        modelSlots={[
          { profileId: "openai-compatible", configId: "openai-default", providerType: "openai-compatible", defaultModel: "m", configured: true },
          { profileId: "kimi", configId: "kimi-default", providerType: "openai-compatible", defaultModel: "m", configured: false },
        ]}
      />,
    );

    expect(screen.getByText(copy.providerKeyMissing)).toBeTruthy();
    expect(startButton().disabled).toBe(true);
  });

  // getModelCatalog is best-effort and App swallows its rejection. Treating the
  // resulting empty catalog as "nothing is configured" would hold every job back
  // on a transient read failure.
  it("says nothing when the catalog could not be read", () => {
    render(<Harness draft={{ hasPaddleocrCredentials: true }} modelSlots={[]} />);

    expect(screen.queryByText(copy.providerKeyMissing)).toBeNull();
    expect(startButton().disabled).toBe(false);
  });
});

describe("InputIsland Zotero search", () => {
  it("searches on Enter and lists what came back", async () => {
    const onSearchZotero = vi.fn();
    const user = userEvent.setup();
    render(
      <Harness
        onSearchZotero={onSearchZotero}
        zoteroSources={[{ kind: "zotero_attachment", title: "Leviathan", selector: "CCCC3333" }]}
      />,
    );

    await user.type(screen.getByLabelText(copy.zoteroTitleSearch), "leviathan{Enter}");

    expect(onSearchZotero).toHaveBeenCalledWith("leviathan");
    expect(screen.getByRole("option", { name: "Leviathan" })).toBeTruthy();
  });
});

describe("droppedFolder", () => {
  // The conversion wrapper only takes a directory, so a dropped PDF resolves to
  // its folder. The preflight then names every book that folder will run.
  it("keeps a dropped directory as-is", () => {
    expect(droppedFolder(["/shelf/books"])).toBe("/shelf/books");
  });

  it("resolves a dropped PDF to its containing folder", () => {
    expect(droppedFolder(["/shelf/books/Leviathan.pdf"])).toBe("/shelf/books");
    expect(droppedFolder(["C:\\Books\\Leviathan.PDF"])).toBe("C:\\Books");
  });

  it("ignores an empty drop", () => {
    expect(droppedFolder([])).toBeNull();
    expect(droppedFolder(["  "])).toBeNull();
  });
});

describe("sourceIdentity", () => {
  it("separates two Zotero items so a swap re-previews", () => {
    const base = { ...defaultPipelineDraft, sourceKind: "zotero_attachment" as const };
    expect(sourceIdentity({ ...base, zoteroSelector: "A" })).not.toBe(
      sourceIdentity({ ...base, zoteroSelector: "B" }),
    );
  });

  it("separates two local folders", () => {
    const base = { ...defaultPipelineDraft, sourceKind: "local_pdf_folder" as const };
    expect(sourceIdentity({ ...base, localPdfFolder: "/a" })).not.toBe(
      sourceIdentity({ ...base, localPdfFolder: "/b" }),
    );
  });
});

/**
 * The reflow ⇄ layout-preserving choice. The rule it renders is the backend's:
 * one runnable item, routed `direct_text`. Anything else has exactly one track
 * it can take, and the island picks it without asking.
 */
describe("InputIsland track choice", () => {
  const oneTextPdf = (): BookPipelineRouteItem[] => [
    {
      id: "r1",
      title: "A Plain PDF",
      sourceKind: "zotero_attachment",
      sourceRef: "AAAA1111",
      routeKind: "direct_text",
      canRun: true,
      summary: "",
    },
  ];
  const oneScannedPdf = (): BookPipelineRouteItem[] => [
    { ...oneTextPdf()[0], routeKind: "remote_paddleocr" },
  ];
  const layoutOption = () => screen.queryByRole("radio", { name: /Layout-preserving PDF/ });

  it("offers both tracks for a single text PDF", () => {
    render(<Harness routes={oneTextPdf} />);

    expect(screen.getByRole("radio", { name: /Reflowed EPUB/ })).toBeTruthy();
    expect(layoutOption()).toBeTruthy();
    // The reflow track is the default, so nothing changes for a user who
    // ignores the control entirely.
    expect(screen.getByRole("radio", { name: /Reflowed EPUB/ }).getAttribute("aria-checked")).toBe("true");
  });

  it("stays hidden for a scanned book", () => {
    render(<Harness routes={oneScannedPdf} />);
    expect(layoutOption()).toBeNull();
  });

  it("stays hidden for a batch of several books", () => {
    render(<Harness routes={routesFor} draft={{ hasPaddleocrCredentials: true }} />);
    expect(layoutOption()).toBeNull();
  });

  it("records the pick on the draft", async () => {
    const user = userEvent.setup();
    render(<Harness routes={oneTextPdf} />);

    await user.click(layoutOption()!);

    expect(layoutOption()!.getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("radio", { name: /Reflowed EPUB/ }).getAttribute("aria-checked")).toBe("false");
  });

  it("says so when the pick stops applying instead of dropping it silently", async () => {
    const user = userEvent.setup();
    // A draft already carrying the layout choice, against a book that cannot
    // take it: App queues the reflow track, so the island has to say that
    // rather than let the choice evaporate between the click and the shelf.
    render(<Harness routes={oneScannedPdf} draft={{ mode: "layout_preserving" }} />);

    expect(layoutOption()).toBeNull();
    expect(screen.getByText(/needs a single text PDF/)).toBeTruthy();
    await user.click(startButton());
  });

  it("does not block the launch on the draft's provider slot", () => {
    // The layout track resolves its endpoint from the Settings active slot, so
    // an unconfigured draft slot is not a reason to hold the book back.
    render(
      <Harness
        routes={oneTextPdf}
        draft={{ mode: "layout_preserving" }}
        modelSlots={[
          {
            profileId: "somewhere-else",
            configId: "default",
            providerType: "openai-compatible",
            defaultModel: "m",
            configured: true,
          },
        ]}
      />,
    );

    expect(startButton().disabled).toBe(false);
  });
});
