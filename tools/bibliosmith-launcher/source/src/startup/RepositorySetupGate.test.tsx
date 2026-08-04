import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LauncherState } from "../types";
import { RepositorySetupGate } from "./RepositorySetupGate";

const api = vi.hoisted(() => ({
  getLauncherState: vi.fn(),
  chooseRepoFolder: vi.fn(),
  setRepoFolder: vi.fn(),
}));

vi.mock("../api", () => api);

function launcherState(overrides: Partial<LauncherState> = {}): LauncherState {
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
    ...overrides,
  };
}

describe("RepositorySetupGate", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("keeps the main launcher blocked when the configured repository is unavailable", async () => {
    api.getLauncherState.mockResolvedValueOnce(launcherState());

    render(
      <RepositorySetupGate locale="zh-CN" version="v1.15.0">
        <div>Launcher main workspace</div>
      </RepositorySetupGate>,
    );

    expect(await screen.findByRole("heading", { name: "先设置 BiblioSmith 仓库" })).toBeTruthy();
    expect(screen.getByText("已设置的项目目录不存在：/missing/bibliosmith")).toBeTruthy();
    expect(screen.queryByText("Launcher main workspace")).toBeNull();
    await waitFor(() => expect(api.getLauncherState).toHaveBeenCalledTimes(1));
  });

  it("opens the main launcher immediately when the saved repository is ready", async () => {
    api.getLauncherState.mockResolvedValueOnce(
      launcherState({
        repoRoot: "/workspace/bibliosmith",
        repoReady: true,
        repoStatus: "ready",
        branch: "main",
      }),
    );

    render(
      <RepositorySetupGate locale="zh-CN" version="v1.15.0">
        <div>Launcher main workspace</div>
      </RepositorySetupGate>,
    );

    expect(await screen.findByText("Launcher main workspace")).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "先设置 BiblioSmith 仓库" })).toBeNull();
  });

  it("admits the main launcher after the user selects and saves a valid repository", async () => {
    const user = userEvent.setup();
    api.getLauncherState
      .mockResolvedValueOnce(launcherState())
      .mockResolvedValueOnce(
        launcherState({
          repoRoot: "/workspace/bibliosmith",
          repoReady: true,
          repoStatus: "ready",
          branch: "main",
        }),
      );
    api.chooseRepoFolder.mockResolvedValueOnce({
      ok: true,
      message: "selected",
      repoRoot: "/workspace/bibliosmith",
      requiresDownload: false,
    });
    api.setRepoFolder.mockResolvedValueOnce({ ok: true, message: "saved" });

    render(
      <RepositorySetupGate locale="zh-CN" version="v1.15.0">
        <div>Launcher main workspace</div>
      </RepositorySetupGate>,
    );

    await user.click(await screen.findByRole("button", { name: "选择仓库文件夹" }));

    await waitFor(() => expect(api.setRepoFolder).toHaveBeenCalledWith("/workspace/bibliosmith"));
    expect(await screen.findByText("Launcher main workspace")).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "先设置 BiblioSmith 仓库" })).toBeNull();
  });

  it("keeps the gate in place when the selected folder is not an existing repository", async () => {
    const user = userEvent.setup();
    api.getLauncherState.mockResolvedValueOnce(launcherState());
    api.chooseRepoFolder.mockResolvedValueOnce({
      ok: true,
      message: "empty folder selected",
      repoRoot: "/workspace/empty",
      requiresDownload: true,
    });

    render(
      <RepositorySetupGate locale="zh-CN" version="v1.15.0">
        <div>Launcher main workspace</div>
      </RepositorySetupGate>,
    );

    await user.click(await screen.findByRole("button", { name: "选择仓库文件夹" }));

    expect(await screen.findByText("请选择现有的 BiblioSmith 仓库；空文件夹不能用于启动。")).toBeTruthy();
    expect(screen.queryByText("Launcher main workspace")).toBeNull();
    expect(api.setRepoFolder).not.toHaveBeenCalled();
  });

  it("rechecks the saved repository before admitting the main launcher", async () => {
    const user = userEvent.setup();
    api.getLauncherState
      .mockResolvedValueOnce(launcherState())
      .mockResolvedValueOnce(
        launcherState({ repoRoot: "/workspace/bibliosmith", repoStatus: "missing" }),
      );
    api.chooseRepoFolder.mockResolvedValueOnce({
      ok: true,
      message: "selected",
      repoRoot: "/workspace/bibliosmith",
      requiresDownload: false,
    });
    api.setRepoFolder.mockResolvedValueOnce({ ok: true, message: "saved" });

    render(
      <RepositorySetupGate locale="zh-CN" version="v1.15.0">
        <div>Launcher main workspace</div>
      </RepositorySetupGate>,
    );

    await user.click(await screen.findByRole("button", { name: "选择仓库文件夹" }));

    expect(await screen.findByText("已设置的项目目录不存在：/workspace/bibliosmith")).toBeTruthy();
    expect(screen.queryByText("Launcher main workspace")).toBeNull();
    expect(api.getLauncherState).toHaveBeenCalledTimes(2);
  });
});
