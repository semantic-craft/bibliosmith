import type { LucideIcon } from "lucide-react";
import type { DownloadProgress } from "../types";
import type { Copy } from "../i18n";

/* Shared product action/version bundle built once in App and consumed by the
 * Overview product rows and the update-center version cards. */
export type ProductCardProps = {
  accent: "blue" | "green";
  icon: LucideIcon;
  title: string;
  subtitle: string;
  current: string;
  latest: string;
  status: string;
  statusTone: "success" | "warning" | "muted";
  latestUpdated: string;
  progress?: DownloadProgress | null;
  progressLabel?: string;
  primaryLabel: string;
  primaryIcon: LucideIcon;
  secondaryLabel: string;
  secondaryIcon: LucideIcon;
  secondaryTone?: "default" | "green" | "muted";
  secondaryDisabled?: boolean;
  secondaryBusy?: boolean;
  secondaryBusyText?: string;
  busy: boolean;
  busyText: string;
  onPrimary: () => void;
  onSecondary: () => void;
  onMore: () => void;
  moreLabel: string;
  moreBusy?: boolean;
  moreDisabled?: boolean;
  copy: Copy;
};
