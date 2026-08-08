// Bilingual strings for the launcher update panel, kept local (like
// modelsCopy.ts and ocrCopy.ts) rather than threaded through the four-locale
// i18n object.
export function updatesCopy(locale: string) {
  const zh = locale.startsWith("zh");
  return {
    title: zh ? "应用更新" : "App updates",
    description: zh
      ? "启动时会在后台检查一次新版本。更新包由本项目签名，安装前会校验签名；是否安装由你决定。"
      : "Launcher checks for a new version once in the background at startup. Update bundles are signed by this project and the signature is verified before anything is installed; installing is your decision.",
    currentVersion: zh ? "当前版本" : "Current version",
    check: zh ? "检查更新" : "Check for updates",
    checking: zh ? "检查中…" : "Checking…",
    upToDate: zh ? "已是最新版本" : "Up to date",
    checkedAt: (time: string) => (zh ? `上次检查 ${time}` : `Last checked ${time}`),
    available: (version: string) =>
      zh ? `新版本 ${version} 可用` : `Version ${version} is available`,
    releasedOn: (date: string) => (zh ? `发布于 ${date}` : `Released ${date}`),
    install: zh ? "下载并安装" : "Download and install",
    downloading: zh ? "下载中…" : "Downloading…",
    downloadedOf: (downloaded: string, total: string) =>
      zh ? `已下载 ${downloaded} / ${total}` : `${downloaded} of ${total} downloaded`,
    downloadedSoFar: (downloaded: string) =>
      zh ? `已下载 ${downloaded}` : `${downloaded} downloaded`,
    installing: zh ? "正在安装…" : "Installing…",
    readyTitle: (version: string) =>
      zh ? `${version} 已安装，重启后生效` : `${version} is installed and takes effect after a restart`,
    restart: zh ? "立即重启" : "Restart now",
    restarting: zh ? "重启中…" : "Restarting…",
    // The install replaces the whole App bundle, including the runtime the
    // running jobs are executing out of, so it cannot be allowed mid-flight.
    blockedByJobs: (count: number) =>
      zh
        ? `有 ${count} 个任务正在运行，安装会替换整个应用并中断它们。等任务结束后再安装。`
        : `${count} job(s) are running. Installing replaces the whole app and would interrupt them. Install once they finish.`,
    checkFailed: (error: string) =>
      zh ? `检查更新失败：${error}` : `Update check failed: ${error}`,
    installFailed: (error: string) =>
      zh ? `安装失败：${error}` : `Install failed: ${error}`,
    restartFailed: (error: string) =>
      zh ? `重启失败：${error}。请手动退出并重新打开应用。` : `Restart failed: ${error}. Quit and reopen the app manually.`,
    updateFoundToast: (version: string) =>
      zh ? `发现新版本 ${version}，可在设置中安装` : `Version ${version} is available; install it in Settings`,
    releaseNotesLabel: zh ? "更新内容" : "What's new",
  };
}

export type UpdatesCopy = ReturnType<typeof updatesCopy>;
