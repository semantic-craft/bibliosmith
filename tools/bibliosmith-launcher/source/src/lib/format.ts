import type { CommitInfo, DownloadProgress } from "../types";
import type { Copy } from "../i18n";

export function nowLabel() {
  return new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function formatBytes(value: number) {
  if (!value) return "0.0 KB";
  return `${(value / 1024).toFixed(1)} KB`;
}

export function clampPercent(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

export function formatPercent(value: number) {
  return `${clampPercent(value).toFixed(2)}%`;
}

export function progressWidth(value: number) {
  return `${clampPercent(value)}%`;
}

export function formatDownloadProgress(copy: Copy, progress?: DownloadProgress | null) {
  if (!progress) return "";
  if (progress.message) {
    if (progress.message.includes("%") || progress.message.includes("KB")) {
      return progress.message;
    }
    return `${progress.message} ${formatPercent(progress.percent)}`;
  }
  if (progress.totalBytes === 100 && progress.downloadedBytes <= 100) {
    return `${copy.working} ${formatPercent(progress.percent)}`;
  }
  if (!progress.totalBytes && !progress.downloadedBytes) {
    return `${copy.downloading} ${formatPercent(progress.percent)}`;
  }
  const total = progress.totalBytes ? ` / ${formatBytes(progress.totalBytes)}` : "";
  return `${copy.downloading} ${formatPercent(progress.percent)} (${formatBytes(progress.downloadedBytes)}${total})`;
}

export function sleep(ms: number) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

/** Shown wherever a real value is genuinely unknown. */
export const UNKNOWN_VALUE = "—";

// Both of these used to fall back to a fabricated date ("v2025.05.25" /
// "2025-05-25 10:15"), which the Updates page then presented as the project's
// actual version and timestamp whenever no commit was available. An unknown
// value has to look unknown.
export function versionFromDate(date?: string) {
  if (!date) return UNKNOWN_VALUE;
  return `v${date.slice(0, 10).replaceAll("-", ".")}`;
}

export function commitDate(commit?: CommitInfo) {
  return commit?.date?.slice(0, 16).replace("T", " ") || UNKNOWN_VALUE;
}
