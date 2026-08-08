import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UpdateSettingsPanel } from "./UpdateSettingsPanel";
import type { LauncherUpdateController, LauncherUpdateState } from "../../updates";

function controllerFor(state: LauncherUpdateState): LauncherUpdateController {
  return {
    state,
    updateAvailable: state.kind === "available",
    check: vi.fn().mockResolvedValue(undefined),
    install: vi.fn().mockResolvedValue(undefined),
    restart: vi.fn().mockResolvedValue(undefined),
  };
}

const RELEASE_NOTES = [
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

describe("UpdateSettingsPanel", () => {
  it("shows the installed version and nothing about a newer one before a check answers", () => {
    render(
      <UpdateSettingsPanel
        locale="en"
        currentVersion="v1.16.4"
        controller={controllerFor({ kind: "idle" })}
        runningJobCount={0}
      />,
    );

    expect(screen.getByText("v1.16.4")).not.toBeNull();
    expect(screen.queryByText("Up to date")).toBeNull();
    expect(screen.queryByRole("button", { name: "Download and install" })).toBeNull();
  });

  it("says up to date only for a check that came back empty", () => {
    render(
      <UpdateSettingsPanel
        locale="en"
        currentVersion="v1.16.4"
        controller={controllerFor({ kind: "upToDate", checkedAt: new Date("2026-08-09T04:05:00Z") })}
        runningJobCount={0}
      />,
    );

    expect(screen.getByText(/Up to date/)).not.toBeNull();
  });

  it("renders the release notes section matching the interface language", () => {
    const { unmount } = render(
      <UpdateSettingsPanel
        locale="ja"
        currentVersion="v1.16.4"
        controller={controllerFor({ kind: "available", version: "1.17.0", notes: RELEASE_NOTES, date: "2026-08-09T00:00:00Z" })}
        runningJobCount={0}
      />,
    );

    expect(screen.getByText("いくつかの問題を修正しました。")).not.toBeNull();
    expect(screen.queryByText("Fixes a handful of problems.")).toBeNull();
    unmount();

    render(
      <UpdateSettingsPanel
        locale="zh-CN"
        currentVersion="v1.16.4"
        controller={controllerFor({ kind: "available", version: "1.17.0", notes: RELEASE_NOTES, date: null })}
        runningJobCount={0}
      />,
    );
    expect(screen.getByText("修复了若干问题。")).not.toBeNull();
  });

  it("offers the install once an update is available", async () => {
    const user = userEvent.setup();
    const controller = controllerFor({ kind: "available", version: "1.17.0", notes: RELEASE_NOTES, date: null });
    render(
      <UpdateSettingsPanel locale="en" currentVersion="v1.16.4" controller={controller} runningJobCount={0} />,
    );

    expect(screen.getByText("Version 1.17.0 is available")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "Download and install" }));
    expect(controller.install).toHaveBeenCalledTimes(1);
  });

  // Installing swaps the App bundle the running job's interpreters live in.
  it("blocks the install while jobs are running and says why", () => {
    const controller = controllerFor({ kind: "available", version: "1.17.0", notes: RELEASE_NOTES, date: null });
    render(
      <UpdateSettingsPanel locale="en" currentVersion="v1.16.4" controller={controller} runningJobCount={2} />,
    );

    const install = screen.getByRole("button", { name: "Download and install" }) as HTMLButtonElement;
    expect(install.disabled).toBe(true);
    expect(screen.getByText(/2 job\(s\) are running/)).not.toBeNull();
  });

  it("reports real byte counts while downloading rather than a fabricated percentage", () => {
    render(
      <UpdateSettingsPanel
        locale="en"
        currentVersion="v1.16.4"
        controller={controllerFor({ kind: "downloading", version: "1.17.0", downloadedBytes: 536_870_912, totalBytes: 1_073_741_824 })}
        runningJobCount={0}
      />,
    );

    expect(screen.getByText("512.0 MB of 1.0 GB downloaded")).not.toBeNull();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("50");
  });

  it("leaves the progress bar indeterminate when the server announced no length", () => {
    render(
      <UpdateSettingsPanel
        locale="en"
        currentVersion="v1.16.4"
        controller={controllerFor({ kind: "downloading", version: "1.17.0", downloadedBytes: 4096, totalBytes: null })}
        runningJobCount={0}
      />,
    );

    expect(screen.getByText("4.0 KB downloaded")).not.toBeNull();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBeNull();
  });

  it("asks for a restart once the new bundle is installed", async () => {
    const user = userEvent.setup();
    const controller = controllerFor({ kind: "installed", version: "1.17.0" });
    render(
      <UpdateSettingsPanel locale="en" currentVersion="v1.16.4" controller={controller} runningJobCount={0} />,
    );

    expect(screen.getByText("1.17.0 is installed and takes effect after a restart")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "Restart now" }));
    expect(controller.restart).toHaveBeenCalledTimes(1);
  });

  it("shows a failed check as a failure instead of silently looking up to date", () => {
    render(
      <UpdateSettingsPanel
        locale="en"
        currentVersion="v1.16.4"
        controller={controllerFor({ kind: "error", message: "network unreachable" })}
        runningJobCount={0}
      />,
    );

    expect(screen.getByRole("alert").textContent).toBe("Update check failed: network unreachable");
    expect(screen.queryByText(/Up to date/)).toBeNull();
  });
});
