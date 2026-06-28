// BizVoice — Tauri Phase 0 spike.
//
// Goal of this file right now is ONLY to de-risk the migration:
//   1. Prove the Rust/MSVC toolchain + Tauri build works on this machine.
//   2. Prove the existing React UI renders in WebView2 (handled by tauri.conf.json
//      pointing the window at the already-built `login.html`).
//   3. Prove we can drive a Windows paste-injection from Rust (the `spike_paste`
//      command below) the same way the Electron main process does today.
//
// The full IPC surface is stubbed in `commands` (Phase 2 skeleton) and wired
// into the handler below; real logic is filled in per TAURI_MIGRATION.md.

mod commands;
mod overlay;
mod paste;
mod pipeline;
mod whisper;

/// Trivial round-trip so the frontend can confirm the Rust backend is reachable.
#[tauri::command]
fn ping() -> String {
    "pong from rust".into()
}

/// Phase 0 paste spike: stage `text` on the clipboard and send Ctrl+V to whatever
/// window currently has focus — mirrors how Electron's main process injects text.
/// The shipped version will reuse the existing SendInput PowerShell helper for
/// reliability; this is just enough to prove Rust can spawn it and it works.
#[tauri::command]
fn spike_paste(text: String) -> Result<String, String> {
    use std::process::Command;

    // Embed the text in a single-quoted PowerShell here-string (literal — no
    // interpolation). Doubling any lone `'@` terminator is overkill for a spike.
    let script = format!(
        "Set-Clipboard -Value @'\n{text}\n'@\nStart-Sleep -Milliseconds 300\nAdd-Type -AssemblyName System.Windows.Forms\n[System.Windows.Forms.SendKeys]::SendWait('^v')"
    );

    Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &script,
        ])
        .spawn()
        .map_err(|e| format!("failed to spawn powershell: {e}"))?;

    Ok("paste dispatched".into())
}


/// Dev-only: load the project-root `.env.local` (then `.env` as fallback) into the
/// process environment so the Rust backend honors the same `BIZGROWHUB_API` config
/// the old Electron main process read via dotenv. Tauri/cargo do NOT read .env files
/// on their own, so without this a `tauri dev` (debug) build always falls back to
/// `http://localhost:8080` in `api_base()` regardless of what .env.local says.
///
/// Precedence (highest first): real OS env var > .env.local > .env — we never
/// overwrite a key that is already set. Hand-rolled (no dotenv crate) to keep the
/// build offline-friendly and dependency-free, matching the rest of Cargo.toml.
/// Gated to debug builds so release never picks up a stray .env next to the exe.
#[cfg(debug_assertions)]
fn load_dotenv_dev() {
    // CARGO_MANIFEST_DIR is src-tauri/ (resolved at build time); the env files live
    // one level up at the project root next to package.json.
    let Some(root) = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(|p| p.to_path_buf())
    else {
        return;
    };
    // .env.local first so its values win over .env (first writer per key below).
    for name in [".env.local", ".env"] {
        let Ok(contents) = std::fs::read_to_string(root.join(name)) else {
            continue;
        };
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            // Strip optional surrounding single/double quotes from the value.
            let val = val.trim().trim_matches(|c| c == '"' || c == '\'');
            // Don't clobber a real OS env var or an earlier (.env.local) entry.
            if !key.is_empty() && std::env::var_os(key).is_none() {
                std::env::set_var(key, val);
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Pull BIZGROWHUB_API (and friends) from .env.local/.env in dev so the redirect
    // host matches the configured backend instead of the localhost debug fallback.
    #[cfg(debug_assertions)]
    load_dotenv_dev();

    tauri::Builder::default()
        // single-instance MUST be registered first; focuses the existing app
        // instead of launching a second copy.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("micbar") {
                let _ = w.show();
                let _ = w.set_focus();
            } else if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Auto-update via tauri-plugin-updater. Configured in tauri.conf.json
        // (`plugins.updater` pubkey + GitHub release endpoint). Releases must be
        // signed with TAURI_SIGNING_PRIVATE_KEY at `tauri build` time.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(
            // Phase 3: global hotkeys → emit events to the mic-bar window, which
            // MicBar.tsx listens to via window.api.onHotkey/onPttStart/onPttStop.
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    use tauri::{Emitter, Manager};
                    use tauri_plugin_global_shortcut::ShortcutState;
                    let pressed = event.state() == ShortcutState::Pressed;
                    // Map the incoming shortcut back to its action via the
                    // user-configured set held in HotkeyState.
                    let (is_toggle, is_ptt, is_cycle) =
                        match app.try_state::<commands::HotkeyState>() {
                            Some(state) => match state.lock() {
                                Ok(hk) => (
                                    hk.toggle.as_ref() == Some(shortcut),
                                    hk.ptt.as_ref() == Some(shortcut),
                                    hk.cycle.as_ref() == Some(shortcut),
                                ),
                                Err(_) => (false, false, false),
                            },
                            None => (false, false, false),
                        };
                    if is_toggle {
                        if pressed {
                            let _ = app.emit_to("micbar", "hotkey:toggle", ());
                        }
                    } else if is_ptt {
                        let ev = if pressed { "hotkey:ptt-start" } else { "hotkey:ptt-stop" };
                        let _ = app.emit_to("micbar", ev, ());
                    } else if is_cycle {
                        if pressed {
                            commands::cycle_mode(app);
                        }
                    }
                })
                .build(),
        )
        // Route menu clicks from the mic-bar context menu (popup) through the
        // shared handler. The tray menu has its own handler wired in build_tray.
        .on_menu_event(|app, event| commands::handle_menu(app, event.id.as_ref()))
        .manage(commands::HotkeyState::default())
        .manage(paste::PasteWorker::default())
        .manage(whisper::WhisperCache::default())
        .manage(commands::PendingUpdate::default())
        .setup(|app| {
            // Register the user's configured hotkeys (toggle / PTT / cycle) read
            // from settings, recording them in HotkeyState for the handler above.
            commands::register_hotkeys(app.handle());
            // System tray (Show Mic Bar / Settings / Check for Updates / Quit).
            commands::build_tray(app.handle())?;
            // Keep the transparent mic-bar overlay pinned topmost + non-activating.
            overlay::start_micbar_pin(app.handle());
            // Gate startup: show the mic bar if locally licensed, else login.
            commands::startup_gate(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Phase 0 spikes
            ping,
            spike_paste,
            // Phase 2 skeleton — settings & windows
            commands::get_settings,
            commands::set_settings,
            commands::open_settings,
            commands::close_window,
            commands::minimize_window,
            // Core transcribe + paste (Phase 4)
            commands::transcribe,
            commands::paste,
            // Mic bar window (Phase 3)
            commands::mic_bar_context_menu,
            commands::resize_mic_bar,
            commands::get_win_pos,
            commands::set_win_pos,
            commands::mute_system,
            // History & stats (Phase 2)
            commands::get_history,
            commands::clear_history,
            commands::refine_text,
            commands::get_refined_cache,
            commands::get_stats,
            // Local whisper models (Phase 4)
            commands::whisper_list_models,
            commands::whisper_download_model,
            commands::whisper_delete_model,
            // Auth + license (Phase 5)
            commands::start_browser_login,
            commands::cancel_browser_login,
            commands::logout,
            commands::auth_status,
            commands::open_subscribe,
            commands::open_register,
            // App updates (Phase 5)
            commands::update_info,
            commands::update_download,
            commands::update_install,
            commands::update_later
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            // Tray-resident model: closing the last window must NOT quit the app
            // (mirrors Electron's no-op `window-all-closed`). A real quit comes
            // from the tray "Quit" item via app.exit(0), which sets `code` and is
            // therefore allowed through.
            tauri::RunEvent::ExitRequested { code, api, .. } => {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            // On actual exit, kill the resident paste worker so no orphan
            // PowerShell process is left behind (mirrors index.ts `before-quit`).
            tauri::RunEvent::Exit => paste::kill_worker(app),
            _ => {}
        });
}
