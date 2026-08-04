import { useEffect, useState, type ReactNode } from "react";
import { FolderOpen } from "lucide-react";
import { chooseRepoFolder, getLauncherState, setRepoFolder } from "../api";
import type { LauncherState } from "../types";
import { LogoMark } from "../shell/LogoMark";
import { copies, type Locale } from "../i18n";
import "./repository-setup-gate.css";

type RepositoryGateState =
  | { phase: "checking" }
  | { phase: "blocked"; launcherState: LauncherState | null; error: string | null; busy: boolean }
  | { phase: "ready" };

const startupCopy: Record<Locale, {
  checking: string;
  title: string;
  description: string;
  chooseFolder: string;
  choosingFolder: string;
  existingRepositoryRequired: string;
}> = {
  "zh-CN": {
    checking: "正在检查 BiblioSmith 仓库…",
    title: "先设置 BiblioSmith 仓库",
    description: "Launcher 需要连接一个现有的 BiblioSmith 仓库，设置完成后才能进入。",
    chooseFolder: "选择仓库文件夹",
    choosingFolder: "正在验证…",
    existingRepositoryRequired: "请选择现有的 BiblioSmith 仓库；空文件夹不能用于启动。",
  },
  "zh-TW": {
    checking: "正在檢查 BiblioSmith 儲存庫…",
    title: "先設定 BiblioSmith 儲存庫",
    description: "Launcher 需要連接一個現有的 BiblioSmith 儲存庫，設定完成後才能進入。",
    chooseFolder: "選擇儲存庫資料夾",
    choosingFolder: "正在驗證…",
    existingRepositoryRequired: "請選擇現有的 BiblioSmith 儲存庫；空資料夾不能用於啟動。",
  },
  ja: {
    checking: "BiblioSmith リポジトリを確認しています…",
    title: "BiblioSmith リポジトリを設定してください",
    description: "Launcher を使用する前に、既存の BiblioSmith リポジトリへ接続する必要があります。",
    chooseFolder: "リポジトリフォルダを選択",
    choosingFolder: "確認しています…",
    existingRepositoryRequired: "既存の BiblioSmith リポジトリを選択してください。空のフォルダでは起動できません。",
  },
  en: {
    checking: "Checking the BiblioSmith repository…",
    title: "Set up the BiblioSmith repository",
    description: "Connect an existing BiblioSmith repository before entering the Launcher.",
    chooseFolder: "Choose repository folder",
    choosingFolder: "Validating…",
    existingRepositoryRequired: "Choose an existing BiblioSmith repository; an empty folder cannot launch the app.",
  },
};

export function RepositorySetupGate({
  locale,
  version,
  children,
}: {
  locale: Locale;
  version: string;
  children: ReactNode;
}) {
  const copy = startupCopy[locale];
  const appCopy = copies[locale];
  const [state, setState] = useState<RepositoryGateState>({ phase: "checking" });

  useEffect(() => {
    let cancelled = false;
    void getLauncherState()
      .then((launcherState) => {
        if (cancelled) return;
        setState(
          launcherState.repoReady
            ? { phase: "ready" }
            : { phase: "blocked", launcherState, error: null, busy: false },
        );
      })
      .catch((error) => {
        if (!cancelled) {
          setState({ phase: "blocked", launcherState: null, error: errorMessage(error), busy: false });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const chooseRepository = async () => {
    if (state.phase !== "blocked" || state.busy) return;
    const previousLauncherState = state.launcherState;
    setState({ ...state, error: null, busy: true });
    try {
      const selection = await chooseRepoFolder();
      if (!selection.ok) {
        setState({ phase: "blocked", launcherState: previousLauncherState, error: null, busy: false });
        return;
      }
      if (!selection.repoRoot || selection.requiresDownload) {
        throw new Error(copy.existingRepositoryRequired);
      }
      const saved = await setRepoFolder(selection.repoRoot);
      if (!saved.ok || saved.requiresDownload) {
        throw new Error(copy.existingRepositoryRequired);
      }
      const refreshed = await getLauncherState();
      if (!refreshed.repoReady) {
        setState({ phase: "blocked", launcherState: refreshed, error: null, busy: false });
        return;
      }
      setState({ phase: "ready" });
    } catch (error) {
      setState({
        phase: "blocked",
        launcherState: previousLauncherState,
        error: errorMessage(error),
        busy: false,
      });
    }
  };

  if (state.phase === "ready") return children;

  return (
    <div className="launcher-frame startup-gate">
      <header className="startup-gate__toolbar" data-tauri-drag-region>
        <LogoMark />
        <span>BiblioSmith</span>
        <span className="startup-gate__version">{version}</span>
      </header>
      <main className="startup-gate__main" aria-live="polite">
        {state.phase === "checking" ? (
          <p className="startup-gate__checking">{copy.checking}</p>
        ) : (
          <section className="startup-gate__card">
            <LogoMark />
            <h1>{copy.title}</h1>
            <p>{copy.description}</p>
            {state.launcherState && (
              <p className="startup-gate__reason">
                {repositoryUnavailableDescription(appCopy, state.launcherState)}
              </p>
            )}
            {state.error && <p className="startup-gate__error">{state.error}</p>}
            <button
              className="startup-gate__choose"
              type="button"
              disabled={state.busy}
              onClick={() => void chooseRepository()}
            >
              <FolderOpen size={17} />
              {state.busy ? copy.choosingFolder : copy.chooseFolder}
            </button>
          </section>
        )}
      </main>
    </div>
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function repositoryUnavailableDescription(
  copy: (typeof copies)[Locale],
  launcherState: LauncherState,
): string {
  switch (launcherState.repoStatus) {
    case "missing":
      return copy.workspaceMissingDescription(launcherState.repoRoot);
    case "empty":
      return copy.workspaceEmptyDescription(launcherState.repoRoot);
    case "occupied":
      return copy.workspaceOccupiedDescription(launcherState.repoRoot);
    default:
      return copy.workspaceUnavailableTitle;
  }
}
