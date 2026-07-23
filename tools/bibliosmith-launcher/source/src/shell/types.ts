import type { ProjectDocument } from "../types";

export type TabId = "overview" | "updates" | "pipeline" | "tutorial" | "settings" | "logs";
export type TutorialKind = "readme" | "howto";
export type TutorialHistoryEntry = { kind: TutorialKind; document: ProjectDocument };
export type ToastTone = "info" | "success" | "warning" | "error";
export type FloatingToast = { id: number; message: string; tone: ToastTone };
export type DownloadHudState = "idle" | "downloading" | "cancelling" | "stopped" | "failed";
export type RuntimeBootstrapState = "checking" | "preparing" | "ready" | "failed";
export type ConfirmDialogState = {
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel: string;
  resolve: (value: boolean) => void;
};
