import { useCallback, useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { approveBookPipelineCleanup, previewBookPipelineCleanup } from "../../api";
import type { BookPipelineCleanupCandidate } from "../../types";

// The commands, their api.ts wrappers and their types have all existed since the
// cleanup approval landed; nothing ever called them, so the approval a user was
// supposed to give had no way to be given. This is that entry point.
function cleanupCopy(locale: string) {
  const zh = locale.startsWith("zh");
  return {
    title: zh ? "源文件清理" : "Source cleanup",
    description: zh
      ? "列出证据齐全、可以批准删除源 PDF 的书。启动器自己不删除任何文件 —— 批准只是留下一条可校验的记录，实际删除仍由既有清理脚本执行。"
      : "Lists books whose evidence is complete enough to approve deleting the source PDF. The launcher deletes nothing itself — approving only records a verifiable decision; the existing cleanup scripts remain the deletion path.",
    refresh: zh ? "刷新候选" : "Refresh",
    empty: zh ? "当前没有可清理的候选。" : "No cleanup candidates right now.",
    loading: zh ? "正在读取候选…" : "Loading candidates…",
    approve: zh ? "确认清理" : "Approve cleanup",
    approved: zh ? "已记录批准" : "Approval recorded",
    blocked: zh ? "证据不齐，无法批准" : "Evidence incomplete",
    evidenceOk: zh ? "通过" : "ok",
    evidenceBad: zh ? "缺失" : "missing",
    reclaim: zh ? "预计回收" : "Reclaims",
  };
}

const EVIDENCE_LABELS: Record<string, { zh: string; en: string }> = {
  markdown_output: { zh: "Markdown 产物", en: "Markdown output" },
  local_output: { zh: "本地成品", en: "Local deliverable" },
  zotero_child_attachment: { zh: "Zotero 子附件", en: "Zotero child attachment" },
  validated_reading: { zh: "阅读校验完成", en: "Reading validated" },
};

function evidenceLabel(kind: string, zh: boolean): string {
  const label = EVIDENCE_LABELS[kind];
  return label ? (zh ? label.zh : label.en) : kind;
}

export function SourceCleanupPanel({ locale }: { locale: string }) {
  const copy = cleanupCopy(locale);
  const zh = locale.startsWith("zh");
  const [candidates, setCandidates] = useState<BookPipelineCleanupCandidate[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");

  // Refreshing must not clear the message: the reload that follows an approval
  // would otherwise wipe the confirmation the user just earned.
  const refresh = useCallback(async () => {
    setBusy(true);
    try {
      setCandidates((await previewBookPipelineCleanup()).candidates);
    } catch (error) {
      setCandidates([]);
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const approve = useCallback(
    async (candidateId: string) => {
      setBusy(true);
      try {
        const result = await approveBookPipelineCleanup(candidateId, true);
        setMessage(result.message);
        await refresh();
      } catch (error) {
        setMessage(String(error));
      } finally {
        setBusy(false);
      }
    },
    [refresh],
  );

  return (
    <>
      <div className="st-row">
        <div className="st-row-copy">
          <strong>{copy.title}</strong>
          <span>{copy.description}</span>
          {message && <code>{message}</code>}
        </div>
        <button className="st-btn" type="button" disabled={busy} onClick={() => void refresh()}>
          <RefreshCw size={14} />
          {copy.refresh}
        </button>
      </div>
      {candidates === null && <div className="st-row-copy"><span>{copy.loading}</span></div>}
      {candidates?.length === 0 && <div className="st-row-copy"><span>{copy.empty}</span></div>}
      {candidates?.map((candidate) => (
        <div className="st-row" key={candidate.id}>
          <div className="st-row-copy">
            <strong>{candidate.title}</strong>
            <span>{candidate.sourceRef}</span>
            {candidate.sourcePath && (
              <code title={candidate.sourcePath}>
                {copy.reclaim}: {candidate.sourcePath}
              </code>
            )}
            <span>
              {candidate.checks
                .map(
                  (check) =>
                    `${evidenceLabel(check.kind, zh)} · ${check.ok ? copy.evidenceOk : copy.evidenceBad}`,
                )
                .join(" | ")}
            </span>
          </div>
          <button
            className="st-btn"
            type="button"
            disabled={busy || !candidate.canApprove}
            title={candidate.canApprove ? undefined : copy.blocked}
            onClick={() => void approve(candidate.id)}
          >
            {copy.approve}
          </button>
        </div>
      ))}
    </>
  );
}
