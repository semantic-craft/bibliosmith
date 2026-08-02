import { describe, expect, it } from "vitest";
import { previewBookPipelineRoute, queueBookPipelineJob } from "./api";
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
