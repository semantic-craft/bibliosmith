import { describe, expect, it } from "vitest";
import { localizedReleaseNotes } from "./release-notes";

// The two shapes that reach this parser: RELEASE_NOTES.md, which the release
// workflow requires to carry "## ZH", "## EN" and "## JA" headings and which
// becomes the update manifest's notes, and commit bodies, which
// tools/git/check_commit_messages.py requires to label sections "ZH:".
const MARKDOWN_NOTES = [
  "# BiblioSmith Launcher 1.17.0",
  "",
  "## ZH",
  "",
  "修复了若干问题。",
  "",
  "## EN",
  "",
  "Fixes a handful of problems.",
  "",
  "## JA",
  "",
  "いくつかの問題を修正しました。",
].join("\n");

const LABELLED_NOTES = ["ZH: 修复了若干问题。", "EN: Fixes a handful of problems.", "JA: いくつかの問題を修正しました。"].join("\n");

describe("localizedReleaseNotes", () => {
  it("reads Markdown headings, the shape RELEASE_NOTES.md uses", () => {
    expect(localizedReleaseNotes(MARKDOWN_NOTES, "en", "none")).toBe("Fixes a handful of problems.");
    expect(localizedReleaseNotes(MARKDOWN_NOTES, "zh-CN", "none")).toBe("修复了若干问题。");
  });

  it("still reads the labelled shape commit bodies use", () => {
    expect(localizedReleaseNotes(LABELLED_NOTES, "en", "none")).toBe("Fixes a handful of problems.");
  });

  // ja used to map to the EN section even though a JA section was sitting in
  // the same file.
  it("gives Japanese readers the Japanese section", () => {
    expect(localizedReleaseNotes(MARKDOWN_NOTES, "ja", "none")).toBe("いくつかの問題を修正しました。");
    expect(localizedReleaseNotes(LABELLED_NOTES, "ja", "none")).toBe("いくつかの問題を修正しました。");
  });

  it("falls back to Simplified Chinese for Traditional, which has no section of its own", () => {
    expect(localizedReleaseNotes(MARKDOWN_NOTES, "zh-TW", "none")).toBe("修复了若干问题。");
  });

  it("falls back to English when the requested language has no section", () => {
    expect(localizedReleaseNotes("## EN\n\nOnly English here.", "ja", "none")).toBe("Only English here.");
  });

  it("uses the caller's fallback for an empty body", () => {
    expect(localizedReleaseNotes("", "en", "none")).toBe("none");
    expect(localizedReleaseNotes(null, "en", "none")).toBe("none");
  });

  it("keeps unsectioned prose rather than dropping it", () => {
    expect(localizedReleaseNotes("Just one line.", "en", "none")).toBe("Just one line.");
  });
});
