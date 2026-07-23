// Bilingual strings for the models settings panel, kept local (like
// pipeline/copy.ts) rather than threaded through the four-locale i18n object.
export function modelsCopy(locale: string) {
  const zh = locale.startsWith("zh");
  return {
    title: zh ? "翻译模型" : "Translation model",
    description: zh
      ? "选择用于翻译的模型并填入 API 密钥。密钥保存在系统钥匙串（Keychain）里，不写入任何文件。"
      : "Choose the model used for translation and enter its API key. Keys are stored in the system Keychain, never in a file.",
    activeBadge: zh ? "当前使用" : "In use",
    setActive: zh ? "设为当前" : "Use this",
    apiKey: zh ? "API 密钥" : "API key",
    keyPlaceholder: zh ? "粘贴 API 密钥" : "Paste the API key",
    save: zh ? "保存" : "Save",
    saving: zh ? "保存中…" : "Saving…",
    remove: zh ? "删除密钥" : "Remove key",
    test: zh ? "测试连接" : "Test connection",
    testing: zh ? "测试中…" : "Testing…",
    configured: zh ? "已配置密钥" : "Key stored",
    notConfigured: zh ? "未配置密钥" : "No key yet",
    model: zh ? "模型" : "Model",
    getKey: zh ? "获取密钥 ↗" : "Get a key ↗",
    saved: zh ? "已保存" : "Saved",
    removed: zh ? "已删除" : "Removed",
    emptyKey: zh ? "请先填入密钥。" : "Enter a key first.",
    activeHint: (label: string, model: string) =>
      zh
        ? `翻译将使用：${label} · ${model}`
        : `Translation will use: ${label} · ${model}`,
    noActive: zh
      ? "还没有选择翻译模型。填入一个密钥并点「设为当前」。"
      : "No translation model chosen yet. Add a key and click “Use this”.",
  };
}

export type ModelsCopy = ReturnType<typeof modelsCopy>;
