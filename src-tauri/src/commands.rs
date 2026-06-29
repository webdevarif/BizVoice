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
        "skipGpt": false,
        "launchOnStartup": false,
        "micDeviceId": "",
        "vocabulary": "",
        "modes": default_modes(),
        "activeMode": "transcript",
        "silenceMs": 1200,
        "autoStop": true,
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
pub fn local_license_ok(app: &tauri::AppHandle) -> bool {
    let Ok(store) = app.store(SETTINGS_STORE) else {
        return false;
    };
    let s = store.get(SETTINGS_KEY).unwrap_or_else(default_settings);
    let has_token = !decode_key(&s, "bizgrowhubTokenEncrypted").is_empty();
    let ok_at = s.get("licenseOkAt").and_then(|v| v.as_i64()).unwrap_or(0);
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
        skip_gpt: s.get("skipGpt").and_then(|v| v.as_bool()).unwrap_or(false),
        style_prompt,
        vocabulary: str_field("vocabulary", ""),
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
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("");
            let (token, st) = parse_callback(path);
            let ok = !token.is_empty() && st == state;

            let body = if ok {
                "<h2>BizVoice connected \u{2713}</h2><p>You can close this tab and return to the app.</p>"
            } else {
                "<h2>Invalid sign-in</h2><p>Please retry from the BizVoice app.</p>"
            };
            let html = format!(
                "<!doctype html><html><body style=\"font-family:system-ui;background:#0A0A0F;color:#fff;display:flex;align-items:center;justify-content:center;height:100vh;margin:0\"><div style=\"text-align:center\">{body}</div></body></html>"
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(resp.as_bytes());

            if ok {
                if let Ok(store) = app2.store(SETTINGS_STORE) {
                    use base64::Engine;
                    let enc = base64::engine::general_purpose::STANDARD.encode(token.as_bytes());
                    store.set("bizgrowhubTokenEncrypted", json!(enc));
                    let _ = store.save();
                }
                let _ = app2.emit(
                    "auth:changed",
                    json!({ "active": true, "loggedIn": true, "email": "" }),
                );
                // Reveal the mic bar / dismiss login on a successful sign-in.
                on_licensed(&app2);
            }
        }
    });

    Ok(json!({ "ok": true }))
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
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    store.set("bizgrowhubTokenEncrypted", json!(""));
    store.set("licenseOkAt", json!(0));
    store.save().map_err(|e| e.to_string())?;
    if let Some(w) = app.get_webview_window("micbar") {
        let _ = w.hide();
    }
    // Return the user to the sign-in window (mirrors the re-lock in index.ts).
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
    let email = s
        .get("bizgrowhubEmail")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if token.is_empty() {
        return Ok(json!({ "loggedIn": false, "active": false, "email": "" }));
    }
    let resp = reqwest::Client::new()
        .get(format!("{}/api/bizvoice/license", api_base()))
        .bearer_auth(&token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;
    match resp {
        Ok(r) if r.status().as_u16() == 401 => {
            store.set("bizgrowhubTokenEncrypted", json!(""));
            let _ = store.save();
            Ok(json!({ "loggedIn": false, "active": false, "email": "" }))
        }
        Ok(r) if r.status().is_success() => {
            let data: Value = r.json().await.unwrap_or_else(|_| json!({}));
            let active = data.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
            if active {
                store.set("licenseOkAt", json!(now_ms()));
                let _ = store.save();
                on_licensed(&app);
            }
            Ok(json!({ "loggedIn": true, "active": active, "email": email }))
        }
        _ => {
            // Offline grace: trust the last successful check for 7 days.
            let ok_at = s.get("licenseOkAt").and_then(|v| v.as_i64()).unwrap_or(0);
            let active = ok_at > 0 && (now_ms() - ok_at) < 7 * 24 * 60 * 60 * 1000;
            if active {
                on_licensed(&app);
            }
            Ok(json!({ "loggedIn": true, "active": active, "email": email, "offline": true }))
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
