import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { InputIsland, droppedFolder, sourceIdentity } from "./InputIsland";
import { pipelineCopy } from "./copy";
import { defaultPipelineDraft, type PipelineDraft, type RouteOverride } from "./model";
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
}: {
  onPreview?: () => void;
  onQueueRun?: () => Promise<boolean>;
  onSearchZotero?: (query: string) => void;
  modelSlots?: ModelSlotView[];
  zoteroSources?: BookPipelineSource[];
  draft?: Partial<PipelineDraft>;
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
        setPreview([]);
      }}
      onPreview={() => {
        onPreview?.();
        setPreview(routesFor(draft));
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
