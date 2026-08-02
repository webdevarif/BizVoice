// Phase 2 backend skeleton.
//
// Every command here corresponds 1:1 to a `window.api.*` method in the frontend
// shim (src/lib/tauriApi.ts), which itself mirrors the Electron preload bridge.
// Bodies are SAFE STUBS that return type-correct defaults so the existing React
// UI (Settings.tsx in particular) renders without crashing under Tauri. Real
// logic is filled in per TAURI_MIGRATION.md phases:
//   • settings/store/history/stats  → Phase 2
//   • windows/mic-bar/mute          → Phase 3
//   • transcribe/paste/whisper      → Phase 4
//   • auth/update                   → Phase 5
//
// Return types use serde_json::Value to avoid prematurely freezing 20+ structs;
// they get replaced with typed structs as each command is implemented.

use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{Manager, Window};
use tauri_plugin_store::StoreExt;

const SETTINGS_STORE: &str = "settings.json";
const SETTINGS_KEY: &str = "settings";
const REFINED_CACHE_KEY: &str = "refinedCache";

// ─────────────────────────────────────────────────────────────────────────────
// Global hotkeys (Phase 3)
// ─────────────────────────────────────────────────────────────────────────────

/// The three currently-registered shortcuts, kept in managed state so the
/// global-shortcut handler can map an incoming `Shortcut` back to its action
/// (toggle / push-to-talk / cycle). Mirrors `registerHotkey()` in
/// electron/main/index.ts, which reads `S.hotkey`/`S.pttHotkey`/`S.cycleHotkey`.
#[derive(Default)]
pub struct Hotkeys {
    pub toggle: Option<tauri_plugin_global_shortcut::Shortcut>,
    pub ptt: Option<tauri_plugin_global_shortcut::Shortcut>,
    pub cycle: Option<tauri_plugin_global_shortcut::Shortcut>,
}

pub type HotkeyState = Mutex<Hotkeys>;

/// Read the three hotkey strings from settings, (re-)register them with the OS,
/// and record them in `HotkeyState`. Safe to call repeatedly — unregisters the
/// previous set first. Called once at startup and again from `set_settings`
/// whenever a hotkey field changes.
pub fn register_hotkeys(app: &tauri::AppHandle) {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

    let Ok(store) = app.store(SETTINGS_STORE) else {
        return;
    };
    let s = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
    let parse = |key: &str, default: &str| -> Option<Shortcut> {
        let raw = s
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(default)
            .trim()
            .to_string();
        if raw.is_empty() {
            return None;
        }
        Shortcut::from_str(&raw).ok()
    };
    let toggle = parse("hotkey", "Alt+Q");
    let ptt = parse("pttHotkey", "Alt+W");
    let cycle = parse("cycleHotkey", "Alt+E");

    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    for sc in [toggle.clone(), ptt.clone(), cycle.clone()]
        .into_iter()
        .flatten()
    {
        let _ = gs.register(sc);
    }
    if let Some(state) = app.try_state::<HotkeyState>() {
        if let Ok(mut hk) = state.lock() {
            hk.toggle = toggle;
            hk.ptt = ptt;
            hk.cycle = cycle;
        }
    }
}

/// Shallow-merge `patch` (object) into `base` (object): each key in patch
/// replaces the one in base. Arrays (modes/dictionary/history) replace wholesale,
/// matching the Electron app's object-spread behavior.
fn merge_into(base: &mut Value, patch: &Value) {
    if let (Some(b), Some(p)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            b.insert(k.clone(), v.clone());
        }
    }
}

/// Add the `hasKey`/`hasGroqKey`/`hasOpenrouterKey`/`hasCustomKey` booleans the
/// Settings UI reads, derived from whether the encrypted key fields are present.
fn add_key_flags(s: &mut Value) {
    let (has_key, has_groq, has_or, has_custom) = {
        let flag = |k: &str| s.get(k).and_then(|v| v.as_str()).map_or(false, |v| !v.is_empty());
        (
            flag("openaiKeyEncrypted"),
            flag("groqKeyEncrypted"),
            flag("openrouterKeyEncrypted"),
            flag("customKeyEncrypted"),
        )
    };
    if let Some(o) = s.as_object_mut() {
        o.insert("hasKey".into(), Value::Bool(has_key));
        o.insert("hasGroqKey".into(), Value::Bool(has_groq));
        o.insert("hasOpenrouterKey".into(), Value::Bool(has_or));
        o.insert("hasCustomKey".into(), Value::Bool(has_custom));
    }
}

const KEY_FIELDS: [(&str, &str); 4] = [
    ("openaiKey", "openaiKeyEncrypted"),
    ("groqKey", "groqKeyEncrypted"),
    ("openrouterKey", "openrouterKeyEncrypted"),
    ("customKey", "customKeyEncrypted"),
];

/// Move plaintext provider keys from a settings patch into base64-obfuscated
/// `*Encrypted` fields and drop the plaintext. Mirrors index.ts
/// persistEncryptedKey, minus DPAPI (TODO: real OS-keyring in Phase 2 cont.).
fn obfuscate_keys(patch: &mut Value) {
    use base64::Engine;
    let Some(obj) = patch.as_object_mut() else {
        return;
    };
    for (plain, enc) in KEY_FIELDS {
        if let Some(v) = obj.remove(plain) {
            if let Some(raw) = v.as_str() {
                let stored = if raw.is_empty() {
                    String::new()
                } else {
                    base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
                };
                obj.insert(enc.to_string(), Value::String(stored));
            }
        }
    }
}

/// BizGrowHub backend base URL. Env override → dev localhost:8080 / prod prod host.
fn api_base() -> String {
    std::env::var("BIZGROWHUB_API")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                "http://localhost:8080".into()
            } else {
                "https://bizgrowhub.shop".into()
            }
        })
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Decode a base64-obfuscated `*Encrypted` key field back to plaintext.
fn decode_key(settings: &Value, enc_field: &str) -> String {
    use base64::Engine;
    let enc = settings.get(enc_field).and_then(|v| v.as_str()).unwrap_or("");
    if enc.is_empty() {
        return String::new();
    }
    base64::engine::general_purpose::STANDARD
        .decode(enc.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

/// Persist a single field INSIDE the `settings` object (where every reader looks
/// via `store.get(SETTINGS_KEY)`). The auth flow used to write the token as a
/// TOP-LEVEL store key (`store.set("bizgrowhubTokenEncrypted", …)`), which no
/// reader ever saw — so login never "stuck" and the app stayed logged-out.
fn settings_merge(app: &tauri::AppHandle, key: &str, value: Value) {
    let Ok(store) = app.store(SETTINGS_STORE) else { return };
    let mut s = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
    if let Some(obj) = s.as_object_mut() {
        obj.insert(key.to_string(), value);
    }
    store.set(SETTINGS_KEY, s);
    let _ = store.save();
}

/// Open the webview DevTools for the window the call came from (bound to F12 in
/// the frontend). Available because the tauri `devtools` feature is enabled in
/// Cargo.toml (so it works in release builds too, not just `tauri dev`).
#[tauri::command]
pub fn open_devtools_cmd(window: tauri::WebviewWindow) {
    window.open_devtools();
}

// ─────────────────────────────────────────────────────────────────────────────
// Settings & windows
// ─────────────────────────────────────────────────────────────────────────────

/// Default Settings object — mirrors the `Settings` type in electron/main/index.ts.
/// Used until the real tauri-plugin-store load lands (Phase 2).
fn default_settings() -> Value {
    json!({
        "openaiKeyEncrypted": "",
        "groqKeyEncrypted": "",
        "openrouterKeyEncrypted": "",
        "customKeyEncrypted": "",
        "useBetterBangla": false,
        "useScribe": false,
        "customBaseUrl": "",
        "customChatModel": "",
        "customHeaders": "",
        "hotkey": "Alt+Q",
        "pttHotkey": "Alt+W",
        "cycleHotkey": "Alt+E",
        "inputLang": "auto",
        "outputLang": "auto",
        "gptModel": "gpt-4o-mini",
        "gptProvider": "openai",
        "sttModel": "whisper-1",
        "sttProvider": "openai",
        // AI Formatting OFF by default: paste the exact raw transcript, and only
        // run the GPT refine step when the user explicitly turns AI Formatting on.
        "skipGpt": true,
        "launchOnStartup": false,
        "micDeviceId": "",
        "vocabulary": "",
        "modes": default_modes(),
        "activeMode": "transcript",
        "silenceMs": 1200,
        "autoStop": true,
        // Live dictation: keep the VAD running and transcribe + type each phrase
        // the moment the user pauses, instead of one utterance then stop.
        "liveDictation": false,
        "useLocalWhisper": false,
        "localModel": "",
        "micFallbackId": "",
        "muteWhileRecording": false,
        "dictionary": [],
        "theme": "dark",
        "widgetStyle": "logoText",
        "instructions": "",
        "history": [],
        "stats": { "recordings": 0, "words": 0, "durationMs": 0 },
        "bizgrowhubTokenEncrypted": "",
        "bizgrowhubEmail": "",
        "bizgrowhubName": "",
        "licenseOkAt": 0
    })
}

fn default_modes() -> Value {
    json!([
        { "id": "transcript", "name": "Transcript", "color": "sky",
          "prompt": "You are a transcription corrector. Fix ONLY spelling and grammar mistakes." },
        { "id": "ai", "name": "AI Prompt", "color": "purple",
          "prompt": "Convert raw voice dictation into a concise, direct AI prompt." },
        { "id": "client", "name": "Client", "color": "emerald",
          "prompt": "Convert raw voice dictation into a polite, professional client message." }
    ])
}

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<Value, String> {
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    let mut settings = default_settings();
    match store.get(SETTINGS_KEY) {
        Some(stored) => merge_into(&mut settings, &stored),
        None => {
            // Seed the store with defaults on first run so persistence is visible.
            store.set(SETTINGS_KEY, settings.clone());
            let _ = store.save();
        }
    }
    add_key_flags(&mut settings);
    Ok(settings)
}

#[tauri::command]
pub fn set_settings(app: tauri::AppHandle, mut patch: Value) -> Result<Value, String> {
    obfuscate_keys(&mut patch);
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    let mut current = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
    merge_into(&mut current, &patch);
    // Capture appearance values from the merged result before it's moved into the
    // store, so we can push them to the mic bar below.
    let theme = current
        .get("theme")
        .and_then(|v| v.as_str())
        .unwrap_or("dark")
        .to_string();
    let widget_style = current
        .get("widgetStyle")
        .and_then(|v| v.as_str())
        .unwrap_or("logoText")
        .to_string();
    store.set(SETTINGS_KEY, current);
    store.save().map_err(|e| e.to_string())?;
    // Apply the OS autostart toggle when launchOnStartup is in the patch
    // (the Electron app stored this but never applied it — porting fixes that).
    if let Some(enable) = patch.get("launchOnStartup").and_then(|v| v.as_bool()) {
        use tauri_plugin_autostart::ManagerExt;
        let mgr = app.autolaunch();
        let _ = if enable { mgr.enable() } else { mgr.disable() };
    }
    // Re-register global shortcuts when any hotkey changed (mirrors index.ts
    // calling registerHotkey() after a hotkey edit).
    if patch.get("hotkey").is_some()
        || patch.get("pttHotkey").is_some()
        || patch.get("cycleHotkey").is_some()
    {
        register_hotkeys(&app);
    }
    // Notify the mic bar of a theme/widget change so it restyles live
    // (mirrors the `appearance:changed` push in index.ts).
    if patch.get("theme").is_some() || patch.get("widgetStyle").is_some() {
        use tauri::Emitter;
        let _ = app.emit_to(
            "micbar",
            "appearance:changed",
            json!({ "theme": theme, "widgetStyle": widget_style }),
        );
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    focus_or_open_settings(&app)
}

/// Focus the existing settings window or build it from `settings.html`.
/// Mirrors the JS-singleton `openSettings()` in electron/main/index.ts (focus if
/// it already exists, otherwise create). The conf-declared window uses label
/// "main"; a window we rebuild after the user closed it uses label "settings".
pub fn focus_or_open_settings(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    if let Some(w) = app
        .get_webview_window("main")
        .or_else(|| app.get_webview_window("settings"))
    {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("BizVoice — Settings")
        .inner_size(720.0, 580.0)
        .min_inner_size(600.0, 420.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Reveal the mic bar if licensed, else surface the login window. Mirrors the
/// tray click / "Show Mic Bar" handler in index.ts (`micBar ? show : openLogin`).
pub fn show_micbar_or_login(app: &tauri::AppHandle) {
    if local_license_ok(app) {
        if let Some(w) = app.get_webview_window("micbar") {
            let _ = w.show();
            let _ = w.set_focus();
            return;
        }
    }
    show_login(app);
}

/// Show the login window (created hidden from tauri.conf.json).
pub fn show_login(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("login") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Fast, offline local license check used to pick the startup/tray window:
/// a stored token plus a `licenseOkAt` inside the 7-day grace. The authoritative
/// network re-check runs in `auth_status` (called by the frontend on mount).
///
/// As a fallback for users who signed in during an older build that never wrote
/// `licenseOkAt`, we also peek into the stored JWT: if it has an `exp` claim and
/// that claim lies in the future, the token itself is authoritative proof of a
/// valid login — we treat it as locally licensed and immediately backfill
/// `licenseOkAt` so subsequent boots don't re-decode the JWT.
pub fn local_license_ok(app: &tauri::AppHandle) -> bool {
    let Ok(store) = app.store(SETTINGS_STORE) else {
        return false;
    };
    // Force a reload from disk on the first check (the plugin caches in mem
    // between API calls but load() guarantees we read what was last saved).
    let _ = store.reload();
    let s = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
    let has_token = !decode_key(&s, "bizgrowhubTokenEncrypted").is_empty();
    let ok_at = s.get("licenseOkAt").and_then(|v| v.as_i64()).unwrap_or(0);
    if !has_token {
        return false;
    }
    if ok_at > 0 && (now_ms() - ok_at) < 7 * 24 * 60 * 60 * 1000 {
        return true;
    }
    // Fallback: valid unexpired JWT → still good enough for local gating.
    // Decode once, backfill ok_at so this branch is skipped on every next boot.
    let token = decode_key(&s, "bizgrowhubTokenEncrypted");
    if !token.is_empty() {
        use base64::Engine;
        let maybe_exp = token
            .split('.')
            .nth(1)
            .and_then(|middle| {
                let padded = match middle.len() % 4 {
                    0 => middle.to_string(),
                    2 => format!("{middle}=="),
                    3 => format!("{middle}="),
                    _ => middle.to_string(),
                };
                let url_standard: String = padded
                    .chars()
                    .map(|c| match c {
                        '-' => '+',
                        '_' => '/',
                        other => other,
                    })
                    .collect();
                base64::engine::general_purpose::STANDARD
                    .decode(url_standard.as_bytes())
                    .ok()
            })
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|claims| claims.get("exp").and_then(|v| v.as_i64()));
        if let Some(exp_s) = maybe_exp {
            let exp_ms = exp_s * 1000;
            if exp_ms > now_ms() {
                settings_merge(app, "licenseOkAt", json!(now_ms()));
                return true;
            }
        }
    }
    has_token && ok_at > 0 && (now_ms() - ok_at) < 7 * 24 * 60 * 60 * 1000
}

/// Startup window gate (mirrors `app.whenReady` in index.ts): show the mic bar if
/// locally licensed, otherwise the login window. The frontend's `authStatus()`
/// call then does the authoritative network check and `on_licensed` if needed.
pub fn startup_gate(app: &tauri::AppHandle) {
    if local_license_ok(app) {
        on_licensed(app);
    } else {
        show_login(app);
    }
}

#[tauri::command]
pub fn close_window(window: Window) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn minimize_window(window: Window) -> Result<(), String> {
    // Electron's window:minimize hides the window rather than minimizing.
    window.hide().map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Core transcribe + paste (Phase 4)
// ─────────────────────────────────────────────────────────────────────────────

#[tauri::command(rename_all = "camelCase")]
pub async fn transcribe(
    app: tauri::AppHandle,
    audio_base64: String,
    duration_ms: Option<u64>,
) -> Result<String, String> {
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    let s = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);

    let str_field = |k: &str, default: &str| -> String {
        match s.get(k).and_then(|v| v.as_str()) {
            Some(v) if !v.is_empty() => v.to_string(),
            _ => default.to_string(),
        }
    };

    // Active mode's prompt + optional custom instructions → the style system prompt.
    let active = str_field("activeMode", "transcript");
    let mode_prompt = s
        .get("modes")
        .and_then(|m| m.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|m| m.get("id").and_then(|i| i.as_str()) == Some(active.as_str()))
        })
        .and_then(|m| m.get("prompt"))
        .and_then(|p| p.as_str())
        .unwrap_or("Fix ONLY spelling and grammar. Output ONLY the corrected text.")
        .to_string();
    let instructions = s
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let style_prompt = if instructions.is_empty() {
        mode_prompt
    } else {
        format!("{mode_prompt}\n\nAdditional user instructions: {instructions}")
    };

    let gpt_provider = str_field("gptProvider", "openai");
    // Custom provider uses its own chat-model field when set.
    let gpt_model = if gpt_provider == "custom" {
        let cm = str_field("customChatModel", "");
        if cm.is_empty() {
            str_field("gptModel", "gpt-4o-mini")
        } else {
            cm
        }
    } else {
        str_field("gptModel", "gpt-4o-mini")
    };

    let opts = crate::pipeline::PipelineOpts {
        audio_base64,
        openai_key: decode_key(&s, "openaiKeyEncrypted"),
        groq_key: decode_key(&s, "groqKeyEncrypted"),
        stt_provider: str_field("sttProvider", "openai"),
        stt_model: str_field("sttModel", "whisper-1"),
        gpt_provider,
        gpt_model,
        openrouter_key: decode_key(&s, "openrouterKeyEncrypted"),
        custom_key: decode_key(&s, "customKeyEncrypted"),
        custom_base_url: str_field("customBaseUrl", ""),
        custom_headers: str_field("customHeaders", ""),
        input_lang: str_field("inputLang", "auto"),
        // Absent key → skip refine (AI Formatting defaults off), matching default_settings.
        skip_gpt: s.get("skipGpt").and_then(|v| v.as_bool()).unwrap_or(true),
        style_prompt,
        vocabulary: str_field("vocabulary", ""),
        use_scribe: s.get("useScribe").and_then(|v| v.as_bool()).unwrap_or(false),
        use_better_bangla: s.get("useBetterBangla").and_then(|v| v.as_bool()).unwrap_or(false),
        auth_token: decode_key(&s, "bizgrowhubTokenEncrypted"),
        api_base: api_base(),
    };

    // Local whisper path: when enabled with a downloaded model, run STT via
    // whisper-rs instead of the cloud, then apply the SAME GPT formatting
    // (mirrors pipeline.ts choosing local STT but still refining). Otherwise the
    // cloud pipeline does STT + GPT together.
    let use_local = s.get("useLocalWhisper").and_then(|v| v.as_bool()).unwrap_or(false);
    let local_model = str_field("localModel", "");
    let local_path = if use_local && !local_model.is_empty() {
        model_file(&local_model).and_then(|f| {
            let p = whisper_dir(&app).ok()?.join(f);
            p.exists().then_some(p)
        })
    } else {
        None
    };

    let text = if let Some(model_path) = local_path {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(opts.audio_base64.as_bytes())
            .map_err(|e| format!("bad audio base64: {e}"))?;
        let cache = app.state::<crate::whisper::WhisperCache>().inner().clone();
        let mp = model_path.to_string_lossy().to_string();
        let lang = opts.input_lang.clone();
        // whisper.cpp inference is CPU-blocking — run it off the async runtime.
        let raw = tauri::async_runtime::spawn_blocking(move || {
            cache.transcribe_local(&mp, &bytes, Some(&lang))
        })
        .await
        .map_err(|e| e.to_string())??;
        if raw.is_empty() || opts.skip_gpt {
            raw
        } else {
            crate::pipeline::gpt_refine(&opts, raw).await
        }
    } else {
        crate::pipeline::run_pipeline(opts).await?
    };
    let final_text = apply_dictionary(&text, &s);

    // Record to BizGrowHub history (fire-and-forget; offline = lost, acceptable
    // for a dictation log). Mirrors recordTranscription in index.ts.
    if !final_text.is_empty() {
        if let Some(token) = auth_token(&app) {
            let words = final_text.split_whitespace().filter(|w| !w.is_empty()).count();
            let dur = duration_ms.unwrap_or(0);
            let base = api_base();
            let body = json!({ "text": final_text, "words": words, "durationMs": dur });
            tauri::async_runtime::spawn(async move {
                let _ = reqwest::Client::new()
                    .post(format!("{base}/api/bizvoice/history"))
                    .bearer_auth(&token)
                    .json(&body)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await;
            });
        }
    }
    Ok(final_text)
}

/// Word-boundary, case-insensitive find/replace from the user's dictionary.
/// Mirrors index.ts applyDictionary.
fn apply_dictionary(text: &str, settings: &Value) -> String {
    let Some(dict) = settings.get("dictionary").and_then(|d| d.as_array()) else {
        return text.to_string();
    };
    let mut out = text.to_string();
    for entry in dict {
        let from = entry
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if from.is_empty() {
            continue;
        }
        let to = entry.get("to").and_then(|v| v.as_str()).unwrap_or("");
        let pattern = format!(r"\b{}\b", regex::escape(from));
        if let Ok(re) = regex::RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .build()
        {
            out = re.replace_all(&out, to).into_owned();
        }
    }
    out
}

#[tauri::command]
pub fn paste(app: tauri::AppHandle, text: String) -> Result<(), String> {
    // Per-app SendInput injection via the resident PowerShell worker
    // (Shift+Insert for editors/terminals, char-type for browsers, Ctrl+V
    // otherwise). See src/paste.rs — a faithful port of index.ts TYPE_PS_SCRIPT.
    crate::paste::paste_text(&app, text)
}

// ─────────────────────────────────────────────────────────────────────────────
// Mic bar window (Phase 3)
// ─────────────────────────────────────────────────────────────────────────────

/// Right-click menu on the mic bar: Settings / Hide / Quit. Mirrors the
/// `micBar:contextMenu` popup in electron/main/index.ts. Menu clicks are routed
/// to `handle_menu` via the app-level menu-event handler registered in lib.rs.
#[tauri::command]
pub fn mic_bar_context_menu(app: tauri::AppHandle, window: Window) -> Result<(), String> {
    use tauri::menu::{ContextMenu, Menu, MenuItem, PredefinedMenuItem};
    let settings = MenuItem::with_id(&app, "ctx_settings", "Settings", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let hide = MenuItem::with_id(&app, "ctx_hide", "Hide", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let sep = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
    let quit = MenuItem::with_id(&app, "ctx_quit", "Quit BizVoice", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let menu = Menu::with_items(&app, &[&settings, &hide, &sep, &quit])
        .map_err(|e| e.to_string())?;
    menu.popup(window).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn resize_mic_bar(active: bool) {
    // DEAD CHANNEL — Electron's main process never handled `micBar:resize`
    // (see PORT_SPEC.md). Kept as a no-op so the fire-and-forget call site in
    // MicBar.tsx doesn't reject under Tauri.
    let _ = active;
}

/// Current window position in physical pixels. Drives frameless drag for both
/// the mic bar and settings (mirrors the generic `micBar:getPos` in index.ts,
/// which returned the position of whichever window invoked it).
#[tauri::command]
pub fn get_win_pos(window: Window) -> Result<[i32; 2], String> {
    let p = window.outer_position().map_err(|e| e.to_string())?;
    Ok([p.x, p.y])
}

#[tauri::command]
pub fn set_win_pos(window: Window, x: i32, y: i32) -> Result<(), String> {
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn mute_system(mute: bool) {
    let _ = mute;
}

/// Advance `activeMode` to the next mode and emit `mode:changed` to the mic bar.
/// Called from the Alt+E global-shortcut handler (not an invoke command).
pub fn cycle_mode(app: &tauri::AppHandle) {
    use tauri::Emitter;
    let Ok(store) = app.store(SETTINGS_STORE) else {
        return;
    };
    let Some(mut s) = store.get(SETTINGS_KEY) else {
        return;
    };
    let modes = match s.get("modes").and_then(|m| m.as_array()) {
        Some(m) if !m.is_empty() => m.clone(),
        _ => return,
    };
    let active = s
        .get("activeMode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let idx = modes
        .iter()
        .position(|m| m.get("id").and_then(|i| i.as_str()) == Some(active.as_str()))
        .unwrap_or(0);
    let next = modes[(idx + 1) % modes.len()].clone();
    let next_id = next
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(obj) = s.as_object_mut() {
        obj.insert("activeMode".into(), json!(next_id));
    }
    store.set(SETTINGS_KEY, s);
    let _ = store.save();
    let _ = app.emit_to("micbar", "mode:changed", next);
}

// ─────────────────────────────────────────────────────────────────────────────
// System tray + menu routing (Phase 3)
// ─────────────────────────────────────────────────────────────────────────────

/// Build the system tray icon + menu. Mirrors `createTray()` in
/// electron/main/index.ts: Show Mic Bar / Settings / Check for Updates / Quit,
/// with a left-click that reveals the mic bar.
pub fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::with_id(app, "show_micbar", "Show Mic Bar", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let updates = MenuItem::with_id(app, "check_updates", "Check for Updates", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &settings, &updates, &sep, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("BizVoice")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_micbar_or_login(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// Central handler for both the tray menu and the mic-bar context menu, keyed by
/// menu-item id. Registered app-wide via `Builder::on_menu_event` (context menu)
/// and the tray's own `on_menu_event`.
pub fn handle_menu(app: &tauri::AppHandle, id: &str) {
    match id {
        "show_micbar" => show_micbar_or_login(app),
        "settings" | "ctx_settings" => {
            let _ = focus_or_open_settings(app);
        }
        "check_updates" => {
            // Phase 5: trigger tauri-plugin-updater check here.
        }
        "ctx_hide" => {
            if let Some(w) = app.get_webview_window("micbar") {
                let _ = w.hide();
            }
        }
        "quit" | "ctx_quit" => app.exit(0),
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// History & stats (Phase 2)
// ─────────────────────────────────────────────────────────────────────────────

/// The stored BizGrowHub bearer token, if signed in.
fn auth_token(app: &tauri::AppHandle) -> Option<String> {
    let store = app.store(SETTINGS_STORE).ok()?;
    let s = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
    let token = decode_key(&s, "bizgrowhubTokenEncrypted");
    (!token.is_empty()).then_some(token)
}

/// Dictation history (last 200) from BizGrowHub. Mirrors `history:get` in
/// index.ts — entries live in the backend, not locally. Empty list when signed
/// out or offline.
#[tauri::command]
pub async fn get_history(app: tauri::AppHandle) -> Value {
    let Some(token) = auth_token(&app) else {
        return json!([]);
    };
    let resp = reqwest::Client::new()
        .get(format!("{}/api/bizvoice/history?limit=200", api_base()))
        .bearer_auth(&token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => {
            let data: Value = r.json().await.unwrap_or_else(|_| json!({}));
            data.get("history").cloned().unwrap_or_else(|| json!([]))
        }
        _ => json!([]),
    }
}

#[tauri::command]
pub async fn clear_history(app: tauri::AppHandle) -> Result<(), String> {
    // Clear the backend history (best-effort) AND the local refined-correction
    // cache, mirroring electron/main/index.ts:851-854.
    if let Some(token) = auth_token(&app) {
        let _ = reqwest::Client::new()
            .delete(format!("{}/api/bizvoice/history", api_base()))
            .bearer_auth(&token)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
    }
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    store.set(REFINED_CACHE_KEY, json!({}));
    store.save().map_err(|e| e.to_string())?;
    Ok(())
}

const REFINE_SYSTEM_PROMPT: &str = "You correct grammar, spelling, and punctuation in the user's transcribed speech. Keep the original meaning, tone, and language. Do NOT translate. Do NOT add greetings, explanations, or commentary. Return ONLY the corrected sentence — nothing else.";

/// Resolve a chat-completions provider (chosen → groq → openai → openrouter →
/// custom fallback) from settings and refine `text`. Mirrors buildRefineClient
/// + callRefine in electron/main/index.ts.
async fn refine_via_provider(s: &Value, text: &str) -> Result<String, String> {
    use std::time::Duration;
    let str_field = |k: &str, d: &str| -> String {
        s.get(k)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .unwrap_or(d)
            .to_string()
    };
    let want = str_field("gptProvider", "openai");
    let openai = decode_key(s, "openaiKeyEncrypted");
    let groq = decode_key(s, "groqKeyEncrypted");
    let openrouter = decode_key(s, "openrouterKeyEncrypted");
    let custom = decode_key(s, "customKeyEncrypted");
    let custom_base = str_field("customBaseUrl", "");
    let gpt_model = str_field("gptModel", "");

    let build = |p: &str| -> Option<(String, String, String, Vec<(String, String)>)> {
        match p {
            "openai" if !openai.is_empty() => Some((
                "https://api.openai.com/v1".into(),
                openai.clone(),
                if gpt_model.is_empty() { "gpt-4o-mini".into() } else { gpt_model.clone() },
                vec![],
            )),
            "groq" if !groq.is_empty() => Some((
                "https://api.groq.com/openai/v1".into(),
                groq.clone(),
                "llama-3.1-8b-instant".into(),
                vec![],
            )),
            "openrouter" if !openrouter.is_empty() => Some((
                "https://openrouter.ai/api/v1".into(),
                openrouter.clone(),
                if gpt_model.is_empty() {
                    "meta-llama/llama-3.1-8b-instruct".into()
                } else {
                    gpt_model.clone()
                },
                vec![
                    ("HTTP-Referer".into(), "https://github.com/webdevarif/BizVoice".into()),
                    ("X-Title".into(), "BizVoice".into()),
                ],
            )),
            "custom" if !custom.is_empty() && !custom_base.is_empty() => {
                let mut h = vec![];
                if let Ok(Value::Object(m)) =
                    serde_json::from_str::<Value>(&str_field("customHeaders", ""))
                {
                    for (k, v) in m {
                        if let Some(sv) = v.as_str() {
                            h.push((k, sv.to_string()));
                        }
                    }
                }
                let cm = str_field("customChatModel", "");
                let model = if !cm.is_empty() {
                    cm
                } else if !gpt_model.is_empty() {
                    gpt_model.clone()
                } else {
                    "auto".into()
                };
                Some((custom_base.clone(), custom.clone(), model, h))
            }
            _ => None,
        }
    };

    let mut order = vec![want.clone()];
    for p in ["groq", "openai", "openrouter", "custom"] {
        if p != want {
            order.push(p.to_string());
        }
    }
    let (base, key, model, headers) = order
        .iter()
        .find_map(|p| build(p))
        .ok_or("Refine needs an API key (OpenAI, Groq, OpenRouter, or Custom). Open Settings → Transcription.")?;

    let max_tokens = std::cmp::min(1024usize, (text.len() as f64 * 1.5).ceil() as usize + 64);
    let body = json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": REFINE_SYSTEM_PROMPT },
            { "role": "user", "content": text }
        ]
    });
    let mut req = reqwest::Client::new()
        .post(format!("{base}/chat/completions"))
        .bearer_auth(&key)
        .json(&body)
        .timeout(Duration::from_secs(20));
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.map_err(|e| format!("refine request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Refine failed (HTTP {})", resp.status()));
    }
    let data: Value = resp.json().await.map_err(|e| e.to_string())?;
    let raw = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Strip any "Here is the corrected sentence:"-style preamble the model leaks.
    let refined = crate::pipeline::sanitize_refined(raw);
    if refined.is_empty() {
        return Err("Empty refinement result".into());
    }
    Ok(refined)
}

/// On-demand grammar refine of a past transcription, cached locally by entry ts
/// (mirrors the `history:refine` handler in index.ts).
#[tauri::command]
pub async fn refine_text(app: tauri::AppHandle, text: String, ts: i64) -> Result<String, String> {
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    let mut cache = store.get(REFINED_CACHE_KEY).unwrap_or_else(|| json!({}));
    let key = ts.to_string();
    if let Some(cached) = cache.get(&key).and_then(|v| v.as_str()) {
        if !cached.is_empty() {
            return Ok(cached.to_string());
        }
    }
    let s = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
    let refined = refine_via_provider(&s, &text).await?;
    if let Some(obj) = cache.as_object_mut() {
        obj.insert(key, json!(refined));
    }
    store.set(REFINED_CACHE_KEY, cache);
    let _ = store.save();
    Ok(refined)
}

#[tauri::command]
pub fn get_refined_cache(app: tauri::AppHandle) -> Result<Value, String> {
    // Local refined-correction cache (keyed by entry ts), mirrors index.ts:929.
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    Ok(store.get(REFINED_CACHE_KEY).unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub async fn get_stats(app: tauri::AppHandle) -> Value {
    let default = json!({ "recordings": 0, "words": 0, "durationMs": 0 });
    let Some(token) = auth_token(&app) else {
        return default;
    };
    let resp = reqwest::Client::new()
        .get(format!("{}/api/bizvoice/stats", api_base()))
        .bearer_auth(&token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(default),
        _ => default,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Local whisper models (Phase 4)
// ─────────────────────────────────────────────────────────────────────────────

/// (name, on-disk filename, display size) — mirrors MODEL_META in
/// electron/main/localWhisper.ts. Filenames are byte-identical to the HF
/// `ggerganov/whisper.cpp` assets so whisper-rs (Phase 4 inference) can consume
/// them with no re-download.
const MODELS: [(&str, &str, &str); 9] = [
    ("tiny", "ggml-tiny.bin", "75 MB"),
    ("tiny.en", "ggml-tiny.en.bin", "75 MB"),
    ("base", "ggml-base.bin", "142 MB"),
    ("base.en", "ggml-base.en.bin", "142 MB"),
    ("small", "ggml-small.bin", "466 MB"),
    ("small.en", "ggml-small.en.bin", "466 MB"),
    ("medium", "ggml-medium.bin", "1.5 GB"),
    ("medium.en", "ggml-medium.en.bin", "1.5 GB"),
    ("large-v3-turbo", "ggml-large-v3-turbo.bin", "1.5 GB"),
];

fn model_file(name: &str) -> Option<&'static str> {
    MODELS.iter().find(|(n, _, _)| *n == name).map(|(_, f, _)| *f)
}

/// `<app_data_dir>/whisper-models`. NOTE: Electron stored these under
/// `userData/whisper-models` (= %APPDATA%/BizVoice); Tauri's app_data_dir is
/// %APPDATA%/com.bizvoice.app, so previously-downloaded models live elsewhere —
/// a one-time migration/move is a Phase 6 follow-up (see PORT_SPEC.md).
fn whisper_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("whisper-models"))
}

#[tauri::command]
pub fn whisper_list_models(app: tauri::AppHandle) -> Value {
    let dir = whisper_dir(&app).ok();
    let out: Vec<Value> = MODELS
        .iter()
        .map(|(name, file, size)| {
            let downloaded = dir.as_ref().map_or(false, |d| d.join(file).exists());
            json!({ "name": name, "size": size, "downloaded": downloaded })
        })
        .collect();
    json!(out)
}

/// Stream a ggml model from HuggingFace to disk, emitting
/// `whisper:downloadProgress {model, pct}` (throttled to integer pct). Writes to
/// a `.tmp` then renames for atomicity. Mirrors localWhisper.ts downloadModel,
/// but streams to disk instead of buffering the whole (up to ~1.5 GB) file.
#[tauri::command]
pub async fn whisper_download_model(app: tauri::AppHandle, name: String) -> Result<Value, String> {
    use std::io::Write;
    use tauri::Emitter;

    let Some(file) = model_file(&name) else {
        return Ok(json!({ "ok": false, "error": format!("Unknown model: {name}") }));
    };
    let dir = whisper_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(file);
    if dest.exists() {
        return Ok(json!({ "ok": true }));
    }

    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file}"
    );
    let mut resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("download request failed: {e}"))?;
    if !resp.status().is_success() {
        return Ok(json!({ "ok": false, "error": format!("HTTP {}", resp.status()) }));
    }

    let total = resp.content_length().unwrap_or(0);
    let tmp = dest.with_extension("bin.tmp");
    let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut received: u64 = 0;
    let mut last_pct: i64 = -1;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        out.write_all(&chunk).map_err(|e| e.to_string())?;
        received += chunk.len() as u64;
        if total > 0 {
            let pct = ((received as f64 / total as f64) * 100.0) as i64;
            if pct != last_pct {
                last_pct = pct;
                let _ = app.emit("whisper:downloadProgress", json!({ "model": name, "pct": pct }));
            }
        }
    }
    drop(out);
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    let _ = app.emit("whisper:downloadProgress", json!({ "model": name, "pct": 100 }));
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn whisper_delete_model(app: tauri::AppHandle, name: String) -> bool {
    let Some(file) = model_file(&name) else {
        return false;
    };
    let Ok(dir) = whisper_dir(&app) else {
        return false;
    };
    let p = dir.join(file);
    p.exists() && std::fs::remove_file(&p).is_ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// BizGrowHub auth + license (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

/// Browser-based sign-in: spin up a one-shot loopback HTTP server on
/// 127.0.0.1:0, open the BizGrowHub desktop-auth page, and accept the JWT it
/// hands back to /callback. Mirrors index.ts startBrowserLogin.
#[tauri::command]
pub fn start_browser_login(app: tauri::AppHandle) -> Result<Value, String> {
    use std::io::{Read, Write};
    use tauri::Emitter;
    use tauri_plugin_opener::OpenerExt;

    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("could not start local sign-in server: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();
    // CSRF state. TODO: replace with a CSPRNG; sufficient for a localhost one-shot.
    let state = format!("{:x}{:x}", now_ms(), std::process::id());

    let url = format!("{}/desktop-auth?port={}&state={}", api_base(), port, state);
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())?;

    let app2 = app.clone();
    std::thread::spawn(move || {
        let expected_state = state;
        // Accept up to ~10 incoming attempts before giving up. Handles the
        // common case where:
        //   • browser sends GET /favicon.ico before the real /callback redirect,
        //   • CORS preflight OPTIONS request lands ahead of the real payload,
        //   • user refreshes the success page and triggers a second GET.
        for _ in 0..10 {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buf = [0u8; 16384];
            let n = stream.read(&mut buf).unwrap_or(0);
            if n == 0 {
                continue;
            }
            let req = String::from_utf8_lossy(&buf[..n]);
            let method = req.lines().next().unwrap_or("").split_whitespace().next().unwrap_or("");
            let path = req
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("");

            // Silently short-circuit favicon / static probes that browsers
            // fire automatically — they shouldn't consume the callback.
            if path.starts_with("/favicon") || path == "/robots.txt" {
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                continue;
            }

            // CORS preflight: modern browsers send OPTIONS before an XHR/fetch
            // from HTTPS bizgrowhub.shop → HTTP localhost. Without Access-Control
            // headers the subsequent real callback is BLOCKED by the browser.
            if method == "OPTIONS" {
                let _ = stream.write_all(
                    b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                continue;
            }

            let (token, st) = parse_callback(path);
            let ok = !token.is_empty() && st == expected_state;

            // Always include permissive CORS headers on the response too, so
            // even if the page uses XHR/fetch (not top-level navigate) the
            // browser can read the success body.
            let cors = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Credentials: true\r\n";

            let body = if ok {
                "<h2>BizVoice connected \u{2713}</h2><p>You can close this tab and return to the app.</p>"
            } else if !token.is_empty() {
                "<h2>Invalid sign-in</h2><p>State mismatch — please retry from the BizVoice app.</p>"
            } else {
                // Waiting page — probably just favicon / preflight noise
                // that slipped past, so keep the listener alive for another
                // attempt instead of surfacing a scary error.
                "<h2>Waiting for sign-in…</h2><p>If this page stays open, go back to BizVoice and click Sign In again.</p>"
            };
            let html = format!(
                "<!doctype html><html><body style=\"font-family:system-ui;background:#0A0A0F;color:#fff;display:flex;align-items:center;justify-content:center;height:100vh;margin:0\"><div style=\"text-align:center\">{body}</div></body></html>"
            );
            let status = if ok {
                "HTTP/1.1 200 OK"
            } else if !token.is_empty() {
                "HTTP/1.1 403 Forbidden"
            } else {
                "HTTP/1.1 200 OK"
            };
            let resp = format!(
                "{status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {len}\r\n{cors}Connection: close\r\n\r\n{html}",
                len = html.len(),
                cors = cors,
                status = status,
                html = html,
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
            drop(stream);

            if ok {
                // ── FIX A: persist token + license grace + email in ONE step.
                // Previously we only saved the token here and relied on a
                // successful `auth_status()` network call to backfill
                // `licenseOkAt`.  If that call timed out or the user restarted
                // the app *before* it ran, `local_license_ok` would return
                // false on next boot and force the user through login again
                // even though the token is perfectly valid.
                let (email, name) = decode_jwt_claims(&token);
                {
                    use base64::Engine;
                    let enc = base64::engine::general_purpose::STANDARD.encode(token.as_bytes());
                    settings_merge(&app2, "bizgrowhubTokenEncrypted", json!(enc));
                }
                // The token was just minted by BizGrowHub → treat the current
                // millisecond as the start of the 7-day offline grace window.
                settings_merge(&app2, "licenseOkAt", json!(now_ms()));
                if !email.is_empty() {
                    settings_merge(&app2, "bizgrowhubEmail", json!(email));
                }
                if !name.is_empty() {
                    settings_merge(&app2, "bizgrowhubName", json!(name));
                }

                let _ = app2.emit(
                    "auth:changed",
                    json!({ "active": true, "loggedIn": true, "email": email, "name": name }),
                );
                on_licensed(&app2);
                break;
            }
            // state mismatch with a token = abort, don't accept more attempts.
            if !token.is_empty() {
                break;
            }
        }
    });

    Ok(json!({ "ok": true }))
}

/// Pull email + display name out of the middle base64url payload of a JWT.
/// Returns ("", "") on any parse failure (callers fall back to "Signed in" + ?).
fn decode_jwt_claims(token: &str) -> (String, String) {
    use base64::Engine;
    let middle = match token.split('.').nth(1) {
        Some(m) => m,
        None => return (String::new(), String::new()),
    };
    // JWT payloads use base64url (-_), not standard (+/). Pad to a multiple of
    // 4 bytes so the decoder doesn't reject short payloads.
    let padded = match middle.len() % 4 {
        0 => middle.to_string(),
        2 => format!("{middle}=="),
        3 => format!("{middle}="),
        _ => middle.to_string(),
    };
    let url_standard: String = padded.chars().map(|c| match c {
        '-' => '+',
        '_' => '/',
        other => other,
    }).collect();
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(url_standard.as_bytes()) else {
        return (String::new(), String::new());
    };
    let Ok(json) = serde_json::from_slice::<Value>(&bytes) else {
        return (String::new(), String::new());
    };
    let email = json
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Common claim names for display name (in order of BizGrowHub likelihood).
    let name = json
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| json.get("displayName").and_then(|v| v.as_str()))
        .or_else(|| json.get("full_name").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    (email, name)
}

/// Parse `token` and `state` from a `/callback?token=..&state=..` path.
fn parse_callback(path: &str) -> (String, String) {
    let (mut token, mut state) = (String::new(), String::new());
    if let Some(q) = path.split('?').nth(1) {
        for pair in q.split('&') {
            let mut it = pair.splitn(2, '=');
            match (it.next(), it.next()) {
                (Some("token"), Some(v)) => token = urldecode(v),
                (Some("state"), Some(v)) => state = urldecode(v),
                _ => {}
            }
        }
    }
    (token, state)
}

/// Minimal percent-decoding for query params.
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[tauri::command]
pub fn cancel_browser_login() -> Value {
    json!({ "ok": true })
}

#[tauri::command]
pub fn logout(app: tauri::AppHandle) -> Result<Value, String> {
    settings_merge(&app, "bizgrowhubTokenEncrypted", json!(""));
    settings_merge(&app, "licenseOkAt", json!(0));
    settings_merge(&app, "bizgrowhubEmail", json!(""));
    settings_merge(&app, "bizgrowhubName", json!(""));
    if let Some(w) = app.get_webview_window("micbar") {
        let _ = w.hide();
    }
    use tauri::Emitter;
    let _ = app.emit(
        "auth:changed",
        json!({ "active": false, "loggedIn": false, "email": "", "name": "" }),
    );
    show_login(&app);
    Ok(json!({ "ok": true }))
}

/// User is authenticated AND licensed — reveal the mic bar and dismiss the login
/// window. Mirrors `onLicensed()` in index.ts (the remote-settings pull is the
/// frontend's responsibility under Tauri; hotkeys are registered at startup).
pub fn on_licensed(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("micbar") {
        let _ = w.show();
    }
    if let Some(w) = app.get_webview_window("login") {
        let _ = w.hide();
    }
}

/// License/auth status — GET /api/bizvoice/license with the stored token, with a
/// 7-day offline grace (mirrors index.ts checkLicense).
#[tauri::command]
pub async fn auth_status(app: tauri::AppHandle) -> Result<Value, String> {
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    let s = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
    let token = decode_key(&s, "bizgrowhubTokenEncrypted");
    if token.is_empty() {
        return Ok(json!({ "loggedIn": false, "active": false, "email": "", "name": "" }));
    }

    // ── Back-compat fill: users who signed in before the JWT-claim decode
    //    change landed have bizgrowhubEmail == "" even though their token
    //    contains the email.  Pull email/name out of the stored JWT once and
    //    persist so the UI shows them without a network round-trip.
    let stored_email = s
        .get("bizgrowhubEmail")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let stored_name = s
        .get("bizgrowhubName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let (jwt_email, jwt_name) = decode_jwt_claims(&token);
    let mut email = stored_email.clone();
    let mut name = stored_name.clone();
    if email.is_empty() && !jwt_email.is_empty() {
        email = jwt_email.clone();
        settings_merge(&app, "bizgrowhubEmail", json!(jwt_email));
    }
    if name.is_empty() && !jwt_name.is_empty() {
        name = jwt_name.clone();
        settings_merge(&app, "bizgrowhubName", json!(jwt_name));
    }

    let resp = reqwest::Client::new()
        .get(format!("{}/api/bizvoice/license", api_base()))
        .bearer_auth(&token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;
    match resp {
        Ok(r) if r.status().as_u16() == 401 => {
            settings_merge(&app, "bizgrowhubTokenEncrypted", json!(""));
            settings_merge(&app, "licenseOkAt", json!(0));
            settings_merge(&app, "bizgrowhubEmail", json!(""));
            settings_merge(&app, "bizgrowhubName", json!(""));
            if let Some(w) = app.get_webview_window("micbar") {
                let _ = w.hide();
            }
            {
                use tauri::Emitter;
                let _ = app.emit(
                    "auth:changed",
                    json!({ "active": false, "loggedIn": false, "email": "", "name": "" }),
                );
            }
            show_login(&app);
            Ok(json!({ "loggedIn": false, "active": false, "email": "", "name": "" }))
        }
        Ok(r) if r.status().is_success() => {
            let data: Value = r.json().await.unwrap_or_else(|_| json!({}));
            let active = data.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
            // If the license endpoint sends us fresher profile info
            // (email/name) than what we cached from the JWT, prefer it.
            if let Some(v) = data.get("email").and_then(|v| v.as_str()) {
                if !v.is_empty() && v != email.as_str() {
                    email = v.to_string();
                    settings_merge(&app, "bizgrowhubEmail", json!(email));
                }
            }
            if let Some(v) = data.get("name").and_then(|v| v.as_str()) {
                if !v.is_empty() && v != name.as_str() {
                    name = v.to_string();
                    settings_merge(&app, "bizgrowhubName", json!(name));
                }
            }
            if active {
                settings_merge(&app, "licenseOkAt", json!(now_ms()));
                on_licensed(&app);
            }
            Ok(json!({ "loggedIn": true, "active": active, "email": email, "name": name }))
        }
        _ => {
            // Offline grace: trust the last successful check for 7 days.
            let ok_at = s.get("licenseOkAt").and_then(|v| v.as_i64()).unwrap_or(0);
            let active = ok_at > 0 && (now_ms() - ok_at) < 7 * 24 * 60 * 60 * 1000;
            if active {
                on_licensed(&app);
            }
            Ok(json!({ "loggedIn": true, "active": active, "email": email, "name": name, "offline": true }))
        }
    }
}

#[tauri::command]
pub fn open_subscribe(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(format!("{}/marketplace", api_base()), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_register(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(format!("{}/register", api_base()), None::<&str>)
        .map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// App updates (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

/// Holds the `Update` returned by a check between the `update_info` →
/// `update_download` → `update_install` calls (the two-step "ask before
/// download / ask before restart" UX), plus the downloaded bytes.
#[derive(Default)]
pub struct PendingUpdate {
    pub update: Mutex<Option<tauri_plugin_updater::Update>>,
    pub bytes: Mutex<Option<Vec<u8>>>,
}

/// Check GitHub for a newer release (tauri-plugin-updater). Caches the `Update`
/// so download/install can use it. Mirrors `update:info` in index.ts.
#[tauri::command(rename_all = "camelCase")]
pub async fn update_info(app: tauri::AppHandle, force_check: Option<bool>) -> Result<Value, String> {
    use tauri_plugin_updater::UpdaterExt;
    let _ = force_check;
    let current = app.package_info().version.to_string();
    let pending = app.state::<PendingUpdate>();

    let result = match app.updater() {
        Ok(u) => u.check().await,
        Err(e) => return Err(e.to_string()),
    };
    match result {
        Ok(Some(update)) => {
            let version = update.version.clone();
            let notes: Vec<String> = update
                .body
                .clone()
                .map(|b| b.lines().map(str::to_string).filter(|l| !l.is_empty()).collect())
                .unwrap_or_default();
            let date = update.date.map(|d| d.to_string()).unwrap_or_default();
            if let Ok(mut g) = pending.update.lock() {
                *g = Some(update);
            }
            Ok(json!({
                "current": current,
                "latest": { "version": version, "notes": notes, "releasedAt": date },
                "updateAvailable": true,
                "downloaded": false
            }))
        }
        Ok(None) => {
            if let Ok(mut g) = pending.update.lock() {
                *g = None;
            }
            Ok(json!({ "current": current, "latest": Value::Null, "updateAvailable": false, "downloaded": false }))
        }
        // No feed / offline / parse error — report no update (matches Electron's
        // best-effort check that simply leaves latestUpdate null).
        Err(_) => Ok(json!({ "current": current, "latest": Value::Null, "updateAvailable": false, "downloaded": false })),
    }
}

/// Download the pending update, emitting `update:progress` per chunk and
/// `update:downloaded` on completion (mirrors index.ts download-progress flow).
#[tauri::command]
pub async fn update_download(app: tauri::AppHandle) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tauri::Emitter;

    // Take the cached Update out (can't hold the MutexGuard across .await).
    let update = app
        .state::<PendingUpdate>()
        .update
        .lock()
        .map_err(|_| "update lock poisoned")?
        .take();
    let Some(update) = update else {
        return Err("No update available — run update_info first.".into());
    };

    let total = Arc::new(AtomicU64::new(0));
    let transferred = Arc::new(AtomicU64::new(0));
    let app_ev = app.clone();
    let (t1, x1) = (total.clone(), transferred.clone());
    let bytes = update
        .download(
            move |chunk: usize, content_len: Option<u64>| {
                if let Some(cl) = content_len {
                    t1.store(cl, Ordering::Relaxed);
                }
                let x = x1.fetch_add(chunk as u64, Ordering::Relaxed) + chunk as u64;
                let tot = t1.load(Ordering::Relaxed);
                let percent = if tot > 0 { (x as f64 / tot as f64) * 100.0 } else { 0.0 };
                let _ = app_ev.emit_to(
                    "update",
                    "update:progress",
                    json!({ "percent": percent, "transferred": x, "total": tot }),
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;

    // Stash bytes + put the Update back for the install step.
    let pending = app.state::<PendingUpdate>();
    if let Ok(mut g) = pending.bytes.lock() {
        *g = Some(bytes);
    }
    if let Ok(mut g) = pending.update.lock() {
        *g = Some(update);
    }
    let _ = app.emit_to("update", "update:downloaded", ());
    Ok(())
}

/// Install the downloaded update and restart (mirrors `quitAndInstall`).
#[tauri::command]
pub fn update_install(app: tauri::AppHandle) -> Result<(), String> {
    let pending = app.state::<PendingUpdate>();
    let update = pending.update.lock().map_err(|_| "update lock poisoned")?.take();
    let bytes = pending.bytes.lock().map_err(|_| "update lock poisoned")?.take();
    let (Some(update), Some(bytes)) = (update, bytes) else {
        return Err("Nothing downloaded to install.".into());
    };
    update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}

#[tauri::command]
pub fn update_later(window: Window) {
    // Electron closes the update window. Mirror that loosely.
    if let Some(w) = window.get_webview_window("update") {
        let _ = w.close();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Live streaming (VoiceEngine) — 3-Tier Hybrid Transcription
// ─────────────────────────────────────────────────────────────────────────────
//
// Tier 1 (LIVE PARTIAL, blue text):
//   Every ~2s of accumulated audio, run the FASTEST available STT engine
//   (Groq / Local Whisper / OpenAI fallback). Scribe and ImprovedBangla are
//   explicitly SKIPPED here because their batch BizGrowHub proxies have 10-60s
//   latency. No GPT refine — raw STT is fine for a live tentative preview.
//   → emits `transcript:partial`
//
// Tier 2 (ACCURATE FINAL, black text):
//   Triggered by EITHER:
//     (a) VAD silence heuristic: ~3.5s since the last "meaningful" partial
//         (user paused speaking). Runs the premium-accurate pipeline on the
//         utterance accumulated so far, locks it in as final, then resets the
//         rolling buffer for the next utterance.
//     (b) is_final=true from the frontend (user pressed Stop). Same accurate
//         pipeline, but clears the whole session.
//   Respects ALL user settings: Scribe → ImprovedBangla (for Bangla) → provider.
//   → emits `transcript:final`
//
// Tier 3 (misc):
//   • dedupe: never run two partial/final jobs in parallel for the same session.
//   • silence gate: tiny/empty partials don't reset the "last voice" timestamp.
//
use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

/// Minimum bytes to accumulate before we're willing to kick off a fast partial
/// pass.  Kept deliberately small: one ~500 ms 16 kHz mono 16-bit WAV chunk is
/// roughly 16 KB, so one single chunk is enough to start.  We want the user to
/// see SOMETHING within ~1 s of starting to talk.
const MIN_PARTIAL_BYTES: usize = 14_000;
/// Minimum wall-clock gap between successive partial passes.  The provider-side
/// Whisper inference itself is ~0.7-1.2 s for short clips on Groq, so a ~900 ms
/// cadence means we're effectively streaming: next result comes back almost as
/// soon as the previous one rendered.  Google Live Transcribe targets ~300 ms;
/// 900 ms is the sweet spot between API rate limits and perceived live-ness.
const PARTIAL_INTERVAL_MS: i64 = 900;
/// Silence = "user finished this sentence".  Tuned quite tight: natural Bangla
/// speech has ~150-400 ms gaps between words, so anything >1.8 s without a
/// *new* meaningful partial is almost certainly the end of an utterance.  We
/// used to wait 3.5 s which felt like forever.
const SILENCE_TRIGGER_MS: i64 = 1800;
/// Minimum transcript length (chars) we consider "actual voice" for the
/// silence heuristic. Short blips ("ok", "uh", "ওহে") are ignored.
const MIN_MEANINGFUL_LEN: usize = 4;

#[derive(Default)]
pub(crate) struct StreamSession {
    /// All WAV chunks received since the last "final utterance boundary".
    /// Gets cleared after a silence-triggered final pass so the next sentence
    /// starts with a clean buffer (stop-triggered final removes the session).
    chunks: Vec<Vec<u8>>,
    /// Language the USER PICKED in the VoiceEngine dropdown — NOT the global
    /// settings `input_lang` default.  We ALWAYS prefer this over settings for
    /// live work, otherwise "Bangla" chosen in the window is silently ignored
    /// and Whisper wastes 500+ ms on auto-detect (often guessing English).
    language: String,
    provider: String,
    model: String,
    /// `now_ms()` timestamp of the last partial STT we actually kicked off
    /// (used with PARTIAL_INTERVAL_MS to rate-limit).
    last_partial_ms: i64,
    /// `now_ms()` timestamp of the last partial result that exceeded
    /// MIN_MEANINGFUL_LEN (used with SILENCE_TRIGGER_MS for VAD).
    last_voice_ms: i64,
    /// Guards against running two in-flight STT jobs (partial or final) for
    /// the same session.  Set to true under the lock just before spawning
    /// the task, set back to false under the lock after the task joins.
    in_flight: bool,
    /// Guards against running a silence-triggered final pass more than once
    /// while waiting for the accurate Tier-2 network round trip.  Once we
    /// emit a final for a given chunk window we consider the utterance
    /// "locked in" and stop re-triggering from the silence window.
    final_pending: bool,
}

#[derive(Default)]
pub(crate) struct StreamSessions(pub(crate) StdMutex<HashMap<String, StreamSession>>);

/// Concatenate a slice of WAV chunks into one contiguous byte buffer.
fn concat_chunks(chunks: &[Vec<u8>]) -> Vec<u8> {
    let total: usize = chunks.iter().map(|c| c.len()).sum();
    let mut buf = Vec::with_capacity(total);
    for c in chunks {
        buf.extend(c.as_slice());
    }
    buf
}

/// Build a PipelineOpts from the current settings + a raw WAV buffer.
///
/// `force_fast_partial_mode` = true disables Scribe, ImprovedBangla, and GPT
/// refine so the call comes back as fast as possible for Tier-1 live text.
///
/// `force_lang_override` = the VoiceEngine window's dropdown selection, if the
/// user explicitly picked one there.  When non-empty it ALWAYS wins over the
/// global settings `input_lang` default; otherwise we fall back to settings.
fn build_opts(
    s: &Value,
    buf: &[u8],
    _app: &tauri::AppHandle,
    force_fast_partial_mode: bool,
    force_lang_override: Option<&str>,
) -> crate::pipeline::PipelineOpts {
    use base64::Engine;
    let str_field = |k: &str, default: &str| -> String {
        match s.get(k).and_then(|v| v.as_str()) {
            Some(v) if !v.is_empty() => v.to_string(),
            _ => default.to_string(),
        }
    };

    let active = str_field("activeMode", "transcript");
    let mode_prompt = s
        .get("modes")
        .and_then(|m| m.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|m| m.get("id").and_then(|i| i.as_str()) == Some(active.as_str()))
        })
        .and_then(|m| m.get("prompt"))
        .and_then(|p| p.as_str())
        .unwrap_or("Fix ONLY spelling and grammar. Output ONLY the corrected text.")
        .to_string();
    let instructions = s
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let style_prompt = if instructions.is_empty() {
        mode_prompt
    } else {
        format!("{mode_prompt}\n\nAdditional user instructions: {instructions}")
    };

    let gpt_provider = str_field("gptProvider", "openai");
    let gpt_model = if gpt_provider == "custom" {
        let cm = str_field("customChatModel", "");
        if cm.is_empty() {
            str_field("gptModel", "gpt-4o-mini")
        } else {
            cm
        }
    } else {
        str_field("gptModel", "gpt-4o-mini")
    };

    let (use_scribe, use_better_bangla, skip_gpt) = if force_fast_partial_mode {
        (false, false, true)
    } else {
        (
            s.get("useScribe").and_then(|v| v.as_bool()).unwrap_or(false),
            s.get("useBetterBangla")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            s.get("skipGpt").and_then(|v| v.as_bool()).unwrap_or(true),
        )
    };

    // CRITICAL for perceived speed: VoiceEngine dropdown > global settings.
    // Without this, a user who picked "Bangla (বাংলা)" in the live window
    // gets auto-detect, and Whisper wastes 300-800 ms disambiguating, often
    // landing on English first and emitting a wrong partial.
    let settings_lang = str_field("inputLang", "auto");
    let input_lang = match force_lang_override {
        Some(v) if !v.trim().is_empty() && v != "auto" => v.trim().to_string(),
        _ => settings_lang,
    };

    // Custom vocabulary (names/jargon like "Next.js, Supabase, Prisma") is a
    // useful STT prompt bias but adds form-building overhead; for live
    // partials we skip it — the ~50 ms saved per call × 20 calls/min helps.
    let vocabulary = if force_fast_partial_mode {
        String::new()
    } else {
        str_field("vocabulary", "")
    };

    crate::pipeline::PipelineOpts {
        audio_base64: base64::engine::general_purpose::STANDARD.encode(buf),
        openai_key: decode_key(s, "openaiKeyEncrypted"),
        groq_key: decode_key(s, "groqKeyEncrypted"),
        stt_provider: str_field("sttProvider", "openai"),
        stt_model: str_field("sttModel", "whisper-1"),
        gpt_provider,
        gpt_model,
        openrouter_key: decode_key(s, "openrouterKeyEncrypted"),
        custom_key: decode_key(s, "customKeyEncrypted"),
        custom_base_url: str_field("customBaseUrl", ""),
        custom_headers: str_field("customHeaders", ""),
        input_lang,
        skip_gpt,
        style_prompt,
        vocabulary,
        use_scribe,
        use_better_bangla,
        auth_token: decode_key(s, "bizgrowhubTokenEncrypted"),
        api_base: api_base(),
    }
}

/// Tier 1 — fast partial STT. Returns the raw transcript text.
///
/// Priority order (fastest → slowest):
///   1. Local Whisper tiny/base (no network at all) if configured + downloaded
///   2. Groq whisper-large-v3 when a Groq key is present (~1s cloud STT)
///   3. OpenAI whisper-1 as last resort
///
/// Falls back to Ok("") when no provider has credentials available.
/// `pref_lang` comes from `StreamSession.language` set by the VoiceEngine start
/// call and overrides any global default.
async fn run_fast_partial(
    app: &tauri::AppHandle,
    buf: Vec<u8>,
    s: Value,
    pref_lang: String,
) -> Result<String, String> {
    let str_field = |k: &str, default: &str| -> String {
        match s.get(k).and_then(|v| v.as_str()) {
            Some(v) if !v.is_empty() => v.to_string(),
            _ => default.to_string(),
        }
    };

    // Apply the same dropdown > settings precedence here too.
    let resolved_lang = if pref_lang.trim().is_empty() || pref_lang == "auto" {
        str_field("inputLang", "auto")
    } else {
        pref_lang.clone()
    };

    // 1) Local whisper?
    let use_local = s
        .get("useLocalWhisper")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let local_model = str_field("localModel", "");
    let local_path = if use_local && !local_model.is_empty() {
        model_file(&local_model).and_then(|f| {
            let p = whisper_dir(app).ok()?.join(f);
            p.exists().then_some(p)
        })
    } else {
        None
    };
    if let Some(model_path) = local_path {
        let cache = app.state::<crate::whisper::WhisperCache>().inner().clone();
        let mp = model_path.to_string_lossy().to_string();
        let lang = resolved_lang.clone();
        return tauri::async_runtime::spawn_blocking(move || {
            cache.transcribe_local(&mp, &buf, Some(&lang))
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    }

    // 2) Groq? (fastest cloud STT — whisper-large-v3, ~1s latency)
    let groq_key = decode_key(&s, "groqKeyEncrypted");
    if !groq_key.is_empty() {
        let mut opts = build_opts(&s, &buf, app, true, Some(&pref_lang));
        opts.stt_provider = "groq".to_string();
        opts.stt_model = "whisper-large-v3".to_string();
        // Force-set language explicitly — Groq's Whisper benefits hugely from
        // a pinned "bn" for Bangla clips instead of auto-detect.
        if !resolved_lang.is_empty() && resolved_lang != "auto" {
            opts.input_lang = resolved_lang.clone();
        }
        if opts.groq_key.is_empty() {
            opts.groq_key = groq_key;
        }
        return crate::pipeline::run_pipeline(opts).await;
    }

    // 3) OpenAI whisper-1 fallback.
    let openai_key = decode_key(&s, "openaiKeyEncrypted");
    if !openai_key.is_empty() {
        let mut opts = build_opts(&s, &buf, app, true, Some(&pref_lang));
        opts.stt_provider = "openai".to_string();
        if !resolved_lang.is_empty() && resolved_lang != "auto" {
            opts.input_lang = resolved_lang.clone();
        }
        if opts.openai_key.is_empty() {
            opts.openai_key = openai_key;
        }
        return crate::pipeline::run_pipeline(opts).await;
    }

    Ok(String::new())
}

/// Tier 2 — accurate final STT. Uses the EXISTING full `run_pipeline` contract
/// exactly as the old code did: respects Scribe → ImprovedBangla → user STT
/// provider priority AND runs GPT refine / dictionary post-processing.
async fn run_accurate_final(
    app: &tauri::AppHandle,
    buf: Vec<u8>,
    s: Value,
    pref_lang: String,
) -> Result<String, String> {
    let str_field = |k: &str, default: &str| -> String {
        match s.get(k).and_then(|v| v.as_str()) {
            Some(v) if !v.is_empty() => v.to_string(),
            _ => default.to_string(),
        }
    };

    let use_local = s
        .get("useLocalWhisper")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let local_model = str_field("localModel", "");
    let local_path = if use_local && !local_model.is_empty() {
        model_file(&local_model).and_then(|f| {
            let p = whisper_dir(app).ok()?.join(f);
            p.exists().then_some(p)
        })
    } else {
        None
    };

    let opts = build_opts(&s, &buf, app, false, Some(&pref_lang));
    if let Some(model_path) = local_path {
        let cache = app.state::<crate::whisper::WhisperCache>().inner().clone();
        let mp = model_path.to_string_lossy().to_string();
        let lang = opts.input_lang.clone();
        let raw_res = tauri::async_runtime::spawn_blocking(move || {
            cache.transcribe_local(&mp, &buf, Some(&lang))
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        return match raw_res {
            Ok(raw) => {
                if raw.is_empty() || opts.skip_gpt {
                    Ok(raw)
                } else {
                    Ok(crate::pipeline::gpt_refine(&opts, raw).await)
                }
            }
            Err(e) => Err(e),
        };
    }
    crate::pipeline::run_pipeline(opts).await
}

#[tauri::command(rename_all = "camelCase")]
pub fn start_stream_session(
    app: tauri::AppHandle,
    session_id: String,
    opts: Option<Value>,
) -> Result<Value, String> {
    let mut sess = StreamSession::default();
    if let Some(o) = opts {
        sess.language = o
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("auto")
            .to_string();
        sess.provider = o
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("openai")
            .to_string();
        sess.model = o
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("whisper-1")
            .to_string();
    }
    let t0 = now_ms();
    // Seed "now" so the first chunk won't immediately fire a partial (we
    // want MIN_PARTIAL_BYTES + PARTIAL_INTERVAL_MS to accumulate first).
    sess.last_partial_ms = t0;
    sess.last_voice_ms = t0;
    if let Ok(mut map) = app.state::<StreamSessions>().0.lock() {
        map.insert(session_id, sess);
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn stream_audio_chunk(
    app: tauri::AppHandle,
    window: Window,
    audio_base64: String,
    session_id: String,
    meta: Option<Value>,
) -> Result<Value, String> {
    use tauri::Emitter;
    use base64::Engine;
    let _ = window;

    let is_final = meta
        .as_ref()
        .and_then(|m| m.get("isFinal"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // ── Step 1: decode + append chunk, capture session snapshot ──────────
    let now = now_ms();
    let sessions = app.state::<StreamSessions>();
    let (partial_ready, silence_ready, final_stop) = {
        let Ok(mut map) = sessions.0.lock() else {
            return Ok(json!({ "ok": true }));
        };
        let Some(sess) = map.get_mut(&session_id) else {
            return Ok(json!({ "ok": true }));
        };
        if let Some(bytes) = base64::engine::general_purpose::STANDARD
            .decode(audio_base64.as_bytes())
            .ok()
            .filter(|b| !b.is_empty())
        {
            sess.chunks.push(bytes);
        }
        let total_bytes: usize = sess.chunks.iter().map(|c| c.len()).sum();
        let interval_gap = now - sess.last_partial_ms;
        let silence_gap = now - sess.last_voice_ms;
        let pr = !sess.in_flight
            && !sess.final_pending
            && total_bytes >= MIN_PARTIAL_BYTES
            && interval_gap >= PARTIAL_INTERVAL_MS;
        let sr = !sess.in_flight
            && !sess.final_pending
            && total_bytes >= MIN_PARTIAL_BYTES
            && silence_gap >= SILENCE_TRIGGER_MS;
        if pr {
            sess.in_flight = true;
            sess.last_partial_ms = now;
        }
        if sr {
            sess.final_pending = true;
            sess.in_flight = true;
        }
        (pr, sr, is_final)
    };

    // ── Step 2a: Tier 1 — kick off a fast partial pass ────────────────────
    if partial_ready && !silence_ready && !final_stop {
        let app = app.clone();
        let sid = session_id.clone();
        let (buf, settings_snapshot, pref_lang) = {
            let sessions_inner = app.state::<StreamSessions>();
            let Ok(map) = sessions_inner.0.lock() else {
                let ss = app.state::<StreamSessions>();
                if let Ok(mut m) = ss.0.lock() {
                    if let Some(s) = m.get_mut(&sid) {
                        s.in_flight = false;
                    }
                }
                return Ok(json!({ "ok": true }));
            };
            let Some(sess) = map.get(&sid) else {
                return Ok(json!({ "ok": true }));
            };
            let buf = concat_chunks(&sess.chunks);
            let lang = sess.language.clone();
            let store = match app.store(SETTINGS_STORE).map_err(|e| e.to_string()) {
                Ok(st) => st,
                Err(_) => {
                    let ss = app.state::<StreamSessions>();
                    if let Ok(mut m) = ss.0.lock() {
                        if let Some(s) = m.get_mut(&sid) {
                            s.in_flight = false;
                        }
                    }
                    return Ok(json!({ "ok": true }));
                }
            };
            let snap = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
            (buf, snap, lang)
        };

        tauri::async_runtime::spawn(async move {
            let text = run_fast_partial(&app, buf, settings_snapshot, pref_lang)
                .await
                .unwrap_or_default()
                .trim()
                .to_string();

            let emit_partial = {
                let sessions_inner = app.state::<StreamSessions>();
                let Ok(mut map) = sessions_inner.0.lock() else {
                    return;
                };
                let Some(sess) = map.get_mut(&sid) else {
                    return;
                };
                sess.in_flight = false;
                let meaningful = text.chars().count() >= MIN_MEANINGFUL_LEN;
                if meaningful {
                    sess.last_voice_ms = now_ms();
                }
                let silence_now = now_ms() - sess.last_voice_ms >= SILENCE_TRIGGER_MS
                    && !meaningful;
                if silence_now && !sess.final_pending {
                    sess.final_pending = true;
                }
                !text.is_empty()
            };

            if emit_partial {
                let _ = app.emit_to(
                    "voiceengine",
                    "transcript:partial",
                    json!({ "sessionId": sid, "text": text }),
                );
            }
        });
    }

    // ── Step 2b: Tier 2 — silence-triggered accurate final pass ───────────
    if silence_ready && !final_stop {
        let app = app.clone();
        let sid = session_id.clone();
        let (buf, settings_snapshot, pref_lang) = {
            let sessions_inner = app.state::<StreamSessions>();
            let Ok(map) = sessions_inner.0.lock() else {
                let ss = app.state::<StreamSessions>();
                if let Ok(mut m) = ss.0.lock() {
                    if let Some(s) = m.get_mut(&sid) {
                        s.in_flight = false;
                        s.final_pending = false;
                    }
                }
                return Ok(json!({ "ok": true }));
            };
            let Some(sess) = map.get(&sid) else {
                return Ok(json!({ "ok": true }));
            };
            let buf = concat_chunks(&sess.chunks);
            let lang = sess.language.clone();
            if buf.len() < 1000 {
                drop(map);
                let ss = app.state::<StreamSessions>();
                if let Ok(mut m) = ss.0.lock() {
                    if let Some(s) = m.get_mut(&sid) {
                        s.in_flight = false;
                        s.final_pending = false;
                    }
                }
                return Ok(json!({ "ok": true }));
            }
            let store = match app.store(SETTINGS_STORE).map_err(|e| e.to_string()) {
                Ok(st) => st,
                Err(_) => {
                    let ss = app.state::<StreamSessions>();
                    if let Ok(mut m) = ss.0.lock() {
                        if let Some(s) = m.get_mut(&sid) {
                            s.in_flight = false;
                            s.final_pending = false;
                        }
                    }
                    return Ok(json!({ "ok": true }));
                }
            };
            let snap = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
            (buf, snap, lang)
        };

        tauri::async_runtime::spawn(async move {
            let raw = run_accurate_final(&app, buf, settings_snapshot.clone(), pref_lang)
                .await
                .unwrap_or_default();
            let final_text = apply_dictionary(&raw, &settings_snapshot);

            if !final_text.trim().is_empty() {
                let _ = app.emit_to(
                    "voiceengine",
                    "transcript:final",
                    json!({ "sessionId": sid, "text": final_text }),
                );
            }

            let t = now_ms();
            let ss = app.state::<StreamSessions>();
            let lock_res = ss.0.lock();
            if let Ok(mut map) = lock_res {
                if let Some(sess) = map.get_mut(&sid) {
                    sess.chunks.clear();
                    sess.last_partial_ms = t;
                    sess.last_voice_ms = t;
                    sess.in_flight = false;
                    sess.final_pending = false;
                }
            }
        });
    }

    // ── Step 2c: Tier 2 — stop-triggered accurate final pass (is_final) ───
    if final_stop {
        let store = app
            .store(SETTINGS_STORE)
            .map_err(|e| e.to_string())?;
        let s = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
        let sessions = app.state::<StreamSessions>();

        let maybe_buf = if let Ok(mut map) = sessions.0.lock() {
            map.remove(&session_id).map(|sess| {
                let pref_lang = sess.language.clone();
                (concat_chunks(&sess.chunks), pref_lang)
            })
        } else {
            None
        };

        if let Some((buf, pref_lang)) = maybe_buf {
            if buf.len() > 1000 {
                let sid = session_id.clone();
                let app = app.clone();
                let snap = s.clone();
                tauri::async_runtime::spawn(async move {
                    let raw = run_accurate_final(&app, buf, snap.clone(), pref_lang)
                        .await
                        .unwrap_or_default();
                    let final_text = apply_dictionary(&raw, &snap);
                    if !final_text.trim().is_empty() {
                        let _ = app.emit_to(
                            "voiceengine",
                            "transcript:final",
                            json!({ "sessionId": sid, "text": final_text }),
                        );
                    }
                });
            }
        }
    }

    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn end_stream_session(app: tauri::AppHandle, session_id: String) -> Result<Value, String> {
    if let Ok(mut map) = app.state::<StreamSessions>().0.lock() {
        map.remove(&session_id);
    }
    Ok(json!({ "ok": true }))
}

/// Focus or open the VoiceEngine live-transcript window. Exposed as a
/// dedicated command so Settings / tray / context-menu entries can summon it.
pub fn focus_or_open_voiceengine(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    if let Some(w) = app.get_webview_window("voiceengine") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "voiceengine", WebviewUrl::App("voiceEngine.html".into()))
        .title("BizVoice — Live Transcript")
        .inner_size(900.0, 680.0)
        .min_inner_size(620.0, 480.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn open_voice_engine(app: tauri::AppHandle) -> Result<(), String> {
    focus_or_open_voiceengine(&app)
}
