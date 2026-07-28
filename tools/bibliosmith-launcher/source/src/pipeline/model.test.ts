import { describe, expect, it } from "vitest";
import { pipelineCopy } from "./copy";
import {
  FOUR_STEPS,
  PIPELINE_STAGE_ORDER,
  activeStepIndex,
  approvalRecords,
  buildGateView,
  flattenBookUnits,
  fourStepStates,
  orderedStages,
  pendingGates,
  providerDefaultConfig,
  routeKindLabel,
  routeTone,
  stageLabel,
  stepCaption,
  unitAdvanceAction,
  unitNote,
  unitProgress,
} from "./model";
import { MODEL_BRANDS } from "../pages/settings/modelCatalog";
import {
  approvalRef,
  artifact,
  bookUnit,
  childJob,
  job,
  stage,
  unitSummary,
} from "../test/fixtures";

const copy = pipelineCopy("en");

describe("stage contract", () => {
  // Regression: PIPELINE_STAGE_ORDER used to list 12 stages while the backend
  // persisted 14. `index` and `build_digest` were the missing ids.
  it("carries every id the backend can persist, in backend order", () => {
    expect(PIPELINE_STAGE_ORDER).toContain("index");
    expect(PIPELINE_STAGE_ORDER).toContain("build_digest");
    const order = PIPELINE_STAGE_ORDER as readonly string[];
    expect(order.indexOf("index")).toBeGreaterThan(order.indexOf("extract"));
    expect(order.indexOf("index")).toBeLessThan(order.indexOf("handoff"));
    expect(order.indexOf("build_digest")).toBeGreaterThan(order.indexOf("validate_reading"));
  });

  // Regression: an id missing from the order ranks last, so `index` rendered
  // after `validate_reading` and read as the newest thing that happened.
  it("orders index ahead of the late stages regardless of input order", () => {
    const ordered = orderedStages([
      stage("validate_reading", "pending"),
      stage("index", "running"),
      stage("extract", "completed"),
    ]);
    expect(ordered.map((entry) => entry.stageId)).toEqual([
      "extract",
      "index",
      "validate_reading",
    ]);
  });

  it("appends unknown stages after the contract, keeping their relative order", () => {
    const ordered = orderedStages([
      stage("some_future_stage", "pending"),
      stage("another_future_stage", "pending"),
      stage("route", "completed"),
    ]);
    expect(ordered.map((entry) => entry.stageId)).toEqual([
      "route",
      "some_future_stage",
      "another_future_stage",
    ]);
  });

  // Regression: with no label entry, the timeline showed the bare id.
  it("labels index and build_digest instead of showing the raw id", () => {
    expect(stageLabel("index", copy)).toBe(copy.stageIndex);
    expect(stageLabel("index", copy)).not.toBe("index");
    expect(stageLabel("build_digest", copy)).toBe(copy.stageBuildDigest);
    expect(stageLabel("build_digest", copy)).not.toBe("build digest");
  });

  it("falls back to a de-underscored id for a stage it does not know", () => {
    expect(stageLabel("some_future_stage", copy)).toBe("some future stage");
  });

  // Regression: `index` belonged to no group, so its failure turned no circle
  // red. The AssertNever guard in model.ts is compile-time only — this checks
  // the grouping itself, including that no stage was double-counted.
  it("assigns every contract stage to exactly one of the four steps", () => {
    const counts = new Map<string, number>();
    for (const step of FOUR_STEPS) {
      for (const stageId of step.stageIds) {
        counts.set(stageId, (counts.get(stageId) ?? 0) + 1);
      }
    }
    for (const stageId of PIPELINE_STAGE_ORDER) {
      expect(counts.get(stageId), `${stageId} is in ${counts.get(stageId) ?? 0} groups`).toBe(1);
    }
    expect(counts.size).toBe(PIPELINE_STAGE_ORDER.length);
  });
});

describe("fourStepStates", () => {
  // Regression: this is the defect the missing stage id actually caused —
  // a book stuck on a failed index reported itself as fine.
  it("turns the tidy step red when index fails", () => {
    const unit = bookUnit({
      status: "failed",
      stages: [
        stage("route", "completed"),
        stage("extract", "completed"),
        stage("index", "failed"),
      ],
    });
    expect(fourStepStates(unit)).toEqual(["done", "error", "none", "none"]);
  });

  it("turns a step red when any of its stages is blocked outside a gate", () => {
    const unit = bookUnit({
      status: "blocked",
      stages: [stage("route", "completed"), stage("extract", "blocked")],
    });
    expect(fourStepStates(unit)[1]).toBe("error");
  });

  it("reports a pending gate as gate, not as an error", () => {
    const unit = bookUnit({
      status: "waiting_for_approval",
      stages: [
        stage("route", "completed"),
        stage("prepare", "completed"),
        stage("approve_translation", "waiting_for_approval"),
      ],
    });
    expect(fourStepStates(unit)[2]).toBe("gate");
  });

  it("treats a gate the backend reports as ready as pending too", () => {
    const unit = bookUnit({
      status: "ready",
      stages: [stage("approve_promotion", "ready")],
    });
    expect(fourStepStates(unit)[3]).toBe("gate");
  });

  it("leaves a step with no stages as none rather than done", () => {
    const unit = bookUnit({
      status: "completed",
      stages: [stage("route", "completed"), stage("extract", "completed")],
    });
    expect(fourStepStates(unit)).toEqual(["done", "done", "none", "none"]);
  });

  it("counts skipped stages as done", () => {
    const unit = bookUnit({
      status: "completed",
      stages: [stage("route", "completed"), stage("extract", "skipped")],
    });
    expect(fourStepStates(unit)[1]).toBe("done");
  });

  it("promotes the first open step to current for an idle book", () => {
    const unit = bookUnit({
      status: "pending",
      stages: [stage("route", "completed"), stage("extract", "pending"), stage("split", "pending")],
    });
    expect(fourStepStates(unit)).toEqual(["done", "current", "none", "none"]);
  });

  it("does not invent a current step for a finished book", () => {
    const unit = bookUnit({
      status: "completed",
      stages: [stage("route", "completed"), stage("extract", "pending")],
    });
    expect(fourStepStates(unit)).toEqual(["done", "todo", "none", "none"]);
  });
});

describe("activeStepIndex", () => {
  it("picks the first step that needs attention", () => {
    expect(activeStepIndex(["done", "current", "gate", "todo"])).toBe(1);
    expect(activeStepIndex(["done", "todo", "error", "todo"])).toBe(2);
  });

  it("returns -1 when nothing is open", () => {
    expect(activeStepIndex(["done", "done", "none", "none"])).toBe(-1);
  });
});

describe("stepCaption", () => {
  it("names the gate rather than the step when one is pending", () => {
    const translation = bookUnit({
      status: "waiting_for_approval",
      stages: [stage("approve_translation", "waiting_for_approval")],
    });
    expect(stepCaption(translation, copy)).toBe(copy.capGateTranslation);

    const promotion = bookUnit({
      status: "waiting_for_approval",
      stages: [stage("approve_promotion", "waiting_for_approval")],
    });
    expect(stepCaption(promotion, copy)).toBe(copy.capGatePromotion);
  });

  it("appends the error summary on a failed step", () => {
    const unit = bookUnit({
      status: "failed",
      stages: [
        stage("extract", "failed", {
          safeError: {
            code: "E_EXTRACT",
            summary: "no text layer",
            retryable: false,
            attempt: 1,
            stageId: "extract",
            timestamp: "2026-01-01T00:00:00Z",
          },
        }),
      ],
    });
    expect(stepCaption(unit, copy)).toBe(`${copy.step2} · no text layer`);
  });

  it("falls back to a generic note when a failed step carries no error text", () => {
    const unit = bookUnit({ status: "failed", stages: [stage("extract", "failed")] });
    expect(stepCaption(unit, copy)).toBe(`${copy.step2} · ${copy.capNeedsAttention}`);
  });

  it("shows unit counts while a stage is running", () => {
    const unit = bookUnit({
      status: "running",
      stages: [
        stage("translate", "running", { unitSummary: unitSummary({ total: 26, completed: 12 }) }),
      ],
    });
    expect(stepCaption(unit, copy)).toBe(`${copy.step3} · 12/26`);
  });

  it("reports a finished book as all done", () => {
    const unit = bookUnit({ status: "completed", stages: [stage("route", "completed")] });
    expect(stepCaption(unit, copy)).toBe(copy.capAllDone);
  });
});

describe("unitProgress", () => {
  it("is 1 for a finished book", () => {
    expect(unitProgress(bookUnit({ status: "completed" }))).toBe(1);
    expect(unitProgress(bookUnit({ status: "skipped" }))).toBe(1);
  });

  it("is null when there are no stages to reason about", () => {
    expect(unitProgress(bookUnit({ status: "pending" }))).toBeNull();
  });

  it("is null for a queued book that has not closed a stage", () => {
    expect(unitProgress(bookUnit({ status: "pending", stages: [stage("route", "pending")] }))).toBeNull();
  });

  it("is the closed fraction when nothing is running", () => {
    const unit = bookUnit({
      status: "running",
      stages: [stage("route", "completed"), stage("extract", "pending")],
    });
    expect(unitProgress(unit)).toBe(0.5);
  });

  it("interpolates within the running stage using its unit counts", () => {
    const unit = bookUnit({
      status: "running",
      stages: [
        stage("route", "completed"),
        stage("translate", "running", {
          unitSummary: unitSummary({ total: 10, completed: 4, skipped: 1 }),
        }),
      ],
    });
    // stage 1 of 2, half its units closed → (1 + 0.5) / 2
    expect(unitProgress(unit)).toBe(0.75);
  });
});

describe("unitNote", () => {
  it("shows stage progress while running", () => {
    const unit = bookUnit({
      status: "running",
      stages: [
        stage("translate", "running", { unitSummary: unitSummary({ total: 26, completed: 12 }) }),
      ],
    });
    expect(unitNote(unit, copy)).toBe(`${copy.stageTranslate} · 12/26`);
  });

  it("names the gate and asks for a decision while waiting for approval", () => {
    const unit = bookUnit({
      status: "waiting_for_approval",
      stages: [stage("approve_promotion", "waiting_for_approval")],
    });
    expect(unitNote(unit, copy)).toBe(`${copy.gatePromotion} · ${copy.stageWaitingYou}`);
  });

  it("prefers the stage error text on a failed book", () => {
    const unit = bookUnit({
      status: "failed",
      stages: [stage("extract", "failed", { error: "pdftotext exited 1" })],
    });
    expect(unitNote(unit, copy)).toBe("pdftotext exited 1");
  });

  it("lists the reading formats a finished book produced", () => {
    const unit = bookUnit({
      status: "completed",
      stages: [stage("build_reading", "completed")],
      childOver: {
        artifacts: [artifact("reading_markdown"), artifact("reading_epub")],
      },
    });
    expect(unitNote(unit, copy)).toBe(`${copy.statusCompleted} · MD / EPUB`);
  });
});

describe("flattenBookUnits", () => {
  it("gives a collection one row per child", () => {
    const children = [
      childJob({ id: "child-1", source: { kind: "zotero_attachment", title: "First" }, status: "running" }),
      childJob({ id: "child-2", source: { kind: "zotero_attachment", title: "Second" }, status: "completed" }),
    ];
    const units = flattenBookUnits([job({ id: "job-1", kind: "collection", children })]);
    expect(units.map((unit) => [unit.key, unit.title, unit.status])).toEqual([
      ["job-1/child-1", "First", "running"],
      ["job-1/child-2", "Second", "completed"],
    ]);
  });

  // A dropped book cannot leave the job — its collection membership is frozen —
  // so the shelf is where it stops existing.
  it("leaves a dropped book off the shelf without touching its siblings", () => {
    const children = [
      childJob({ id: "child-1", source: { kind: "zotero_attachment", title: "First" }, status: "completed", removedAt: "2026-07-27T00:00:00Z" }),
      childJob({ id: "child-2", source: { kind: "zotero_attachment", title: "Second" }, status: "completed" }),
      childJob({ id: "child-3", source: { kind: "zotero_attachment", title: "Third" }, status: "running" }),
    ];
    const units = flattenBookUnits([job({ id: "job-1", kind: "collection", children })]);
    expect(units.map((unit) => unit.title)).toEqual(["Second", "Third"]);
  });

  // Down to one live book, the row is the job again — same as a job that only
  // ever had one — so the drawer's next/prev and delete keep working.
  it("keys the last remaining book of a batch on the job", () => {
    const children = [
      childJob({ id: "child-1", status: "completed", removedAt: "2026-07-27T00:00:00Z" }),
      childJob({ id: "child-2", source: { kind: "zotero_attachment", title: "Only" }, status: "completed" }),
    ];
    const units = flattenBookUnits([job({ id: "job-1", kind: "collection", children })]);
    expect(units).toHaveLength(1);
    expect(units[0].key).toBe("job-1");
    expect(units[0].child?.id).toBe("child-2");
  });

  it("gives a single job one row keyed on the job, taking the child's status", () => {
    const child = childJob({ id: "child-1", status: "running" });
    const units = flattenBookUnits([job({ id: "job-1", children: [child], status: "pending" })]);
    expect(units).toHaveLength(1);
    expect(units[0].key).toBe("job-1");
    expect(units[0].child).toBe(child);
    expect(units[0].status).toBe("running");
  });

  it("still lists a job whose children the backend has not materialized", () => {
    const units = flattenBookUnits([
      job({ id: "job-1", children: [], status: "pending", source: { kind: "local_pdf_folder", path: "/books" } }),
    ]);
    expect(units).toHaveLength(1);
    expect(units[0].child).toBeNull();
    expect(units[0].title).toBe("/books");
    expect(units[0].status).toBe("pending");
  });

  it("falls back to the id when a source carries no title, selector or path", () => {
    const units = flattenBookUnits([job({ id: "job-1", children: [], source: { kind: "fake" } })]);
    expect(units[0].title).toBe("job-1");
  });
});

describe("buildGateView", () => {
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
    });
  }

  it("returns null when the gate stage is absent or already closed", () => {
    expect(buildGateView(bookUnit({ stages: [stage("route", "completed")] }), "approve_translation", copy)).toBeNull();
    expect(
      buildGateView(
        bookUnit({ stages: [stage("approve_translation", "completed")] }),
        "approve_translation",
        copy,
      ),
    ).toBeNull();
  });

  it("accepts both vocabularies the backend uses for a reached gate", () => {
    for (const status of ["waiting_for_approval", "ready"]) {
      const unit = bookUnit({ stages: [stage("approve_translation", status)] });
      expect(buildGateView(unit, "approve_translation", copy)).not.toBeNull();
    }
  });

  it("reports unknown hashes when the backend bound none", () => {
    const view = buildGateView(gateUnit(), "approve_translation", copy);
    const hashes = view?.checks.find((check) => check.title === copy.checkHashes);
    expect(hashes?.ok).toBe("unknown");
    expect(view?.invalidated).toBe(false);
  });

  it("invalidates the gate when a bound artifact hash no longer matches", () => {
    const unit = gateUnit({
      childOver: {
        artifacts: [artifact("chapter_source", { artifactId: "art-1", sha256: "ffff" })],
      },
      jobOver: {
        approvalReferences: [
          approvalRef({ boundArtifactHashes: { "art-1": "aaaa" }, childJobId: "child-1" }),
        ],
      },
    });
    const view = buildGateView(unit, "approve_translation", copy);
    expect(view?.invalidated).toBe(true);
    expect(view?.checks.find((check) => check.title === copy.checkHashes)?.ok).toBe(false);
    expect(view?.boundEntries).toEqual([
      { artifactId: "art-1", bound: "aaaa", current: "ffff", match: false },
    ]);
  });

  it("passes the hash check when every bound artifact still matches", () => {
    const unit = gateUnit({
      childOver: {
        artifacts: [artifact("chapter_source", { artifactId: "art-1", sha256: "aaaa" })],
      },
      jobOver: {
        approvalReferences: [
          approvalRef({ boundArtifactHashes: { "art-1": "aaaa" }, childJobId: "child-1" }),
        ],
      },
    });
    const view = buildGateView(unit, "approve_translation", copy);
    expect(view?.invalidated).toBe(false);
    expect(view?.checks.find((check) => check.title === copy.checkHashes)?.ok).toBe(true);
  });

  it("fails the blocker check when an upstream stage is still failed", () => {
    const unit = bookUnit({
      status: "waiting_for_approval",
      stages: [
        stage("extract", "failed"),
        stage("approve_translation", "waiting_for_approval"),
      ],
    });
    const view = buildGateView(unit, "approve_translation", copy);
    const blocker = view?.checks.find((check) => check.title === copy.checkNoBlocker);
    expect(blocker?.ok).toBe(false);
    expect(blocker?.detail).toBe(copy.blockerFound);
  });

  it("ignores downstream stages when looking for blockers", () => {
    const unit = bookUnit({
      status: "waiting_for_approval",
      stages: [
        stage("approve_translation", "waiting_for_approval"),
        stage("promote", "blocked"),
      ],
    });
    expect(
      buildGateView(unit, "approve_translation", copy)?.checks.find(
        (check) => check.title === copy.checkNoBlocker,
      )?.ok,
    ).toBe(true);
  });

  it("reports the provider on the translation gate", () => {
    const unit = gateUnit({
      jobOver: { translationProfileId: "deepseek", translationConfigId: "deepseek-default" },
    });
    const provider = buildGateView(unit, "approve_translation", copy)?.checks.find(
      (check) => check.title === copy.checkProvider,
    );
    expect(provider?.ok).toBe(true);
    expect(provider?.detail).toBe("deepseek · deepseek-default");
  });

  it("holds the promotion gate until expert QA is closed", () => {
    const stages = (qaStatus: string, summary = unitSummary({ total: 4, completed: 4 })) => [
      stage("expert_qa", qaStatus, { unitSummary: summary }),
      stage("approve_promotion", "waiting_for_approval"),
    ];

    const open = bookUnit({ status: "waiting_for_approval", stages: stages("running") });
    expect(
      buildGateView(open, "approve_promotion", copy)?.checks.find((check) => check.title === copy.checkQa)?.ok,
    ).toBe(false);

    const failing = bookUnit({
      status: "waiting_for_approval",
      stages: stages("completed", unitSummary({ total: 4, completed: 3, failed: 1 })),
    });
    expect(
      buildGateView(failing, "approve_promotion", copy)?.checks.find((check) => check.title === copy.checkQa)?.ok,
    ).toBe(false);

    const closed = bookUnit({ status: "waiting_for_approval", stages: stages("completed") });
    const check = buildGateView(closed, "approve_promotion", copy)?.checks.find(
      (candidate) => candidate.title === copy.checkQa,
    );
    expect(check?.ok).toBe(true);
    expect(check?.detail).toBe("PASS 4/4");
  });
});

describe("pendingGates", () => {
  it("returns nothing when no gate is open", () => {
    expect(pendingGates(bookUnit({ stages: [stage("translate", "running")] }), copy)).toEqual([]);
  });

  it("returns both gates in contract order when both are open", () => {
    const unit = bookUnit({
      status: "waiting_for_approval",
      stages: [
        stage("approve_translation", "waiting_for_approval"),
        stage("approve_promotion", "ready"),
      ],
    });
    expect(pendingGates(unit, copy).map((gate) => gate.stageId)).toEqual([
      "approve_translation",
      "approve_promotion",
    ]);
  });
});

describe("approvalRecords", () => {
  it("keeps only decided approvals for the focused child", () => {
    const unit = bookUnit({
      childOver: { id: "child-1" },
      jobOver: {
        approvalReferences: [
          approvalRef({ approvalId: "a-pending", decision: "pending", childJobId: "child-1" }),
          approvalRef({ approvalId: "a-approved", decision: "approved", childJobId: "child-1" }),
          approvalRef({ approvalId: "a-rejected", decision: "rejected", childJobId: "child-1" }),
          approvalRef({ approvalId: "a-sibling", decision: "approved", childJobId: "child-2" }),
        ],
      },
    });
    expect(approvalRecords(unit).map((ref) => ref.approvalId)).toEqual(["a-approved", "a-rejected"]);
  });
});

describe("unitAdvanceAction", () => {
  it("offers the first advanceable stage that is waiting", () => {
    const unit = bookUnit({
      status: "ready",
      stages: [stage("extract", "completed"), stage("split", "ready")],
    });
    expect(unitAdvanceAction(unit)).toEqual({
      childId: "child-1",
      stageId: "split",
      stageStatus: "ready",
    });
  });

  it("offers nothing for a job with no child to advance", () => {
    const units = flattenBookUnits([job({ children: [] })]);
    expect(unitAdvanceAction(units[0])).toBeNull();
  });

  it("offers nothing when the open stage is a gate", () => {
    const unit = bookUnit({
      status: "waiting_for_approval",
      stages: [stage("approve_translation", "waiting_for_approval")],
    });
    expect(unitAdvanceAction(unit)).toBeNull();
  });

  it("offers nothing while the open stage is already running", () => {
    const unit = bookUnit({ status: "running", stages: [stage("translate", "running")] });
    expect(unitAdvanceAction(unit)).toBeNull();
  });

  it("lets the user push a blocked translate stage, but not a blocked promote", () => {
    const translate = bookUnit({ status: "blocked", stages: [stage("translate", "blocked")] });
    expect(unitAdvanceAction(translate)?.stageId).toBe("translate");

    const promote = bookUnit({ status: "blocked", stages: [stage("promote", "blocked")] });
    expect(unitAdvanceAction(promote)).toBeNull();
  });

  // The stage the user sees as "next" is the first open one in contract order,
  // which is not the order the backend happens to serialize them in.
  it("picks the next stage by contract order, not array order", () => {
    const unit = bookUnit({
      status: "ready",
      stages: [stage("build_reading", "ready"), stage("split", "ready")],
    });
    expect(unitAdvanceAction(unit)?.stageId).toBe("split");
  });
});

describe("route vocabulary", () => {
  it("translates the route kinds the backend emits", () => {
    expect(routeKindLabel("direct_text", copy)).toBe(copy.routeDirectText);
    expect(routeKindLabel("missing_credentials", copy)).toBe(copy.routeMissingCredentials);
    expect(routeKindLabel("some_future_route", copy)).toBe("some future route");
  });

  // Regression: an empty Zotero discovery used to arrive as the bare "blocked"
  // kind, which no label covered, so a Chinese UI showed the raw wire string.
  // The tone was right the whole time — only the label was missing.
  it("labels an empty discovery in the user's language", () => {
    const zh = pipelineCopy("zh");
    expect(routeKindLabel("blocked_no_attachment", zh)).toBe(zh.routeNoAttachment);
    expect(routeKindLabel("blocked_no_attachment", zh)).not.toBe("blocked no attachment");
    expect(routeKindLabel("blocked_no_attachment", copy)).toBe(copy.routeNoAttachment);
  });

  it("tones a route by what it means for the user", () => {
    expect(routeTone("direct_text")).toBe("ok");
    expect(routeTone("translation_ready")).toBe("ok");
    expect(routeTone("remote_paddleocr")).toBe("info");
    expect(routeTone("mineru")).toBe("info");
    expect(routeTone("blocked_dirty_text_layer")).toBe("block");
    expect(routeTone("blocked_no_attachment")).toBe("block");
    expect(routeTone("missing_credentials")).toBe("warn");
    expect(routeTone("already_converted")).toBe("neutral");
    expect(routeTone("some_future_route")).toBe("neutral");
  });
});

describe("providerDefaultConfig", () => {
  // Regression: this was a hand-written map of three brands while the catalog
  // carried six, so half the providers resolved to the "default" fallback and
  // the drawer offered only the three that were spelled out.
  it("resolves every brand in the catalog to its first slot's config", () => {
    expect(MODEL_BRANDS.length).toBeGreaterThanOrEqual(6);
    for (const brand of MODEL_BRANDS) {
      expect(providerDefaultConfig(brand.profileId), brand.profileId).toBe(brand.slots[0].configId);
      expect(providerDefaultConfig(brand.profileId), brand.profileId).not.toBe("default");
    }
  });

  it("falls back for a profile the catalog does not list", () => {
    expect(providerDefaultConfig("expert-agent")).toBe("default");
  });
});
