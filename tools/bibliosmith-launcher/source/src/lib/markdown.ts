import type { Copy } from "../i18n";

export function renderMarkdownToHtml(source: string, copy: Copy) {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const html: string[] = [];
  let inList = false;
  let inCode = false;
  let codeLines: string[] = [];
  let inHtmlBlock = false;
  let htmlLines: string[] = [];

  const closeList = () => {
    if (inList) {
      html.push("</ul>");
      inList = false;
    }
  };

  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith("```")) {
      closeList();
      if (inCode) {
        html.push(renderCodeBlock(codeLines.join("\n"), copy.copyCode));
        codeLines = [];
        inCode = false;
      } else {
        inCode = true;
      }
      continue;
    }
    if (inCode) {
      codeLines.push(line);
      continue;
    }
    if (inHtmlBlock) {
      htmlLines.push(line);
      if (trimmed.toLowerCase().includes("</table>")) {
        html.push(renderDocTable(htmlLines.join("\n")));
        htmlLines = [];
        inHtmlBlock = false;
      }
      continue;
    }
    if (trimmed.toLowerCase().startsWith("<table")) {
      closeList();
      inHtmlBlock = true;
      htmlLines = [line];
      if (trimmed.toLowerCase().includes("</table>")) {
        html.push(renderDocTable(htmlLines.join("\n")));
        htmlLines = [];
        inHtmlBlock = false;
      }
      continue;
    }
    if (!trimmed) {
      closeList();
      continue;
    }
    const heading = trimmed.match(/^(#{1,4})\s+(.+)$/);
    if (heading) {
      closeList();
      const level = heading[1].length;
      html.push(`<h${level}>${formatInlineMarkdown(heading[2])}</h${level}>`);
      continue;
    }
    const listItem = trimmed.match(/^[-*]\s+(.+)$/);
    if (listItem) {
      if (!inList) {
        html.push("<ul>");
        inList = true;
      }
      html.push(`<li>${formatInlineMarkdown(listItem[1])}</li>`);
      continue;
    }
    closeList();
    html.push(`<p>${formatInlineMarkdown(trimmed)}</p>`);
  }
  closeList();
  if (inCode) {
    html.push(renderCodeBlock(codeLines.join("\n"), copy.copyCode));
  }
  if (inHtmlBlock && htmlLines.length) {
    html.push(renderDocTable(htmlLines.join("\n")));
  }
  return html.join("\n");
}

function renderCodeBlock(code: string, copyLabel: string) {
  return [
    `<div class="code-block">`,
    `<button type="button" class="code-copy-button" data-copy-code="${encodeCodePayload(code)}">${escapeHtml(copyLabel)}</button>`,
    `<pre><code>${escapeHtml(code)}</code></pre>`,
    `</div>`,
  ].join("");
}

function encodeCodePayload(value: string) {
  return encodeURIComponent(value);
}

export function decodeCodePayload(value: string) {
  return decodeURIComponent(value);
}

export async function copyTextToClipboard(text: string) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  document.body.appendChild(textarea);
  textarea.select();
  const ok = document.execCommand("copy");
  textarea.remove();
  if (!ok) throw new Error("copy failed");
}

function formatInlineMarkdown(value: string) {
  let html = escapeHtml(value);
  html = html.replace(/`([^`]+)`/g, "<code>$1</code>");
  html = html.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_match, label, href) => {
    const safeHref = sanitizeHref(String(href));
    return `<a href="${safeHref}">${label}</a>`;
  });
  return html;
}

function escapeHtml(value: string) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

export function sanitizeHref(value: string) {
  const trimmed = value.trim();
  try {
    const protocol = new URL(trimmed, "https://bibliosmith.invalid/").protocol;
    if (protocol !== "http:" && protocol !== "https:" && protocol !== "mailto:") return "#";
  } catch {
    return "#";
  }
  return escapeHtml(trimmed);
}

function safeTableTagName(value: string) {
  switch (value) {
    case "a": return "a";
    case "br": return "br";
    case "caption": return "caption";
    case "code": return "code";
    case "em": return "em";
    case "strong": return "strong";
    case "table": return "table";
    case "tbody": return "tbody";
    case "td": return "td";
    case "tfoot": return "tfoot";
    case "th": return "th";
    case "thead": return "thead";
    case "tr": return "tr";
    default: return "";
  }
}

function parseQuotedAttributes(value: string) {
  const attributes: Record<string, string> = {};
  let index = 0;
  while (index < value.length) {
    while (index < value.length && /\s/.test(value[index])) index += 1;
    if (index >= value.length || value[index] === "/") break;

    const nameStart = index;
    while (index < value.length && /[A-Za-z0-9_-]/.test(value[index])) index += 1;
    if (index === nameStart) return null;
    const name = value.slice(nameStart, index).toLowerCase();

    while (index < value.length && /\s/.test(value[index])) index += 1;
    if (value[index] !== "=") return null;
    index += 1;
    while (index < value.length && /\s/.test(value[index])) index += 1;

    const quote = value[index];
    if (quote !== '"' && quote !== "'") return null;
    const end = value.indexOf(quote, index + 1);
    if (end < 0) return null;
    attributes[name] = value.slice(index + 1, end);
    index = end + 1;
  }
  return attributes;
}

function renderSafeTableTag(value: string) {
  const trimmed = value.trim();
  const closing = trimmed.startsWith("/");
  const content = closing ? trimmed.slice(1).trim() : trimmed;
  const nameEnd = content.search(/\s/);
  const rawName = (nameEnd < 0 ? content : content.slice(0, nameEnd)).replace(/\/$/, "").toLowerCase();
  const name = safeTableTagName(rawName);
  if (!name) return "";

  const rest = nameEnd < 0 ? "" : content.slice(nameEnd);
  if (closing) return rest.trim() ? "" : `</${name}>`;
  const attributes = parseQuotedAttributes(rest);
  if (!attributes) return "";

  if (name === "a") {
    return `<a href="${sanitizeHref(attributes.href ?? "#")}">`;
  }
  if (name === "td" || name === "th") {
    const span = ["colspan", "rowspan"]
      .flatMap((attribute) => {
        const raw = attributes[attribute];
        const parsed = raw ? Number(raw) : 0;
        return Number.isInteger(parsed) && parsed > 0 && parsed <= 100
          ? [` ${attribute}="${parsed}"`]
          : [];
      })
      .join("");
    return `<${name}${span}>`;
  }
  return name === "br" ? "<br>" : `<${name}>`;
}

export function renderDocTable(value: string) {
  const html: string[] = [];
  let cursor = 0;
  while (cursor < value.length) {
    const tagStart = value.indexOf("<", cursor);
    if (tagStart < 0) {
      html.push(escapeHtml(value.slice(cursor)));
      break;
    }
    html.push(escapeHtml(value.slice(cursor, tagStart)));
    const tagEnd = value.indexOf(">", tagStart + 1);
    if (tagEnd < 0) {
      html.push(escapeHtml(value.slice(tagStart)));
      break;
    }
    html.push(renderSafeTableTag(value.slice(tagStart + 1, tagEnd)));
    cursor = tagEnd + 1;
  }
  return html.join("");
}
