// Launcher-owned builder copied into each local reading project at build time.
const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');
const { randomUUID } = require('crypto');

const root = path.resolve(__dirname, '..');
const finalDir = path.join(root, 'chapters', 'final');
const frontmatterDir = path.join(root, 'frontmatter');
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

function inline(text) {
  return escapeHtml(text).replace(/`([^`]+)`/g, '<code>$1</code>');
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

function tableHtml(rows) {
  const header = rows[0];
  const bodyRows = rows.slice(1);
  const thead = `<thead><tr>${header.map((cell) => `<th scope="col">${inline(cell)}</th>`).join('')}</tr></thead>`;
  const tbody = bodyRows.map((row) => {
    const padded = [...row, ...Array(Math.max(0, header.length - row.length)).fill('')].slice(0, header.length);
    return `<tr>${padded.map((cell) => `<td>${inline(cell)}</td>`).join('')}</tr>`;
  }).join('');
  return `<div class="table-wrap" role="region" aria-label="结构化表格"><table><caption>结构化表格</caption>${thead}<tbody>${tbody}</tbody></table></div>`;
}

function isRawHtmlLine(line) {
  return /^<\/?(section|p|aside|div|span|blockquote|ol|ul|li|dl|dt|dd|sup|a|em|strong|br)\b[^>]*>/.test(line.trim());
}

const COMMENT_OPEN = '<!--';
const COMMENT_CLOSE = '-->';

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

function slug(file) {
  return path.basename(file, path.extname(file)).replace(/[^A-Za-z0-9_-]+/g, '_');
}

function frontmatterRank(file) {
  const name = path.basename(file, path.extname(file)).toLowerCase();
  const ranks = {
    cover: 0,
    book_info: 1,
    'book-info': 1,
    translator_note: 2,
    'translator-note': 2,
    edition_note: 2,
    'edition-note': 2,
    preface: 3,
  };
  return Object.prototype.hasOwnProperty.call(ranks, name) ? ranks[name] : 10;
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
function markdownToBody(file, imageMap) {
  const out = [];
  let title = null;
  let para = [];
  const flush = () => {
    if (!para.length) return;
    const text = para.join(' ').trim();
    para = [];
    // Fences are emitted below without passing through here, so a code sample
    // that happens to be one comment is still rendered in full.
    if (!isCommentOnly(text)) out.push(`<p>${inline(text)}</p>`);
  };

  const lines = readText(file).split('\n');
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
    if (isRawHtmlLine(line)) {
      flush();
      out.push(line.trim());
      continue;
    }
    const table = parseMarkdownTable(lines, i);
    if (table) {
      flush();
      out.push(tableHtml(table.rows));
      i = table.next - 1;
      continue;
    }
    const image = /^!\[([^\]]*)\]\(([^)]+)\)$/.exec(line.trim());
    if (image) {
      flush();
      const src = resolveBookPath(file, image[2]);
      const copied = copyAsset(src, 'images');
      imageMap.set(copied.href, copied);
      out.push(`<figure><img src="${copied.href}" alt="${escapeHtml(image[1])}" /><figcaption>${inline(image[1])}</figcaption></figure>`);
      continue;
    }
    const heading = /^(#{1,6})\s+(.+)$/.exec(line);
    if (heading) {
      flush();
      const level = Math.min(heading[1].length, 3);
      if (title === null && heading[1].length === 1) title = heading[2].trim();
      out.push(`<h${level}>${inline(heading[2].trim())}</h${level}>`);
      continue;
    }
    const ordered = /^\d+\.\s+(.+)$/.exec(line.trim());
    if (ordered) {
      flush();
      out.push(`<p class="list-item">${inline(ordered[1])}</p>`);
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
  const targetName = path.basename(src);
  const rel = `${folder}/${targetName}`;
  const target = path.join(workDir, 'EPUB', rel);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.copyFileSync(src, target);
  return { href: rel, mediaType: mediaType(src) };
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
  writeText(path.join(workDir, 'EPUB', 'styles', 'book.css'), `body{line-height:1.72;margin:0;padding:1.2em;overflow-wrap:break-word}p{margin:0 0 .7em;text-indent:2em}h1{font-size:1.55em;line-height:1.25}h2{font-size:1.2em}h3{font-size:1.05em}img{max-width:100%;height:auto}figure{margin:1.2em 0;text-align:center;break-inside:avoid}figcaption{font-size:.88em;line-height:1.45}code{font-family:monospace;overflow-wrap:anywhere}pre{text-indent:0;margin:.9em 0;padding:.6em .7em;background:#f4f4f4;border:1px solid #e0e0e0;border-radius:3px;font-size:.82em;line-height:1.45;white-space:pre-wrap;overflow-wrap:anywhere;break-inside:avoid}pre code{font-size:inherit}.list-item{text-indent:0;margin-left:1.5em}.parallel-passage{margin:0 0 1.15em}.source-text{color:#4c3828}.modern-text{color:#1f1f1f}aside{font-size:.88em;line-height:1.55;margin:.25em 0 .75em 2em;color:#4b4b4b}.table-wrap{display:block;width:100%;max-width:100%;margin:.8em 0 1.2em;overflow:visible}table{border-collapse:collapse;width:100%;max-width:100%;table-layout:fixed;font-size:.74em;line-height:1.34;page-break-inside:auto;break-inside:auto}caption{font-size:.9em;line-height:1.35;margin:0 0 .35em;text-align:left}th,td{border:1px solid #777;padding:.22em .28em;vertical-align:top;white-space:normal;overflow-wrap:anywhere;word-break:break-word}th{font-weight:600;background:#f2f2f2}tr{page-break-inside:avoid;break-inside:avoid}`);
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
  const result = spawnSync(process.execPath, [path.join(__dirname, 'run_python.js'), '-c', code], { encoding: 'utf8' });
  if (result.status !== 0) {
    process.stderr.write(result.stderr || result.stdout);
    process.exit(result.status || 1);
  }
}

function main() {
  const chapters = listFiles(finalDir, '.md');
  if (!chapters.length) {
    console.error('No final chapters found under chapters/final. Build is blocked until chapter gates pass.');
    process.exit(1);
  }

  const metadata = parseYaml(path.join(root, 'metadata', 'book.yaml'));
  const sourceManifest = readJson(path.join(root, 'metadata', 'source_manifest.json'));
  const sourceFileName = typeof sourceManifest.source_file_name === 'string'
    ? sourceManifest.source_file_name.trim()
    : '';
  const projectTitle = path.basename(root).replace(/^\d+_/, '').replace(/_/g, ' ').trim();
  const title = metadata.title || metadata['title_zh'] || metadata['title_zh_hans']
    || (sourceFileName ? path.parse(sourceFileName).name : projectTitle) || path.basename(root);
  const creator = metadata.author || metadata.creator || '';
  const contributor = metadata.contributor || '';
  const publisher = metadata.publisher || '';
  const source = metadata.source_url || metadata.source || metadata.source_text_url || sourceFileName;
  const description = metadata.description || metadata.subtitle || '';
  const rights = metadata.rights || '';
  const date = metadata.date || '';
  const manifestLanguage = typeof sourceManifest.target_language === 'string'
    ? sourceManifest.target_language.trim()
    : '';
  const requestedLanguage = metadata.language || manifestLanguage;
  const language = requestedLanguage && !['auto', 'unknown'].includes(requestedLanguage.toLowerCase())
    ? requestedLanguage
    : 'und';
  const id = metadata.identifier || `urn:uuid:${randomUUID()}`;
  const imageMap = new Map();

  cleanWorkDir();
  writeContainer();
  writeCss();

  const spine = [];
  const manifestItems = [
    '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav" />',
    '<item id="css" href="styles/book.css" media-type="text/css" />',
  ];
  const navItems = [];

  const frontmatter = listFiles(frontmatterDir, '.md').sort((a, b) => {
    const rank = frontmatterRank(a) - frontmatterRank(b);
    return rank || path.basename(a).localeCompare(path.basename(b));
  });
  const allDocs = [...frontmatter, ...chapters];
  allDocs.forEach((file, index) => {
    const idref = `doc${index + 1}`;
    const href = `${slug(file)}.xhtml`;
    const { body, title } = markdownToBody(file, imageMap);
    const firstHeading = title ?? path.basename(file, '.md');
    writeText(path.join(workDir, 'EPUB', href), xhtml(firstHeading, body, language));
    manifestItems.push(`<item id="${idref}" href="${href}" media-type="application/xhtml+xml" />`);
    spine.push(`<itemref idref="${idref}" />`);
    navItems.push(`<li><a href="${href}">${escapeHtml(firstHeading)}</a></li>`);
  });

  for (const asset of imageMap.values()) {
    const idref = `asset-${manifestItems.length}`;
    const properties = asset.href === 'images/cover.jpg' ? ' properties="cover-image"' : '';
    manifestItems.push(`<item id="${idref}" href="${asset.href}" media-type="${asset.mediaType}"${properties} />`);
  }

  const coverHref = allDocs.find((file) => slug(file) === 'cover') ? 'cover.xhtml' : '';
  const bookInfoFile = allDocs.find((file) => ['book_info', 'book-info'].includes(slug(file)));
  const bookInfoHref = bookInfoFile ? `${slug(bookInfoFile)}.xhtml` : '';
  const firstChapterHref = `${slug(chapters[0])}.xhtml`;

  writeText(path.join(workDir, 'EPUB', 'nav.xhtml'), `<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="${escapeHtml(language)}" lang="${escapeHtml(language)}">
<head><meta charset="utf-8" /><title>目录</title><link rel="stylesheet" type="text/css" href="styles/book.css" /></head>
<body>
<nav epub:type="toc" id="toc"><h1>目录</h1><ol>${navItems.join('\n')}</ol></nav>
<nav epub:type="landmarks" id="landmarks" hidden="hidden"><h2>导览</h2><ol>
${coverHref ? `<li><a epub:type="cover" href="${coverHref}">封面</a></li>` : ''}
${bookInfoHref ? `<li><a epub:type="frontmatter" href="${bookInfoHref}">书籍信息</a></li>` : ''}
<li><a epub:type="bodymatter" href="${firstChapterHref}">正文</a></li>
</ol></nav>
</body>
</html>
`);

  const modified = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z');
  writeText(path.join(workDir, 'EPUB', 'package.opf'), `<?xml version="1.0" encoding="utf-8"?>
<package version="3.0" unique-identifier="bookid" xmlns="http://www.idpf.org/2007/opf">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/">
    <dc:identifier id="bookid">${escapeHtml(id)}</dc:identifier>
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
  </metadata>
  <manifest>
    ${manifestItems.join('\n    ')}
  </manifest>
  <spine>
    ${spine.join('\n    ')}
  </spine>
</package>
`);

  fs.mkdirSync(readingDir, { recursive: true });
  fs.rmSync(htmlDir, { recursive: true, force: true });
  fs.cpSync(path.join(workDir, 'EPUB'), htmlDir, { recursive: true });
  zipEpub();
  console.log(`wrote ${path.relative(root, epubPath)}`);
}

main();
