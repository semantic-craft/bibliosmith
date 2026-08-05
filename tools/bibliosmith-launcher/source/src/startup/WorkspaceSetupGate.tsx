import { useEffect, useState, type ReactNode } from "react";
import { FolderOpen, LibraryBig } from "lucide-react";
import { chooseAndCreateWorkspace, createRecommendedWorkspace, getWorkspaceState } from "../api";
import type { WorkspaceState } from "../types";
import { LogoMark } from "../shell/LogoMark";
import type { Locale } from "../i18n";
import "./workspace-setup-gate.css";

type WorkspaceGateState =
  | { phase: "checking" }
  | { phase: "blocked"; workspace: WorkspaceState | null; error: string | null; busy: boolean }
  | { phase: "ready" };

const startupCopy: Record<Locale, {
  checking: string;
  title: string;
  description: string;
  recommended: string;
  createRecommended: string;
  chooseOther: string;
  creating: string;
  unavailable: string;
}> = {
  "zh-CN": {
    checking: "正在检查 BiblioSmith 书库…",
    title: "创建你的 BiblioSmith 书库",
    description: "书籍、翻译、QA 和成书都保存在你拥有的 Documents 目录；App 资源与个人内容彼此分开。",
    recommended: "推荐位置",
    createRecommended: "在推荐位置创建",
    chooseOther: "选择其他位置",
    creating: "正在创建…",
    unavailable: "书库尚未创建。",
  },
  "zh-TW": {
    checking: "正在檢查 BiblioSmith 書庫…",
    title: "建立你的 BiblioSmith 書庫",
    description: "書籍、翻譯、QA 與成書都保存在你擁有的 Documents 目錄；App 資源與個人內容彼此分開。",
    recommended: "建議位置",
    createRecommended: "在建議位置建立",
    chooseOther: "選擇其他位置",
    creating: "正在建立…",
    unavailable: "書庫尚未建立。",
  },
  ja: {
    checking: "BiblioSmith ライブラリを確認しています…",
    title: "BiblioSmith ライブラリを作成",
    description: "本、翻訳、QA、完成版はユーザー所有の Documents に保存され、App のリソースとは分離されます。",
    recommended: "推奨場所",
    createRecommended: "推奨場所に作成",
    chooseOther: "別の場所を選択",
    creating: "作成中…",
    unavailable: "ライブラリはまだ作成されていません。",
  },
  en: {
    checking: "Checking your BiblioSmith library…",
    title: "Create your BiblioSmith library",
    description: "Books, translations, QA, and finished editions stay in your Documents folder, separate from App resources.",
    recommended: "Recommended location",
    createRecommended: "Create in recommended location",
    chooseOther: "Choose another location",
    creating: "Creating…",
    unavailable: "The library has not been created yet.",
  },
};

export function WorkspaceSetupGate({
  locale,
  version,
  children,
}: {
  locale: Locale;
  version: string;
  children: ReactNode;
}) {
  const copy = startupCopy[locale];
  const [state, setState] = useState<WorkspaceGateState>({ phase: "checking" });

  useEffect(() => {
    let cancelled = false;
    void getWorkspaceState()
      .then((workspace) => {
        if (!cancelled) {
          setState(workspace.workspaceReady
            ? { phase: "ready" }
            : { phase: "blocked", workspace, error: null, busy: false });
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setState({ phase: "blocked", workspace: null, error: errorMessage(error), busy: false });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const createRecommended = async () => {
    if (state.phase !== "blocked" || state.busy) return;
    const previous = state.workspace;
    setState({ ...state, error: null, busy: true });
    try {
      const workspace = await createRecommendedWorkspace();
      setState(workspace.workspaceReady
        ? { phase: "ready" }
        : { phase: "blocked", workspace, error: null, busy: false });
    } catch (error) {
      setState({ phase: "blocked", workspace: previous, error: errorMessage(error), busy: false });
    }
  };

  const chooseOther = async () => {
    if (state.phase !== "blocked" || state.busy) return;
    const previous = state.workspace;
    setState({ ...state, error: null, busy: true });
    try {
      const workspace = await chooseAndCreateWorkspace();
      if (!workspace) {
        setState({ phase: "blocked", workspace: previous, error: null, busy: false });
        return;
      }
      setState(workspace.workspaceReady
        ? { phase: "ready" }
        : { phase: "blocked", workspace, error: null, busy: false });
    } catch (error) {
      setState({ phase: "blocked", workspace: previous, error: errorMessage(error), busy: false });
    }
  };

  if (state.phase === "ready") return children;

  const recommendedRoot = state.phase === "blocked"
    ? state.workspace?.recommendedWorkspaceRoot
    : null;

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
            {recommendedRoot && (
              <div className="startup-gate__location">
                <span>{copy.recommended}</span>
                <strong>{recommendedRoot}</strong>
              </div>
            )}
            {!recommendedRoot && <p className="startup-gate__reason">{copy.unavailable}</p>}
            {state.error && <p className="startup-gate__error">{state.error}</p>}
            <div className="startup-gate__actions">
              <button
                className="startup-gate__create"
                type="button"
                disabled={state.busy || !recommendedRoot}
                onClick={() => void createRecommended()}
              >
                <LibraryBig size={17} />
                {state.busy ? copy.creating : copy.createRecommended}
              </button>
              <button
                className="startup-gate__choose"
                type="button"
                disabled={state.busy}
                onClick={() => void chooseOther()}
              >
                <FolderOpen size={17} />
                {copy.chooseOther}
              </button>
            </div>
          </section>
        )}
      </main>
    </div>
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
