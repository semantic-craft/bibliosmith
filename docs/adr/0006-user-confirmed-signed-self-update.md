# ADR 0006: User-confirmed, independently signed self-update

- Status: Accepted
- Date: 2026-08-07
- Decision owner: Launcher distribution discussion

## Context

The launcher shipped as a manually downloaded DMG with no way to update itself.
Users learned about a new version only by revisiting the releases page, so any
fix reached them whenever they happened to look.

It had a self-update control once, and that control lied. `check_launcher_updates`
never made a network call: it echoed the installed version back as the latest
one, and the interface announced "up to date" without even reading the result.
Three separate controls were wired to that answer. All three were removed in
`affc7eb` and replaced with a static link to the releases page, on the reasoning
that a link which performs no check cannot misreport one. That left the honesty
problem solved and the distribution problem untouched.

Two properties of this particular app shape the decision. Its update is not
small or incidental: the bundle carries a Node sidecar, a uv sidecar, a Chromium
runtime and the pipeline packages, and installing one replaces all of it. And
the app is not idle between launches — a Book Pipeline job runs Python, Node and
Chromium out of the resources inside the installed bundle, for hours at a time.

## Decision

The launcher updates itself through `tauri-plugin-updater`, and the user decides
when.

**One silent check per launch, no polling.** A background check runs once at
startup. It never installs anything; it reports. A check that failed is a failed
check, distinct in the state machine from one that came back empty, and only the
latter may render as "up to date" — the interface may claim exactly what it
verified and nothing more. A failed startup check stays quiet, because the user
did not ask; a failed manual check says so, because they did.

**Downloading and installing are user actions.** A found update produces a
notice and a mark on the settings gear. It does not download in the background,
does not stage an install for the next quit, and does not restart the app. The
install and the restart are two separate presses.

**The install is refused while jobs run**, in the hook and not only in the
disabled button. Replacing the bundle pulls the interpreters out from under a
running job, so the refusal has to be a property of the operation rather than a
property of the screen that usually precedes it.

**Two independent signatures, neither substituting for the other.** Apple's
Developer ID signature and notarization answer what Gatekeeper asks when a human
opens the app. A minisign signature over the updater bundle answers what the
installed launcher asks before it overwrites itself, checked against the public
key compiled into the running build. The second is what makes the update path
safe to automate at all: a swapped release asset or a hijacked endpoint is
rejected on the user's machine, rather than trusted because it arrived over
HTTPS from a plausible URL.

**The published release is the update server.** `latest.json` is a release asset
built on the runner from the artifacts it just verified, and the endpoint is
GitHub's `/releases/latest/download/` redirect. There is no separate service to
run, and no manifest that can describe a build other than the one published
beside it.

## Consequences

ADR 0003 says the installed App bundle owns read-only code and the user workspace
owns the user's books. Self-update is the one operation that rewrites the first
layer, and it is now the reason that boundary has to hold: an update replaces
app resources wholesale and must leave Application Support, caches and the
workspace untouched. Anything a future feature stores inside the bundle is
destroyed on every update.

The release workflow gains four gates. It builds the `app` target alongside the
DMG, because the updater bundle is a tarball of the app and the bundler only
produces it for app bundles it built. It unpacks that tarball and puts the same
Gatekeeper and stapling questions to the app inside, since an in-app update
installs that copy and never the DMG's. It generates the manifest from the
verified artifacts. And after publishing it requests the live endpoint and
confirms the answer describes this release — a manifest that 404s is
indistinguishable from "no update available" to every installed launcher, so
without that step the failure would be silent and permanent.

A build that produces updater artifacts now requires `TAURI_SIGNING_PRIVATE_KEY`
and fails without it, rather than publishing a bundle no launcher can install.
Local builds without the key must pass `--no-sign`.

The signing key becomes unrecoverable state. Installed launchers trust only the
public key compiled into them, so losing the private key does not merely break
the next release — it permanently strands every existing installation on a
manual DMG download. It lives outside the repository, in GitHub Secrets for CI
and in the maintainer's own backup otherwise.

Only `darwin-aarch64` appears in the manifest. A launcher on any other platform
is told there is no update, which is true, instead of being handed a bundle it
cannot run.
