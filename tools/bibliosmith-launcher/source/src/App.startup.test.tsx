import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LauncherState } from "./types";

const api = vi.hoisted(() => ({
  getLauncherState: vi.fn(),
  prepareBiblioSmithProject: vi.fn(),
}));

vi.mock("./api", async (importOriginal) => ({
  ...await importOriginal<typeof import("./api")>(),
  getLauncherState: api.getLauncherState,
  prepareBiblioSmithProject: api.prepareBiblioSmithProject,
}));

vi.mock("@tauri-apps/plugin-autostart", () => ({
  disable: vi.fn(),
  enable: vi.fn(),
  isEnabled: vi.fn(() => Promise.resolve(false)),
}));

import App from "./App";

function missingLauncherState(): LauncherState {
  return {
    repoRoot: "/missing/bibliosmith",
    repoReady: false,
    repoStatus: "missing",
    branch: "not-ready",
    localCommit: "",
    localCommitShort: "",
    remoteUrl: "local-git",
    dirty: false,
    proxyConfigured: false,
    platform: "macos aarch64",
  };
}

describe("App repository startup gate", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    const values = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
      clear: () => values.clear(),
      key: (index: number) => [...values.keys()][index] ?? null,
      get length() {
        return values.size;
      },
    } satisfies Storage);
    api.getLauncherState.mockResolvedValue(missingLauncherState());
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.resetAllMocks();
  });

  it("does not mount the launcher or start repository preparation before setup", async () => {
    render(<App />);

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.getByRole("heading", { name: "先设置 BiblioSmith 仓库" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "添加书" })).toBeNull();
    act(() => vi.advanceTimersByTime(1_000));
    expect(api.prepareBiblioSmithProject).not.toHaveBeenCalled();
  });
});
