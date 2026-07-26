import { describe, expect, it } from "vitest";
import { copies } from "../i18n";
import type { CommitInfo } from "../types";
import {
  UNKNOWN_VALUE,
  clampPercent,
  commitDate,
  formatBytes,
  formatDownloadProgress,
  formatPercent,
  progressWidth,
  versionFromDate,
} from "./format";

const copy = copies.en;

describe("versionFromDate", () => {
  it("derives a version from the date part of a timestamp", () => {
    expect(versionFromDate("2026-07-25T22:43:22+08:00")).toBe("v2026.07.25");
  });

  // Regression: this used to return a hard-coded "v2025.05.25" whenever no
  // commit was available, which the Updates page then presented as the
  // project's actual version. An unknown value has to look unknown.
  it("reports an unknown date as unknown instead of inventing one", () => {
    expect(versionFromDate()).toBe(UNKNOWN_VALUE);
    expect(versionFromDate("")).toBe(UNKNOWN_VALUE);
    expect(versionFromDate()).not.toMatch(/^v\d{4}\./);
  });
});

describe("commitDate", () => {
  const commit = (date: string): CommitInfo => ({
    hash: "abc1234",
    date,
    title: "fix: something",
    summary: "",
    fullMessage: "fix: something",
  });

  it("trims a commit timestamp to minutes", () => {
    expect(commitDate(commit("2026-07-25T22:43:22+08:00"))).toBe("2026-07-25 22:43");
  });

  // Regression: the fallback was a fabricated "2025-05-25 10:15".
  it("reports a missing commit as unknown instead of inventing a date", () => {
    expect(commitDate()).toBe(UNKNOWN_VALUE);
    expect(commitDate(commit(""))).toBe(UNKNOWN_VALUE);
    expect(commitDate()).not.toMatch(/\d{4}-\d{2}-\d{2}/);
  });
});

describe("formatBytes", () => {
  it("renders kilobytes to one decimal", () => {
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
  });

  it("renders zero without dividing", () => {
    expect(formatBytes(0)).toBe("0.0 KB");
  });
});

describe("clampPercent", () => {
  it("keeps a percentage inside 0-100", () => {
    expect(clampPercent(-5)).toBe(0);
    expect(clampPercent(42.5)).toBe(42.5);
    expect(clampPercent(180)).toBe(100);
  });

  it("treats a non-finite percentage as zero", () => {
    expect(clampPercent(Number.NaN)).toBe(0);
    expect(clampPercent(Number.POSITIVE_INFINITY)).toBe(0);
  });
});

describe("formatPercent / progressWidth", () => {
  it("formats a clamped percentage", () => {
    expect(formatPercent(12.345)).toBe("12.35%");
    expect(formatPercent(999)).toBe("100.00%");
  });

  it("gives a CSS width from the same clamp", () => {
    expect(progressWidth(50)).toBe("50%");
    expect(progressWidth(-1)).toBe("0%");
  });
});

describe("formatDownloadProgress", () => {
  it("is empty with nothing to report", () => {
    expect(formatDownloadProgress(copy, null)).toBe("");
    expect(formatDownloadProgress(copy)).toBe("");
  });

  it("passes through a message that already carries its own units", () => {
    expect(
      formatDownloadProgress(copy, { percent: 10, downloadedBytes: 0, totalBytes: 0, message: "Extracting 40%" }),
    ).toBe("Extracting 40%");
  });

  it("appends a percentage to a message that has none", () => {
    expect(
      formatDownloadProgress(copy, { percent: 40, downloadedBytes: 0, totalBytes: 0, message: "Extracting" }),
    ).toBe("Extracting 40.00%");
  });

  it("reads a 100-byte total as the synthetic progress it is", () => {
    expect(formatDownloadProgress(copy, { percent: 40, downloadedBytes: 40, totalBytes: 100 })).toBe(
      `${copy.working} 40.00%`,
    );
  });

  it("reports a percentage alone when no byte counts are known", () => {
    expect(formatDownloadProgress(copy, { percent: 40, downloadedBytes: 0, totalBytes: 0 })).toBe(
      `${copy.downloading} 40.00%`,
    );
  });

  it("reports downloaded and total bytes when both are known", () => {
    expect(
      formatDownloadProgress(copy, { percent: 50, downloadedBytes: 1024, totalBytes: 4096 }),
    ).toBe(`${copy.downloading} 50.00% (1.0 KB / 4.0 KB)`);
  });

  it("omits the total when the server did not send one", () => {
    expect(formatDownloadProgress(copy, { percent: 50, downloadedBytes: 1024, totalBytes: 0 })).toBe(
      `${copy.downloading} 50.00% (1.0 KB)`,
    );
  });
});
