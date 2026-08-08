import { localizedReleaseNotes, type Locale } from "../../i18n";
import { formatFileSize, clampPercent, progressWidth } from "../../lib/format";
import type { LauncherUpdateController } from "../../updates";
import { updatesCopy } from "./updatesCopy";

/**
 * Presentational: the lifecycle lives in useLauncherUpdate so the startup check
 * and this card read the same state. Everything shown here comes from a check
 * that actually ran — there is no branch that reports "up to date" without one.
 */
export function UpdateSettingsPanel({
  locale,
  currentVersion,
  controller,
  runningJobCount,
}: {
  locale: Locale;
  currentVersion: string;
  controller: LauncherUpdateController;
  runningJobCount: number;
}) {
  const copy = updatesCopy(locale);
  const { state } = controller;
  const blocked = runningJobCount > 0;

  return (
    <section className="st-update" aria-label={copy.title}>
      <div className="st-row">
        <div className="st-row-copy">
          <strong>{copy.currentVersion}</strong>
          <span>{copy.description}</span>
        </div>
        <div className="st-update-version">
          <code className="st-update-current">{currentVersion}</code>
          <button
            className="st-btn"
            type="button"
            onClick={() => void controller.check()}
            disabled={state.kind === "checking" || state.kind === "downloading" || state.kind === "restarting"}
          >
            {state.kind === "checking" ? copy.checking : copy.check}
          </button>
        </div>
      </div>

      {state.kind === "upToDate" && (
        <p className="st-update-note">
          {copy.upToDate}
          {" · "}
          {copy.checkedAt(state.checkedAt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }))}
        </p>
      )}

      {state.kind === "error" && (
        <p className="st-update-note error" role="alert">
          {copy.checkFailed(state.message)}
        </p>
      )}

      {state.kind === "available" && (
        <div className="st-update-release">
          <div className="st-update-release-head">
            <strong>{copy.available(state.version)}</strong>
            {state.date && <span>{copy.releasedOn(state.date.slice(0, 10))}</span>}
          </div>
          <ReleaseNotes locale={locale} notes={state.notes} label={copy.releaseNotesLabel} />
          {blocked && (
            <p className="st-update-note warning" role="status">
              {copy.blockedByJobs(runningJobCount)}
            </p>
          )}
          <button
            className="st-btn primary"
            type="button"
            onClick={() => void controller.install()}
            disabled={blocked}
          >
            {copy.install}
          </button>
        </div>
      )}

      {state.kind === "downloading" && (
        <div className="st-update-release">
          <div className="st-update-release-head">
            <strong>{copy.downloading}</strong>
            <span>
              {state.totalBytes
                ? copy.downloadedOf(formatFileSize(state.downloadedBytes), formatFileSize(state.totalBytes))
                : copy.downloadedSoFar(formatFileSize(state.downloadedBytes))}
            </span>
          </div>
          {/* Indeterminate until the server announces a length: a bar computed
              from an unknown total would be a made-up number. */}
          {state.totalBytes ? (
            <div
              className="st-update-progress"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(clampPercent((state.downloadedBytes / state.totalBytes) * 100))}
            >
              <span style={{ width: progressWidth((state.downloadedBytes / state.totalBytes) * 100) }} />
            </div>
          ) : (
            <div className="st-update-progress indeterminate" role="progressbar" aria-valuetext={copy.downloading}>
              <span />
            </div>
          )}
        </div>
      )}

      {(state.kind === "installed" || state.kind === "restarting") && (
        <div className="st-update-release">
          <div className="st-update-release-head">
            <strong>{copy.readyTitle(state.version)}</strong>
          </div>
          <button
            className="st-btn primary"
            type="button"
            onClick={() => void controller.restart()}
            disabled={state.kind === "restarting"}
          >
            {state.kind === "restarting" ? copy.restarting : copy.restart}
          </button>
        </div>
      )}
    </section>
  );
}

function ReleaseNotes({ locale, notes, label }: { locale: Locale; notes: string; label: string }) {
  const text = localizedReleaseNotes(notes, locale, "");
  if (!text) return null;
  return (
    <div className="st-update-notes">
      <span className="st-update-notes-label">{label}</span>
      <p>{text}</p>
    </div>
  );
}
