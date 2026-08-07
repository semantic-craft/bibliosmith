import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceState } from "./types";

const api = vi.hoisted(() => ({
  getWorkspaceState: vi.fn(),
  createRecommendedWorkspace: vi.fn(),
}));

vi.mock("./api", async (importOriginal) => ({
  ...await importOriginal<typeof import("./api")>(),
  getWorkspaceState: api.getWorkspaceState,
  createRecommendedWorkspace: api.createRecommendedWorkspace,
}));

vi.mock("@tauri-apps/plugin-autostart", () => ({
  disable: vi.fn(),
  enable: vi.fn(),
  isEnabled: vi.fn(() => Promise.resolve(false)),
}));

import App from "./App";

function missingWorkspaceState(): WorkspaceState {
  return {
    workspaceRoot: "/test-data/Documents/BiblioSmith",
    recommendedWorkspaceRoot: "/test-data/Documents/BiblioSmith",
    workspaceReady: false,
    workspaceStatus: "missing",
    platform: "macos aarch64",
  };
}

describe("App workspace startup gate", () => {
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
    api.getWorkspaceState.mockResolvedValue(missingWorkspaceState());
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.resetAllMocks();
  });

  it("does not mount the launcher before the user-owned workspace exists", async () => {
    render(<App />);

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.getByRole("heading", { name: "创建你的 BiblioSmith 书库" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "添加书" })).toBeNull();
    act(() => vi.advanceTimersByTime(1_000));
    expect(api.createRecommendedWorkspace).not.toHaveBeenCalled();
  });
});
