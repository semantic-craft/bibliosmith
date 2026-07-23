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
} finally {
  rmSync(output, { recursive: true, force: true });
}

console.log("markdown security contract ok");
