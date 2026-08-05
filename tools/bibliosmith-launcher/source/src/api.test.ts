import { describe, expect, it, vi } from "vitest";
import {
  copyTranslationPromptPack,
  deleteTranslationPromptPack,
  diffTranslationPromptPackRevisions,
  getBookPipelineState,
  getTranslationPromptPackDefault,
  listTranslationPromptPacks,
  previewBookTranslationPrompt,
  previewBookPipelineRoute,
  queueBookPipelineJob,
  saveTranslationPromptPackRevision,
  setTranslationPromptPackDefault,
} from "./api";
import type { BookPipelineSource, BookPipelineTranslationIntent } from "./types";

// Every test here runs under jsdom with no __TAURI_INTERNALS__, so `api.ts`
// takes its browser-preview branch. That branch is the subject: it is a hand
// written mirror of the Rust backend, and the point of these tests is that the
// two answer the same way about which modes may be queued and which of them
// hand off to translation. Wording is asserted verbatim against
// `validate_enqueue_mode` in book_pipeline.rs, so a reworded backend message
// fails here instead of drifting quietly.

const source: BookPipelineSource = { kind: "fake", title: "Fake source", selector: "fake://source" };

const intent: BookPipelineTranslationIntent = {
  translationMode: "fast",
  profileId: "fake-provider-profile",
  configId: "fake-provider-config",
  skillIds: [],
  promptPackReference: {
    packId: "builtin.structure-fidelity",
    revisionId: "2026.08.05-1",
    contentSha256: "fb5dae8c498d46a1a3501acd0d6b00645b7dfe4c5c797e8e71732482c5a0c26f",
  },
  secondPassEnabled: false,
  textCleanup: false,
  digestMode: false,
  outputFormats: ["md"],
};

describe("queueBookPipelineJob in the browser preview", () => {
  it("refuses the retired conversion_only mode by name, in the backend's words", async () => {
    await expect(queueBookPipelineJob(source, "conversion_only", intent)).rejects.toThrow(
      "Book Pipeline mode conversion_only was retired: conversion now always continues into translation. Enqueue convert_then_translate instead. Jobs queued before the retirement keep running and stay readable.",
    );
  });

  it("refuses a mode nobody named, and lists the ones that exist", async () => {
    await expect(queueBookPipelineJob(source, "convert_then_translat", intent)).rejects.toThrow(
      "Unknown Book Pipeline mode convert_then_translat. Valid modes: convert_then_translate, translate_only, layout_preserving.",
    );
  });

  it("still queues each enqueueable mode", async () => {
    for (const mode of ["convert_then_translate", "translate_only", "layout_preserving"]) {
      const job = await queueBookPipelineJob(source, mode, intent);
      expect(job.mode, mode).toBe(mode);
    }
  });

  it("gives the layout track the two stages that are its whole run", async () => {
    // ordered_child_stage_ids answers layout_preserving before it asks about the
    // handoff, and answers with two. The mirror used to let it fall through to
    // the arm that keeps pre-retirement conversion_only jobs readable, which put
    // an "index" stage in the preview that the backend never creates -- and
    // ensure_item_index_stage refuses to create it for exactly this mode.
    const job = await queueBookPipelineJob(source, "layout_preserving", intent);

    expect(job.children[0].stages.map((stage) => stage.stageId)).toEqual(["route", "extract"]);
  });

  it("gives a queued translating job the full stage list, never the retired three", async () => {
    const job = await queueBookPipelineJob(source, "convert_then_translate", intent);
    const stageIds = job.children[0].stages.map((stage) => stage.stageId);

    expect(stageIds.slice(0, 3)).toEqual(["route", "extract", "index"]);
    expect(stageIds).toContain("handoff");
    expect(stageIds).toContain("translate");
  });
});

describe("previewBookPipelineRoute in the browser preview", () => {
  const handoffs = (route: Awaited<ReturnType<typeof previewBookPipelineRoute>>) =>
    route.filter((item) => item.routeKind === "translation_handoff");

  it("appends the translation handoff for a mode it has never heard of", async () => {
    // The backend phrases this as an exclusion, so an unfamiliar mode translates.
    // Listing the known translating modes instead looked equivalent and was not:
    // it silently gave anything unrecognised the retired conversion-only shape.
    expect(handoffs(await previewBookPipelineRoute(source, "convert_then_translat"))).toHaveLength(1);
  });

  it("appends it for the translating modes", async () => {
    expect(handoffs(await previewBookPipelineRoute(source, "convert_then_translate"))).toHaveLength(1);
    expect(handoffs(await previewBookPipelineRoute(source, "translate_only"))).toHaveLength(1);
  });

  it("withholds it from the two modes that stop after extraction", async () => {
    // conversion_only never translated; the layout-preserving track already has,
    // in the single pass that is its whole run -- there is no Markdown to hand
    // anywhere. Queueing is refused for the first and allowed for the second, so
    // only the route preview can show that they share this answer.
    expect(handoffs(await previewBookPipelineRoute(source, "conversion_only"))).toHaveLength(0);
    expect(handoffs(await previewBookPipelineRoute(source, "layout_preserving"))).toHaveLength(0);
  });
});

describe("prompt-pack management in the browser preview", () => {
  it("rejects local drafts that change locked stage metadata", async () => {
    const catalog = await listTranslationPromptPacks();
    const sourceRevision = catalog.packs
      .find((pack) => pack.packId === "builtin.structure-fidelity")!
      .revisions.at(-1)!;
    const copied = await copyTranslationPromptPack({
      packId: sourceRevision.packId,
      revisionId: sourceRevision.revisionId,
      contentSha256: sourceRevision.contentSha256,
    }, "预览契约锁定测试");
    const first = copied.revisions[0];

    await expect(saveTranslationPromptPackRevision({
      packId: copied.packId,
      displayName: first.displayName,
      parameters: {},
      stages: first.stages.map((stage) => ({
        ...stage,
        label: `${stage.label}（已改写）`,
      })),
    })).rejects.toThrow("Prompt pack executor contract is read-only.");
  });

  it("adopts a new default only for subsequent jobs and leaves queued bindings unchanged", async () => {
    const originalDefault = await getTranslationPromptPackDefault("programmatic");
    const nextDefault = {
      packId: "builtin.four-dimension-refinement",
      revisionId: "2026.08.05-2",
      contentSha256: "e86141d65f8bfb4a674c597f157a86c6da80d49ac02d081d21f2ca325c8ea2e8",
    };
    try {
      await setTranslationPromptPackDefault("programmatic", originalDefault);
      const existing = await queueBookPipelineJob(source, "translate_only", {
        ...intent,
        promptPackReference: originalDefault,
      });

      await setTranslationPromptPackDefault("programmatic", nextDefault);
      const subsequent = await queueBookPipelineJob(source, "translate_only", {
        ...intent,
        promptPackReference: nextDefault,
        secondPassEnabled: true,
      });
      const state = await getBookPipelineState();

      expect(await getTranslationPromptPackDefault("programmatic")).toEqual(nextDefault);
      expect(state.jobs.find((job) => job.id === existing.id)?.promptPackReference).toEqual(originalDefault);
      expect(state.jobs.find((job) => job.id === subsequent.id)?.promptPackReference).toEqual(nextDefault);
      expect(state.jobs.find((job) => job.id === existing.id)?.promptPackSelectionSource).toBe("default");
    } finally {
      await setTranslationPromptPackDefault("programmatic", originalDefault);
    }
  });

  it("assigns distinct identities to copies created in the same millisecond", async () => {
    const now = vi.spyOn(Date, "now").mockReturnValue(1_786_000_000_000);
    try {
      const catalog = await listTranslationPromptPacks();
      const sourceRevision = catalog.packs
        .find((pack) => pack.packId === "builtin.structure-fidelity")!
        .revisions.at(-1)!;
      const reference = {
        packId: sourceRevision.packId,
        revisionId: sourceRevision.revisionId,
        contentSha256: sourceRevision.contentSha256,
      };

      const [first, second] = await Promise.all([
        copyTranslationPromptPack(reference, "同毫秒副本一"),
        copyTranslationPromptPack(reference, "同毫秒副本二"),
      ]);

      expect(first.packId).not.toBe(second.packId);
    } finally {
      now.mockRestore();
    }
  });

  it("normalizes expert skill dependency versions into the public preview contract", async () => {
    const expertIntent: BookPipelineTranslationIntent = {
      ...intent,
      translationMode: "expert",
      skillIds: ["expert-translation-quality"],
      promptPackReference: {
        packId: "builtin.context-backtracking",
        revisionId: "2026.08.05-1",
        contentSha256: "13d0d89ed81c8572311c31dbb8be56c95b583a9a0a86f779ad7ae8b1ec1e5fc7",
      },
    };
    const job = await queueBookPipelineJob(source, "translate_only", expertIntent);

    const preview = await previewBookTranslationPrompt(job.id, job.children[0].id);

    expect(preview.skillDependencyVersions?.["expert-translation-quality"]).toMatch(
      /^sha256:[0-9a-f]{64}$/,
    );
  });

  it("uses content hashes and retains tombstoned revisions for historical resolution", async () => {
    const catalog = await listTranslationPromptPacks();
    const sourceRevision = catalog.packs
      .find((pack) => pack.packId === "builtin.context-backtracking")!
      .revisions.at(-1)!;
    const copied = await copyTranslationPromptPack({
      packId: sourceRevision.packId,
      revisionId: sourceRevision.revisionId,
      contentSha256: sourceRevision.contentSha256,
    }, "本地语境方案");
    const first = copied.revisions[0];
    expect(first.contentSha256).toMatch(/^[0-9a-f]{64}$/);
    expect((first.source.skillVersions as Record<string, string>)["expert-translation-quality"]).toMatch(/^sha256:[0-9a-f]{64}$/);

    const second = await saveTranslationPromptPackRevision({
      packId: copied.packId,
      displayName: "本地语境方案（二版）",
      parameters: { styleGuidance: "克制的现代汉语" },
      stages: first.stages,
    });
    expect(second.contentSha256).toMatch(/^[0-9a-f]{64}$/);
    expect(second.contentSha256).not.toBe(first.contentSha256);

    await deleteTranslationPromptPack(copied.packId);
    const diff = await diffTranslationPromptPackRevisions(
      { packId: first.packId, revisionId: first.revisionId, contentSha256: first.contentSha256 },
      { packId: second.packId, revisionId: second.revisionId, contentSha256: second.contentSha256 },
    );
    expect(diff.afterMetadata.parameters.styleGuidance).toBe("克制的现代汉语");
  });
});
