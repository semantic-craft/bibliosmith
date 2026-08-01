import type { PipelineCopy } from "../copy";
import type { BookUnit, PipelineBusy } from "../model";

export type TabProps = {
  unit: BookUnit;
  copy: PipelineCopy;
  busy: PipelineBusy;
  onRetry: (jobId: string) => void;
  onAdvance: (jobId: string, childId: string, invalidateDownstream?: boolean) => void;
  onApproveGate: (jobId: string, childId: string, stageId: "approve_translation" | "approve_promotion") => void;
  onOpenOutput: (jobId: string) => void;
  onHandoff: (jobId: string, artifactPath?: string | null) => void;
  onRouteOverride: (jobId: string, childId: string, routeItemId: string, routeOverride: string) => void;
  onRecordReaderEvidence: (
    jobId: string,
    childId: string,
    artifactKind: string,
    reader: string,
    readerVersion: string,
    conclusion: string,
  ) => void;
  onGoApproval: () => void;
};
