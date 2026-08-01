// Bilingual strings for the OCR settings panel, kept local (like
// modelsCopy.ts) rather than threaded through the four-locale i18n object.
export function ocrCopy(locale: string) {
  const zh = locale.startsWith("zh");
  return {
    title: zh ? "扫描 OCR" : "Scanned-book OCR",
    description: zh
      ? "没有文字层的扫描 PDF 需要云端 OCR 提取文字；自带文字层的书不经过这里。密钥保存在系统钥匙串（Keychain）里，不写入任何文件；仓库根 .env 里已有的密钥继续有效，作为兜底。"
      : "Scanned PDFs without a text layer need remote OCR to extract text; born-digital books skip this entirely. Keys are stored in the system Keychain, never in a file; a key already in the repository-root .env keeps working as the fallback.",
    paddleName: zh ? "PaddleOCR（百度飞桨）" : "PaddleOCR (Baidu)",
    paddleHint: zh ? "扫描书的主力 OCR 路线" : "The main OCR route for scanned books",
    mineruName: zh ? "MinerU 精准解析" : "MinerU Precision Extract",
    mineruHint: zh
      ? "V4 精准解析：PDF 默认 VLM，长文档自动按 200 页拆分重组"
      : "V4 Precision Extract: VLM for PDFs, with automatic 200-page splitting and reassembly",
    apiKey: zh ? "API Token" : "API token",
    keyPlaceholder: zh ? "粘贴 API Token" : "Paste the API token",
    save: zh ? "保存" : "Save",
    saving: zh ? "保存中…" : "Saving…",
    test: zh ? "测试连接" : "Test connection",
    testing: zh ? "测试中…" : "Testing…",
    configuredKeychain: zh ? "已配置（钥匙串）" : "Key stored (Keychain)",
    configuredEnv: zh ? "已配置（.env 兜底）" : "Key found (.env fallback)",
    notConfigured: zh ? "未配置" : "No key yet",
    getKey: zh ? "获取密钥 ↗" : "Get a key ↗",
    saved: zh ? "已保存" : "Saved",
    removed: zh ? "已删除" : "Removed",
    emptyKey: zh ? "请先填入密钥。" : "Enter a key first.",
  };
}

export type OcrCopy = ReturnType<typeof ocrCopy>;
