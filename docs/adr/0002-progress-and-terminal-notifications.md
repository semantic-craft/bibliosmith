# ADR 0002: Progress UI and terminal notifications

- Status: Accepted
- Date: 2026-07-17
- Decision ticket: #51

## Context

The repository already has a Tauri desktop shell and a public, versioned Book
Pipeline state. Adding a separate web UI would duplicate the control surface.
SRT/DOCX input, TTS output, notifications, and progress also have different
coupling and privacy risks, so they should not enter v1 as one undifferentiated
extension bundle.

## Decision

The v1 scope is:

1. **Real-time stage/unit progress in the Tauri launcher.** Public job state
   exposes aggregate stage counts, percent, active stage ID, and the active
   stage's unit summary. The frontend polls the existing state command during a
   run, retry, or stage advance and renders that public contract.
2. **One terminal webhook per terminal outcome.** Notifications are dispatched
   after run, retry, or advance reaches `completed`, `partial`, `failed`,
   `blocked`, or `skipped`. The event ID is deterministic and is also sent as the
   `Idempotency-Key`; a delivered event is not delivered again.

The webhook payload contains only its schema/event ID, job ID/kind, terminal
status, current stage ID, aggregate progress/summary, and update time. It excludes
source titles and paths, logs, errors, artifacts, private text, credentials, and
the configured endpoint. Delivery failure is recorded as a safe delivery status
and does not rewrite the pipeline outcome.

`BOOK_PIPELINE_WEBHOOK_URL` is optional and remains environment-only. The GUI
uses the process environment first, then reads only that named value from the
repository-root `.env`; it does not load or log the other credentials. With no
configured endpoint, running the pipeline has no notification side effect.

## Deferred follow-up capabilities

- **SRT input**: later input-adapter ticket; normalize into the unified job model
  with timestamp/segment traceability, not an engine-side bypass.
- **DOCX input**: later input-adapter ticket; preserve headings, notes, and
  paragraph mapping before it enters the same source-map stages.
- **TTS audio**: later reading-output ticket after promotion; audio is a derived
  reading artifact with its own voice/config provenance, not part of EPUB or
  digest validation.

These are explicitly outside v1. Deferring them is the #51 scope decision, not an
assertion that those capabilities already exist.

The existing staged runner remains gated independently from notification scope.
The launcher exposes separate Continue and Approve actions: approval records a
current hash-bound decision but does not execute the next stage, and the
translation action explicitly warns that it sends source text.

## Consequences

The launcher remains the only live progress surface, and all display data comes
from durable public state rather than React-local inference. Terminal webhook
delivery is deliberately coarse-grained; per-stage/per-unit webhooks are not part
of this contract.
