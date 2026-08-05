// Bilingual strings for the OCR settings panel, kept local (like
// modelsCopy.ts) rather than threaded through the four-locale i18n object.
export function ocrCopy(locale: string) {
  const zh = locale.startsWith("zh");
  return {
    title: zh ? "扫描 OCR" : "Scanned-book OCR",
    modelPicker: zh ? "OCR 模型" : "OCR model",
    modelPickerDescription: zh
      ? "选择一个 OCR 服务；下方只显示这一项的配置。"
      : "Choose one OCR service; only its configuration is shown below.",
    description: zh
      ? "没有文字层的扫描 PDF 需要云端 OCR 提取文字；自带文字层的书不经过这里。密钥只保存在系统钥匙串（Keychain）里，不写入书库或普通配置文件。"
      : "Scanned PDFs without a text layer need remote OCR to extract text; born-digital books skip this entirely. Keys are stored only in the system Keychain, never in the library or ordinary config files.",
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
    notConfigured: zh ? "未配置" : "No key yet",
    getKey: zh ? "获取密钥 ↗" : "Get a key ↗",
    saved: zh ? "已保存" : "Saved",
    removed: zh ? "已删除" : "Removed",
    emptyKey: zh ? "请先填入密钥。" : "Enter a key first.",
  };
}

export type OcrCopy = ReturnType<typeof ocrCopy>;
