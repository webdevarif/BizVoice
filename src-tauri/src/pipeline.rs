// Phase 4: cloud transcription pipeline — ported from electron/main/pipeline.ts.
//
// First cut: OpenAI / Groq Whisper STT (multipart) + OpenAI-compatible GPT
// refine (chat/completions). Custom provider, BizGrowHub-Bangla proxy, and
// local whisper (whisper-rs) are layered on next. All providers are
// OpenAI-compatible HTTP, so a single reqwest path parametrised by base URL +
// key + model covers OpenAI, Groq, and custom.

use serde_json::Value;
use std::time::Duration;

pub struct PipelineOpts {
    pub audio_base64: String,
    pub openai_key: String,
    pub groq_key: String,
    pub stt_provider: String, // "openai" | "groq"
    pub stt_model: String,
    pub gpt_provider: String, // "openai" | "groq" | "openrouter" | "custom"
    pub gpt_model: String,
    pub openrouter_key: String,
    pub custom_key: String,
    pub custom_base_url: String,
    pub custom_headers: String, // JSON string, e.g. {"HTTP-Referer":"..."}
    pub input_lang: String,     // app language label or "auto"
    pub skip_gpt: bool,
    pub style_prompt: String,
    pub vocabulary: String,      // custom terms/names → Whisper STT prompt bias
    pub use_scribe: bool,        // premium ElevenLabs Scribe STT (user's own keys)
    /// The user's ElevenLabs keys as (label, key), tried in order. More than one
    /// lets dictation survive an account running out of credits mid-session.
    pub elevenlabs_keys: Vec<(String, String)>,
    pub use_better_bangla: bool, // route Bangla through BizGrowHub's tuned ASR
    pub auth_token: String,      // BizGrowHub JWT (auth for the Bangla proxy)
    pub api_base: String,        // BizGrowHub base URL (e.g. https://bizgrowhub.shop)
}

/// Subset of pipeline.ts LANG_TO_ISO (app label → ISO-639-1). Returns None for
/// "auto"/unknown so the provider auto-detects.
fn to_iso(lang: &str) -> Option<String> {
    let lower = lang.to_lowercase();
    let v = match lower.as_str() {
        "" | "auto" => return None,
        "english" => "en",
        "bangla" => "bn",
        // Banglish = spoken Bangla, romanized at the refine step. STT still
        // recognizes it as Bangla ("bn"); gpt_refine transliterates the output.
        "banglish" => "bn",
        "hindi" => "hi",
        "urdu" => "ur",
        "arabic" => "ar",
        "spanish" => "es",
        "french" => "fr",
        "german" => "de",
        "italian" => "it",
        "portuguese" => "pt",
        "russian" => "ru",
        "chinese" => "zh",
        "japanese" => "ja",
        "korean" => "ko",
        "turkish" => "tr",
        other => other, // assume already an ISO code if short/unknown
    };
    Some(v.to_string())
}

pub async fn run_pipeline(opts: PipelineOpts) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(opts.audio_base64.as_bytes())
        .map_err(|e| format!("bad audio base64: {e}"))?;
    if bytes.len() < 1000 {
        return Err(format!("Audio too short ({} bytes). Try again.", bytes.len()));
    }

    let client = reqwest::Client::new();
    let iso = to_iso(&opts.input_lang);

    // ── Premium STT — ElevenLabs Scribe, direct with the user's own keys ────
    // Best-in-class accuracy. Takes priority over the other STT paths when on
    // and at least one key is saved. No fallback to Whisper: the user chose
    // premium, so we surface failures instead of silently returning worse text.
    if opts.use_scribe && !opts.elevenlabs_keys.is_empty() {
        let raw = scribe_stt(&client, &opts, &bytes).await?;
        let raw = raw.trim().to_string();
        #[cfg(debug_assertions)]
        eprintln!("[stt] scribe raw = {raw:?}");
        if raw.is_empty() {
            return Ok(String::new());
        }
        // Banglish needs the LLM to romanize, so it always refines even when
        // "AI Formatting" is off — otherwise you'd get raw Bangla script.
        if opts.skip_gpt && !opts.input_lang.eq_ignore_ascii_case("banglish") {
            return Ok(raw);
        }
        return Ok(gpt_refine(&opts, raw).await);
    }

    // ── Improved Bangla transcription ──────────────────────────────────────
    // When the user enables it AND is speaking Bangla AND is signed in, route
    // the audio through BizGrowHub's Bangla-tuned ASR (HF BanglaASR) using their
    // login token — far more accurate than Whisper for Bangla. No fallback to
    // Whisper on error: the user explicitly chose this model, so we surface the
    // failure instead of silently returning the garbled Whisper output.
    if opts.use_better_bangla && iso.as_deref() == Some("bn") && !opts.auth_token.is_empty() {
        let raw = better_bangla_stt(&client, &opts, &bytes).await?;
        let raw = raw.trim().to_string();
        #[cfg(debug_assertions)]
        eprintln!("[stt] better-bangla raw = {raw:?}");
        if raw.is_empty() {
            return Ok(String::new());
        }
        // Banglish needs the LLM to romanize, so it always refines even when
        // "AI Formatting" is off — otherwise you'd get raw Bangla script.
        if opts.skip_gpt && !opts.input_lang.eq_ignore_ascii_case("banglish") {
            return Ok(raw);
        }
        return Ok(gpt_refine(&opts, raw).await);
    }

    // ── STT (OpenAI or Groq, both OpenAI-compatible /audio/transcriptions) ──
    let (stt_base, stt_key, stt_model) = if opts.stt_provider == "groq" && !opts.groq_key.is_empty() {
        (
            "https://api.groq.com/openai/v1",
            opts.groq_key.clone(),
            // Full large-v3 (NOT turbo): turbo prunes the decoder and loses accuracy
            // on low-resource languages like Bangla. Full v3 is best for Bangla.
            "whisper-large-v3".to_string(),
        )
    } else {
        if opts.openai_key.is_empty() {
            return Err("Missing API key. Open Settings → Transcription.".into());
        }
        ("https://api.openai.com/v1", opts.openai_key.clone(), opts.stt_model.clone())
    };

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let mut form = reqwest::multipart::Form::new()
        .text("model", stt_model)
        .part("file", part);
    if let Some(ref l) = iso {
        form = form.text("language", l.clone());
    }
    // Bias recognition toward the user's custom vocabulary (names/jargon like
    // "Next.js, Supabase, Prisma") via Whisper's `prompt` parameter.
    if !opts.vocabulary.trim().is_empty() {
        form = form.text("prompt", opts.vocabulary.clone());
    }

    let resp = client
        .post(format!("{stt_base}/audio/transcriptions"))
        .bearer_auth(&stt_key)
        .multipart(form)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("STT request failed: {e}"))?;
    if !resp.status().is_success() {
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(format!("STT failed ({code}): {snippet}"));
    }
    let stt: Value = resp.json().await.map_err(|e| e.to_string())?;
    let raw = stt
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    #[cfg(debug_assertions)]
    eprintln!("[stt] cloud ({}) raw = {raw:?}", opts.stt_provider);

    if raw.is_empty() {
        return Ok(String::new());
    }
    // Banglish needs the LLM to romanize, so it always refines even when
    // "AI Formatting" is off — otherwise you'd get raw Bangla script.
    if opts.skip_gpt && !opts.input_lang.eq_ignore_ascii_case("banglish") {
        return Ok(raw);
    }
    Ok(gpt_refine(&opts, raw).await)
}

/// Transcribe Bangla audio via BizGrowHub's tuned ASR proxy
/// (POST /api/bizvoice/transcribe-indic, authed with the user's JWT). Returns the
/// raw transcript text, or a user-facing error on failure / cold-start.
async fn better_bangla_stt(
    client: &reqwest::Client,
    opts: &PipelineOpts,
    audio: &[u8],
) -> Result<String, String> {
    let url = format!(
        "{}/api/bizvoice/transcribe-indic",
        opts.api_base.trim_end_matches('/')
    );
    let part = reqwest::multipart::Part::bytes(audio.to_vec())
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .text("language", "bangla")
        .part("file", part);

    let resp = client
        .post(&url)
        .bearer_auth(&opts.auth_token)
        .multipart(form)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Bangla STT request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        // 503 = HF model cold-start; guide the user to retry rather than dump the body.
        if status.as_u16() == 503 {
            return Err("Bangla model is warming up — please try again in a few seconds.".into());
        }
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(format!("Bangla STT failed ({status}): {snippet}"));
    }

    let data: Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(data
        .get("transcript")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

// ── ElevenLabs Scribe (direct, multi-key) ───────────────────────────────────

const EL_API: &str = "https://api.elevenlabs.io/v1";

/// App language label / ISO-639-1 → ElevenLabs `language_code` (ISO-639-3).
/// Scribe auto-detects, so this is only a hint and an unknown value is dropped.
fn scribe_lang(lang: &str) -> Option<&'static str> {
    match lang.to_lowercase().as_str() {
        // Banglish is spoken Bangla; gpt_refine romanizes it afterwards.
        "bangla" | "bn" | "ben" | "banglish" => Some("ben"),
        "english" | "en" | "eng" => Some("eng"),
        "hindi" | "hi" | "hin" => Some("hin"),
        "urdu" | "ur" | "urd" => Some("urd"),
        "arabic" | "ar" | "ara" => Some("ara"),
        _ => None,
    }
}

/// True when ElevenLabs is telling us this account is out of credits rather
/// than that the key is wrong. It reports both as 400 *or* 401, so the body has
/// to be inspected — status alone can't tell the two apart.
fn is_quota_error(body: &str) -> bool {
    let lower = body.to_lowercase();
    lower.contains("quota_exceeded") || lower.contains("insufficient")
}

/// Check one key and report its balance. Powers the per-key "Test" button.
/// Never returns Err: a rejected key is a result to render, not a failure.
pub async fn probe_elevenlabs_key(key: &str) -> Value {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{EL_API}/user/subscription"))
        .header("xi-api-key", key)
        .timeout(Duration::from_secs(15))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "ok": false, "error": format!("Network error: {e}") }),
    };

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        let msg = if is_quota_error(&body) {
            "Out of credits".to_string()
        } else if status.as_u16() == 401 {
            "Key rejected — check it was copied correctly".to_string()
        } else {
            let snippet: String = body.chars().take(160).collect();
            format!("{status}: {snippet}")
        };
        return serde_json::json!({ "ok": false, "error": msg });
    }

    let data: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    serde_json::json!({
        "ok": true,
        "tier": data.get("tier").and_then(|v| v.as_str()).unwrap_or(""),
        "charsUsed": data.get("character_count").and_then(|v| v.as_u64()),
        "charsLimit": data.get("character_limit").and_then(|v| v.as_u64()),
    })
}

/// One Scribe attempt with a single key. `Ok(text)` on success; `Err((msg,
/// key_spent))` otherwise, where `key_spent` marks the errors worth failing
/// over for rather than reporting immediately.
async fn scribe_attempt(
    client: &reqwest::Client,
    key: &str,
    audio: &[u8],
    // 'static because reqwest's multipart text parts must own or outlive the
    // request; every value comes from scribe_lang's fixed set of literals.
    lang: Option<&'static str>,
) -> Result<String, (String, bool)> {
    let part = reqwest::multipart::Part::bytes(audio.to_vec())
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| (e.to_string(), false))?;
    let mut form = reqwest::multipart::Form::new()
        .text("model_id", "scribe_v1")
        .part("file", part);
    if let Some(code) = lang {
        form = form.text("language_code", code);
    }

    let resp = client
        .post(format!("{EL_API}/speech-to-text"))
        .header("xi-api-key", key)
        .multipart(form)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        // A network blip isn't this key's fault, but another key is on a fresh
        // connection anyway, so it's still worth moving on.
        .map_err(|e| (format!("Scribe request failed: {e}"), true))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        let snippet: String = body.chars().take(200).collect();
        let retryable =
            is_quota_error(&body) || status.as_u16() == 401 || status.as_u16() == 429 || status.is_server_error();
        return Err((format!("Scribe failed ({status}): {snippet}"), retryable));
    }

    let data: Value = serde_json::from_str(&body).map_err(|e| (e.to_string(), false))?;
    Ok(data
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

/// Transcribe via ElevenLabs Scribe using the user's own keys, walking the list
/// until one succeeds. A key that reports `quota_exceeded` is skipped and the
/// same audio is retried on the next one, so a drained account costs the user
/// a little latency instead of a failed dictation.
async fn scribe_stt(
    client: &reqwest::Client,
    opts: &PipelineOpts,
    audio: &[u8],
) -> Result<String, String> {
    let lang = scribe_lang(&opts.input_lang);
    let mut last = "no ElevenLabs key configured".to_string();

    for (label, key) in &opts.elevenlabs_keys {
        match scribe_attempt(client, key, audio, lang).await {
            Ok(text) => return Ok(text),
            Err((err, retryable)) => {
                #[cfg(debug_assertions)]
                eprintln!("[stt] scribe key {label:?} failed (retryable={retryable}): {err}");
                if !retryable {
                    return Err(err);
                }
                last = format!("{label}: {err}");
            }
        }
    }

    Err(format!(
        "All {} ElevenLabs key(s) failed. Last error — {last}",
        opts.elevenlabs_keys.len()
    ))
}

/// GPT format/refine step (OpenAI-compatible chat/completions) using the active
/// mode's `style_prompt`. Shared by the cloud pipeline and the local-whisper
/// path (whose STT comes from whisper-rs, but still wants the same formatting).
/// Falls back to the raw text on any provider/network failure.
pub async fn gpt_refine(opts: &PipelineOpts, raw: String) -> String {
    // Resolve base URL + key + provider-specific headers for the chosen provider.
    let (gpt_base, gpt_key, extra_headers): (String, String, Vec<(String, String)>) =
        match opts.gpt_provider.as_str() {
            "groq" if !opts.groq_key.is_empty() => (
                "https://api.groq.com/openai/v1".into(),
                opts.groq_key.clone(),
                vec![],
            ),
            "openrouter" if !opts.openrouter_key.is_empty() => (
                "https://openrouter.ai/api/v1".into(),
                opts.openrouter_key.clone(),
                vec![
                    ("HTTP-Referer".into(), "https://github.com/webdevarif/BizVoice".into()),
                    ("X-Title".into(), "BizVoice".into()),
                ],
            ),
            "custom" if !opts.custom_key.is_empty() && !opts.custom_base_url.is_empty() => {
                let mut h = vec![];
                if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&opts.custom_headers) {
                    for (k, v) in m {
                        if let Some(s) = v.as_str() {
                            h.push((k, s.to_string()));
                        }
                    }
                }
                (opts.custom_base_url.clone(), opts.custom_key.clone(), h)
            }
            _ if !opts.openai_key.is_empty() => (
                "https://api.openai.com/v1".into(),
                opts.openai_key.clone(),
                vec![],
            ),
            _ => return raw, // no usable GPT provider — return the raw transcription
        };

    // Language-aware refine guidance. Bengali STT output is phonetically rough,
    // so nudge the model to normalize to standard Bangla without changing meaning.
    // For "Banglish" input the user speaks Bangla but wants Roman-letter output,
    // so additionally transliterate the corrected text.
    let is_bangla = matches!(to_iso(&opts.input_lang).as_deref(), Some("bn"));
    let mut system_prompt = opts.style_prompt.clone();
    if is_bangla {
        system_prompt.push_str(
            "\n\nThis is Bengali speech-to-text output. Correct phonetic/spelling \
             mistakes to standard Bangla (e.g. আছা→আচ্ছা, খুপ→খুব, শুন্দর→সুন্দর, কাস→কাজ) \
             and fix word boundaries + punctuation. Do NOT change the meaning, add or \
             drop content, or translate. Keep any English words exactly as spoken.",
        );
    }
    if opts.input_lang.eq_ignore_ascii_case("banglish") {
        system_prompt.push_str(
            "\n\nThen transliterate the corrected Bangla into Banglish — Roman/English \
             letters the way Bangladeshis text (e.g. 'আমি ভালো আছি' -> 'ami valo achi'). \
             Output ONLY the Banglish text, with no Bangla script.",
        );
    }

    // Try the configured model first, then provider-specific fallbacks, so a
    // single dead/rate-limited free model (e.g. today's gemini-2.0-flash-exp:free
    // returning 404) doesn't silently drop cleanup — we retry a known-good model.
    let mut models = vec![opts.gpt_model.clone()];
    match opts.gpt_provider.as_str() {
        "openrouter" => {
            models.push("google/gemma-4-31b-it:free".into());
            models.push("meta-llama/llama-3.3-70b-instruct:free".into());
        }
        "groq" => {
            models.push("openai/gpt-oss-120b".into());
            models.push("llama-3.3-70b-versatile".into());
        }
        _ => {}
    }
    models.dedup();

    let client = reqwest::Client::new();
    for model in &models {
        let body = serde_json::json!({
            "model": model,
            "temperature": 0.2,
            "messages": [
                { "role": "system", "content": system_prompt.clone() },
                { "role": "user", "content": raw.clone() }
            ]
        });
        let mut req = client
            .post(format!("{gpt_base}/chat/completions"))
            .bearer_auth(&gpt_key)
            .json(&body)
            .timeout(Duration::from_secs(20));
        for (k, v) in &extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        #[cfg(debug_assertions)]
        eprintln!("[refine] try provider={} model={model}", opts.gpt_provider);
        match req.send().await {
            Ok(r) if r.status().is_success() => {
                let v = r.json::<Value>().await.unwrap_or(Value::Null);
                let out = v
                    .pointer("/choices/0/message/content")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let clean = sanitize_refined(&out);
                if !clean.is_empty() {
                    #[cfg(debug_assertions)]
                    eprintln!("[refine] out={clean:?}");
                    return clean;
                }
            }
            // Non-2xx (dead model, rate-limit…) — log and fall through to the next.
            Ok(_r) => {
                #[cfg(debug_assertions)]
                {
                    let st = _r.status();
                    let body = _r.text().await.unwrap_or_default();
                    eprintln!(
                        "[refine] model={model} HTTP {st} — trying next. body: {}",
                        body.chars().take(200).collect::<String>()
                    );
                }
            }
            // Network/transport error — log and try the next model.
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("[refine] model={model} request failed — trying next: {_e}");
            }
        }
    }
    // Every model failed — return the raw transcription unchanged.
    raw
}

/// Strip conversational scaffolding a chat model sometimes wraps around the
/// refined text despite being told to "output ONLY the corrected text" — a
/// leading preamble line ("Here is the corrected sentence:"), surrounding
/// quotes, or a markdown code fence.
///
/// This matters beyond cosmetics: a leaked preamble carries a trailing newline,
/// and the per-app char-typer turns every '\n' into an Enter keypress (see
/// `Type` in paste.rs). So an un-stripped "Here is the corrected sentence:\n<text>"
/// gets typed into the focused chat input and the newline *submits the preamble
/// as its own message*, leaving the real text behind. Stripping it kills both
/// the stray text and the stray submit.
///
/// Conservative by design: only strips at the very start, only when a known
/// refine cue is present, and never blanks the result.
pub fn sanitize_refined(text: &str) -> String {
    let mut out = strip_code_fence(text.trim());

    // Drop a leading "Here is the corrected sentence:"-style preamble.
    if let Some(colon) = out.find(':') {
        let prefix = &out[..colon];
        if !prefix.contains('\n') && prefix.chars().count() <= 70 {
            let p = prefix.to_lowercase();
            let starts_cue = p.starts_with("here")
                || p.starts_with("sure")
                || p.starts_with("corrected")
                || p.starts_with("the corrected");
            let refine_cue = p.contains("correct")
                || p.contains("fixed")
                || p.contains("revised")
                || p.contains("here is the")
                || p.contains("here's the");
            if starts_cue && refine_cue {
                let rest = out[colon + 1..].trim_start();
                // Never blank the result: keep the original if nothing follows.
                if !rest.is_empty() {
                    out = rest.to_string();
                }
            }
        }
    }

    unwrap_quotes(out.trim())
}

/// Remove a single ```fence … ``` wrapper if the model returned one.
fn strip_code_fence(text: &str) -> String {
    let t = text.trim();
    let Some(after_open) = t.strip_prefix("```") else {
        return t.to_string();
    };
    // Drop the remainder of the opening line (an optional language tag).
    let body = match after_open.find('\n') {
        Some(nl) => &after_open[nl + 1..],
        None => return t.to_string(), // lone "```…": not a real block, leave it
    };
    let body = match body.rfind("```") {
        Some(idx) => &body[..idx],
        None => body,
    };
    body.trim().to_string()
}

/// Strip one matching pair of surrounding quotes, unless doing so would split
/// content that legitimately contains interior quotes (e.g. `"a" and "b"`).
fn unwrap_quotes(text: &str) -> String {
    let t = text.trim();
    const PAIRS: [(char, char); 5] =
        [('"', '"'), ('\'', '\''), ('\u{201C}', '\u{201D}'), ('\u{2018}', '\u{2019}'), ('\u{00AB}', '\u{00BB}')];
    let Some(first) = t.chars().next() else {
        return t.to_string();
    };
    let last = t.chars().last().unwrap();
    for (open, close) in PAIRS {
        if first == open && last == close && t.chars().count() >= 2 {
            let inner = &t[open.len_utf8()..t.len() - close.len_utf8()];
            if !inner.contains(open) && !inner.contains(close) {
                return inner.trim().to_string();
            }
        }
    }
    t.to_string()
}

#[cfg(test)]
mod sanitize_tests {
    use super::sanitize_refined;

    #[test]
    fn strips_preamble_on_new_line() {
        // The exact failure from the report: preamble + newline → just the text.
        let s = "Here is the corrected sentence:\nThis is all my Shopify client.";
        assert_eq!(sanitize_refined(s), "This is all my Shopify client.");
    }

    #[test]
    fn strips_inline_preamble() {
        let s = "Sure! Here's the corrected text: This is all my Shopify client.";
        assert_eq!(sanitize_refined(s), "This is all my Shopify client.");
    }

    #[test]
    fn strips_corrected_label() {
        assert_eq!(sanitize_refined("Corrected: hello there"), "hello there");
    }

    #[test]
    fn unwraps_surrounding_quotes() {
        assert_eq!(
            sanitize_refined("\"This is all my Shopify client.\""),
            "This is all my Shopify client."
        );
    }

    #[test]
    fn strips_code_fence() {
        assert_eq!(sanitize_refined("```\nhello world\n```"), "hello world");
    }

    #[test]
    fn keeps_legit_colon_sentence() {
        // No refine cue in the prefix → must be left untouched.
        let s = "Tell him the meeting is at 5: bring the deck.";
        assert_eq!(sanitize_refined(s), s);
    }

    #[test]
    fn keeps_interior_quotes() {
        let s = "\"a\" and \"b\"";
        assert_eq!(sanitize_refined(s), s);
    }

    #[test]
    fn never_blanks_a_bare_preamble() {
        // Preamble with nothing after the colon → leave as-is rather than blank.
        let s = "Here is the corrected sentence:";
        assert_eq!(sanitize_refined(s), s);
    }
}
