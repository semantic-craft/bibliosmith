export type PipelineLocale = string;

export function pipelineCopy(locale: PipelineLocale) {
  const zh = locale.startsWith("zh");
  return {
    title: zh ? "Book Pipeline" : "Book Pipeline",
    provider: zh ? "模型后端" : "Model provider",
    providerKeyMissing: zh
      ? "所选模型还没有 API Key。请先在「设置 → 模型」里配置，否则这批书会先跑完 OCR，再在翻译阶段挂在鉴权错误上。"
      : "The selected model has no API key. Configure it under Settings → Models first, or this batch runs OCR to completion and then dies on provider auth at the translate stage.",
    textCleanup: zh ? "翻译时修复段内 OCR / 排版瑕疵" : "Fix within-paragraph OCR / layout defects while translating",
    continueStage: zh ? "继续下一阶段" : "Continue",
    recheckHandoff: zh ? "重新检查交接" : "Recheck handoff",
    runTranslation: zh ? "运行翻译（将发送正文）" : "Run translation (sends source text)",
    chooseFolder: zh ? "选择文件夹" : "Choose folder",
    selector: zh ? "精确定位（key / itemType=）" : "Exact selector (key / itemType=)",
    zoteroTitleSearch: zh ? "按书名搜索" : "Search by title",
    zoteroTitleSearchPlaceholder: zh ? "书名、作者或年份" : "Title, author, or year",
    zoteroTitleSearchButton: zh ? "搜索" : "Search",
    paddleCreds: zh ? "PaddleOCR 凭据" : "PaddleOCR credentials",
    mineruCreds: zh ? "MinerU 凭据" : "MinerU credentials",
    handoff: zh ? "翻译交接" : "Handoff",
    jobs: zh ? "任务" : "Jobs",
    artifacts: zh ? "产物" : "Artifacts",
    logs: zh ? "Logs" : "Logs",
    retry: zh ? "重试" : "Retry",
    openOutput: zh ? "打开输出" : "Open output",
    noPreview: zh ? "先预览路线。" : "Preview a route first.",

    // Shelf frame
    newJob: zh ? "添加书" : "Add a book",
    inboxEmptyBody: zh
      ? "放一本书上来，BiblioSmith 会整理文字、翻译成中文、做成电子书。开始翻译前、成书前会各问你一次，其余都自动。"
      : "Add a book and BiblioSmith will tidy up the text, translate it, and build a reading copy. It asks you once before translating and once before finalizing; everything else is automatic.",

    // Three user-facing phases the fifteen internal stages fold into
    phase1: zh ? "转换" : "Convert",
    phase2: zh ? "翻译" : "Translate",
    phase3: zh ? "成书" : "Build",
    capQueued: zh ? "排队中" : "Queued",
    capWorking: zh ? "进行中" : "In progress",
    capCompleted: zh ? "已完成" : "Complete",
    capNotStarted: zh ? "等待开始" : "Not started",
    capWaitingConfirmation: zh ? "等你确认" : "Needs confirmation",
    capAllDone: zh ? "全部完成" : "All done",
    capNeedsAttention: zh ? "需要处理" : "Needs attention",
    capGateTranslation: zh ? "翻译 · 等你确认后开始" : "Translate · starts after you confirm",
    capGatePromotion: zh ? "生成阅读版 · 等你确认后进行" : "Build reading copy · runs after you confirm",
    capTranslationQaPending: zh
      ? "翻译：正文已完成；专家QA：等待开始"
      : "Translate: body complete; Expert QA: not started",
    capPromotionApprovedPending: zh
      ? "生成阅读版：已确认；等待开始"
      : "Build reading copy: approved; not started",
    phaseCurrentPrefix: zh ? "当前：" : "Now: ",
    phaseNotInJob: zh ? "本次任务不包含这一段" : "Not part of this job",
    phaseSummaryPair: (completedStep: string, currentCaption: string) =>
      zh
        ? `${completedStep}：已完成；${currentCaption.replace(" · ", "：")}`
        : `${completedStep}: Complete; ${currentCaption.replace(" · ", ": ")}`,

    // Shelf ribbons
    ribbonWaiting: zh ? "等你确认" : "Needs you",
    ribbonProblem: zh ? "需要处理" : "Attention",
    ribbonDone: zh ? "可阅读" : "Ready",

    // Drawer
    advancedDetails: zh ? "高级详情" : "Advanced details",
    drawerPrev: zh ? "上一本" : "Previous book",
    drawerNext: zh ? "下一本" : "Next book",
    drawerClose: zh ? "关闭" : "Close",
    resizeDrawer: zh ? "调整书架与详情栏宽度" : "Resize bookshelf and details pane",
    customInstructionsTitle: zh ? "本书自定义翻译指令" : "Custom instructions for this book",
    customInstructionsHelp: zh
      ? "两相互相隔离；占位符、标题和段落结构保护始终优先。"
      : "The two phases stay isolated; placeholder, heading, and paragraph protection always wins.",
    customTranslationLabel: zh ? "翻译遍" : "Translation pass",
    customTranslationPlaceholder: zh ? "例如：保持克制、简洁的文学中文。" : "For example: Use restrained, concise literary prose.",
    customReflectionLabel: zh ? "Reflection 二遍" : "Reflection pass",
    customReflectionPlaceholder: zh ? "例如：重点检查时代错置的措辞。" : "For example: Critique anachronistic wording.",
    customInstructionsCount: (count: number) => `${count} / 2000`,
    customInstructionsTooLong: zh ? "每项最多 2000 个字符。" : "Each field is limited to 2000 characters.",
    saveCustomInstructions: zh ? "保存指令" : "Save instructions",
    savingCustomInstructions: zh ? "保存中…" : "Saving…",
    customInstructionsSaved: zh ? "本书自定义指令已保存" : "Custom instructions saved for this book",
    abNoAction: zh ? "不需要操作，好了会提醒你" : "Nothing to do — you'll be notified",
    abAdvanceRequired: zh ? "需要操作：继续到下一阶段" : "Action needed: continue to the next stage",
    abGatePrefix: zh ? "下一步需要你：" : "Next step needs you: ",
    abRetryHint: zh ? "已完成的部分都保留着，重试只补失败的部分" : "Completed work is preserved; retry only re-runs what failed",
    sourceChangedTitle: zh
      ? "来源已变化：旧分章和译文不再有效"
      : "Source changed: old chapters and translations are no longer valid",
    sourceChangedBody: zh
      ? "检测到 MinerU 精准解析结果与旧来源不同。需要按新来源重新分章并重做后续阶段；旧文件仍保留在备份中。"
      : "The MinerU Precision result differs from the old source. Rebuild chapters and downstream stages from the new source; old files remain backed up.",
    rebuildFromMineru: zh ? "按 MinerU 新源重建" : "Rebuild from MinerU source",

    // Live worker progress
    progressUnitPages: zh ? "页" : "pages",
    progressUnitChapters: zh ? "章" : "chapters",
    progressUnitChunks: zh ? "段" : "chunks",
    progressUnitItems: zh ? "项" : "items",
    progressStarting: zh ? "任务已启动" : "Task started",
    progressUploading: zh ? "正在上传到 OCR 服务" : "Uploading to OCR service",
    progressExtracting: zh ? "OCR 正在识别" : "OCR is extracting text",
    progressDownloading: zh ? "正在下载识别结果" : "Downloading OCR results",
    progressTranslating: zh ? "AI 正在翻译" : "AI is translating",
    progressReviewing: zh ? "AI 正在二遍检查" : "AI is reviewing",
    progressAssembling: zh ? "正在整理结果" : "Assembling results",
    progressWorking: zh ? "任务正在运行" : "Task is running",
    progressHeartbeat: zh ? "工作进度会自动更新，无需操作" : "Progress updates automatically; no action needed",
    progressCount: (completed: number, total: number, unit: string) =>
      zh ? `${completed} / ${total} ${unit}` : `${completed} / ${total} ${unit}`,
    progressAria: (phase: string, count?: string) => (count ? `${phase}：${count}` : phase),

    // Gate cards（3-5 先看样张）
    gateCardTitle: zh ? "等你确认 · 先看一眼" : "Needs you · take a look",
    gate1Lead: (n: string) =>
      zh
        ? `文字整理好了${n ? `（${n}）` : ""}。下一步会把这本书的正文发送给 AI 翻译服务来翻译。`
        : `The text is tidied up${n ? ` (${n})` : ""}. Next, the book's body text is sent to the AI translation service.`,
    gate1Privacy: zh
      ? "只发送正文文字；文件本身、笔记与凭证都留在本机。已发送的内容无法召回。"
      : "Only the body text is sent; the file itself, notes, and credentials stay on this machine. Sent content cannot be recalled.",
    gate1Ok: zh ? "没问题，开始翻译" : "Looks good — start translating",
    gate1Alt: zh ? "查看整理好的文字" : "View the tidied text",
    gate2Lead: (n: string) =>
      zh
        ? `全书翻译完了${n ? `（${n}）` : ""}。建议抽查几章译文，确认后在本机生成最终的阅读版。`
        : `Translation is complete${n ? ` (${n})` : ""}. Spot-check a few chapters, then confirm to build the final reading copy locally.`,
    gate2Privacy: zh
      ? "这一步在本机完成，不会再发送任何内容；成品随时可以重新生成。"
      : "This step runs locally and sends nothing; outputs can be rebuilt at any time.",
    gate2Ok: zh ? "没问题，生成阅读版" : "Looks good — build the reading copy",
    gate2Alt: zh ? "打开译文抽查" : "Open the translation to spot-check",
    gateScopeChapters: (n: number) => (zh ? `${n} 个章节` : `${n} chapter(s)`),
    gateSampleSourceTitle: zh ? "将发送的文字 · 样张" : "Text to be sent · sample",
    gateSampleTranslationTitle: zh ? "译文 · 样张" : "Translation · sample",
    sampleCompareTitle: zh ? "先抽样，再决定" : "Sample before deciding",
    sampleCompareIntro: zh
      ? "用当前模型翻译几段书中正文，先比较原文与译文；满意后再批准全书翻译。"
      : "Translate a few passages with the current model, compare source and target, then approve the full book.",
    sampleProvider: zh ? "抽样模型" : "Sample provider",
    // Diagnostic bundle export
    jobProvider: zh ? "本书正式模型" : "This book's model",
    sampleProviderDiffers: zh
      ? "样张用的模型和本书正式模型不是同一个。批准后全书会用正式模型翻译，不是你在样张里看到的那个。"
      : "The sample was run with a different model than this book's. Approving sends the whole book through this book's model, not the one you sampled.",
    applySampleProvider: zh ? "以此模型翻译本书" : "Translate this book with it",
    appliedSampleProvider: zh ? "本书正式模型已更新" : "This book's model was updated",
    sampleRun: zh ? "生成对照样张" : "Generate comparison",
    sampleRetry: zh ? "换模型后重试" : "Retry with provider",
    sampleReady: zh ? "翻译对照样张已生成" : "Translation comparison is ready",
    sampleSource: zh ? "原文" : "Source",
    sampleTranslation: zh ? "译文" : "Translation",
    ocrCompareTitle: zh ? "OCR 引擎对比" : "OCR engine comparison",
    ocrCompareLead: zh
      ? "抽几页正文，两个引擎各跑一次，挑出更适合这本书的那个。只上传抽出的这几页，和全书长度无关。"
      : "Sample a few interior pages, run both engines over them, and pick the one that suits this book. Only the sampled pages are uploaded, whatever the book's length.",
    ocrCompareRun: zh ? "生成对比" : "Compare engines",
    ocrCompareRetry: zh ? "重抽" : "Re-sample",
    ocrComparePages: zh ? "抽样页数" : "Pages",
    ocrCompareReady: zh ? "OCR 引擎对比已生成" : "OCR comparison is ready",
    ocrCompareSampledPages: zh ? "抽样页" : "Sampled pages",
    ocrCompareCharacters: zh ? "字符" : "characters",
    ocrCompareSeconds: zh ? "秒" : "s",
    ocrCompareEmpty: zh ? "这个引擎没有返回文字。" : "This engine returned no text.",
    ocrCompareFailed: zh ? "未能完成" : "Did not finish",
    ocrComparePick: zh ? "用这个引擎转换" : "Convert with this engine",
    // Said plainly because the pages cost money: the report is a preview of the
    // real conversion, not the conversion itself.
    ocrCompareNote: zh
      ? "对比不会改动这本书，选定后才会写入转换路由。"
      : "Comparing changes nothing about this book; only picking a side writes the conversion route.",
    sampleDegradationNone: zh ? "完整" : "Complete",
    sampleDegradationAligned: zh ? "已对齐修复" : "Aligned fallback",
    sampleDegradationSource: zh ? "保留原文" : "Source fallback",
    gateBlockedByChecks: zh ? "有检查未通过，展开高级详情查看。" : "Some checks failed — see advanced details.",
    gateBlockedByRetiredProvider: zh
      ? "本书正式模型已不再受支持，就这样批准的话翻译起不来。请在上面选一个当前支持的模型，点「以此模型翻译本书」，再批准。"
      : "This book's model is no longer supported, and approving as-is leaves a translation that cannot start. Pick a supported model above, apply it to the book, then approve.",
    gateInvalidatedNote: zh
      ? "内容在审批后发生了变化，需要重新生成审批包。"
      : "Content changed after the packet was built; it needs re-packaging.",

    // Status vocabulary
    statusRunning: zh ? "进行中" : "Running",
    statusWaiting: zh ? "待审批" : "Awaiting approval",
    statusBlocked: zh ? "已拦截" : "Blocked",
    statusFailed: zh ? "失败" : "Failed",
    statusCompleted: zh ? "已完成" : "Completed",
    statusPartial: zh ? "部分完成" : "Partial",
    statusQueued: zh ? "排队" : "Queued",
    statusReady: zh ? "就绪" : "Ready",
    statusSkipped: zh ? "已跳过" : "Skipped",

    // Detail tabs
    tabOverview: zh ? "总览" : "Overview",
    tabStages: zh ? "阶段" : "Stages",
    tabArtifacts: zh ? "工件" : "Artifacts",
    tabApproval: zh ? "审批" : "Approval",
    tabLogs: zh ? "日志" : "Logs",

    // Stage labels — one per PIPELINE_STAGE_ORDER entry (model.ts type-checks this)
    stageRoute: zh ? "路由" : "Route",
    stageExtract: zh ? "提取" : "Extract",
    stageIndex: zh ? "建索引" : "Index",
    stageHandoff: zh ? "交接" : "Handoff",
    stageSplit: zh ? "分章" : "Split",
    stagePrepare: zh ? "备译" : "Prepare",
    stageApproveTranslation: zh ? "译前审批" : "Pre-translation gate",
    stageTranslate: zh ? "翻译" : "Translate",
    stageExpertQa: zh ? "专家QA" : "Expert QA",
    stageApprovePromotion: zh ? "晋升审批" : "Promotion gate",
    stagePromote: zh ? "晋升" : "Promote",
    stageBuildReading: zh ? "成书" : "Build",
    stageValidateReading: zh ? "校验" : "Validate",
    stageDiscover: zh ? "发现" : "Discover",
    stageBuildDigest: zh ? "摘要" : "Digest",

    // Route kind chips
    routeDirectText: zh ? "直读文本层" : "Direct text",
    routeRemotePaddle: zh ? "PaddleOCR-VL" : "PaddleOCR-VL",
    routeMineru: zh ? "MinerU 精解" : "MinerU",
    routeDirty: zh ? "脏文本层·已拦截" : "Dirty text layer · blocked",
    routeNoAttachment: zh ? "未找到附件" : "No attachment found",
    routeAlreadyConverted: zh ? "已有转换" : "Already converted",
    routeMissingCredentials: zh ? "缺少凭证" : "Missing credentials",
    routeTranslationHandoff: zh ? "翻译交接" : "Translation handoff",
    routeTranslationReady: zh ? "直进翻译" : "Translation ready",
    routeExternalAdapter: zh ? "外部适配器" : "External adapter",
    routeEpubSource: zh ? "EPUB 章节抽取" : "EPUB extraction",

    // Overview tab
    evidenceTitle: zh ? "路由证据" : "Route evidence",
    artifactDigestTitle: zh ? "工件摘要" : "Artifact summary",
    evTextLayer: zh ? "当前来源依据" : "Current source evidence",
    evRoute: zh ? "选定路线" : "Selected route",
    evSourceKind: zh ? "来源类型" : "Source kind",
    evFingerprint: zh ? "源指纹" : "Source fingerprint",
    artifactExtractionMarkdown: zh ? "extraction_markdown" : "extraction_markdown",
    artifactZoteroAttachment: zh ? "Zotero 子附件" : "Zotero child attachment",
    artifactSourceMap: zh ? "source_map" : "source_map",
    artifactReading: zh ? "阅读成品" : "Reading outputs",
    artifactPresent: zh ? "✓" : "✓",
    artifactAttached: zh ? "已挂 ✓" : "attached ✓",
    artifactGenerating: zh ? "生成中" : "In progress",
    waitingHintPrefix: zh ? "◆ 此书正在人工门等待：" : "◆ This book waits at a human gate: ",
    goApprovalTab: zh ? "去审批页签" : "Open approval tab",
    failedHintRetryable: zh ? "已保留已完成部分，重试只补失败边界。" : "Completed work is preserved; retry only re-runs the failed boundary.",
    retryJob: zh ? "重试该书" : "Retry this book",
    deleteBook: zh ? "删除该书" : "Delete this book",
    deleteBookConfirmHint: zh
      ? "从书架移除这本书？磁盘上的已转换文件与 Zotero 附件都会保留，之后重新添加可复用。"
      : "Remove this book from the shelf? Converted files on disk and Zotero attachments are kept and can be reused if you re-add it.",
    deleteBookConfirm: zh ? "确认删除" : "Confirm delete",
    deleteBookCancel: zh ? "取消" : "Cancel",
    deleteBookDone: zh ? "已从书架删除" : "Removed from the shelf",
    blockedKeepMineru: zh ? "保留 MinerU 结果" : "Keep MinerU result",
    blockedForcePaddle: zh ? "强制 Paddle" : "Force Paddle",
    blockedDefer: zh ? "推迟" : "Defer",
    runnerPendingNote: zh ? "该操作等待 runner 命令接入，当前不可用。" : "This action waits on a runner command and is currently unavailable.",
    handoffReadyHint: zh ? "转换完成，可以交接进本地阅读项目。" : "Conversion completed; ready to hand off into a local reading project.",

    // Stages tab
    attemptLabel: (n: number) => (zh ? `尝试 ${n}` : `attempt ${n}`),
    stageWaitingYou: zh ? "等待你的决定" : "Waiting for your decision",
    stageRetryable: zh ? "可重试" : "retryable",
    stageRetriesLeft: (n: number) => (zh ? `将自动重试 ${n} 次` : `${n} auto-retry(s) left`),
    stageRetryScheduled: (at: string) => {
      const due = new Date(at);
      const label = Number.isNaN(due.getTime()) ? at : due.toLocaleTimeString();
      return zh ? `将于 ${label} 自动重试` : `auto-retry at ${label}`;
    },
    stageGaveUp: (reason: string) =>
      reason === "not_retryable"
        ? (zh ? "不可自动重试" : "not auto-retryable")
        : (zh ? "自动重试已用尽" : "auto-retries exhausted"),
    stageBlockedMeta: zh ? "已拦截 · 等待裁决" : "Blocked · awaiting decision",
    stageRunningMeta: zh ? "进行中" : "Running",
    stageSkippedMeta: zh ? "已跳过" : "Skipped",
    stageErrorLabel: zh ? "错误" : "Error",
    stageUnitsLabel: zh ? "单元" : "Units",
    stageFailedUnitsLabel: zh ? "失败单元" : "Failed units",
    failureProviderTimeout: zh ? "模型请求超时" : "provider timeout",
    failureProviderServer: zh ? "模型服务端错误（5xx）" : "provider server error (5xx)",
    failureProviderRateLimit: zh ? "模型限流" : "provider rate limit",
    failureProviderUnavailable: zh ? "模型服务不可用" : "provider unavailable",
    failureStructureInvalid: zh ? "译文结构不合格" : "translation structure mismatch",
    failureProviderFatal: zh ? "模型拒绝请求" : "provider rejected the request",
    failureReasonSeparator: zh ? "；" : "; ",
    translationFailureSummary: (count: number, reasons: string) =>
      zh ? `翻译失败 ${count} 个单元：${reasons}` : `Translation failed for ${count} unit(s): ${reasons}`,
    stageArtifactsLabel: zh ? "登记工件" : "Artifacts",
    stageInputsLabel: zh ? "输入哈希" : "Input hashes",
    stageStartedLabel: zh ? "开始" : "Started",
    stageFinishedLabel: zh ? "完成于" : "Finished",

    // Artifacts tab
    // Story 18's other half: EPUBCheck says the file is well-formed, a person
    // says it actually reads. Only the second half needs a human to record it.
    thArtifact: zh ? "工件" : "Artifact",
    thPath: zh ? "路径 / 位置" : "Path / location",
    thSha: zh ? "SHA-256" : "SHA-256",
    thValidation: zh ? "校验" : "Validation",
    thProducer: zh ? "产自" : "Produced by",
    formatBilingualShort: zh ? "双语EPUB" : "Bilingual EPUB",
    artifactsEmpty: zh ? "尚无注册工件。工件在各阶段完成时登记：路径、SHA-256、产出阶段与校验状态。" : "No registered artifacts yet. Artifacts are registered as stages complete: path, SHA-256, producer stage, and validation state.",
    validationOk: zh ? "✓" : "✓",
    validationFailed: zh ? "✗" : "✗",
    validationUnknown: zh ? "—" : "—",

    // Approval tab
    gateTranslation: zh ? "译前披露审批" : "Translation disclosure",
    gatePromotion: zh ? "成品晋升审批" : "Final promotion",
    humanGatePrefix: zh ? "人工门" : "Human gate",
    invalidPrefix: zh ? "已失效" : "Invalidated",
    scopeLabel: zh ? "范围" : "Scope",
    eventPrereqTranslation: zh ? "分章与备译完成" : "Split & prepare completed",
    eventPrereqPromotion: zh ? "QA 全部闭环" : "QA fully closed",
    eventPacket: zh ? "审批包生成 · 哈希绑定" : "Review packet bound to hashes",
    eventInvalidated: zh ? "绑定哈希变更 · 审批失效" : "Bound hash changed · approval invalidated",
    eventDecision: zh ? "你的决定" : "Your decision",
    eventNow: zh ? "现在" : "now",
    eventRepackage: zh ? "待重新打包" : "awaiting re-packaging",
    eventAfterTranslation: zh ? "翻译启动" : "Translation starts",
    eventAfterPromotion: zh ? "晋升与成书" : "Promotion & build",
    afterApproval: zh ? "批准后" : "after approval",
    checkPacket: zh ? "审批包完整" : "Review packet complete",
    checkHashes: zh ? "绑定哈希与当前工件一致" : "Bound hashes match current artifacts",
    checkProvider: zh ? "提供方与配置就绪" : "Provider and config ready",
    checkQa: zh ? "QA 全部闭环" : "QA fully closed",
    checkNoBlocker: zh ? "范围内无未决阻塞" : "No unresolved blockers in scope",
    hashUnknown: zh ? "后端未提供绑定哈希，无法在前端校验" : "Bound hashes not provided by the backend; cannot verify in the UI",
    providerUnknown: zh ? "凭证预检由 runner 在发车时执行" : "Credential preflight runs in the runner at launch",
    qaNotClosed: zh ? "存在未通过或未决的 QA 单元" : "Some QA units are unresolved",
    blockerFound: zh ? "存在失败或拦截的阶段遗留" : "Failed or blocked stages remain in scope",
    beforeNow: zh ? "批准前 · 现在" : "Before · now",
    afterWill: zh ? "批准后 · 将发生" : "After · will happen",
    diffSourceText: zh ? "源文本" : "Source text",
    diffChapterUnits: zh ? "章节单元" : "Chapter units",
    diffTranslated: zh ? "审定译文" : "Reviewed translation",
    diffReading: zh ? "阅读成品" : "Reading outputs",
    diffSourceLocal: zh ? "仅在本机" : "local only",
    diffSourceSend: zh ? "发往翻译提供方" : "sent to the translation provider",
    diffUnitsReady: zh ? "备译完成" : "prepared",
    diffUnitsStart: zh ? "翻译启动 · 失败单元可独立重试" : "translation starts · failed units retry independently",
    diffTranslatedDraft: zh ? "chapters/translated" : "chapters/translated",
    diffPromoteFinal: zh ? "晋升至 chapters/final" : "promoted to chapters/final",
    diffReadingNone: zh ? "—" : "—",
    diffReadingBuilt: zh ? "按请求格式构建 + 校验" : "built in requested formats + validated",
    reversibility: zh ? "可回退性" : "Reversibility",
    irreversibleSend: zh ? "已发送内容不可召回" : "sent content cannot be recalled",
    finalOnlyByApproval: zh ? "成品可重建 · final 仅由审批产生" : "outputs rebuildable · final only via approval",
    approve: zh ? "全部通过 · 批准" : "All checks pass · Approve",
    reject: zh ? "驳回" : "Reject",
    openPacket: zh ? "打开审批包文件" : "Open review packet",
    regeneratePacket: zh ? "基于当前哈希重新生成审批包" : "Regenerate packet from current hashes",
    checksPassed: (passed: number, total: number) => (zh ? `${passed} / ${total} 项通过` : `${passed} / ${total} checks pass`),
    checksFailedNote: zh ? "项检查未通过 · 批准不可用" : "check(s) failed · approval unavailable",
    approvalActionPending: zh ? "重新生成审批包与驳回等待 runner 命令接入。" : "Packet regeneration and reject wait on runner commands.",
    approvalEmpty: zh ? "此书当前没有待决审批。到达译前披露或成品晋升门时，审批包会出现在这里。" : "No pending approval for this book. Review packets appear here at the translation disclosure and final promotion gates.",
    approvedRecordsNote: zh ? "审批记录如下（哈希绑定，不可变更）：" : "Recorded approvals (hash-bound, immutable):",
    decisionApproved: zh ? "已批准" : "approved",
    decisionRejected: zh ? "已驳回" : "rejected",
    boundWith: zh ? "绑定" : "bound",

    // Logs tab
    logRedactionNote: zh ? "日志已脱敏：不含源文本、提供方原始响应与凭证；导出遵循 redacted-support 档。" : "Logs are redacted: no source text, provider payloads, or credentials; exports follow the redacted-support profile.",
    logsEmpty: zh ? "暂无日志。" : "No log lines yet.",

    // Input island — the single "add a book" surface that replaced the wizard
    islandDropHint: zh
      ? "把 PDF 拖到这里，或按书名搜索 Zotero"
      : "Drop a PDF here, or search Zotero by title",
    islandDropActive: zh ? "松手即可加入书架" : "Release to add to the shelf",
    islandDroppedFolder: zh ? "已选文件夹" : "Chosen folder",
    islandFolderBatchNote: zh
      ? "这个文件夹里的 PDF 会一起转换，下面列出的就是全部。"
      : "Every PDF in this folder is converted together; the list below is all of them.",
    islandZoteroResults: zh ? "Zotero 搜索结果" : "Zotero results",
    islandOcrReady: zh ? "已配置" : "ready",
    islandOcrMissing: zh ? "未配置" : "no key",
    islandOcrHint: zh
      ? "OCR 密钥在「设置 → OCR」里配置。"
      : "OCR keys are configured under Settings → OCR.",
    islandEnqueue: (n: number) => (zh ? `开始 · ${n} 本` : `Start · ${n} book(s)`),
    islandEnqueueEmpty: zh ? "开始" : "Start",
    islandPreflightBusy: zh ? "正在预检路线…" : "Checking routes…",
    shelfCount: (n: number) => (zh ? `书架 · ${n} 本` : `Bookshelf · ${n} book(s)`),

    // Route preflight (shown inline in the input island)
    preflightReady: zh ? "可发车" : "Ready",
    preflightBlocked: zh ? "需人工裁决" : "Needs decision",
    preflightSkip: zh ? "跳过" : "Skip",
    thBook: zh ? "书目" : "Book",
    thAutoRoute: zh ? "自动路由" : "Auto route",
    thOverride: zh ? "覆写" : "Override",
    thPreflight: zh ? "预检" : "Preflight",
    overrideAuto: zh ? "自动（推荐）" : "Auto (recommended)",
    overrideForceDirect: zh ? "强制直读文本层" : "Force direct text",
    overrideForcePaddle: zh ? "强制 PaddleOCR" : "Force PaddleOCR",
    overrideForceMineru: zh ? "强制 MinerU" : "Force MinerU",
    overrideKeep: zh ? "保留现有结果" : "Keep existing result",
    batchSource: zh ? "来源" : "Source",
  };
}

export type PipelineCopy = ReturnType<typeof pipelineCopy>;
