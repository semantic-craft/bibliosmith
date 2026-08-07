import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceState } from "../types";
import { WorkspaceSetupGate } from "./WorkspaceSetupGate";

const api = vi.hoisted(() => ({
  getWorkspaceState: vi.fn(),
  createRecommendedWorkspace: vi.fn(),
  chooseAndCreateWorkspace: vi.fn(),
}));

vi.mock("../api", () => api);

function workspaceState(overrides: Partial<WorkspaceState> = {}): WorkspaceState {
  return {
    workspaceRoot: "/test-data/Documents/BiblioSmith",
    recommendedWorkspaceRoot: "/test-data/Documents/BiblioSmith",
    workspaceReady: false,
    workspaceStatus: "missing",
    platform: "macos aarch64",
    ...overrides,
  };
}

describe("WorkspaceSetupGate", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("shows the recommended user-owned location and keeps the app blocked", async () => {
    api.getWorkspaceState.mockResolvedValueOnce(workspaceState());

    render(
      <WorkspaceSetupGate locale="zh-CN" version="v1.15.0">
        <div>BiblioSmith main workspace</div>
      </WorkspaceSetupGate>,
    );

    expect(await screen.findByRole("heading", { name: "创建你的 BiblioSmith 书库" })).toBeTruthy();
    expect(screen.getByText("/test-data/Documents/BiblioSmith")).toBeTruthy();
    expect(screen.queryByText("BiblioSmith main workspace")).toBeNull();
  });

  it("creates the workspace at the recommended location without downloading a repository", async () => {
    const user = userEvent.setup();
    const ready = workspaceState({ workspaceReady: true, workspaceStatus: "ready" });
    api.getWorkspaceState.mockResolvedValueOnce(workspaceState());
    api.createRecommendedWorkspace.mockResolvedValueOnce(ready);

    render(
      <WorkspaceSetupGate locale="zh-CN" version="v1.15.0">
        <div>BiblioSmith main workspace</div>
      </WorkspaceSetupGate>,
    );

    await user.click(await screen.findByRole("button", { name: "在推荐位置创建" }));

    expect(api.createRecommendedWorkspace).toHaveBeenCalledOnce();
    expect(await screen.findByText("BiblioSmith main workspace")).toBeTruthy();
    expect(screen.queryByText(/仓库|repository/i)).toBeNull();
  });

  it("creates a workspace in an empty custom folder selected by the user", async () => {
    const user = userEvent.setup();
    const ready = workspaceState({
      workspaceRoot: "/Volumes/Books/BiblioSmith",
      workspaceReady: true,
      workspaceStatus: "ready",
    });
    api.getWorkspaceState.mockResolvedValueOnce(workspaceState());
    api.chooseAndCreateWorkspace.mockResolvedValueOnce(ready);

    render(
      <WorkspaceSetupGate locale="zh-CN" version="v1.15.0">
        <div>BiblioSmith main workspace</div>
      </WorkspaceSetupGate>,
    );

    await user.click(await screen.findByRole("button", { name: "选择其他位置" }));

    await waitFor(() => {
      expect(api.chooseAndCreateWorkspace).toHaveBeenCalledOnce();
    });
    expect(await screen.findByText("BiblioSmith main workspace")).toBeTruthy();
  });

  it("preserves the gate and surfaces the backend refusal for an occupied folder", async () => {
    const user = userEvent.setup();
    api.getWorkspaceState.mockResolvedValueOnce(workspaceState());
    api.chooseAndCreateWorkspace.mockRejectedValueOnce(new Error("目标目录已有其他文件，BiblioSmith 不会覆盖或混入其中"));

    render(
      <WorkspaceSetupGate locale="zh-CN" version="v1.15.0">
        <div>BiblioSmith main workspace</div>
      </WorkspaceSetupGate>,
    );

    await user.click(await screen.findByRole("button", { name: "选择其他位置" }));

    expect(await screen.findByText("目标目录已有其他文件，BiblioSmith 不会覆盖或混入其中")).toBeTruthy();
    expect(screen.queryByText("BiblioSmith main workspace")).toBeNull();
  });
});
