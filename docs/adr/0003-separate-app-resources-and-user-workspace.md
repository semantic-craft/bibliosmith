# ADR 0003: Separate app resources from the user workspace

- Status: Accepted
- Date: 2026-08-04
- Decision owner: Product architecture discussion in the Codex task

## Context

The launcher currently treats a Git checkout as both its executable resource
tree and the home of private reading projects. A single `repoRoot` therefore
selects scripts, Python packages, OCR staging, pipeline state, books,
translations, QA evidence, and final reading output. That coupling makes an
installed desktop app depend on a developer repository and mixes app-owned
files with user-owned content.

BiblioSmith is an independent desktop application. A user must not download or
understand its source repository in order to run it, and an app update must not
own, move, or overwrite the user's books.

## Decision

BiblioSmith uses four storage layers with separate ownership:

| Layer | Contents | Location and mutability |
| --- | --- | --- |
| App resources | App code, provider registry, pipeline packages, and runtime scripts | Inside the installed App bundle; read-only |
| Application support | Configuration, diagnostic logs, and app-managed runtime state | The operating system's Application Support directory; model and service credentials remain in Keychain |
| Cache and temporary data | Download caches, OCR staging, samples, and other reproducible intermediates | The operating system's Cache or temporary directory; disposable |
| User workspace | Source books, translation projects, QA records, and final reading artifacts | `~/Documents/BiblioSmith` by default, or an explicitly selected directory; user-owned |

One deep path module exposes these roots to the launcher. Production resolves
resources from Tauri's bundled resource directory. Development uses an explicit
checkout adapter so `tauri dev` can run without pretending the checkout is a
user workspace.

Startup checks only the workspace contract. The recommended action creates
`~/Documents/BiblioSmith`, a versioned workspace marker, and the local-reading
project container. It does not clone Git, create `.git`, copy runtime scripts,
or set `BIBLIOSMITH_HOME`. A non-empty unmarked directory is rejected rather
than overwritten.

Existing repository-local projects are not moved, deleted, or silently adopted.
Import or migration is a separate, explicit operation that must copy and verify
user data before any source is removed.

## Consequences

- Production commands resolve packages and scripts only from the read-only App
  resource root.
- Book Pipeline durable projects and final artifacts resolve only beneath the
  configured workspace.
- Pipeline state remains in Application Support; OCR and other reproducible
  intermediates move to Cache.
- Repository update, clone, Git status, and `BIBLIOSMITH_HOME` are not product
  startup or runtime concepts.
- Builders execute from bundled resources and write only to the selected book
  project or Cache; runtime scripts are not copied into user projects.
