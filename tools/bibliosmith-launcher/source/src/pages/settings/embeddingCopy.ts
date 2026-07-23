// Bilingual strings for the embedding settings panel, kept local (like
// modelsCopy.ts) rather than threaded through the four-locale i18n object.
export function embeddingCopy(locale: string) {
  const zh = locale.startsWith("zh");
  return {
    title: zh ? "全文搜索" : "Full-text search",
    description: zh
      ? "Zotero PDF 的语义搜索索引需要 Gemini API 密钥做 embedding。密钥保存在系统钥匙串（Keychain）里，不写入任何文件。"
      : "Semantic search over your Zotero PDFs needs a Gemini API key for embedding. The key is stored in the system Keychain, never in a file.",
    lockedHint: zh
      ? "已用 Gemini 建过全文索引，换后端需要整库重建，这里暂不提供切换。"
      : "Your full-text index was already built with Gemini; switching backends means rebuilding it from scratch, so that isn't offered here yet.",
    apiKey: zh ? "Gemini API 密钥" : "Gemini API key",
    keyPlaceholder: zh ? "粘贴 API 密钥" : "Paste the API key",
    save: zh ? "保存" : "Save",
    saving: zh ? "保存中…" : "Saving…",
    test: zh ? "测试连接" : "Test connection",
    testing: zh ? "测试中…" : "Testing…",
    configured: zh ? "已配置密钥" : "Key stored",
    notConfigured: zh ? "未配置密钥" : "No key yet",
    getKey: zh ? "获取密钥 ↗" : "Get a key ↗",
    saved: zh ? "已保存" : "Saved",
    removed: zh ? "已删除" : "Removed",
    emptyKey: zh ? "请先填入密钥。" : "Enter a key first.",
  };
}

export type EmbeddingCopy = ReturnType<typeof embeddingCopy>;
