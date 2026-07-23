# Runbook: real-backend translation smoke

Validates that the translation engine works against a **live** LLM provider —
the last mile after the engine internals (#59/#60/#61/#62) landed. Fake-provider
unit tests already pass; this exercises real keys, the provider registry, and
the glossary path end to end.

This is a manual live-provider check, not an automated test gate. The runner
lives at [`tools/smoke/real_backend_smoke.py`](../../tools/smoke/real_backend_smoke.py)
and removes its throwaway project after a successful run unless `--keep` is set.

## Verified live result (2026-07-17)

- Gemini native completed one fabricated chapter through
  `gemini-native/gemini-default`.
- The OpenAI-compatible adapter completed the same chapter against DashScope's
  [documented OpenAI-compatible endpoint](https://help.aliyun.com/en/model-studio/compatibility-of-openai-with-dashscope)
  with `qwen-plus`. The versioned `openai-default` registry entry was restored
  immediately after the smoke.
- No key value was written to Git, a manifest, a report, or command output.

## Prerequisites (your part)

Put a real key in the repository-root `.env` (gitignored). Key names match
`key_env` in `packages/translation-engine/src/translation_engine/providers.toml`:

| Provider profile / config        | `.env` variable                | Model (registry default) |
|----------------------------------|--------------------------------|--------------------------|
| `openai-compatible` / `openai-default` | `OPENAI_COMPATIBLE_API_KEYS` | `gpt-4.1-mini`           |
| `gemini-native` / `gemini-default`     | `GEMINI_API_KEYS`            | `gemini-2.5-flash`       |

Multiple keys → comma-separate them in the single variable (the engine's KeyPool
rotates and holds an independent 429 budget). `base_url`/`model` are overridable
by editing `providers.toml` (e.g. point `openai-compatible` at DeepSeek/OpenRouter).

## Level 1 — engine-direct smoke (isolates the provider)

Bypasses the Tauri launcher; builds a throwaway one-chapter project and runs the
engine directly. Fastest way to answer "does the real provider work?".

```sh
# OpenAI-compatible
uv run --package translation-engine python tools/smoke/real_backend_smoke.py

# Gemini native
uv run --package translation-engine python tools/smoke/real_backend_smoke.py \
    --provider-profile-id gemini-native --config-id gemini-default

# Also exercise the #62 windowed reflection pass
uv run --package translation-engine python tools/smoke/real_backend_smoke.py --second-pass
```

**Check:**
- `[report] {"total":1,"completed":1,"failed":0}` and unit status `completed`.
- The printed `chapter_001.md` is fluent Simplified Chinese.
- Glossary reached the model: **灯塔** for *lighthouse*, **港湾** for *harbor*.
- With `--second-pass`: a reflection critique excerpt prints and the draft is
  under `qa/reflection/` (revised text replaces the chapter only if it keeps the
  protected placeholders).

Exit codes: `3` registry/profile problem · `4` key env empty · `1` a unit failed
(read the `error.code`) · `0` success.

## Level 2 — full launcher fast-chain smoke (true end-to-end)

Only after Level 1 is green. Drive the Tauri launcher (`tools/bibliosmith-launcher`)
to enqueue a real book in **fast** mode and select either OpenAI compatible
(`openai-compatible/openai-default`) or Gemini native
(`gemini-native/gemini-default`). Then run the fast chain (prepare →
approve_translation → translate → build_reading → validate_reading). Confirm the
launcher resolves the selected registry entry and that the produced
`chapters/translated/*.md` match Level 1 quality.

The book-level `secondPassEnabled` toggle is wired through the launcher by #72.
Enable it for a fast-mode Level 2 smoke when the reflection pass is in scope.

## Troubleshooting

- `[registry] ... not in registry` — profile/config id doesn't match
  `providers.toml`; use one of the pairs in the table above.
- `glossary_hash_mismatch` (Level 2) — the prepared task's glossary hash drifted;
  rerun the launcher prepare stage so it re-binds `glossary/terms.csv`.
- `source_hash_mismatch` — the source chapter changed after prepare; re-prepare.
- Provider `FatalError` (4xx) — bad key or model name; `RateLimitError` persists
  after rotation — every key is throttled, wait or add keys.
