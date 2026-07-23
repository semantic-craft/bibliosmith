import type { Locale } from "./copy";

function releaseNotesLanguage(locale: Locale) {
  return locale.startsWith("zh") ? "ZH" : "EN";
}

export function localizedReleaseNotes(body: string | null | undefined, locale: Locale, fallback: string) {
  const normalized = (body ?? "").replace(/\r\n/g, "\n").trim();
  if (!normalized) return fallback;
  const sections = parseLabeledSections(normalized);
  const preferred = sections.get(releaseNotesLanguage(locale));
  const english = sections.get("EN");
  const fallbackSection = sections.values().next().value;
  return trimReleaseNotes(preferred || english || fallbackSection || normalized || fallback);
}

function parseLabeledSections(body: string) {
  const sections = new Map<string, string>();
  let current: string | null = null;
  const buffer: string[] = [];
  const flush = () => {
    if (!current) return;
    const text = buffer.join("\n").trim();
    if (text) sections.set(current, text);
  };
  for (const line of body.split("\n")) {
    const match = line.trim().match(/^(ZH|EN|JA)\s*:\s*(.*)$/i);
    if (match) {
      flush();
      current = match[1].toUpperCase();
      buffer.length = 0;
      if (match[2]) buffer.push(match[2]);
      continue;
    }
    if (current) buffer.push(line);
  }
  flush();
  return sections;
}

function trimReleaseNotes(value: string) {
  const lines = value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .slice(0, 5);
  const text = lines.join("\n");
  return text.length > 520 ? `${text.slice(0, 517).trimEnd()}...` : text;
}
