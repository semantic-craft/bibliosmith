import type { Locale } from "./copy";

// Release notes carry ZH, EN and JA sections. Mapping ja to EN, as this did,
// handed Japanese readers English prose that was sitting right there in the
// same file. zh-TW has no section of its own, so it keeps taking ZH.
function releaseNotesLanguage(locale: Locale) {
  if (locale.startsWith("zh")) return "ZH";
  if (locale.startsWith("ja")) return "JA";
  return "EN";
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
    // Two shapes reach this parser. Commit bodies label their sections
    // "ZH:", the form check_commit_messages.py enforces. RELEASE_NOTES.md —
    // which is also the GitHub release body and the update manifest's notes —
    // uses a "## ZH" Markdown heading, the form release-launcher.yml enforces.
    // Reading only the first meant the update card fell back to the whole
    // multilingual file.
    const match = line
      .trim()
      .match(/^(?:#{1,6}\s*(ZH|EN|JA)\s*$|(ZH|EN|JA)\s*:\s*(.*)$)/i);
    if (match) {
      flush();
      current = (match[1] ?? match[2]).toUpperCase();
      buffer.length = 0;
      if (match[3]) buffer.push(match[3]);
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
