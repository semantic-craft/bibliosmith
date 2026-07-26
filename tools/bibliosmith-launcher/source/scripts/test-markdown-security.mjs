import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const output = mkdtempSync(path.join(tmpdir(), "bibliosmith-markdown-security-"));

try {
  execFileSync(
    path.join(root, "node_modules", ".bin", "tsc"),
    [
      "src/lib/markdown.ts",
      "--ignoreConfig",
      "--outDir", output,
      "--target", "ES2022",
      "--module", "ES2022",
      "--moduleResolution", "Bundler",
      "--skipLibCheck",
    ],
    { cwd: root, stdio: "pipe" },
  );
  const markdown = await import(pathToFileURL(path.join(output, "lib", "markdown.js")));

  assert.equal(markdown.sanitizeHref("javascript:alert(1)"), "#");
  assert.equal(markdown.sanitizeHref("data:text/html,<script>alert(1)</script>"), "#");
  assert.equal(markdown.sanitizeHref("vbscript:msgbox(1)"), "#");
  assert.equal(markdown.sanitizeHref("https://example.com/guide"), "https://example.com/guide");
  assert.equal(markdown.sanitizeHref("../guide"), "../guide");

  const rendered = markdown.renderDocTable(
    '<table onclick="alert(1)"><tr><td colspan="2"><a href="https://example.com/guide">Guide</a><img src=x onerror="alert(1)"></td></tr></table>',
  );
  assert.equal(
    rendered,
    '<table><tr><td colspan="2"><a href="https://example.com/guide">Guide</a></td></tr></table>',
  );
  assert.equal(
    markdown.renderDocTable('<table><tr><td><a href="javascript:alert(1)">Bad</a></td></tr></table>'),
    '<table><tr><td><a href="#">Bad</a></td></tr></table>',
  );

  const copy = { copyCode: "copy" };

  // An inline link's href is escaped exactly once. Extracting it from already
  // escaped text used to yield `&amp;amp;`, breaking every link with a query
  // string.
  assert.equal(
    markdown.renderMarkdownToHtml("See [docs](https://example.com/g?a=1&b=2).", copy),
    '<p>See <a href="https://example.com/g?a=1&amp;b=2">docs</a>.</p>',
  );

  // Two links on one line each keep their own single-escaped href.
  assert.equal(
    markdown.renderMarkdownToHtml("[a](https://e.com/1) and [b](https://e.com/2?x=1&y=2)", copy),
    '<p><a href="https://e.com/1">a</a> and <a href="https://e.com/2?x=1&amp;y=2">b</a></p>',
  );

  // Reading the href before escaping must not weaken the allowlist, let a
  // crafted href break out of the attribute, or leave a label unescaped.
  assert.equal(
    markdown.renderMarkdownToHtml("[x](javascript:alert)", copy),
    '<p><a href="#">x</a></p>',
  );
  assert.equal(
    markdown.renderMarkdownToHtml('[x](https://example.com/" onmouseover="alert)', copy),
    '<p><a href="https://example.com/&quot; onmouseover=&quot;alert">x</a></p>',
  );
  assert.equal(
    markdown.renderMarkdownToHtml("[<img src=x onerror=alert>](https://example.com/)", copy),
    '<p><a href="https://example.com/">&lt;img src=x onerror=alert&gt;</a></p>',
  );
  assert.equal(
    markdown.renderMarkdownToHtml("plain <b>text</b> & symbols", copy),
    "<p>plain &lt;b&gt;text&lt;/b&gt; &amp; symbols</p>",
  );
  assert.equal(
    markdown.renderMarkdownToHtml("**bold** and `code` and [l](https://e.com/)", copy),
    '<p><strong>bold</strong> and <code>code</code> and <a href="https://e.com/">l</a></p>',
  );
} finally {
  rmSync(output, { recursive: true, force: true });
}

console.log("markdown security contract ok");
