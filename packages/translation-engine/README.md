# Translation Engine

This package is the programmatic Markdown translation adapter for the v2
pipeline. It accepts a run manifest containing one or more prepared chapter
task manifests, writes one artifact per unit, and prints one redacted JSON
status report. The fake provider is deterministic and performs no network I/O.

## Run one chapter

From this package directory, before the root uv workspace lands in `main`:

```sh
PYTHONPATH=src uv run --no-project python -m translation_engine \
  --manifest /absolute/path/to/translation-run.json
```

After the root workspace is available, the equivalent workspace command is:

```sh
uv run --package translation-engine translation-engine \
  --manifest /absolute/path/to/translation-run.json
```

The run manifest has this shape:

```json
{
  "schema": "translation-engine-run-v1",
  "projectRoot": "/absolute/path/to/private-book-project",
  "sourceMapPath": "metadata/source_map.json",
  "sourceLanguage": "auto",
  "targetLanguage": "zh-Hans",
  "providerProfileId": "fake-provider-profile",
  "providerConfigId": "fake-config-no-secrets",
  "translationPolicyVersion": "translation-policy-v1",
  "maxTokens": 450,
  "placeholderRetries": 1,
  "secondPassEnabled": false,
  "textCleanup": false,
  "promptPack": {
    "schema": "translation-prompt-pack-revision-v1",
    "packId": "builtin.structure-fidelity",
    "revisionId": "2026.08.05-1",
    "contentSha256": "fb5dae8c498d46a1a3501acd0d6b00645b7dfe4c5c797e8e71732482c5a0c26f",
    "displayName": "结构保真翻译",
    "executor": "programmatic",
    "sourceLanguage": "auto",
    "targetLanguage": "zh-Hans",
    "costHint": "1 次模型调用 / 原文块",
    "source": {
      "kind": "bibliosmith",
      "label": "BiblioSmith 内置基线",
      "license": "Project license",
      "adaptation": "将现有结构保护、术语注入和目标语言规则收口为可版本化方案。"
    },
    "stages": [{
      "stageId": "translate",
      "label": "结构保真初译",
      "template": "你是一名专业图书译者。请将当前原文块完整翻译为简体中文，忠实传达含义、语气和文体，不作解释、摘要或删节。只输出译文。"
    }]
  },
  "units": [
    {"taskManifestPath": "qa/tasks/chapter_001.json"}
  ]
}
```

`promptPack` is required and carries the exact immutable revision snapshot whose
ID and content hash are bound to the job and approval. The engine validates that
the stage graph matches `secondPassEnabled`; structure, placeholder, glossary,
target-language, and privacy constraints remain engine-owned and cannot be
overridden by a stage template.

## Preflight sample report

Before approving a full translation run, use the separate sample entry point
with the same prepared task manifests and provider profile/configuration:

```sh
uv run --package translation-engine translation-engine-sample \
  --manifest /absolute/path/to/translation-sample.json
```

The sample manifest uses schema `translation-engine-sample-v1`, replaces the
full-run token and policy fields with positive `sampleCount` and
`characterBudget` fields, and keeps the same `projectRoot`, `sourceMapPath`,
language, provider, retry, and `units` fields. The command prints a
`translation-engine-sample-report-v1` JSON object whose `samples` entries contain
only `chunkRef`, `sourceExcerpt`, `translatedExcerpt`, and `degradation`.

Sampling excludes the first and last task, selects the requested internal tasks
uniformly, and ends each excerpt on a sentence boundary. It runs the normal
placeholder validation and degradation path entirely in memory: it does not
write checkpoints or anything under `chapters/translated/`.

A book with two or fewer tasks therefore has no internal task to sample, and the
command succeeds with an empty `samples` list without calling the provider. That
is the defined outcome, not a failure: the excluded endpoints are where title,
copyright, and trailing metadata live, and sampling them would preview the least
representative pages in the book. A caller showing the report has to say so —
an empty panel reads as a broken preview.

The sample runs one translation pass. A full run with `secondPassEnabled` also
runs the windowed reflection, so a preview is not byte-for-byte what that run
will produce; a caller offering both has to say which it is showing.

`source_map.json` and each task manifest must be outputs of the existing split
and prepare stages. A retry supplies only failed or invalidated unit entries.
The provider config ID must identify the non-secret settings represented by the
manifest, including the token limit. Only `zh-Hans` is implemented; other target
languages are rejected.

## Real provider profiles

The versioned [`providers.toml`](src/translation_engine/providers.toml) registry
contains only non-secret settings. The built-in pairs are:

| `providerProfileId` | `providerConfigId` | Root `.env` key |
| --- | --- | --- |
| `openai-compatible` | `openai-default` | `OPENAI_COMPATIBLE_API_KEYS` |
| `gemini-native` | `gemini-default` | `GEMINI_API_KEYS` |
| `deepseek` | `deepseek-default` | `DEEPSEEK_API_KEYS` |
| `kimi` | `kimi-default` | `KIMI_API_KEYS` |
| `qwen` | `payg` | `QWEN_PAYG_API_KEYS` |
| `doubao` | `cn-beijing` | `VOLCENGINE_ARK_API_KEYS` |
| `mimo` | `payg` | `MIMO_PAYG_API_KEYS` |
| `mimo` | `token-plan` | `MIMO_TOKEN_PLAN_API_KEYS` |

Each key variable accepts comma- or newline-separated values. Empty items are
discarded and duplicates are removed without changing order. Already exported
environment variables take precedence over the repository-root `.env`; package
`.env` files are never read. Do not place key values in the registry, manifests,
commands, logs, or job reports.

Provider types name the wire protocol. `openai-compatible` configurations call
`{base_url}/chat/completions`, so the same adapter can target services that still
use Chat Completions. `openai-responses` configurations call
`{base_url}/responses`, send the system and source messages as `input`, parse
`message` / `output_text` items, and set `store: false` because local-reading
translation units do not need server-side conversation state. Gemini uses the
native `models/{model}:generateContent` endpoint and sends its key in the
`x-goog-api-key` header.

The built-in Qwen slot uses Alibaba Cloud Model Studio's mainland China
pay-as-you-go Responses endpoint and defaults to `qwen3.7-max`. It uses the
shared DashScope hostname until the user optionally saves a Workspace ID in the
Launcher, which selects the corresponding Beijing workspace hostname for both
connection tests and translation runs. The built-in Doubao slot uses Volcengine
Ark's Beijing Responses endpoint and defaults to the versioned
`doubao-seed-2-1-pro-260628` model. Ark standard API calls require a complete
versioned model ID or an `ep-...` inference endpoint ID; the launcher therefore
offers the stable `doubao-seed-evolving` alias alongside the two Seed 2.1 IDs
shipped in this release, and lets the user paste a newer ID. The Qwen slot also
accepts an exact model ID, while keeping the currently documented pay-as-you-go
Max model as its default.

The Launcher also exposes an optional Qwen web-search switch. It is off by
default because search adds cost and sends model-generated queries to Alibaba's
web-search service. When enabled, both connection probes and translation runs
use the Responses built-in-tool contract `tools: [{"type": "web_search"}]`;
the engine never sends the older Chat/DashScope `enable_search` parameter.
Search remains model-directed (`tool_choice` defaults to `auto`), and every
request continues to send `store: false`.

These two slots accept ordinary pay-as-you-go API keys only. Alibaba Token Plan
and Volcengine Agent/Coding Plan credentials are intentionally not registered:
their current service terms exclude non-interactive batch API translation.

### Manual one-chapter smoke

Prepare two one-unit run manifests using the IDs above, then run these commands
from the repository root after the corresponding root `.env` variable has been
filled:

```sh
uv run --package translation-engine translation-engine \
  --manifest /absolute/path/to/openai-smoke-run.json

uv run --package translation-engine translation-engine \
  --manifest /absolute/path/to/gemini-smoke-run.json
```

A successful smoke returns a redacted report whose summary has one completed
unit and writes `chapters/translated/<chapter-id>.md`. The automated suite never
runs these commands or performs real network calls; it uses fake HTTP transports.

The offline `utf8-byte-upper-bound-v1` counter conservatively enforces the token
budget. A `cl100k_base` BPE token always consumes at least one UTF-8 byte, so a
chunk of at most N bytes cannot exceed N cl100k tokens. This produces smaller
chunks than direct tiktoken counting, but avoids tiktoken's first-run encoding
download and keeps the test suite genuinely offline. Protected placeholders are
atomic; `maxTokens` must be large enough to hold one protected atom.

Private resume data is written under
`chapters/translated/.partial/`. Its idempotency key binds the task-manifest
hash, provider profile/config IDs, and translation policy version. A mismatched
key invalidates the cache. Successful atomic output removes the checkpoint;
source-preserving degradation remains partial and reports the unit as failed.

## Unit concurrency

Units translate in parallel, up to the `concurrency_limit` their provider entry
declares; a provider that declares none runs one unit at a time. Chunks within a
unit stay strictly serial, because each chunk's prompt carries the last 25 words
of the previous chunk's translation. Every unit reads and writes only paths
derived from its own unit ID, so units never contend for a checkpoint or an
output file. The report lists units in manifest order regardless of which
finished first.

A rate limit stops dispatch rather than the whole run. When a unit exhausts the
provider's throttle budget, units that have not started are failed retryable
without a request — attempting them would only churn the same throttle — but
units already in flight are left to finish and report their own outcome. One of
them may hold a credential that is still good, and killing it would discard a
paid call along with the checkpoint prefix it was about to write.

## Optional source-text cleanup

`textCleanup` is optional and defaults to `false`. When enabled, the target
profile appends a main-translation instruction to repair only obvious defects
within an existing paragraph: line-break hyphenation, extra or missing spaces,
and clearly incorrect punctuation. It never authorizes adding or removing
content, merging or splitting paragraphs, adding or removing headings, or
rewriting the author's style. The existing structure validator still rejects
any candidate that changes heading shape or paragraph count.

Cleanup-enabled runs use a distinct translation checkpoint pass ID so they do
not resume partial chunks produced with cleanup disabled.

## Book glossary enforcement

The engine has no built-in glossary. Each prepared task must point to the book
project's single source of truth, `glossary/terms.csv`, and bind its SHA-256.
For every translation chunk, the `zh-Hans` target profile adds only terms that
occur in that chunk to a mandatory system-instruction block. CJK terms use
substring matching; Latin terms use word boundaries; `|` separates inflection
variants. At most 50 entries are injected, preferring higher-frequency matches
and then longer variants.

If a reviewer changes `glossary/terms.csv`, the old prepared task fails with
`glossary_hash_mismatch`. Rerun the launcher's prepare stage before translation
so its task manifests and approval gate bind the new glossary hash.

What comes back is checked against what was demanded, and a term whose required
translation is missing is reported on the unit as `glossaryViolations`:

```json
{
  "source": "Zettelkasten",
  "translation": "卡片盒",
  "occurrences": [
    {
      "chunkIndex": 0,
      "sourceExcerpt": "The Zettelkasten is a slip box.",
      "translatedExcerpt": "笔记盒是一只卡片箱。"
    }
  ]
}
```

`chunkIndex` is a zero-based index into the unit's chunks. The two excerpts are
the same stretch of text on each side. A candidate is only accepted when it
carries the chunk's protected placeholders once each and in order, so splitting
source and output on those placeholders yields the same segments in the same
order, and the *i*-th segment of the output is the model's rendering of the
*i*-th segment of the source. A segment is bounded by whatever the protection
pass replaced — usually a paragraph break or heading prefix, but inline atoms
like code spans and link URLs bound one too, so an excerpt can be narrower than
the paragraph it sits in. At most two segments are reported per term, each capped
at 200 characters.

This is evidence, never a gate. Chinese word formation can put a required form
inside a longer compound, and a term can be legitimately absent where the source
form was part of a larger name, so a violation never fails, degrades, or rewrites
a chapter — the unit still completes.

## On-demand NER candidates

NER is a separate, manually triggered command. It samples the first 6000
characters of `source/source.md`, makes one request through the registered
provider factory, and writes a review-only candidate list. It is never invoked
by the translation command or the launcher's prepare stage.

```sh
uv run --package translation-engine translation-engine-ner \
  --project-root /absolute/path/to/private-book-project \
  --provider-profile-id PROFILE_ID \
  --provider-config-id CONFIG_ID
```

Before the root workspace is available, run the equivalent module entry from
this package directory:

```sh
PYTHONPATH=src uv run --no-project python -m translation_engine.ner_cli \
  --project-root /absolute/path/to/private-book-project \
  --provider-profile-id PROFILE_ID \
  --provider-config-id CONFIG_ID
```

The default output is `glossary/ner-candidates.json`. Review it manually, copy
accepted rows into `glossary/terms.csv`, then rerun prepare to bind the updated
hash. The NER command never edits `glossary/terms.csv`.

## Windowed reflection second pass

`secondPassEnabled` is optional and defaults to `false`. When enabled, the
engine uses the same provider for a windowed reflection pass: each block is
critiqued with only its immediately adjacent source/draft blocks, then improved.
The target-language profile and its block-filtered glossary instruction are
included as reflection criteria. Revised blocks must preserve the draft's
protected placeholders exactly; a failing revision is discarded in favor of
that block's draft.

Completed second-pass units retain three report-addressable artifacts:
`qa/reflection/<unit>.draft.md`, `qa/reflection/<unit>.reflection.md`, and the
revised `chapters/translated/<unit>.md`. Reflection progress resumes by block
from `chapters/translated/.partial/reflection/`. Checkpoint idempotency keys use
distinct `passId` values for the draft and reflection passes.

## Test

From the repository root:

```sh
uv run --package translation-engine pytest packages/translation-engine/tests
```

Or from this directory:

```sh
uv run pytest
```

Either way the full 64-test suite runs. Two details make that work, and both are
easy to break:

- `pyproject.toml` sets `pythonpath = ["src"]`, so the tests import this working
  tree instead of whatever happens to be installed in the environment.
- `--package` (or running from this directory) builds and installs the workspace
  member, which the CLI tests need for their console-script entry points.

A plain `uv sync` at the repository root uninstalls this member again, which is
why invoking `pytest` on its own can fail to import `translation_engine`.
