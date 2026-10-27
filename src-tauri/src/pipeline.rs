// Phase 4: cloud transcription pipeline — ported from electron/main/pipeline.ts.
//
// First cut: OpenAI / Groq Whisper STT (multipart) + OpenAI-compatible GPT
// refine (chat/completions). Custom provider and
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
    /// When every ElevenLabs key fails, transcribe with Groq Whisper instead of
    /// failing the dictation (needs a Groq key).
    pub stt_fallback: bool,
}

/// One HTTP client for the whole app. Reusing it keeps TLS connections to the
/// STT providers alive between dictations, so only the first request pays
/// for the handshake.
pub fn http() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Open connections to the STT hosts ahead of the first dictation.
pub async fn warm_up() {
    for url in ["https://api.elevenlabs.io/v1/models", "https://api.groq.com/openai/v1/models"] {
        let _ = http().head(url).timeout(Duration::from_secs(5)).send().await;
    }
}

/// Split the user's vocabulary ("Next.js, Supabase\nSurfacly") into terms.
pub fn vocab_terms(vocab: &str, max_len: usize, max_count: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in vocab.split(|c| c == ',' || c == '\n' || c == ';') {
        let t = t.trim();
        if t.is_empty() || t.chars().count() >= max_len || t.split_whitespace().count() > 5 {
            continue;
        }
        if !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
        if out.len() >= max_count {
            break;
        }
    }
    out
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

    let client = http();
    let iso = to_iso(&opts.input_lang);

    // ── Premium STT — ElevenLabs Scribe, direct with the user's own keys ────
    // Best-in-class accuracy. Takes priority over the other STT paths when on
    // and at least one key is saved. No fallback to Whisper: the user chose
    // premium, so we surface failures instead of silently returning worse text.
    let mut force_groq = false;
    if opts.use_scribe && !opts.elevenlabs_keys.is_empty() {
        let raw = match scribe_stt(client, &opts, &bytes).await {
            Ok(t) => t,
            // Every key failed. Rather than lose the dictation, fall back to
            // Groq Whisper when the user allows it and has a key.
            Err(e) if opts.stt_fallback && !opts.groq_key.is_empty() => {
                applog!("[stt] scribe failed, falling back to Groq: {e}");
                force_groq = true;
                String::new()
            }
            Err(e) => return Err(e),
        };
        let raw = raw.trim().to_string();
        if force_groq {
            // fall through to the Whisper path below
        } else {
        #[cfg(debug_assertions)]
        applog!("[stt] scribe raw = {raw:?}");
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
    }

    // ── STT (OpenAI or Groq, both OpenAI-compatible /audio/transcriptions) ──
    let use_groq = (force_groq || opts.stt_provider == "groq") && !opts.groq_key.is_empty();
    let (stt_base, stt_key, stt_model) = if use_groq {
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
        .timeout(Duration::from_secs(25))
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
    applog!("[stt] cloud ({}) raw = {raw:?}", opts.stt_provider);

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

// ── ElevenLabs Scribe (direct, multi-key) ───────────────────────────────────

const EL_API: &str = "https://api.elevenlabs.io/v1";

/// App language label / ISO-639-1 → ElevenLabs `language_code` (ISO-639-3).
/// Scribe auto-detects, so this is only a hint and an unknown value is dropped.
pub(crate) fn scribe_lang(lang: &str) -> Option<&'static str> {
    // Every language in the Settings dropdown. Anything unmapped would fall
    // back to auto-detect, which for South Asian speech often picks Hindi.
    Some(match lang.trim().to_lowercase().as_str() {
        // Banglish is spoken Bangla; gpt_refine romanizes it afterwards.
        "bangla" | "bengali" | "bn" | "ben" | "banglish" => "ben",
        "english" | "en" | "eng" => "eng",
        "hindi" | "hi" | "hin" => "hin",
        "urdu" | "ur" | "urd" => "urd",
        "arabic" | "ar" | "ara" => "ara",
        "spanish" | "es" | "spa" => "spa",
        "french" | "fr" | "fra" => "fra",
        "german" | "de" | "deu" => "deu",
        "italian" | "it" | "ita" => "ita",
        "portuguese" | "pt" | "por" => "por",
        "russian" | "ru" | "rus" => "rus",
        "chinese" | "zh" | "zho" | "cmn" => "cmn",
        "japanese" | "ja" | "jpn" => "jpn",
        "korean" | "ko" | "kor" => "kor",
        "turkish" | "tr" | "tur" => "tur",
        "dutch" | "nl" | "nld" => "nld",
        "polish" | "pl" | "pol" => "pol",
        "vietnamese" | "vi" | "vie" => "vie",
        _ => return None,
    })
}

/// The writing system a dictation language is expected to come back in. Used
/// to catch the model answering Bangla speech in Devanagari (Hindi).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Bengali,
    Devanagari,
    Arabic,
    /// No check (Latin languages mix freely with English, or auto-detect).
    Any,
}

impl Script {
    pub fn of(lang: &str) -> Script {
        match scribe_lang(lang) {
            Some("ben") => Script::Bengali,
            Some("hin") => Script::Devanagari,
            Some("urd") | Some("ara") => Script::Arabic,
            _ => Script::Any,
        }
    }

    fn contains(self, c: char) -> bool {
        match self {
            Script::Bengali => ('\u{0980}'..='\u{09FF}').contains(&c),
            Script::Devanagari => ('\u{0900}'..='\u{097F}').contains(&c),
            Script::Arabic => ('\u{0600}'..='\u{06FF}').contains(&c) || ('\u{0750}'..='\u{077F}').contains(&c),
            Script::Any => true,
        }
    }

    /// True unless the text is mostly in a *different* South Asian / Arabic
    /// script than expected. English (Latin) words are always fine, since
    /// Bangla dictation is full of them.
    pub fn accepts(self, text: &str) -> bool {
        if self == Script::Any {
            return true;
        }
        let others = [Script::Bengali, Script::Devanagari, Script::Arabic];
        let foreign = text
            .chars()
            .filter(|&c| others.iter().any(|&o| o != self && o.contains(c)))
            .count();
        let own = text.chars().filter(|&c| self.contains(c)).count();
        foreign == 0 || own > foreign * 4
    }
}

#[cfg(test)]
mod script_tests {
    use super::*;

    #[test]
    fn bangla_rejects_devanagari_and_keeps_english() {
        let s = Script::of("Bangla");
        assert!(!s.accepts("अरे, जी, देखो।"));
        assert!(s.accepts("আরে যে দেখো, এগুলা fix করা উচিত।"));
        assert!(s.accepts("Next.js deploy done"));
        assert!(s.accepts(""));
    }

    #[test]
    fn unmapped_and_auto_accept_everything() {
        assert!(Script::of("auto").accepts("अरे"));
        assert!(Script::of("Spanish").accepts("hola अरे"));
        assert_eq!(scribe_lang("Spanish"), Some("spa"));
        assert_eq!(scribe_lang("auto"), None);
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
    let resp = http()
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
        let lower = body.to_lowercase();

        if is_quota_error(&body) {
            return serde_json::json!({ "ok": false, "error": "Out of credits" });
        }

        // ElevenLabs keys are scope-restricted, and reading the balance needs the
        // `user_read` scope that a transcription-only key won't have. Crucially
        // this response PROVES the key authenticated — a wrong key never reaches
        // the permission check — so the key is fine, the balance just isn't
        // visible. Reporting it as rejected would be plainly wrong.
        if lower.contains("missing_permissions") {
            return serde_json::json!({
                "ok": true,
                "restricted": true,
                "note": "Valid key — credits hidden. Enable the \"User: Read\" permission for this key in the ElevenLabs dashboard to show the balance.",
            });
        }

        let msg = if status.as_u16() == 401 {
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

/// Scribe batch model. v2 is the current model: better accuracy and the only
/// one that takes `keyterms`.
const SCRIBE_MODEL: &str = "scribe_v2";

/// One Scribe attempt with a single key.
async fn scribe_attempt(
    client: &reqwest::Client,
    key: &str,
    audio: &[u8],
    // 'static because reqwest's multipart text parts must own or outlive the
    // request; every value comes from scribe_lang's fixed set of literals.
    lang: Option<&'static str>,
    keyterms: &[String],
) -> Result<String, (String, crate::keys::Failure)> {
    use crate::keys::Failure;
    let part = reqwest::multipart::Part::bytes(audio.to_vec())
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| (e.to_string(), Failure::Fatal))?;
    let mut form = reqwest::multipart::Form::new()
        .text("model_id", SCRIBE_MODEL)
        // Scribe otherwise annotates non-speech sounds inline, so a cough or a
        // laugh gets typed into the user's document as "(laughs)". Dictation
        // wants the words only.
        .text("tag_audio_events", "false")
        .part("file", part);
    if let Some(code) = lang {
        form = form.text("language_code", code);
    }
    // Names and jargon from the user's vocabulary. Keyterms cost extra per
    // request, so they are only sent when the user has set some.
    for t in keyterms {
        form = form.text("keyterms", t.clone());
    }

    let resp = client
        .post(format!("{EL_API}/speech-to-text"))
        .header("xi-api-key", key)
        .multipart(form)
        // A dictation clip transcribes in 1-3s. 25s is a hung request, and
        // the next key (or the fallback) is a better bet than waiting longer.
        .timeout(Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| (format!("Scribe request failed: {e}"), Failure::Transient))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        let kind = crate::keys::classify_http(status.as_u16(), &body);
        // A key created without the speech_to_text scope authenticates fine and
        // then fails here, which reads as a baffling 401 unless we say so.
        let msg = if body.to_lowercase().contains("missing_permissions") {
            "key lacks the \"Speech to Text\" permission".to_string()
        } else {
            let snippet: String = body.chars().take(200).collect();
            format!("Scribe failed ({status}): {snippet}")
        };
        return Err((msg, kind));
    }

    let data: Value = serde_json::from_str(&body).map_err(|e| (e.to_string(), Failure::Transient))?;
    Ok(data
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

/// Transcribe via ElevenLabs Scribe using the user's own keys. Starts from the
/// key that worked last, skips keys known to be spent, and retries the same
/// audio on the next key, so a drained account costs a little latency once
/// instead of a failed dictation every time.
pub async fn scribe_stt(
    client: &reqwest::Client,
    opts: &PipelineOpts,
    audio: &[u8],
) -> Result<String, String> {
    use crate::keys;
    let lang = scribe_lang(&opts.input_lang);
    let terms = vocab_terms(&opts.vocabulary, 50, 100);
    let mut worst: Option<keys::Failure> = None;
    let mut last = "no ElevenLabs key configured".to_string();

    for (label, key) in keys::order(&opts.elevenlabs_keys) {
        match scribe_attempt(client, key, audio, lang, &terms).await {
            Ok(text) => {
                keys::mark_good(label);
                return Ok(text);
            }
            Err((err, kind)) => {
                applog!("[stt] scribe key {label:?} failed ({kind:?}): {err}");
                keys::mark_bad(label, kind);
                worst = Some(keys::worst(worst, kind));
                last = format!("{label}: {err}");
                if !kind.try_next_key() {
                    return Err(err);
                }
            }
        }
    }

    Err(match worst {
        Some(f) => keys::explain(f, &last),
        None => last,
    })
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

    let client = http();
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
        applog!("[refine] try provider={} model={model}", opts.gpt_provider);
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
                    applog!("[refine] out={clean:?}");
                    return clean;
                }
            }
            // Non-2xx (dead model, rate-limit…) — log and fall through to the next.
            Ok(_r) => {
                #[cfg(debug_assertions)]
                {
                    let st = _r.status();
                    let body = _r.text().await.unwrap_or_default();
                    applog!(
                        "[refine] model={model} HTTP {st} — trying next. body: {}",
                        body.chars().take(200).collect::<String>()
                    );
                }
            }
            // Network/transport error — log and try the next model.
            Err(_e) => {
                #[cfg(debug_assertions)]
                applog!("[refine] model={model} request failed — trying next: {_e}");
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
