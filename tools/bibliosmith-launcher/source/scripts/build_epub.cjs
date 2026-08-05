// Launcher-owned builder executed from the App's read-only resource directory.
const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');
const { createHash, randomUUID } = require('crypto');
const { compilePublicationStructure } = require('./compile_publication_structure.cjs');

const root = path.resolve(process.cwd());
const finalDir = path.join(root, 'chapters', 'final');
const outDir = path.join(root, 'output');
const readingDir = path.join(outDir, 'reading');
const workDir = path.join(outDir, 'epub_work');
const htmlDir = path.join(readingDir, 'html');
const epubPath = path.join(readingDir, 'book.epub');

function readText(file) {
  return fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n').replace(/\r/g, '\n');
}

function writeText(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text, 'utf8');
}

function listFiles(dir, ext) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir)
    .filter((name) => name.toLowerCase().endsWith(ext))
    .sort()
    .map((name) => path.join(dir, name));
}

function parseYaml(file) {
  if (!fs.existsSync(file)) return {};
  const out = {};
  for (const line of readText(file).split('\n')) {
    const match = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line);
    if (!match) continue;
    out[match[1]] = match[2].replace(/^["']|["']$/g, '').trim();
  }
  return out;
}

function readJson(file) {
  if (!fs.existsSync(file)) return {};
  return JSON.parse(readText(file));
}

function escapeHtml(text) {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function inline(text, semanticNonce = '') {
  const rendered = escapeHtml(text)
    .replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2">$1</a>')
    .replace(/`([^`]+)`/g, '<code>$1</code>');
  if (!semanticNonce) return rendered;
  const noterefToken = new RegExp(
    `@@BIBLIO_NOTEREF__${semanticNonce}__([A-Za-z][A-Za-z0-9_-]*)__([A-Za-z][A-Za-z0-9_-]*)__([0-9]+)__([A-Za-z][A-Za-z0-9_-]*)@@`,
    'g',
  );
  return rendered.replace(
    noterefToken,
    (_, noteId, referenceId, marker, definitionRootId) => `<a epub:type="noteref" id="${referenceId}" href="${definitionRootId}.xhtml#${noteId}">[${marker}]</a>`,
  );
}

function parseMarkdownTable(lines, start) {
  const rows = [];
  let i = start;
  while (i < lines.length) {
    const line = lines[i].trim();
    if (!line.startsWith('|') || !line.endsWith('|')) break;
    rows.push(line.slice(1, -1).split('|').map((cell) => cell.trim()));
    i += 1;
  }
  if (rows.length < 2 || !rows[1].every((cell) => /^:?-{3,}:?$/.test(cell))) return null;
  return { rows: [rows[0], ...rows.slice(2)], next: i };
}

function tableHtml(rows, semanticNonce) {
  const header = rows[0];
  const bodyRows = rows.slice(1);
  const thead = `<thead><tr>${header.map((cell) => `<th scope="col">${inline(cell, semanticNonce)}</th>`).join('')}</tr></thead>`;
  const tbody = bodyRows.map((row) => {
    const padded = [...row, ...Array(Math.max(0, header.length - row.length)).fill('')].slice(0, header.length);
    return `<tr>${padded.map((cell) => `<td>${inline(cell, semanticNonce)}</td>`).join('')}</tr>`;
  }).join('');
  return `<div class="table-wrap" role="region" aria-label="结构化表格"><table><caption>结构化表格</caption>${thead}<tbody>${tbody}</tbody></table></div>`;
}

const COMMENT_OPEN = '<!--';
const COMMENT_CLOSE = '-->';

function isUntrustedHtmlBlockLine(line) {
  return /^<\/?[A-Za-z][^>]*>/.test(line.trim());
}

/**
 * Whether a paragraph is nothing but HTML comments.
 *
 * The PaddleOCR assembler writes a `<!-- page: N -->` anchor between pages so a
 * reviewer can map a passage back to a page of the original, and picked a
 * comment precisely so the marker would stay out of the prose. Nothing here
 * reads it, and `inline` escapes every paragraph it is handed, so an anchor left
 * in place reaches the reader as the literal text `<!-- page: N -->`.
 *
 * Only a paragraph that is *entirely* comments goes; a comment sitting in a real
 * paragraph is that paragraph's content and stays with it. Testing the whole
 * paragraph rather than each line is what makes that hold — and it keeps this
 * builder's reading of a chapter identical to `build_bilingual_epub.py`'s, which
 * drops the same blocks from the same Markdown.
 *
 * A scan rather than a regular expression, because both regex forms are worse
 * here: `text.replace(/<!--[\s\S]*?-->/g, '')` is a single sanitizing pass that
 * can leave a `<!--` behind, and the anchored alternative nests a lazy quantifier
 * inside a `+`, which backtracks exponentially on a long run of comments. This
 * walks the paragraph once.
 */
/**
 * Whether a line leaves a comment open at its end.
 *
 * Scanned pair by pair rather than by a single `indexOf`, because a line may
 * hold several comments and only the last one's fate decides whether the next
 * line is still inside a comment.
 */
function opensUnclosedComment(line) {
  let index = 0;
  for (;;) {
    const open = line.indexOf(COMMENT_OPEN, index);
    if (open < 0) return false;
    const close = line.indexOf(COMMENT_CLOSE, open + COMMENT_OPEN.length);
    if (close < 0) return true;
    index = close + COMMENT_CLOSE.length;
  }
}

function isCommentOnly(text) {
  let rest = text.trim();
  if (!rest) return false;
  while (rest.startsWith(COMMENT_OPEN)) {
    // Searching past the opener is what keeps `<!-->` unterminated: from index
    // zero its own `--` and `>` read as a closer, and the paragraph would
    // vanish. HTML5 does call that an empty comment, but the bilingual builder
    // does not drop it either, and leaving an oddity escaped is the safe half
    // of the trade.
    const close = rest.indexOf(COMMENT_CLOSE, COMMENT_OPEN.length);
    // Unterminated: not a comment as far as any parser is concerned, so not
    // this rule's business to delete.
    if (close < 0) return false;
    rest = rest.slice(close + COMMENT_CLOSE.length).trim();
  }
  // Whatever is left is real content, and the whole paragraph stays with it.
  return rest === '';
}

// A fenced code block opener: up to three spaces of indent, then three or more
// backticks or tildes, then an optional info string. A backtick fence's info
// string may not contain a backtick, which is what keeps `a ``b`` c` from being
// read as a fence.
const FENCE_OPEN = /^([ \t]{0,3})(`{3,}|~{3,})[ \t]*(.*)$/;

function fenceOpener(line) {
  const match = FENCE_OPEN.exec(line);
  if (!match) return null;
  const marker = match[2];
  const info = match[3].trim();
  if (marker.startsWith('`') && info.includes('`')) return null;
  return { indent: match[1].length, marker, info };
}

function isFenceCloser(line, marker) {
  const match = /^[ \t]{0,3}(`{3,}|~{3,})[ \t]*$/.exec(line);
  return Boolean(match) && match[1][0] === marker[0] && match[1].length >= marker.length;
}

function stripFenceIndent(line, indent) {
  let cut = 0;
  while (cut < indent && (line[cut] === ' ' || line[cut] === '\t')) cut += 1;
  return line.slice(cut);
}

/**
 * Render a fenced block as `<pre><code>`.
 *
 * The content is only escaped, never passed through `inline`: inside a code
 * block a backtick is a backtick. Nothing else is touched either — trailing
 * spaces and blank lines before the closing fence are part of the sample, and
 * an "escape-only" conversion that quietly trimmed them would not round-trip.
 * `white-space:pre-wrap` in book.css is what makes long lines wrap — an
 * e-reader page cannot scroll sideways, so an unwrapped line would be cut off.
 */
function codeBlockHtml(bodyLines, info) {
  const language = /^[A-Za-z0-9_+#-]+/.exec(info);
  const attribute = language ? ` class="language-${escapeHtml(language[0].toLowerCase())}"` : '';
  return `<pre><code${attribute}>${escapeHtml(bodyLines.join('\n'))}</code></pre>`;
}

function mediaType(file) {
  const ext = path.extname(file).toLowerCase();
  if (ext === '.css') return 'text/css';
  if (ext === '.svg') return 'image/svg+xml';
  if (ext === '.png') return 'image/png';
  if (ext === '.jpg' || ext === '.jpeg') return 'image/jpeg';
  if (ext === '.webp') return 'image/webp';
  if (ext === '.xhtml') return 'application/xhtml+xml';
  throw new Error(`Unsupported EPUB asset type: ${file}`);
}

function resolveBookPath(fromFile, ref) {
  if (/^[a-z]+:\/\//i.test(ref) || ref.startsWith('file://') || /^[A-Za-z]:[\\/]/.test(ref)) {
    throw new Error(`EPUB asset reference must be relative: ${ref}`);
  }
  const resolved = path.resolve(path.dirname(fromFile), ref);
  if (!resolved.startsWith(root + path.sep)) {
    throw new Error(`EPUB asset escapes book root: ${ref}`);
  }
  if (!fs.existsSync(resolved)) {
    throw new Error(`Missing EPUB asset: ${ref}`);
  }
  return resolved;
}

/**
 * Render one Markdown file, returning both its body and its first real heading.
 *
 * The title comes from here rather than from a second regex pass over the raw
 * file: a fenced sample containing `# code heading` is not a heading, and a
 * separate scan would name the chapter — and the navigation entry — after it.
 */
function markdownTextToBody(
  text,
  file,
  imageMap,
  trustedBlocks = new Map(),
  semanticNonce = '',
) {
  const out = [];
  let title = null;
  let para = [];
  const flush = () => {
    if (!para.length) return;
    const text = para.join(' ').trim();
    para = [];
    // Fences are emitted below without passing through here, so a code sample
    // that happens to be one comment is still rendered in full.
    if (!isCommentOnly(text)) out.push(`<p>${inline(text, semanticNonce)}</p>`);
  };

  const lines = text.split('\n');
  for (let i = 0; i < lines.length; i += 1) {
    const raw = lines[i];
    const line = raw.trimEnd();
    // Fences are matched before every other rule, and their contents before
    // none: inside a block, `# x` is a comment and `1. x` is an argument list,
    // not a heading and a list item. Letting the line rules below see them used
    // to put stray <h1>s into the book and, through them, into the navigation.
    const fence = fenceOpener(line);
    if (fence) {
      flush();
      const body = [];
      let end = i + 1;
      // An unclosed fence runs to the end of the document, as CommonMark says.
      while (end < lines.length && !isFenceCloser(lines[end].trimEnd(), fence.marker)) {
        // Only the opener's permitted indentation comes off; the rest of the
        // line is content, trailing spaces included.
        body.push(stripFenceIndent(lines[end], fence.indent));
        end += 1;
      }
      out.push(codeBlockHtml(body, fence.info));
      i = end;
      continue;
    }
    // A comment that opens a line and does not close on it runs on, and every
    // line it spans is part of it: `# x` inside a comment is no more a heading
    // than it is inside a fence. Without this the rules below saw those lines,
    // so `<!--\n# hidden\n-->` reached the reader as a visible `<!--`, a real
    // `<h1>hidden</h1>` and a visible `-->` — and never got as far as
    // `isCommentOnly`, which is what the bilingual builder drops it by.
    //
    // The lines join the paragraph buffer rather than being emitted on their
    // own, so a run-on comment sitting inside real prose is still that
    // paragraph's content, exactly as a single-line one is.
    if (line.trim().startsWith(COMMENT_OPEN) && opensUnclosedComment(line)) {
      let end = i + 1;
      while (end < lines.length && !lines[end].includes(COMMENT_CLOSE)) end += 1;
      // Only a comment that actually closes is taken whole. An unclosed fence
      // runs to the end of the document because CommonMark says so; nothing
      // says that of a stray `<!--`, and swallowing the rest of the chapter
      // would collapse every remaining paragraph into one over a typo. Falling
      // through leaves it exactly as it was before this rule existed.
      if (end < lines.length) {
        for (let scan = i; scan <= end; scan += 1) {
          // A blank line inside a comment is the comment's, not a paragraph
          // break; it contributes nothing, so it is not buffered either.
          if (lines[scan].trim()) para.push(lines[scan].trim());
        }
        i = end;
        continue;
      }
    }
    if (!line.trim()) {
      flush();
      continue;
    }
    const trustedBlock = trustedBlocks.get(line.trim());
    if (trustedBlock) {
      flush();
      out.push(trustedBlock);
      continue;
    }
    if (isUntrustedHtmlBlockLine(line)) {
      flush();
      out.push(`<p>${inline(line.trim(), semanticNonce)}</p>`);
      continue;
    }
    const table = parseMarkdownTable(lines, i);
    if (table) {
      flush();
      out.push(tableHtml(table.rows, semanticNonce));
      i = table.next - 1;
      continue;
    }
    const image = /^!\[([^\]]*)\]\(([^)]+)\)$/.exec(line.trim());
    if (image) {
      flush();
      const src = resolveBookPath(file, image[2]);
      const copied = copyAsset(src, 'images');
      imageMap.set(copied.href, copied);
      out.push(`<figure><img src="${copied.href}" alt="${escapeHtml(image[1])}" /><figcaption>${inline(image[1], semanticNonce)}</figcaption></figure>`);
      continue;
    }
    const heading = /^(#{1,6})\s+(.+)$/.exec(line);
    if (heading) {
      flush();
      const level = heading[1].length;
      if (title === null && heading[1].length === 1) title = heading[2].trim();
      out.push(`<h${level}>${inline(heading[2].trim(), semanticNonce)}</h${level}>`);
      continue;
    }
    const ordered = /^\d+\.\s+(.+)$/.exec(line.trim());
    if (ordered) {
      flush();
      out.push(`<p class="list-item">${inline(ordered[1], semanticNonce)}</p>`);
      continue;
    }
    para.push(line.trim());
  }
  flush();
  return { body: out.join('\n'), title };
}

function xhtml(title, body, language) {
  return `<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="${escapeHtml(language)}" lang="${escapeHtml(language)}">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${escapeHtml(title)}</title>
  <link rel="stylesheet" type="text/css" href="styles/book.css" />
</head>
<body>
${body}
</body>
</html>
`;
}

function copyAsset(src, folder) {
  const content = fs.readFileSync(src);
  const digest = createHash('sha256').update(content).digest('hex');
  const targetName = `${digest}${path.extname(src).toLowerCase()}`;
  const rel = `${folder}/${targetName}`;
  const target = path.join(workDir, 'EPUB', rel);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  if (!fs.existsSync(target)) fs.writeFileSync(target, content);
  return { href: rel, mediaType: mediaType(src) };
}

function configuredCover(metadata) {
  const configured = metadata.cover || metadata.cover_path || '';
  const candidates = [];
  if (configured) candidates.push(path.isAbsolute(configured) ? configured : path.resolve(root, configured));
  for (const extension of ['jpg', 'jpeg', 'png', 'webp', 'svg']) {
    candidates.push(path.join(root, 'source', `cover.${extension}`));
  }
  const cover = candidates.find((candidate) => fs.existsSync(candidate) && fs.statSync(candidate).isFile());
  if (!cover) return null;
  const resolved = path.resolve(cover);
  if (!resolved.startsWith(root + path.sep)) throw new Error('Configured cover escapes the book project.');
  mediaType(resolved);
  return resolved;
}

function cleanWorkDir() {
  fs.rmSync(workDir, { recursive: true, force: true });
  fs.mkdirSync(path.join(workDir, 'META-INF'), { recursive: true });
  fs.mkdirSync(path.join(workDir, 'EPUB', 'styles'), { recursive: true });
  fs.mkdirSync(path.join(workDir, 'EPUB', 'images'), { recursive: true });
}

function writeContainer() {
  writeText(path.join(workDir, 'mimetype'), 'application/epub+zip');
  writeText(path.join(workDir, 'META-INF', 'container.xml'), `<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml" />
  </rootfiles>
</container>
`);
}

function writeCss() {
  writeText(path.join(workDir, 'EPUB', 'styles', 'book.css'), `html{margin:0;padding:0}
body{line-height:1.68;margin:0;padding:1em;overflow-wrap:anywhere;word-break:normal}
main{display:block;max-width:42em;margin:0 auto}
.publication-cover{max-width:none;padding:0;text-align:center}.publication-cover img{display:block;width:auto;max-width:100%;max-height:95vh;margin:0 auto;object-fit:contain}
p{margin:0;text-indent:2em}
h1,h2,h3,h4,h5,h6{line-height:1.3;text-indent:0;break-after:avoid-page;page-break-after:avoid;margin-left:0;margin-right:0}
h1{font-size:1.7em;margin:1.8em 0 1.2em;text-align:center}
h2{font-size:1.35em;margin:1.65em 0 .85em}
h3{font-size:1.16em;margin:1.4em 0 .65em}
h4{font-size:1.06em;margin:1.2em 0 .55em}
h5,h6{font-size:1em;margin:1.05em 0 .45em}
h1+p,h2+p,h3+p,h4+p,h5+p,h6+p,blockquote+p,figure+p,.table-wrap+p{ text-indent:0 }
.publication-frontmatter p,.publication-backmatter p{ text-indent:0;margin:0 0 .55em }
img{max-width:100%;height:auto}
figure{margin:1.2em 0;text-align:center;break-inside:avoid}
figcaption,caption{font-size:.88em;line-height:1.45;text-align:left}
code{font-family:monospace;overflow-wrap:anywhere}
pre{text-indent:0;margin:.9em 0;padding:.6em .7em;background:#f4f4f4;border:1px solid #e0e0e0;font-size:.82em;line-height:1.45;white-space:pre-wrap;overflow-wrap:anywhere;break-inside:avoid}
pre code{font-size:inherit}.list-item{text-indent:0;margin-left:1.5em}
aside,[epub\\:type~="footnote"]{font-size:.88em;line-height:1.55;margin:.6em 0}
.table-wrap{display:block;width:100%;max-width:100%;margin:.8em 0 1.2em;overflow-x:auto}
table{border-collapse:collapse;width:100%;max-width:100%;font-size:.8em;line-height:1.4}
th,td{border:1px solid currentColor;padding:.25em .35em;vertical-align:top;white-space:normal;overflow-wrap:anywhere;word-break:break-word}
th{font-weight:600}tr{page-break-inside:avoid;break-inside:avoid}
a[epub\\:type~="noteref"]{font-size:.8em;vertical-align:super;text-decoration:none}
@media print{body{padding:0}main{max-width:none}h1{break-before:page;page-break-before:always}a{color:inherit;text-decoration:none}.table-wrap{overflow:visible}}
@media (max-width:430px){body{padding:.75em}h1{font-size:1.5em;margin-top:1.1em}h2{font-size:1.25em}.table-wrap{margin-left:0;margin-right:0}}
`);
}

function zipEpub() {
  fs.rmSync(epubPath, { force: true });
  const code = `
import pathlib, zipfile
root = pathlib.Path(${JSON.stringify(workDir)})
out = pathlib.Path(${JSON.stringify(epubPath)})
with zipfile.ZipFile(out, "w") as zf:
    zf.write(root / "mimetype", "mimetype", compress_type=zipfile.ZIP_STORED)
    for path in sorted(root.rglob("*")):
        if path.is_file() and path.name != "mimetype":
            zf.write(path, path.relative_to(root).as_posix(), compress_type=zipfile.ZIP_DEFLATED)
`;
  const result = spawnSync(process.execPath, [
    '--jitless',
    path.join(__dirname, 'run_python.cjs'),
    '-c',
    code,
  ], { encoding: 'utf8' });
  if (result.status !== 0) {
    process.stderr.write(result.stderr || result.stdout);
    process.exit(result.status || 1);
  }
}

function markdownHeadings(text) {
  const headings = [];
  let activeFence = null;
  const lines = text.split('\n');
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index].trimEnd();
    if (activeFence) {
      if (isFenceCloser(line, activeFence)) activeFence = null;
      continue;
    }
    if (line.trim().startsWith(COMMENT_OPEN) && opensUnclosedComment(line)) {
      const closeAt = lines.findIndex((candidate, candidateIndex) => (
        candidateIndex > index && candidate.includes(COMMENT_CLOSE)
      ));
      if (closeAt >= 0) {
        index = closeAt;
        continue;
      }
    } else if (isCommentOnly(line)) {
      continue;
    }
    const fence = fenceOpener(line);
    if (fence) {
      activeFence = fence.marker;
      continue;
    }
    const match = /^(#{1,6})\s+(.+)$/.exec(line);
    if (match) headings.push({ level: match[1].length, title: match[2].trim() });
  }
  return headings;
}

function isInternalReaderTitle(title) {
  return /^(?:(?:chapter|unit|section)[-_ ]*\d+|continuation(?:\s+\d+)?)$/i.test(title.trim());
}

function prepareSemanticNotes(text, notes, currentRootId, semanticNonce) {
  const trustedBlocks = new Map();
  if (!notes.length) return { text, trustedBlocks };
  const scopedNotes = notes.filter((note) => (
    note.definitionRootId === currentRootId
      || Object.values(note.referenceRootById || {}).includes(currentRootId)
  ));
  const byLabel = new Map(scopedNotes.map((note, index) => [note.sourceLabel, {
    ...note,
    marker: note.ordinal || index + 1,
    currentReferenceIds: note.referenceIds.filter(
      (referenceId) => note.referenceRootById[referenceId] === currentRootId,
    ),
  }]));
  const definitions = new Map();
  const seen = new Map();
  const kept = [];
  const lines = text.split('\n');
  for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
    const line = lines[lineIndex];
    const definition = /^\[\^([^\]]+)\]:\s*(.*)$/.exec(line.trim());
    if (definition && byLabel.get(definition[1])?.definitionRootId === currentRootId) {
      const body = [definition[2]];
      while (lineIndex + 1 < lines.length) {
        const continuation = lines[lineIndex + 1];
        if (/^(?: {4}|\t)/.test(continuation)) {
          body.push(continuation.replace(/^(?: {4}|\t)/, ''));
          lineIndex += 1;
        } else if (!continuation.trim()
          && lineIndex + 2 < lines.length
          && /^(?: {4}|\t)/.test(lines[lineIndex + 2])) {
          body.push('');
          lineIndex += 1;
        } else {
          break;
        }
      }
      definitions.set(definition[1], body.filter((part) => part.trim()).join(' '));
      continue;
    }
    kept.push(line.replace(/\[\^([^\]]+)\]/g, (match, label) => {
      const note = byLabel.get(label);
      if (!note) return match;
      const occurrence = (seen.get(label) || 0);
      const referenceId = note.currentReferenceIds[occurrence];
      if (!referenceId) throw new Error(`Note ${note.id} has more references than its publication contract.`);
      seen.set(label, occurrence + 1);
      return `@@BIBLIO_NOTEREF__${semanticNonce}__${note.id}__${referenceId}__${note.marker}__${note.definitionRootId}@@`;
    }));
  }
  for (const [label, note] of byLabel) {
    const referenceCount = seen.get(label) || 0;
    if (referenceCount !== note.currentReferenceIds.length) {
      throw new Error(`Semantic note reference count changed for ${note.id}: expected ${note.currentReferenceIds.length}, got ${referenceCount}`);
    }
    if (note.definitionRootId !== currentRootId) continue;
    if (!definitions.has(label)) {
      throw new Error(`Semantic note definition is missing: ${note.id}`);
    }
    const backlinks = note.referenceIds.map((referenceId, index) => (
      `<a epub:type="backlink" href="${note.referenceRootById[referenceId]}.xhtml#${referenceId}" aria-label="返回注号 ${index + 1}">↩${note.referenceIds.length > 1 ? index + 1 : ''}</a>`
    )).join(' ');
    const epubType = note.kind === 'endnote' ? 'endnote' : 'footnote';
    const noteKind = escapeHtml(note.kind || 'footnote');
    const block = `<aside epub:type="${epubType}" class="publication-note note-${noteKind}" data-note-kind="${noteKind}" id="${escapeHtml(note.id)}"><p><span class="note-marker">[${note.marker}]</span> ${inline(definitions.get(label), semanticNonce)} ${backlinks}</p></aside>`;
    const token = `@@BIBLIO_NOTE_BLOCK__${semanticNonce}__${note.id}@@`;
    trustedBlocks.set(token, block);
    kept.push(token);
  }
  return { text: kept.join('\n'), trustedBlocks };
}

function sectionEpubType(kind) {
  return ({ title_page: 'titlepage', copyright: 'copyright-page', contents: 'toc', bibliography: 'bibliography', notes: 'endnotes', appendix: 'appendix' })[kind] || '';
}

function anchorSectionHeadings(body, sections, headings) {
  if (headings.length !== sections.length) {
    throw new Error(`Publication section/headings mismatch: sections=${sections.length}, translatedHeadings=${headings.length}`);
  }
  let index = 0;
  const anchored = body.replace(/<h([1-6])>([\s\S]*?)<\/h\1>/g, (match, level) => {
    if (index >= sections.length) return match;
    const section = sections[index];
    const heading = headings[index];
    if (Number(level) !== section.headingLevel || heading.level !== section.headingLevel) {
      throw new Error(`Heading hierarchy changed for ${section.id}: expected h${section.headingLevel}, got h${level}`);
    }
    index += 1;
    const epubType = sectionEpubType(section.kind);
    const semantics = epubType ? ` epub:type="${epubType}"` : '';
    return `<h${level} id="${escapeHtml(section.id)}" class="publication-heading publication-kind-${escapeHtml(section.kind)} publication-role-${escapeHtml(section.role)}"${semantics}>${escapeHtml(section.title)}</h${level}>`;
  });
  if (index !== sections.length) throw new Error('Not every publication section received an XHTML anchor.');
  return anchored;
}

function navList(navigation, parentId) {
  const children = navigation.filter((entry) => (entry.parentId || null) === parentId);
  return children.map((entry) => {
    const nested = navList(navigation, entry.id);
    return `<li><a href="${escapeHtml(entry.href)}">${escapeHtml(entry.label)}</a>${nested ? `<ol>${nested}</ol>` : ''}</li>`;
  }).join('\n');
}

function main() {
  const structure = compilePublicationStructure(root);
  const sections = structure.sections;
  const units = structure.translationUnits;
  const publicationNotes = structure.notes;
  const byId = new Map(sections.map((section) => [section.id, section]));
  const roots = structure.roots;
  const documents = structure.documents;
  const semanticNonce = randomUUID().replace(/-/g, '');

  const finalFiles = new Map(listFiles(finalDir, '.md').map((file) => [path.basename(file, '.md'), file]));
  if (!finalFiles.size) throw new Error('No promoted translation units found under chapters/final.');
  const metadata = parseYaml(path.join(root, 'metadata', 'book.yaml'));
  const sourceManifest = readJson(path.join(root, 'metadata', 'source_manifest.json'));
  const manifestLanguage = typeof sourceManifest.target_language === 'string'
    ? sourceManifest.target_language.trim() : '';
  const language = metadata.language || manifestLanguage;
  if (!language || ['auto', 'unknown', 'und'].includes(language.toLowerCase())) {
    throw new Error('A concrete target language is required for build_reading.');
  }
  const title = metadata.title || metadata.title_zh || metadata.title_zh_hans || roots[0].title;
  if (!title || /^chapter_[0-9]+$/i.test(title) || /^unit_[0-9]+$/i.test(title)) {
    throw new Error('A semantic book title is required; source filenames and internal IDs are forbidden.');
  }
  const creator = metadata.author || metadata.creator || '';
  const contributor = metadata.contributor || '';
  const publisher = metadata.publisher || '';
  const source = metadata.source_url || metadata.source || metadata.source_text_url || '';
  const description = metadata.description || metadata.subtitle || '';
  const rights = metadata.rights || '';
  const date = metadata.date || '';
  const identifier = metadata.identifier || `urn:uuid:${randomUUID()}`;
  const coverPath = configuredCover(metadata);
  const imageMap = new Map();

  cleanWorkDir();
  writeContainer();
  writeCss();
  const manifestItems = [
    '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav" />',
    '<item id="css" href="styles/book.css" media-type="text/css" />',
  ];
  const spine = [];
  const titleBySection = new Map(sections.map((section) => [section.id, section.title]));
  const usedUnits = new Set();
  let coverLandmark = '';
  let coverMetadata = '';
  let coverAsset = null;
  if (coverPath) {
    coverAsset = copyAsset(coverPath, 'images');
    writeText(
      path.join(workDir, 'EPUB', 'cover.xhtml'),
      xhtml(title, `<main epub:type="cover" class="publication-cover"><img src="${escapeHtml(coverAsset.href)}" alt="${escapeHtml(title)}" /></main>`, language),
    );
    manifestItems.push('<item id="cover-page" href="cover.xhtml" media-type="application/xhtml+xml" />');
    manifestItems.push(`<item id="cover-image" href="${coverAsset.href}" media-type="${coverAsset.mediaType}" properties="cover-image" />`);
    spine.push('<itemref idref="cover-page" />');
    coverLandmark = '<li><a epub:type="cover" href="cover.xhtml">封面</a></li>';
    coverMetadata = '<meta name="cover" content="cover-image" />';
  }

  documents.forEach((document, index) => {
    const rootSection = byId.get(document.id);
    const subtree = document.sectionIds.map((sectionId) => byId.get(sectionId));
    const unitIds = new Set(document.translationUnitIds);
    const rootUnits = units.filter((unit) => unitIds.has(unit.id));
    if (!rootUnits.length) throw new Error(`Publication root has no translated content: ${rootSection.id}`);
    const texts = rootUnits.map((unit) => {
      const file = finalFiles.get(unit.id);
      if (!file) throw new Error(`Promoted final translation unit is missing: ${unit.id}`);
      usedUnits.add(unit.id);
      return readText(file);
    });
    const prepared = prepareSemanticNotes(
      texts.join('\n'),
      publicationNotes,
      rootSection.id,
      semanticNonce,
    );
    const headings = markdownHeadings(prepared.text);
    if (headings.length !== subtree.length) {
      throw new Error(`Publication section/headings mismatch: sections=${subtree.length}, translatedHeadings=${headings.length}`);
    }
    headings.forEach((heading) => {
      if (isInternalReaderTitle(heading.title)) {
        throw new Error(`Translated publication title exposes an internal unit: ${heading.title}`);
      }
    });
    const firstFile = finalFiles.get(rootUnits[0].id);
    const rendered = markdownTextToBody(
      prepared.text,
      firstFile,
      imageMap,
      prepared.trustedBlocks,
      semanticNonce,
    );
    const body = anchorSectionHeadings(rendered.body, subtree, headings);
    const href = document.href;
    const bodyType = ['frontmatter', 'bodymatter', 'backmatter'].includes(rootSection.role)
      ? rootSection.role : 'bodymatter';
    const wrapped = `<main epub:type="${bodyType}" class="publication-${bodyType} publication-kind-${escapeHtml(rootSection.kind)}">\n${body}\n</main>`;
    writeText(path.join(workDir, 'EPUB', href), xhtml(titleBySection.get(rootSection.id), wrapped, language));
    const idref = `section-doc-${index + 1}`;
    manifestItems.push(`<item id="${idref}" href="${href}" media-type="application/xhtml+xml" />`);
    spine.push(`<itemref idref="${idref}" />`);
  });

  const unusedFinal = [...finalFiles.keys()].filter((id) => !usedUnits.has(id));
  if (unusedFinal.length) throw new Error(`Final translation units are absent from source_map: ${unusedFinal.join(', ')}`);
  for (const asset of imageMap.values()) {
    if (coverAsset?.href === asset.href) continue;
    manifestItems.push(`<item id="asset-${manifestItems.length}" href="${asset.href}" media-type="${asset.mediaType}" />`);
  }
  const toc = navList(structure.navigation, null);
  const landmarks = structure.landmarks.map((landmark) => `<li><a epub:type="${landmark.role}" href="${landmark.href}">${landmark.role === 'bodymatter' ? '正文' : landmark.role === 'frontmatter' ? '书前' : '书后'}</a></li>`).join('\n');
  if (!roots.some((section) => section.role === 'bodymatter')) {
    throw new Error('Publication map has no bodymatter root.');
  }
  writeText(path.join(workDir, 'EPUB', 'nav.xhtml'), `<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="${escapeHtml(language)}" lang="${escapeHtml(language)}">
<head><meta charset="utf-8" /><title>目录</title><link rel="stylesheet" type="text/css" href="styles/book.css" /></head>
<body><nav epub:type="toc" id="toc"><h1>目录</h1><ol>${toc}</ol></nav>
<nav epub:type="landmarks" id="landmarks" hidden="hidden"><h2>导览</h2><ol>${coverLandmark}${landmarks}</ol></nav></body>
</html>
`);

  const modified = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z');
  writeText(path.join(workDir, 'EPUB', 'package.opf'), `<?xml version="1.0" encoding="utf-8"?>
<package version="3.0" unique-identifier="bookid" xmlns="http://www.idpf.org/2007/opf">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/">
    <dc:identifier id="bookid">${escapeHtml(identifier)}</dc:identifier>
    <dc:title>${escapeHtml(title)}</dc:title>
    ${creator ? `<dc:creator>${escapeHtml(creator)}</dc:creator>` : ''}
    ${contributor ? `<dc:contributor>${escapeHtml(contributor)}</dc:contributor>` : ''}
    ${publisher ? `<dc:publisher>${escapeHtml(publisher)}</dc:publisher>` : ''}
    <dc:language>${escapeHtml(language)}</dc:language>
    ${date ? `<dc:date>${escapeHtml(date)}</dc:date>` : ''}
    ${source ? `<dc:source>${escapeHtml(source)}</dc:source>` : ''}
    ${description ? `<dc:description>${escapeHtml(description)}</dc:description>` : ''}
    ${rights ? `<dc:rights>${escapeHtml(rights)}</dc:rights>` : ''}
    <meta property="dcterms:modified">${modified}</meta>
    ${coverMetadata}
  </metadata>
  <manifest>${manifestItems.join('\n    ')}</manifest>
  <spine>${spine.join('\n    ')}</spine>
</package>
`);
  fs.mkdirSync(readingDir, { recursive: true });
  fs.rmSync(htmlDir, { recursive: true, force: true });
  fs.cpSync(path.join(workDir, 'EPUB'), htmlDir, { recursive: true });
  zipEpub();
  console.log(`wrote ${path.relative(root, epubPath)}`);
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
