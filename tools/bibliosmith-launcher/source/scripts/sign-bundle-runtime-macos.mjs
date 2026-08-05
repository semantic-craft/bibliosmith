import {
  existsSync,
  readFileSync,
  readdirSync,
  renameSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultRuntimeRoot = resolve(
  scriptDir,
  "../src-tauri/bundle-resources/runtime",
);
const runtimeRoot = resolve(process.argv[2] ?? defaultRuntimeRoot);
const signingIdentity = String(process.env.APPLE_SIGNING_IDENTITY ?? "").trim();
const browserEntitlementsPath = join(scriptDir, "browser-runtime.entitlements.plist");

if (!signingIdentity) {
  console.log("Skipping bundled macOS runtime signing without APPLE_SIGNING_IDENTITY.");
  process.exit(0);
}
if (!existsSync(runtimeRoot) || !statSync(runtimeRoot).isDirectory()) {
  throw new Error(`Bundled runtime root does not exist: ${runtimeRoot}`);
}

function run(command, args) {
  const completed = spawnSync(command, args, { encoding: "utf8" });
  if (completed.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed: ${completed.stderr || completed.stdout}`,
    );
  }
  return `${completed.stdout}${completed.stderr}`;
}

function filesUnder(root) {
  const files = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const candidate = join(root, entry.name);
    if (entry.isDirectory()) files.push(...filesUnder(candidate));
    else if (entry.isFile()) files.push(candidate);
  }
  return files;
}

function isMachO(candidate) {
  return run("file", ["-b", candidate]).includes("Mach-O");
}

function sha256File(candidate) {
  return createHash("sha256").update(readFileSync(candidate)).digest("hex");
}

function containedPath(root, relativePath) {
  const candidate = resolve(root, relativePath);
  const boundary = `${root}${sep}`;
  if (candidate === root || !candidate.startsWith(boundary)) {
    throw new Error(`Bundled browser manifest escapes the runtime root: ${relativePath}`);
  }
  return candidate;
}

const browserManifestPath = join(
  runtimeRoot,
  "vendor/playwright-core/browser-manifest.json",
);
if (!existsSync(browserManifestPath)) {
  throw new Error(`Bundled browser manifest is missing: ${browserManifestPath}`);
}
const browserManifest = JSON.parse(readFileSync(browserManifestPath, "utf8"));
if (browserManifest.schema !== "bibliosmith-browser-runtime-v1") {
  throw new Error("Bundled browser manifest has an unsupported schema.");
}
const browserExecutable = containedPath(
  runtimeRoot,
  String(browserManifest.relativePath ?? ""),
);

const machOBinaries = filesUnder(runtimeRoot)
  .filter(isMachO)
  .sort((left, right) => right.split(sep).length - left.split(sep).length
    || left.localeCompare(right));

if (machOBinaries.length === 0) {
  throw new Error(`No Mach-O code found beneath bundled runtime root: ${runtimeRoot}`);
}

for (const candidate of machOBinaries) {
  const signingArguments = [
    "--force",
    "--options",
    "runtime",
    "--timestamp",
  ];
  if (candidate === browserExecutable) {
    signingArguments.push("--entitlements", browserEntitlementsPath);
  }
  signingArguments.push(
    "--sign",
    signingIdentity,
    candidate,
  );
  run("codesign", signingArguments);
  run("codesign", ["--verify", "--strict", "--verbose=2", candidate]);
}

if (!machOBinaries.includes(browserExecutable)) {
  throw new Error("Bundled browser executable was not identified as Mach-O code.");
}
browserManifest.sha256 = sha256File(browserExecutable);
const temporaryManifest = `${browserManifestPath}.tmp-${process.pid}`;
writeFileSync(temporaryManifest, `${JSON.stringify(browserManifest, null, 2)}\n`);
renameSync(temporaryManifest, browserManifestPath);

console.log(
  `Signed ${machOBinaries.length} bundled Mach-O files under ${relative(process.cwd(), runtimeRoot) || "."}.`,
);
