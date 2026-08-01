import type { FloatingToast } from "./types";

export function FloatingFeedback({ toast }: { toast: FloatingToast | null }) {
  if (!toast) return null;
  return (
    <div className="floating-feedback-layer" aria-live="polite">
      <div className={`floating-toast ${toast.tone}`}>{toast.message}</div>
    </div>
  );
}
