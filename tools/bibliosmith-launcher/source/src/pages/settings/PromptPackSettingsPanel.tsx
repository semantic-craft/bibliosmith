import { useMemo, useState } from "react";
import { Copy, Save, Trash2 } from "lucide-react";
import type {
  TranslationPromptPackCatalog,
  TranslationPromptPackDefinition,
  TranslationPromptPackReference,
  TranslationPromptPackRevisionDraft,
  TranslationPromptPackRevisionDiff,
} from "../../types";
import { diffTranslationPromptPackRevisions } from "../../api";

type Executor = "programmatic" | "expert-agent";

function latest(pack: TranslationPromptPackDefinition) {
  return pack.revisions.at(-1)!;
}

function reference(pack: TranslationPromptPackDefinition): TranslationPromptPackReference {
  return revisionReference(latest(pack));
}

function revisionReference(revision: ReturnType<typeof latest>): TranslationPromptPackReference {
  return {
    packId: revision.packId,
    revisionId: revision.revisionId,
    contentSha256: revision.contentSha256,
  };
}

function sameReference(left: TranslationPromptPackReference | null, right: TranslationPromptPackReference) {
  return left?.packId === right.packId
    && left.revisionId === right.revisionId
    && left.contentSha256 === right.contentSha256;
}

export type PromptPackSettingsPanelProps = {
  locale: string;
  catalog: TranslationPromptPackCatalog | null;
  defaults: Record<Executor, TranslationPromptPackReference | null>;
  busy: boolean;
  onCopy: (source: TranslationPromptPackReference, displayName: string) => Promise<void>;
  onSaveRevision: (draft: TranslationPromptPackRevisionDraft) => Promise<void>;
  onDelete: (packId: string) => Promise<void>;
  onSetDefault: (executor: Executor, value: TranslationPromptPackReference) => Promise<void>;
};

export function PromptPackSettingsPanel({
  locale,
  catalog,
  defaults,
  busy,
  onCopy,
  onSaveRevision,
  onDelete,
  onSetDefault,
}: PromptPackSettingsPanelProps) {
  const zh = locale.startsWith("zh");
  const packs = catalog?.packs ?? [];
  const [selectedId, setSelectedId] = useState<string | null>(packs[0]?.packId ?? null);
  const selected = packs.find((pack) => pack.packId === selectedId) ?? packs[0] ?? null;
  const revision = selected ? latest(selected) : null;
  const editKey = revision ? `${revision.packId}:${revision.revisionId}` : "none";

  return (
    <div className="st-prompt-packs">
      <div className="st-pp-intro">
        <strong>{zh ? "翻译提示词方案" : "Translation Prompt Packs"}</strong>
        <span>{zh ? "统一查看、复制和版本化翻译方案。内置方案只读；私人原文不会写入方案库。" : "Inspect, copy, and version translation schemes. Built-ins are read-only; private source text never enters this library."}</span>
      </div>
      <div className="st-pp-layout">
        <nav className="st-pp-list" aria-label={zh ? "提示词方案列表" : "Prompt pack list"}>
          {packs.map((pack) => {
            const current = latest(pack);
            const packReference = reference(pack);
            const isDefault = sameReference(defaults[current.executor], packReference);
            return (
              <button
                key={pack.packId}
                type="button"
                className={`st-pp-item${selected?.packId === pack.packId ? " selected" : ""}`}
                onClick={() => setSelectedId(pack.packId)}
              >
                <span>{current.displayName}</span>
                <small>{current.executor === "programmatic" ? (zh ? "程序执行" : "Programmatic") : (zh ? "专家代理" : "Expert agent")}</small>
                <em>{pack.kind === "builtin" ? (zh ? "内置" : "Built-in") : (zh ? "本地" : "Local")}{isDefault ? ` · ${zh ? "默认" : "Default"}` : ""}</em>
              </button>
            );
          })}
        </nav>
        {selected && revision ? (
          <PromptPackDetail
            key={editKey}
            zh={zh}
            pack={selected}
            defaults={defaults}
            busy={busy}
            onCopy={onCopy}
            onSaveRevision={onSaveRevision}
            onDelete={onDelete}
            onSetDefault={onSetDefault}
          />
        ) : (
          <div className="st-pp-empty">{zh ? "没有可用的提示词方案。" : "No prompt packs available."}</div>
        )}
      </div>
    </div>
  );
}

function PromptPackDetail({
  zh,
  pack,
  defaults,
  busy,
  onCopy,
  onSaveRevision,
  onDelete,
  onSetDefault,
}: {
  zh: boolean;
  pack: TranslationPromptPackDefinition;
  defaults: PromptPackSettingsPanelProps["defaults"];
  busy: boolean;
  onCopy: PromptPackSettingsPanelProps["onCopy"];
  onSaveRevision: PromptPackSettingsPanelProps["onSaveRevision"];
  onDelete: PromptPackSettingsPanelProps["onDelete"];
  onSetDefault: PromptPackSettingsPanelProps["onSetDefault"];
}) {
  const revision = latest(pack);
  const [revisionId, setRevisionId] = useState(revision.revisionId);
  const selectedRevision = pack.revisions.find((item) => item.revisionId === revisionId) ?? revision;
  const packReference = revisionReference(selectedRevision);
  const readOnly = pack.kind === "builtin" || selectedRevision !== revision;
  const [displayName, setDisplayName] = useState(revision.displayName);
  const [templates, setTemplates] = useState(() => revision.stages.map((stage) => stage.template));
  const [styleGuidance, setStyleGuidance] = useState(revision.parameters?.styleGuidance ?? "");
  const [qualityFocus, setQualityFocus] = useState(revision.parameters?.qualityFocus ?? "");
  const [diff, setDiff] = useState<TranslationPromptPackRevisionDiff | null>(null);
  const [diffError, setDiffError] = useState<string | null>(null);
  const dirty = displayName.trim() !== revision.displayName
    || templates.some((template, index) => template !== revision.stages[index]?.template)
    || styleGuidance !== (revision.parameters?.styleGuidance ?? "")
    || qualityFocus !== (revision.parameters?.qualityFocus ?? "");
  const isDefault = sameReference(defaults[selectedRevision.executor], packReference);
  const sourceLabel = useMemo(() => {
    const source = selectedRevision.source;
    return String(source.repository ?? source.label ?? source.kind ?? "BiblioSmith");
  }, [selectedRevision.source]);

  return (
    <section className="st-pp-detail">
      <div className="st-pp-detail-head">
        <div>
          <input
            className="st-pp-name"
            aria-label={zh ? "方案名称" : "Pack name"}
            value={readOnly ? selectedRevision.displayName : displayName}
            readOnly={readOnly}
            onChange={(event) => setDisplayName(event.currentTarget.value)}
          />
          <p>{pack.summary}</p>
        </div>
        <span className={`st-pill ${readOnly ? "muted" : "success"}`}>{readOnly ? (pack.kind === "builtin" ? (zh ? "只读内置" : "Read-only") : (zh ? "历史版本只读" : "Historical revision")) : (zh ? "本地可编辑" : "Editable")}</span>
      </div>
      {pack.revisions.length > 1 && (
        <label className="st-pp-revision-select">
          <span>{zh ? "查看修订" : "View revision"}</span>
          <select value={selectedRevision.revisionId} onChange={(event) => { setRevisionId(event.currentTarget.value); setDiff(null); }}>
            {pack.revisions.map((item) => <option key={item.revisionId} value={item.revisionId}>{item.revisionId}{item === revision ? (zh ? " · 最新" : " · Latest") : ""}</option>)}
          </select>
        </label>
      )}
      <dl className="st-pp-meta">
        <div><dt>{zh ? "执行方式" : "Executor"}</dt><dd>{selectedRevision.executor}</dd></div>
        <div><dt>{zh ? "版本" : "Revision"}</dt><dd>{selectedRevision.revisionId}</dd></div>
        <div><dt>{zh ? "语言范围" : "Languages"}</dt><dd>{selectedRevision.sourceLanguage} → {selectedRevision.targetLanguage}</dd></div>
        <div><dt>{zh ? "内容哈希" : "Content hash"}</dt><dd title={selectedRevision.contentSha256}>{selectedRevision.contentSha256}</dd></div>
        <div><dt>{zh ? "成本提示" : "Cost"}</dt><dd>{selectedRevision.costHint}</dd></div>
        <div><dt>{zh ? "来源" : "Source"}</dt><dd>{sourceLabel}</dd></div>
      </dl>
      <div className="st-pp-source">
        {(["repository", "commit", "license", "commercialUse", "adaptation", "sourcePackId", "sourceRevisionId", "sourceContentSha256"] as const).flatMap((key) => {
          const value = selectedRevision.source[key];
          if (typeof value !== "string" || !value) return [];
          const labels = {
            repository: zh ? "上游仓库" : "Repository",
            commit: zh ? "固定提交" : "Pinned commit",
            license: zh ? "许可证 / 使用边界" : "License / usage boundary",
            commercialUse: zh ? "商业使用状态" : "Commercial-use status",
            adaptation: zh ? "修改说明" : "Adaptation",
            sourcePackId: zh ? "复制来源方案" : "Copied from pack",
            sourceRevisionId: zh ? "复制来源修订" : "Copied from revision",
            sourceContentSha256: zh ? "复制来源哈希" : "Copied from hash",
          };
          return [<div key={key}><strong>{labels[key]}</strong><span>{value}</span></div>];
        })}
        {Array.isArray(selectedRevision.source.referenceFiles) && (
          <div><strong>{zh ? "机制参考文件" : "Mechanism reference files"}</strong><span>{selectedRevision.source.referenceFiles.join(" · ")}</span></div>
        )}
        {(selectedRevision.requiredSkillIds?.length ?? 0) > 0 && (
          <div>
            <strong>{zh ? "固定技能依赖" : "Pinned skill dependencies"}</strong>
            <span>{selectedRevision.requiredSkillIds!.map((skillId) => {
              const versions = selectedRevision.source.skillVersions as Record<string, unknown> | undefined;
              return `${skillId}@${String(versions?.[skillId] ?? "未固定")}`;
            }).join(" · ")}</span>
          </div>
        )}
        {(selectedRevision.excludedResponsibilities?.length ?? 0) > 0 && (
          <div><strong>{zh ? "明确排除的职责" : "Excluded responsibilities"}</strong><span>{selectedRevision.excludedResponsibilities!.join(" · ")}</span></div>
        )}
      </div>
      <div className="st-pp-stages">
        <label className="st-pp-stage">
          <span>{zh ? "风格指导参数" : "Style guidance parameter"} <code>styleGuidance</code></span>
          <textarea
            aria-label={zh ? "风格指导参数" : "Style guidance parameter"}
            value={readOnly ? (selectedRevision.parameters?.styleGuidance ?? "") : styleGuidance}
            readOnly={readOnly}
            placeholder={zh ? "可选；作为开放参数注入，不能覆盖执行器安全层。" : "Optional; injected without overriding executor safety."}
            onChange={(event) => setStyleGuidance(event.currentTarget.value)}
          />
        </label>
        <label className="st-pp-stage">
          <span>{zh ? "质量侧重参数" : "Quality focus parameter"} <code>qualityFocus</code></span>
          <textarea
            aria-label={zh ? "质量侧重参数" : "Quality focus parameter"}
            value={readOnly ? (selectedRevision.parameters?.qualityFocus ?? "") : qualityFocus}
            readOnly={readOnly}
            placeholder={zh ? "可选；例如更重视术语一致性。" : "Optional; for example, emphasize terminology consistency."}
            onChange={(event) => setQualityFocus(event.currentTarget.value)}
          />
        </label>
      </div>
      <div className="st-pp-stages">
        {selectedRevision.stages.map((stage, index) => (
          <label key={stage.stageId} className="st-pp-stage">
            <span>{stage.label} <code>{stage.stageId}</code></span>
            <textarea
              aria-label={`${stage.label} ${zh ? "模板" : "template"}`}
              value={readOnly ? stage.template : templates[index]}
              readOnly={readOnly}
              onChange={(event) => {
                const next = [...templates];
                next[index] = event.currentTarget.value;
                setTemplates(next);
              }}
            />
          </label>
        ))}
      </div>
      <div className="st-pp-safety">
        <strong>{zh ? "执行器安全层（不可编辑）" : "Executor safety layer (locked)"}</strong>
        <span>{zh ? "占位符、标题、段落边界、术语表、目标语言和私人文本边界始终由 BiblioSmith 执行器强制执行；阶段模板不能覆盖。" : "BiblioSmith always enforces placeholders, headings, paragraph boundaries, glossary, target language, and private-text boundaries; stage templates cannot override them."}</span>
      </div>
      <div className="st-pp-actions">
        {!isDefault && (
          <button className="st-btn" type="button" disabled={busy} onClick={() => void onSetDefault(selectedRevision.executor, packReference)}>
            {zh ? "设为该执行方式的默认方案" : "Make executor default"}
          </button>
        )}
        {pack.revisions.length > 1 && selectedRevision !== pack.revisions[0] && (
          <button
            className="st-btn"
            type="button"
            disabled={busy}
            onClick={() => {
              const before = pack.revisions[pack.revisions.indexOf(selectedRevision) - 1];
              void diffTranslationPromptPackRevisions(revisionReference(before), packReference)
                .then((value) => { setDiff(value); setDiffError(null); })
                .catch((error) => setDiffError(String(error)));
            }}
          >{zh ? "与上一修订比较" : "Compare with previous"}</button>
        )}
        {readOnly ? (
          <button className="st-btn" type="button" disabled={busy} onClick={() => void onCopy(packReference, `${revision.displayName}${zh ? "副本" : " Copy"}`)}>
            <Copy size={14} />{zh ? "复制后编辑" : "Copy to edit"}
          </button>
        ) : (
          <>
            <button
              className="st-btn"
              type="button"
              disabled={busy || !dirty || !displayName.trim() || templates.some((template) => !template.trim())}
              onClick={() => void onSaveRevision({
                packId: pack.packId,
                displayName,
                parameters: Object.fromEntries([
                  ["styleGuidance", styleGuidance.trim()],
                  ["qualityFocus", qualityFocus.trim()],
                ].filter(([, value]) => value)),
                stages: selectedRevision.stages.map((stage, index) => ({ ...stage, template: templates[index] })),
              })}
            >
              <Save size={14} />{zh ? "保存为新版本" : "Save new revision"}
            </button>
            <button className="st-btn danger" type="button" disabled={busy || isDefault} onClick={() => void onDelete(pack.packId)}>
              <Trash2 size={14} />{zh ? "删除本地方案" : "Delete local pack"}
            </button>
          </>
        )}
      </div>
      {diff && (
        <div className="st-pp-diff" aria-label={zh ? "修订差异" : "Revision diff"}>
          <strong>{diff.before.revisionId} → {diff.after.revisionId}</strong>
          {diff.beforeMetadata.displayName !== diff.afterMetadata.displayName && (
            <div><code>{zh ? "方案名称" : "Display name"}</code><del>{diff.beforeMetadata.displayName}</del><ins>{diff.afterMetadata.displayName}</ins></div>
          )}
          {([
            ["executor", diff.beforeMetadata.executor, diff.afterMetadata.executor],
            ["sourceLanguage", diff.beforeMetadata.sourceLanguage, diff.afterMetadata.sourceLanguage],
            ["targetLanguage", diff.beforeMetadata.targetLanguage, diff.afterMetadata.targetLanguage],
            ["costHint", diff.beforeMetadata.costHint, diff.afterMetadata.costHint],
            ["contextPolicy", diff.beforeMetadata.contextPolicy ?? "∅", diff.afterMetadata.contextPolicy ?? "∅"],
            ["requiredSkillIds", diff.beforeMetadata.requiredSkillIds.join(" · ") || "∅", diff.afterMetadata.requiredSkillIds.join(" · ") || "∅"],
            ["requiredEvidence", diff.beforeMetadata.requiredEvidence.join(" · ") || "∅", diff.afterMetadata.requiredEvidence.join(" · ") || "∅"],
            ["excludedResponsibilities", diff.beforeMetadata.excludedResponsibilities.join(" · ") || "∅", diff.afterMetadata.excludedResponsibilities.join(" · ") || "∅"],
            ["evidencePolicy", JSON.stringify(diff.beforeMetadata.evidencePolicy ?? null), JSON.stringify(diff.afterMetadata.evidencePolicy ?? null)],
          ] as const).map(([key, beforeValue, afterValue]) => beforeValue === afterValue ? null : (
            <div key={key}><code>{key}</code><del>{beforeValue}</del><ins>{afterValue}</ins></div>
          ))}
          {[...new Set([
            ...Object.keys(diff.beforeMetadata.source),
            ...Object.keys(diff.afterMetadata.source),
          ])].sort().map((key) => {
            const beforeValue = JSON.stringify(diff.beforeMetadata.source[key] ?? null);
            const afterValue = JSON.stringify(diff.afterMetadata.source[key] ?? null);
            return beforeValue === afterValue ? null : <div key={key}><code>{key}</code><del>{beforeValue}</del><ins>{afterValue}</ins></div>;
          })}
          {(["styleGuidance", "qualityFocus"] as const).map((key) => {
            const beforeValue = diff.beforeMetadata.parameters[key] ?? "∅";
            const afterValue = diff.afterMetadata.parameters[key] ?? "∅";
            return beforeValue === afterValue ? null : <div key={key}><code>{key}</code><del>{beforeValue}</del><ins>{afterValue}</ins></div>;
          })}
          {diff.stages.map((stage) => (
            <div key={stage.stageId}>
              <code>{stage.stageId}</code>
              <del>{stage.beforeTemplate ?? "∅"}</del>
              <ins>{stage.afterTemplate ?? "∅"}</ins>
            </div>
          ))}
        </div>
      )}
      {diffError && <p className="st-pp-note">{diffError}</p>}
      {isDefault && <p className="st-pp-note">{zh ? "当前默认方案不能删除；先为该执行方式选择另一个默认方案。" : "The current default cannot be deleted. Select another default first."}</p>}
    </section>
  );
}
