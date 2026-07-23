import type { ConfirmDialogState } from "./types";

export function ConfirmDialog({
  dialog,
  onCancel,
  onConfirm,
}: {
  dialog: ConfirmDialogState | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  if (!dialog) return null;
  return (
    <div className="modal-backdrop" role="presentation">
      <section className="confirm-dialog" role="dialog" aria-modal="true" aria-labelledby="confirm-dialog-title">
        <h2 id="confirm-dialog-title">{dialog.title}</h2>
        <p>{dialog.message}</p>
        <div className="confirm-actions">
          <button className="panel-button" type="button" autoFocus onClick={onCancel}>
            {dialog.cancelLabel}
          </button>
          <button className="panel-button primary-panel" type="button" onClick={onConfirm}>
            {dialog.confirmLabel}
          </button>
        </div>
      </section>
    </div>
  );
}
