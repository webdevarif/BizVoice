# BizVoice diagnostics export (v{{VERSION}})

This folder lets a person or an AI agent find what goes wrong in BizVoice
without access to the user's machine. Nothing here contains API keys.

## Summary (computed at export)

{{SUMMARY}}

## Files

- `diagnostics.jsonl`: one JSON object per line, oldest first. Main source.
- `bizvoice.log`: free-text log (errors, panics, `[stt]`, `[live]`, `[paste]`).
- `corrections.json`: words the user corrected after dictation, with counts
  (`"wrong→right": n`), plus suggestions the user dismissed.
- `settings.redacted.json`: configuration, secrets replaced by `<redacted>`.

## Events in diagnostics.jsonl

Every line has `ts` (UTC) and `version`.

`event: "dictation"`
- `mode`: `batch` (recorded clip) or `live` (streamed preview; the final text
  still comes from batch Scribe on the full audio).
- `audio_ms`, `lang` (user's selected input language), `provider`.
- `attempts[]`: one per API key tried: `key` (label, not the secret), `ms`,
  `ok`, `failure` (Quota, Auth, Flagged, RateLimit, Transient, Fatal).
- `fallback`/`fallback_reason`: set when Scribe failed and Groq was used.
- `stt_ms`, `total_ms`: speech-to-text time and whole dictation time.
- `status`: `ok`, `empty` (no words heard), `error`. On error: `error`,
  `error_kind` (quota, auth, rate_limited, network, no_key, flagged_free_tier,
  other).
- Quality: `low_confidence[]` words Scribe was unsure about (`w`, probability
  `p`), `detected_lang`, `flag_wrong_script` (output not in the selected
  language's script), `live_vs_final_similarity` + `flag_live_mismatch`
  (realtime preview disagreed strongly with the final text).
- `text`/`live_text`: only when the user enabled "Log dictated text".

`event: "paste"`: `ok` (worker confirmed the keystroke), `target_process`,
`strategy` (`ctrl-v` or `shift-insert`), `ack_ms`, `chars`. `ok: false` means
the text stayed on the clipboard and the user was told to press Ctrl+V.

`event: "correction"`: `substitutions[]` (`from` → `to`) the user made via
"Fix last dictation", and overall `similarity`.

`event: "ui_error"`: message shown in the mic bar. `event: "ai_review"`: an
LLM report the user ran from Settings.

## How to read it

1. Start with the summary above, then `grep` the JSONL for `"status":"error"`.
2. Repeated `error_kind: quota` with one key label: that account is out of
   credits. `auth`: key wrong or missing the speech_to_text scope.
3. `paste` with `ok:false` or a target that keeps failing: the app needs a
   different paste strategy for that process.
4. Words in `corrections.json` or frequent `low_confidence` words belong in
   the vocabulary (Settings > Dictionary) so Scribe gets them as keyterms.
