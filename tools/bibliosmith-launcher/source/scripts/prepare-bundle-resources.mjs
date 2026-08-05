import {
  chmodSync,
  closeSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDir, "../../../..");
const booksRoot = join(repositoryRoot, "books");
const epubcheckerRoot = join(booksRoot, "node_modules", "epubchecker");
const vendorRoot = join(epubcheckerRoot, "vendors");
const stagingRoot = join(scriptDir, "..", "src-tauri", "bundle-resources", "runtime");
const sidecarStagingRoot = join(scriptDir, "..", "src-tauri", "bundle-resources", "sidecars");
const legacyStagingRoot = join(scriptDir, "..", "src-tauri", "bundle-resources", "epubchecker");
const prepareLockPath = join(scriptDir, "..", "src-tauri", "bundle-resources", ".prepare.lock");
const nodeVersion = "22.23.2";
const nodeArchives = {
  "aarch64-apple-darwin": ["node-v22.23.2-darwin-arm64.tar.gz", "61130f394c1630d211dd50aecc4353d379480f36d3ac913cd85dbba1aed585c6", "node-v22.23.2-darwin-arm64/bin/node"],
  "x86_64-apple-darwin": ["node-v22.23.2-darwin-x64.tar.gz", "58e99022c2ff89395576cc7fd4d98cea24bb68081475d5f88b801ee8729fb026", "node-v22.23.2-darwin-x64/bin/node"],
  "aarch64-pc-windows-msvc": ["node-v22.23.2-win-arm64.zip", "fec025a6da31757e3b6af84c5a1628e9d38442ca99a2161091d78f2fcfa35ef3", "node-v22.23.2-win-arm64/node.exe"],
  "x86_64-pc-windows-msvc": ["node-v22.23.2-win-x64.zip", "1177b4137ba5adaa56354ae40f1080c7450e8ae09cecb47da459d1c52ac99f97", "node-v22.23.2-win-x64/node.exe"],
};
const uvVersion = "0.11.8";
const uvLicenses = [
  ["LICENSE-MIT", "860e3d7a86b84e6a7012c7a635fc64df475cebc6cce34dfeb73a5982ec58176c"],
  ["LICENSE-APACHE", "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4"],
];
const uvArchives = {
  "aarch64-apple-darwin": ["uv-aarch64-apple-darwin.tar.gz", "c729adb365114e844dd7f9316313a7ed6443b89bb5681d409eebac78b0bd06c8"],
  "x86_64-apple-darwin": ["uv-x86_64-apple-darwin.tar.gz", "c59d73bf34b58bc8e33a11629f7a255c11789fd00f03cd3e68ab2d1603645de9"],
  "aarch64-pc-windows-msvc": ["uv-aarch64-pc-windows-msvc.zip", "bb48716e74e4998993f15bc57a55e4d0d73ccbd27a66d7cbed37605f7c67d747"],
  "x86_64-pc-windows-msvc": ["uv-x86_64-pc-windows-msvc.zip", "c84629a56e0706b69a47ea35862208af827cb6fbfa1d0ca763c52c67594637e8"],
};

const runtimeScripts = ["build_bilingual_epub.py", "build_epub.cjs", "run_python.cjs"];
const runtimeResources = [
  "packages/translation-engine/pyproject.toml",
  "packages/translation-engine/src/translation_engine/__init__.py",
  "packages/translation-engine/src/translation_engine/__main__.py",
  "packages/translation-engine/src/translation_engine/checkpoint.py",
  "packages/translation-engine/src/translation_engine/chunking.py",
  "packages/translation-engine/src/translation_engine/cli.py",
  "packages/translation-engine/src/translation_engine/engine.py",
  "packages/translation-engine/src/translation_engine/files.py",
  "packages/translation-engine/src/translation_engine/glossary.py",
  "packages/translation-engine/src/translation_engine/ner.py",
  "packages/translation-engine/src/translation_engine/ner_cli.py",
  "packages/translation-engine/src/translation_engine/pipeline.py",
  "packages/translation-engine/src/translation_engine/placeholders.py",
  "packages/translation-engine/src/translation_engine/profiles.py",
  "packages/translation-engine/src/translation_engine/progress.py",
  "packages/translation-engine/src/translation_engine/prompts.py",
  "packages/translation-engine/src/translation_engine/providers.py",
  "packages/translation-engine/src/translation_engine/providers.toml",
  "packages/translation-engine/src/translation_engine/sample.py",
  "packages/translation-engine/src/translation_engine/sample_cli.py",
  "packages/translation-engine/src/translation_engine/sampling.py",
  "packages/layout-pdf/pyproject.toml",
  "packages/layout-pdf/src/layout_pdf/__init__.py",
  "packages/layout-pdf/src/layout_pdf/__main__.py",
  "packages/layout-pdf/src/layout_pdf/babeldoc_runner.py",
  "packages/layout-pdf/src/layout_pdf/cli.py",
  "packages/layout-pdf/src/layout_pdf/contract.py",
  "packages/layout-pdf/src/layout_pdf/progress.py",
  "packages/layout-pdf/src/layout_pdf/warnings.py",
  "packages/zotero-cli/pyproject.toml",
  "packages/zotero-cli/README.md",
  "packages/zotero-cli/src/zotero_cli/__init__.py",
  "packages/zotero-cli/src/zotero_cli/agent_contract.py",
  "packages/zotero-cli/src/zotero_cli/cli.py",
  "packages/zotero-cli/src/zotero_cli/embed.py",
  "packages/zotero-cli/src/zotero_cli/fulltext.py",
  "packages/zotero-cli/src/zotero_cli/mcp_server.py",
  "packages/zotero-cli/src/zotero_cli/root_env.py",
  "packages/zotero-cli/src/zotero_cli/search.py",
  "packages/zotero-cli/src/zotero_cli/vector_store.py",
  "packages/zotero-cli/src/zotero_cli/zfulltext_cli.py",
  "packages/zotero-cli/src/zotero_cli/zotero_api.py",
  "packages/zotero-cli/src/zotero_cli/zotero_db.py",
  "packages/digest/pyproject.toml",
  "packages/digest/README.md",
  "packages/digest/__init__.py",
  "packages/digest/bibliosmith_digest/__init__.py",
  "packages/digest/bibliosmith_digest/__main__.py",
  "packages/digest/bibliosmith_digest/core.py",
  "packages/digest/prompts/01_digest_generation.zh-CN.md",
  "packages/digest/prompts/02_digest_review.zh-CN.md",
  "packages/digest/qa/digest_review_checklist.zh-CN.md",
  "packages/digest/schemas/digest.config.schema.json",
  "packages/ocr/pyproject.toml",
  "packages/ocr/mineru.py",
  "packages/ocr/paddle.py",
  "packages/ocr/pdf_text.py",
  "packages/ocr/sample_compare.py",
  "packages/ocr/scripts/epub_to_markdown.py",
  "packages/ocr/scripts/pdf_to_html_paddleocr.py",
  "packages/ocr/scripts/progress.py",
  "packages/ocr/scripts/zotero_llm_worker.py",
];

function bundledJar() {
  try {
    for (const entry of readdirSync(vendorRoot)) {
      const candidate = join(vendorRoot, entry, "epubcheck.jar");
      if (statSync(candidate).isFile()) return candidate;
    }
  } catch {
    return null;
  }
  return null;
}

function processIsAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function acquirePrepareLock() {
  for (let attempt = 0; attempt < 6000; attempt += 1) {
    try {
      const descriptor = openSync(prepareLockPath, "wx");
      writeFileSync(descriptor, `${JSON.stringify({ pid: process.pid, createdAt: new Date().toISOString() })}\n`);
      return descriptor;
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      try {
        const owner = JSON.parse(readFileSync(prepareLockPath, "utf8"));
        if (!Number.isInteger(owner.pid) || !processIsAlive(owner.pid)) {
          rmSync(prepareLockPath, { force: true });
          continue;
        }
      } catch {
        const ageMs = Date.now() - statSync(prepareLockPath).mtimeMs;
        if (ageMs > 60 * 60 * 1000) {
          rmSync(prepareLockPath, { force: true });
          continue;
        }
      }
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
    }
  }
  throw new Error("Timed out waiting for the BiblioSmith bundle preparation lock.");
}

function copyRuntimeResource(relativePath) {
  const source = join(repositoryRoot, relativePath);
  const destination = join(stagingRoot, relativePath);
  if (!existsSync(source)) {
    throw new Error(`Required runtime resource is missing: ${relativePath}`);
  }
  mkdirSync(dirname(destination), { recursive: true });
  if (!statSync(source).isFile()) {
    throw new Error(`Runtime manifest entries must be regular files: ${relativePath}`);
  }
  copyFileAtomic(source, destination);
}

function copyFileAtomic(source, destination, mode) {
  mkdirSync(dirname(destination), { recursive: true });
  const temporary = `${destination}.tmp-${process.pid}`;
  rmSync(temporary, { force: true });
  copyFileSync(source, temporary);
  if (mode !== undefined) chmodSync(temporary, mode);
  renameSync(temporary, destination);
}

function writeFileAtomic(destination, value) {
  mkdirSync(dirname(destination), { recursive: true });
  const temporary = `${destination}.tmp-${process.pid}`;
  rmSync(temporary, { force: true });
  writeFileSync(temporary, value);
  renameSync(temporary, destination);
}

function rustHostTriple() {
  const rustc = process.platform === "win32" ? "rustc.exe" : "rustc";
  const result = spawnSync(rustc, ["-vV"], { encoding: "utf8" });
  const host = result.stdout?.match(/^host:\s*(\S+)$/m)?.[1];
  if (result.status !== 0 || !host) {
    throw new Error("Unable to determine the Rust host triple for App sidecars.");
  }
  return host;
}

function tauriTargetTriple(hostTriple) {
  const platform = process.env.TAURI_ENV_PLATFORM;
  const arch = process.env.TAURI_ENV_ARCH;
  if (!platform || !arch) return hostTriple;
  const normalizedArch = arch === "arm64" ? "aarch64" : arch;
  if (["macos", "darwin"].includes(platform)) return `${normalizedArch}-apple-darwin`;
  if (platform === "windows") return `${normalizedArch}-pc-windows-msvc`;
  throw new Error(`Unsupported BiblioSmith App sidecar target: ${platform}/${arch}`);
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

async function pinnedUvBinary(targetTriple) {
  const specification = uvArchives[targetTriple];
  if (!specification) {
    throw new Error(`No pinned uv ${uvVersion} archive is defined for ${targetTriple}.`);
  }
  const [archiveName, expectedSha256] = specification;
  const cacheRoot = join(tmpdir(), "bibliosmith-bundle-tools", `uv-${uvVersion}`, targetTriple);
  const archivePath = join(cacheRoot, archiveName);
  mkdirSync(cacheRoot, { recursive: true });
  if (!existsSync(archivePath) || sha256File(archivePath) !== expectedSha256) {
    rmSync(archivePath, { force: true });
    const url = `https://github.com/astral-sh/uv/releases/download/${uvVersion}/${archiveName}`;
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Unable to download pinned uv runtime: HTTP ${response.status}`);
    writeFileSync(archivePath, new Uint8Array(await response.arrayBuffer()));
  }
  const actualSha256 = sha256File(archivePath);
  if (actualSha256 !== expectedSha256) {
    throw new Error(`Pinned uv archive checksum mismatch: ${actualSha256}`);
  }
  const extractRoot = join(cacheRoot, "extracted");
  rmSync(extractRoot, { recursive: true, force: true });
  mkdirSync(extractRoot, { recursive: true });
  const tar = process.platform === "win32" ? "tar.exe" : "tar";
  const extraction = spawnSync(tar, ["-xf", archivePath, "-C", extractRoot], { encoding: "utf8" });
  if (extraction.status !== 0) {
    throw new Error(`Unable to extract pinned uv runtime: ${extraction.stderr || extraction.stdout}`);
  }
  const directoryName = archiveName.replace(/\.(?:tar\.gz|zip)$/, "");
  const executableName = process.platform === "win32" ? "uv.exe" : "uv";
  const binary = join(extractRoot, directoryName, executableName);
  if (!existsSync(binary) || !statSync(binary).isFile()) {
    throw new Error(`Pinned uv archive does not contain ${directoryName}/${executableName}.`);
  }
  return { binary, archiveName, archiveSha256: expectedSha256 };
}

async function pinnedUvLicense(fileName, expectedSha256) {
  const cacheRoot = join(tmpdir(), "bibliosmith-bundle-tools", `uv-${uvVersion}`, "licenses");
  const destination = join(cacheRoot, fileName);
  mkdirSync(cacheRoot, { recursive: true });
  if (!existsSync(destination) || sha256File(destination) !== expectedSha256) {
    rmSync(destination, { force: true });
    const url = `https://raw.githubusercontent.com/astral-sh/uv/${uvVersion}/${fileName}`;
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Unable to download pinned uv license: HTTP ${response.status}`);
    writeFileSync(destination, new Uint8Array(await response.arrayBuffer()));
  }
  const actualSha256 = sha256File(destination);
  if (actualSha256 !== expectedSha256) {
    throw new Error(`Pinned uv license checksum mismatch: ${actualSha256}`);
  }
  return destination;
}

async function pinnedNodeBinary(targetTriple) {
  const specification = nodeArchives[targetTriple];
  if (!specification) {
    throw new Error(`No pinned Node ${nodeVersion} archive is defined for ${targetTriple}.`);
  }
  const [archiveName, expectedSha256, binaryRelativePath] = specification;
  const cacheRoot = join(tmpdir(), "bibliosmith-bundle-tools", `node-${nodeVersion}`, targetTriple);
  const archivePath = join(cacheRoot, archiveName);
  mkdirSync(cacheRoot, { recursive: true });
  if (!existsSync(archivePath) || sha256File(archivePath) !== expectedSha256) {
    rmSync(archivePath, { force: true });
    const url = `https://nodejs.org/dist/v${nodeVersion}/${archiveName}`;
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Unable to download pinned Node runtime: HTTP ${response.status}`);
    writeFileSync(archivePath, new Uint8Array(await response.arrayBuffer()));
  }
  const actualSha256 = sha256File(archivePath);
  if (actualSha256 !== expectedSha256) {
    throw new Error(`Pinned Node archive checksum mismatch: ${actualSha256}`);
  }
  const extractRoot = join(cacheRoot, "extracted");
  rmSync(extractRoot, { recursive: true, force: true });
  mkdirSync(extractRoot, { recursive: true });
  const tar = process.platform === "win32" ? "tar.exe" : "tar";
  const extraction = spawnSync(tar, ["-xf", archivePath, "-C", extractRoot], { encoding: "utf8" });
  if (extraction.status !== 0) {
    throw new Error(`Unable to extract pinned Node runtime: ${extraction.stderr || extraction.stdout}`);
  }
  const binary = join(extractRoot, binaryRelativePath);
  if (!existsSync(binary) || !statSync(binary).isFile()) {
    throw new Error(`Pinned Node archive does not contain ${binaryRelativePath}.`);
  }
  const license = join(extractRoot, binaryRelativePath.split(/[\\/]/)[0], "LICENSE");
  if (!existsSync(license) || !statSync(license).isFile()) {
    throw new Error("Pinned Node archive does not contain its LICENSE file.");
  }
  return { binary, license, archiveName, archiveSha256: expectedSha256 };
}

function copyRuntimeSidecar(source, name, targetTriple) {
  const suffix = process.platform === "win32" ? ".exe" : "";
  const destination = join(sidecarStagingRoot, `${name}-${targetTriple}${suffix}`);
  const temporary = `${destination}.tmp-${process.pid}`;
  rmSync(temporary, { force: true });
  copyFileSync(source, temporary);
  if (process.platform !== "win32") chmodSync(temporary, 0o755);
  const probe = spawnSync(temporary, ["--version"], { encoding: "utf8" });
  if (probe.status !== 0) {
    throw new Error(`Bundled ${name} sidecar cannot run independently: ${probe.stderr || probe.stdout}`);
  }
  renameSync(temporary, destination);
  return destination;
}

function stagedFiles(directory, relative = "") {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const childRelative = relative ? join(relative, entry.name) : entry.name;
    if (entry.isDirectory()) {
      files.push(...stagedFiles(join(directory, entry.name), childRelative));
    } else {
      files.push(childRelative.replaceAll("\\", "/"));
    }
  }
  return files;
}

function pruneFilesOutsideManifest(directory, allowedFiles) {
  for (const relativePath of stagedFiles(directory)) {
    if (!allowedFiles.has(relativePath)) {
      rmSync(join(directory, relativePath), { force: true });
    }
  }
}

function expectedRuntimeFiles() {
  return new Set([
    ".gitignore",
    "pyproject.toml",
    "uv.lock",
    "bundle-input.json",
    "sidecar-manifest.json",
    "licenses/node/LICENSE",
    "licenses/uv/LICENSE-APACHE",
    "licenses/uv/LICENSE-MIT",
    ...runtimeResources,
    ...runtimeScripts.map((script) => `tools/bibliosmith-launcher/source/scripts/${script}`),
    ...stagedFiles(epubcheckerRoot).map((path) => `vendor/epubchecker/${path}`),
  ]);
}

function expectedSidecarFiles(targetTriple) {
  const suffix = process.platform === "win32" ? ".exe" : "";
  return new Set([".gitignore", `node-${targetTriple}${suffix}`, `uv-${targetTriple}${suffix}`]);
}

function verifyStagedInventory(targetTriple) {
  const files = stagedFiles(stagingRoot);
  const expected = expectedRuntimeFiles();
  const actual = new Set(files);
  const missing = [...expected].filter((path) => !actual.has(path));
  const extra = [...actual].filter((path) => !expected.has(path));
  if (missing.length > 0 || extra.length > 0) {
    throw new Error(`Bundle inventory mismatch; missing=[${missing.join(", ")}] extra=[${extra.join(", ")}]`);
  }
  const forbidden = files.filter((path) => {
    const segments = path.split("/");
    const name = segments.at(-1) ?? "";
    const lowerName = name.toLowerCase();
    return segments.some((segment) => [".state", "__pycache__", "tests"].includes(segment))
      || lowerName === ".env"
      || lowerName.startsWith(".env.")
      || ["credentials.json", "secrets.json", ".npmrc", ".pypirc"].includes(lowerName)
      || /\.(?:db|sqlite|sqlite3|log|pyc|pyo|pem|key|p12|pfx)$/i.test(name);
  });
  if (forbidden.length > 0) {
    throw new Error(`Private or mutable files entered App resources: ${forbidden.join(", ")}`);
  }
  const sidecars = new Set(stagedFiles(sidecarStagingRoot));
  const expectedSidecars = expectedSidecarFiles(targetTriple);
  const sidecarMissing = [...expectedSidecars].filter((path) => !sidecars.has(path));
  const sidecarExtra = [...sidecars].filter((path) => !expectedSidecars.has(path));
  if (sidecarMissing.length > 0 || sidecarExtra.length > 0) {
    throw new Error(`Sidecar inventory mismatch; missing=[${sidecarMissing.join(", ")}] extra=[${sidecarExtra.join(", ")}]`);
  }
  const manifest = JSON.parse(readFileSync(join(stagingRoot, "sidecar-manifest.json"), "utf8"));
  const suffix = process.platform === "win32" ? ".exe" : "";
  const node = join(sidecarStagingRoot, `node-${targetTriple}${suffix}`);
  const uv = join(sidecarStagingRoot, `uv-${targetTriple}${suffix}`);
  if (manifest.target !== targetTriple
    || manifest.node?.version !== nodeVersion
    || manifest.node?.sha256 !== sha256File(node)
    || manifest.uv?.version !== uvVersion
    || manifest.uv?.sha256 !== sha256File(uv)) {
    throw new Error("Staged sidecar manifest does not match the pinned executable files.");
  }
  for (const [name, path, version] of [["node", node, nodeVersion], ["uv", uv, uvVersion]]) {
    const probe = spawnSync(path, ["--version"], { encoding: "utf8" });
    if (probe.status !== 0 || !`${probe.stdout}${probe.stderr}`.includes(version)) {
      throw new Error(`Staged ${name} sidecar failed its pinned version probe.`);
    }
  }
}

function bundleInputFingerprint(jar, targetTriple) {
  const hash = createHash("sha256");
  hash.update(readFileSync(fileURLToPath(import.meta.url)));
  const inputs = [
    "pyproject.toml",
    "uv.lock",
    "tools/bibliosmith-launcher/source/src-tauri/tauri.conf.json",
    ...runtimeResources,
    ...runtimeScripts.map((script) => `tools/bibliosmith-launcher/source/scripts/${script}`),
  ];
  for (const relativePath of inputs) {
    hash.update(relativePath);
    hash.update(readFileSync(join(repositoryRoot, relativePath)));
  }
  for (const relativePath of stagedFiles(epubcheckerRoot).sort()) {
    hash.update(`epubchecker/${relativePath}`);
    hash.update(readFileSync(join(epubcheckerRoot, relativePath)));
  }
  hash.update(JSON.stringify({ nodeVersion, uvVersion, uvLicenses, targetTriple, jar: jar.split(/[\\/]/).at(-2) }));
  return hash.digest("hex");
}

const prepareLock = await acquirePrepareLock();
try {
  if (!bundledJar()) {
    const npm = process.platform === "win32" ? "npm.cmd" : "npm";
    const install = spawnSync(npm, ["ci", "--omit=dev"], {
      cwd: booksRoot,
      stdio: "inherit",
    });
    if (install.status !== 0) {
      throw new Error(
        `Unable to prepare EPUBCheck bundle resources (npm exit ${install.status ?? "unknown"}).`,
      );
    }
  }

  const jar = bundledJar();
  if (!jar) {
    throw new Error("EPUBCheck installed without its vendor jar; the App bundle would be incomplete.");
  }
  const hostTriple = rustHostTriple();
  const tauriTarget = tauriTargetTriple(hostTriple);
  if (tauriTarget !== hostTriple) {
    throw new Error(`Cross-target sidecar builds are not supported: host=${hostTriple} target=${tauriTarget}`);
  }
  const inputFingerprint = bundleInputFingerprint(jar, tauriTarget);
  const inputManifestPath = join(stagingRoot, "bundle-input.json");
  const suffix = process.platform === "win32" ? ".exe" : "";
  const stagedNodePath = join(sidecarStagingRoot, `node-${tauriTarget}${suffix}`);
  const stagedUvPath = join(sidecarStagingRoot, `uv-${tauriTarget}${suffix}`);
  let reusable = false;
  try {
    const current = JSON.parse(readFileSync(inputManifestPath, "utf8"));
    reusable = current.fingerprint === inputFingerprint
      && current.target === tauriTarget
      && existsSync(stagedNodePath)
      && existsSync(stagedUvPath);
    if (reusable) verifyStagedInventory(tauriTarget);
  } catch {
    reusable = false;
  }

  if (!reusable) {
    rmSync(legacyStagingRoot, { recursive: true, force: true });
    mkdirSync(stagingRoot, { recursive: true });
    mkdirSync(sidecarStagingRoot, { recursive: true });
    copyFileAtomic(join(repositoryRoot, "pyproject.toml"), join(stagingRoot, "pyproject.toml"));
    copyFileAtomic(join(repositoryRoot, "uv.lock"), join(stagingRoot, "uv.lock"));
    for (const resource of runtimeResources) copyRuntimeResource(resource);

    const uv = await pinnedUvBinary(tauriTarget);
    const node = await pinnedNodeBinary(tauriTarget);
    const licenses = await Promise.all(
      uvLicenses.map(([fileName, sha256]) => pinnedUvLicense(fileName, sha256)),
    );
    const stagedNode = copyRuntimeSidecar(node.binary, "node", tauriTarget);
    const stagedUv = copyRuntimeSidecar(uv.binary, "uv", tauriTarget);
    copyFileAtomic(node.license, join(stagingRoot, "licenses", "node", "LICENSE"));
    for (const license of licenses) {
      copyFileAtomic(license, join(stagingRoot, "licenses", "uv", license.split(/[\\/]/).at(-1)));
    }
    writeFileAtomic(join(stagingRoot, "sidecar-manifest.json"), `${JSON.stringify({
      schema: "bibliosmith-sidecars-v1",
      target: tauriTarget,
      node: { version: nodeVersion, sha256: sha256File(stagedNode), archive: node.archiveName, archiveSha256: node.archiveSha256 },
      uv: { version: uvVersion, sha256: sha256File(stagedUv), archive: uv.archiveName, archiveSha256: uv.archiveSha256 },
    }, null, 2)}\n`);

    const stagedScripts = join(stagingRoot, "tools", "bibliosmith-launcher", "source", "scripts");
    mkdirSync(stagedScripts, { recursive: true });
    for (const script of runtimeScripts) {
      copyFileAtomic(join(scriptDir, script), join(stagedScripts, script));
    }

    const stagedEpubchecker = join(stagingRoot, "vendor", "epubchecker");
    mkdirSync(stagedEpubchecker, { recursive: true });
    for (const relativePath of stagedFiles(epubcheckerRoot)) {
      copyFileAtomic(
        join(epubcheckerRoot, relativePath),
        join(stagedEpubchecker, relativePath),
      );
    }
    pruneFilesOutsideManifest(stagingRoot, expectedRuntimeFiles());
    pruneFilesOutsideManifest(sidecarStagingRoot, expectedSidecarFiles(tauriTarget));
    writeFileAtomic(inputManifestPath, `${JSON.stringify({
      schema: "bibliosmith-bundle-input-v1",
      target: tauriTarget,
      fingerprint: inputFingerprint,
    }, null, 2)}\n`);
    verifyStagedInventory(tauriTarget);
  }

  console.log(`${reusable ? "Reused" : "Prepared"} read-only BiblioSmith runtime resources with EPUBCheck: ${jar}`);
} finally {
  closeSync(prepareLock);
  rmSync(prepareLockPath, { force: true });
}
