import type { DownloadProgress } from "../types";
import type { Copy } from "../i18n";

export function nowLabel() {
  return new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function formatBytes(value: number) {
  if (!value) return "0.0 KB";
  return `${(value / 1024).toFixed(1)} KB`;
}

// formatBytes above is fixed at KB, which suits the runtime downloads it was
// written for. A launcher update carries the whole App bundle — Node, uv and a
// Chromium runtime included — so the same helper would render it as a
// seven-digit KB count. This one picks the unit.
const FILE_SIZE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

export function formatFileSize(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < FILE_SIZE_UNITS.length - 1) {
    size /= 1024;
    unit += 1;
  }
  // Bytes are whole things; anything the loop scaled reads better with one
  // decimal than as a rounded integer that jumps 1 GB at a time.
  return `${unit === 0 ? size : size.toFixed(1)} ${FILE_SIZE_UNITS[unit]}`;
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
