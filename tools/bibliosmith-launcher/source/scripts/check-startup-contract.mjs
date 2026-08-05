import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function readText(relativePath) {
  return fs.readFileSync(path.join(root, relativePath), "utf8");
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function extractFunction(source, signature) {
  const start = source.indexOf(signature);
  assert(start >= 0, `Could not find function signature: ${signature}`);
  const braceStart = source.indexOf("{", start);
  assert(braceStart >= 0, `Could not find function body: ${signature}`);
  let depth = 0;
  for (let index = braceStart; index < source.length; index += 1) {
    const char = source[index];
    if (char === "{") depth += 1;
    if (char === "}") depth -= 1;
    if (depth === 0) return source.slice(start, index + 1);
  }
  throw new Error(`Could not extract function body: ${signature}`);
}

const capability = JSON.parse(readText("src-tauri/capabilities/default.json"));
const permissions = new Set(capability.permissions ?? []);
assert(
  permissions.has("core:event:default") || (
    permissions.has("core:event:allow-listen") &&
    permissions.has("core:event:allow-unlisten")
  ),
  "Launcher main window must allow Tauri event listen/unlisten for progress subscriptions.",
);

// Runtime preparation stays background-only, while the user-owned workspace
// is an explicit startup gate. The App must not mount the main launcher until
// that workspace has a valid marker and durable project container.
const appSource = readText("src/App.tsx");
assert(
  !appSource.includes("RuntimeBootstrapScreen"),
  "Launcher must not block the first screen on runtime bootstrap; preparation is background-only.",
);
assert(
  /if\s*\(!runtimePrepareStartedRef\.current\)\s*\{[\s\S]*?startRuntimePrepare\(\)/.test(appSource),
  "Launcher should keep optional runtime preparation in the background after initial UI startup.",
);
assert(
  appSource.includes("<WorkspaceSetupGate") && appSource.includes("<LauncherApp />"),
  "Launcher must gate the main UI on explicit user-workspace setup.",
);

const tauriSource = readText("src-tauri/src/lib.rs");
const runBlocking = extractFunction(tauriSource, "async fn run_blocking");
assert(
  runBlocking.includes("catch_unwind"),
  "Blocking background tasks must convert panics into errors instead of relying on task runtime behavior.",
);
const runtimePrepare = extractFunction(tauriSource, "fn start_runtime_prepare");
assert(
  runtimePrepare.includes("catch_unwind"),
  "Runtime preparation worker must convert panics into logged failure events.",
);
assert(
  tauriSource.includes("create_recommended_workspace")
    && tauriSource.includes("choose_and_create_workspace")
    && !tauriSource.includes("start_node_modules_install"),
  "Startup must create a user workspace without installing repository dependencies.",
);

console.log("startup contract ok");
