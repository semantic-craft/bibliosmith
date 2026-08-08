import { useCallback, useEffect, useRef, useState } from "react";
import { checkLauncherUpdate, relaunchLauncher, type LauncherUpdate } from "../api";

/**
 * Every state the update card can be in. There is deliberately no "unknown"
 * that renders as "up to date": a check that never ran, and a check that
 * failed, are both distinguishable from a check that came back empty. The
 * launcher shipped a stub update check once before that always reported the
 * installed build as latest, and the rule since is that the interface may only
 * claim what it actually verified.
 */
export type LauncherUpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "upToDate"; checkedAt: Date }
  | { kind: "available"; version: string; notes: string; date: string | null }
  | { kind: "downloading"; version: string; downloadedBytes: number; totalBytes: number | null }
  | { kind: "installed"; version: string }
  | { kind: "restarting"; version: string }
  | { kind: "error"; message: string };

export type LauncherUpdateController = {
  state: LauncherUpdateState;
  /** True once a check found a release the installed build is behind. */
  updateAvailable: boolean;
  check: () => Promise<void>;
  install: () => Promise<void>;
  restart: () => Promise<void>;
};

function errorMessage(error: unknown) {
  if (error instanceof Error) return error.message;
  return String(error);
}

/**
 * Owns the update lifecycle for the whole app: App runs the silent startup
 * check and shows the toast, Settings renders the card and drives the buttons,
 * and both read one state so they cannot disagree about what was found.
 *
 * `runningJobCount` is not advisory. Installing unpacks a new App bundle over
 * the installed one, and the pipeline executes Python, Node and Chromium out of
 * that bundle's resources, so an install during a running job pulls the
 * interpreter out from under it. `install` refuses rather than trusting the
 * button to have been disabled.
 */
export function useLauncherUpdate(runningJobCount: number): LauncherUpdateController {
  const [state, setState] = useState<LauncherUpdateState>({ kind: "idle" });
  // The plugin's handle carries the bytes it downloaded; a second check() would
  // discard them and re-fetch, so the pending release is held here rather than
  // rebuilt from the rendered state.
  const pendingUpdate = useRef<LauncherUpdate | null>(null);
  // Mirrored into a ref so `install` stays referentially stable while still
  // reading the current count; it only ever runs from a click, long after the
  // effect below has caught the ref up with the render that changed it.
  const runningJobCountRef = useRef(runningJobCount);
  useEffect(() => {
    runningJobCountRef.current = runningJobCount;
  }, [runningJobCount]);

  const runCheck = useCallback(async (announceFailure: boolean) => {
    setState({ kind: "checking" });
    try {
      const update = await checkLauncherUpdate();
      pendingUpdate.current = update;
      if (!update) {
        setState({ kind: "upToDate", checkedAt: new Date() });
        return;
      }
      setState({
        kind: "available",
        version: update.version,
        notes: update.notes,
        date: update.date,
      });
    } catch (error) {
      pendingUpdate.current = null;
      // A failed startup check leaves the card in its idle state instead of
      // reporting an error nobody asked for. A failed manual check is an
      // answer to a button press and has to say so.
      setState(
        announceFailure
          ? { kind: "error", message: errorMessage(error) }
          : { kind: "idle" },
      );
    }
  }, []);

  const check = useCallback(() => runCheck(true), [runCheck]);

  // One silent check per launch. Deliberately not on a timer: a launcher that
  // polls in the background buys little for an app the user opens by hand.
  const startupCheckStarted = useRef(false);
  useEffect(() => {
    if (startupCheckStarted.current) return;
    startupCheckStarted.current = true;
    void runCheck(false);
  }, [runCheck]);

  const install = useCallback(async () => {
    const update = pendingUpdate.current;
    if (!update) return;
    if (runningJobCountRef.current > 0) return;
    setState({
      kind: "downloading",
      version: update.version,
      downloadedBytes: 0,
      totalBytes: null,
    });
    try {
      await update.downloadAndInstall((progress) => {
        setState({
          kind: "downloading",
          version: update.version,
          downloadedBytes: progress.downloadedBytes,
          totalBytes: progress.totalBytes,
        });
      });
      setState({ kind: "installed", version: update.version });
    } catch (error) {
      setState({ kind: "error", message: errorMessage(error) });
    }
  }, []);

  const restart = useCallback(async () => {
    const version = pendingUpdate.current?.version ?? "";
    setState({ kind: "restarting", version });
    try {
      await relaunchLauncher();
    } catch (error) {
      // The new bundle is already on disk, so this is only about who performs
      // the restart. Say that instead of implying the update was lost.
      setState({ kind: "error", message: errorMessage(error) });
    }
  }, []);

  return {
    state,
    updateAvailable: state.kind === "available",
    check,
    install,
    restart,
  };
}
