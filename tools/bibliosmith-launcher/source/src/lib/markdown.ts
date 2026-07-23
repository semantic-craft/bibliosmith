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
        html.push(sanitizeTrustedDocHtml(htmlLines.join("\n")));
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
        html.push(sanitizeTrustedDocHtml(htmlLines.join("\n")));
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
    html.push(sanitizeTrustedDocHtml(htmlLines.join("\n")));
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

function sanitizeHref(value: string) {
  const trimmed = value.trim();
  if (/^javascript:/i.test(trimmed)) return "#";
  return escapeHtml(trimmed);
}

function sanitizeTrustedDocHtml(value: string) {
  return value
    .replace(/<script[\s\S]*?<\/script>/gi, "")
    .replace(/\son\w+="[^"]*"/gi, "")
    .replace(/\son\w+='[^']*'/gi, "")
    .replace(/javascript:/gi, "");
}
