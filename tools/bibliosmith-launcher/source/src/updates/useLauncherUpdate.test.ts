import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useLauncherUpdate } from "./useLauncherUpdate";

const api = vi.hoisted(() => ({
  checkLauncherUpdate: vi.fn(),
  relaunchLauncher: vi.fn(),
}));

vi.mock("../api", () => api);

function availableUpdate(overrides: Partial<{ version: string; notes: string; date: string | null }> = {}) {
  return {
    version: "1.17.0",
    notes: "## EN\n\nA newer build.",
    date: "2026-08-09T00:00:00Z",
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

describe("useLauncherUpdate", () => {
  beforeEach(() => {
    api.checkLauncherUpdate.mockReset();
    api.relaunchLauncher.mockReset();
    api.relaunchLauncher.mockResolvedValue(undefined);
  });

  it("checks once at startup and reports the release it actually found", async () => {
    api.checkLauncherUpdate.mockResolvedValue(availableUpdate());
    const { result } = renderHook(() => useLauncherUpdate(0));

    await waitFor(() => expect(result.current.state.kind).toBe("available"));
    expect(result.current.updateAvailable).toBe(true);
    expect(result.current.state).toMatchObject({ version: "1.17.0", date: "2026-08-09T00:00:00Z" });
    expect(api.checkLauncherUpdate).toHaveBeenCalledTimes(1);
  });

  it("only claims up to date when a check came back empty", async () => {
    api.checkLauncherUpdate.mockResolvedValue(null);
    const { result } = renderHook(() => useLauncherUpdate(0));

    await waitFor(() => expect(result.current.state.kind).toBe("upToDate"));
    expect(result.current.updateAvailable).toBe(false);
  });

  // Regression guard for the stub this feature replaces: a check that never
  // reached the network reported the installed build as the newest one.
  it("does not report up to date when the startup check failed", async () => {
    api.checkLauncherUpdate.mockRejectedValue(new Error("offline"));
    const { result } = renderHook(() => useLauncherUpdate(0));

    await waitFor(() => expect(api.checkLauncherUpdate).toHaveBeenCalled());
    await waitFor(() => expect(result.current.state.kind).toBe("idle"));
  });

  it("surfaces a manual check failure, unlike the silent startup one", async () => {
    api.checkLauncherUpdate.mockRejectedValue(new Error("offline"));
    const { result } = renderHook(() => useLauncherUpdate(0));
    await waitFor(() => expect(result.current.state.kind).toBe("idle"));

    await act(async () => {
      await result.current.check();
    });

    expect(result.current.state).toEqual({ kind: "error", message: "offline" });
  });

  it("reports download progress and then waits for the user to restart", async () => {
    const update = availableUpdate();
    update.downloadAndInstall.mockImplementation(async (onProgress: (progress: { downloadedBytes: number; totalBytes: number | null }) => void) => {
      onProgress({ downloadedBytes: 0, totalBytes: 2048 });
      onProgress({ downloadedBytes: 1024, totalBytes: 2048 });
    });
    api.checkLauncherUpdate.mockResolvedValue(update);
    const { result } = renderHook(() => useLauncherUpdate(0));
    await waitFor(() => expect(result.current.state.kind).toBe("available"));

    await act(async () => {
      await result.current.install();
    });

    expect(result.current.state).toEqual({ kind: "installed", version: "1.17.0" });
    expect(api.relaunchLauncher).not.toHaveBeenCalled();

    await act(async () => {
      await result.current.restart();
    });
    expect(api.relaunchLauncher).toHaveBeenCalledTimes(1);
  });

  // The install replaces the App bundle a running job is executing out of, so
  // the refusal has to live in the hook and not only in the disabled button.
  it("refuses to install while a job is running", async () => {
    const update = availableUpdate();
    api.checkLauncherUpdate.mockResolvedValue(update);
    const { result } = renderHook(() => useLauncherUpdate(2));
    await waitFor(() => expect(result.current.state.kind).toBe("available"));

    await act(async () => {
      await result.current.install();
    });

    expect(update.downloadAndInstall).not.toHaveBeenCalled();
    expect(result.current.state.kind).toBe("available");
  });

  it("installs once the last running job has finished", async () => {
    const update = availableUpdate();
    api.checkLauncherUpdate.mockResolvedValue(update);
    const { result, rerender } = renderHook((runningJobCount: number) => useLauncherUpdate(runningJobCount), {
      initialProps: 1,
    });
    await waitFor(() => expect(result.current.state.kind).toBe("available"));

    rerender(0);
    await act(async () => {
      await result.current.install();
    });

    expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(result.current.state.kind).toBe("installed");
  });

  it("keeps the installed bundle in view when the restart itself fails", async () => {
    const update = availableUpdate();
    api.checkLauncherUpdate.mockResolvedValue(update);
    api.relaunchLauncher.mockRejectedValue(new Error("relaunch blocked"));
    const { result } = renderHook(() => useLauncherUpdate(0));
    await waitFor(() => expect(result.current.state.kind).toBe("available"));
    await act(async () => {
      await result.current.install();
    });

    await act(async () => {
      await result.current.restart();
    });

    expect(result.current.state).toEqual({ kind: "error", message: "relaunch blocked" });
  });
});
