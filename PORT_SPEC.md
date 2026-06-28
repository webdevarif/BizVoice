# PORT_SPEC.md — BizVoice: Electron → Tauri v2 Migration Specification

> **Scope:** Port the BizVoice desktop app from Electron to Tauri v2 with **ZERO design changes**. The React + Tailwind frontend (`src/**`) renders identically in WebView2; only the `window.api` bridge and the entire main-process backend (`electron/main/index.ts` @ 1429 ln, `localWhisper.ts` @ 144 ln, `pipeline.ts` @ 306 ln) change.
> **Authoritative source files:** `src-tauri/` scaffold already exists (Phase 0 spike: `tauri.conf.json`, `Cargo.toml` @ v0.1.25, `src/lib.rs`, `src/commands.rs`, `capabilities/default.json`). `TAURI_MIGRATION.md` defines phases 0–6.

---

## Executive summary

**Portability verdict: HIGH.** Every one of the 27 `ipcMain.handle` channels, 33 `window.api` members, and 9 `webContents.send` push events has a direct Tauri v2 equivalent. The frontend (37 real call sites + 3 type-only refs across `MicBar.tsx`/`Settings.tsx`) touches **only** `window.api` plus standard Web APIs (`navigator.mediaDevices`, `AudioContext`, `FileReader`, `navigator.clipboard`) that work identically in WebView2. A single side-effect shim (`src/lib/api.ts`) that re-assigns `window.api` keeps all call sites **byte-for-byte unchanged**, so the design is untouched. The IPC plumbing is trivial; the difficulty is concentrated in a handful of native subsystems.

**The 3–4 hardest items (front-load these):**

1. **`paste` / text-injection** (`high`) — a long-lived PowerShell SendInput worker with per-app strategy detection (Shift+Insert for editors/terminals, Unicode char-typing for browsers/Chromium hosts, Ctrl+V for native apps) using `GetForegroundWindow`/`GetClassName`/`GetModuleBaseName`. `enigo` does NOT do foreground-window class/process detection — that still needs the `windows` crate, or keep the PS helper.
2. **`transcribe` pipeline** (`high`) — the entire `pipeline.ts` (multi-provider STT + GPT refine + Better-Bangla proxy + local whisper) must move to Rust to keep decrypted API keys out of the WebView.
3. **Local whisper audio resampling** (`high`-risk) — `whisper-rs` requires **16 kHz mono f32 PCM** but the primary MediaRecorder path records at the mic's native rate (44.1/48 kHz). The existing `lib.rs` spike decodes + downmixes but **does NOT resample** → silent mis-transcription. Must add `rubato` OR pin the recorder to 16 kHz.
4. **Mic-bar overlay + secrets** — transparent, non-focusable, always-on-top "screen-saver" overlay re-pinned every 1.5 s (no `moveTop()` in Tauri → raw `SetWindowPos` via `windows` crate); and `safeStorage` (Windows DPAPI) has **no drop-in** — existing encrypted blobs won't decrypt under stronghold/keyring, forcing a re-login/re-key migration.

**One-line size win:** Replacing bundled Chromium with the system WebView2 cuts the installer from **~170–215 MB → ~25–30 MB** with no UI change.

---

## Complete IPC inventory

> **Direction:** `invoke` = renderer→main request/response; `event` = main→renderer push (`webContents.send` → Tauri `emit`/`listen`). Tauri wraps invoke args in a **named object** (e.g. `invoke('settings_set', { patch })`). Recommended event names keep the colon form verbatim (Tauri event names are arbitrary strings); the shim is the only place they appear.

### Request/response commands (invoke)

| Channel (`window.api` member) | Args → Return | Tauri equivalent | Permission | Effort |
|---|---|---|---|---|
| `auth:status` (`authStatus()`) | `()` → `{loggedIn,active,email,offline?}` | `auth_status` cmd; reqwest GET `/api/bizvoice/license`; 7-day offline grace; runs `onLicensed()` side-effect (create/show mic bar, register shortcuts) | allow `auth_status`; window create/show | medium |
| `auth:startBrowserLogin` (`startBrowserLogin()`) | `()` → `{ok,error?}` | `start_browser_login` cmd; Rust loopback HTTP (`tiny_http`/`axum` on 127.0.0.1:0) OR deep-link; `tauri-plugin-shell` open; emits `auth:changed` | shell/opener:allow-open (bizgrowhub host); allow cmd | high |
| `auth:cancelBrowserLogin` (`cancelBrowserLogin()`) | `()` → `{ok:true}` | `cancel_browser_login`; drop loopback listener handle in managed state | allow cmd | low |
| `auth:logout` (`logout()`) | `()` → `{ok:true}` | `logout`; clear token (stronghold/keyring), reset `licenseOkAt`, hide mic bar; **emit `auth:changed`** | core:window:allow-hide; allow cmd | low |
| `auth:openSubscribe` (`openSubscribe()`) | `()` → `void` | shell/opener `open(${API}/marketplace)` (can be frontend-direct) | opener/shell:allow-open scoped `https://bizgrowhub.shop/*` | low |
| `auth:openRegister` (`openRegister()`) | `()` → `void` | shell/opener `open(${API}/register)` | opener/shell:allow-open scoped | low |
| `update:info` (`updateInfo(force?)`) | `(forceCheck?)` → `{current,latest:{version,notes?,releasedAt?}|null,updateAvailable,downloaded}` | `update_info`; `tauri-plugin-updater` `check()` + `getVersion()`; cache `Update` in managed state | updater:default; allow cmd | medium |
| `update:download` (`updateDownload()`) | `()` → `void` (emits `update:progress`/`update:downloaded`) | `update_download`; `update.download(on_chunk)`; compute `{percent,transferred,total}` from chunks | updater:allow-download; allow cmd | medium |
| `update:install` (`updateInstall()`) | `()` → `void` (quits) | `update_install`; `update.install()` + `app.restart()` from cached handle | updater:allow-install; process:allow-restart | medium |
| `update:later` (`updateLater()`) | `()` → `void` | `update_later`; close the `update` window | core:window:allow-close | low |
| `micBar:contextMenu` (`micBarContextMenu()`) | `()` → `void` | `micbar_context_menu`; `tauri::menu::Menu` + `window.popup_menu()` (Settings/Hide/Quit) | core:menu / core:app:allow-exit; allow cmd | medium |
| `micBar:getPos` (`getWinPos()`) | `()` → `[x,y]` | `getCurrentWindow().outerPosition()` (frontend-direct, no cmd) | core:window:allow-outer-position | low |
| `micBar:setPos` (`setWinPos(x,y)`) | `(x,y)` → `void` | `getCurrentWindow().setPosition(PhysicalPosition)` (frontend-direct) | core:window:allow-set-position | low |
| `micBar:resize` (`resizeMicBar(active)`) **DEAD** | `(active)` → `Promise<void>` (**no handler in main today**) | **Drop it** (recommended) or `resize_mic_bar` → `set_size` | core:window:allow-set-size (only if revived) | low |
| `audio:mute` (`muteSystem(mute)`) | `(mute)` → `void` (fire-and-forget) | `audio_mute`; `windows` crate `IAudioEndpointVolume::SetMute` OR PS via `std::process`; debounce flag in managed state | allow cmd; shell:allow-execute if PS | medium |
| `settings:get` (`getSettings()`) | `()` → large `Settings` DTO (~30 fields + `hasKey`/`hasGroqKey`/`hasOpenrouterKey`/`hasCustomKey`) | `settings_get`; reqwest GET `/api/bizvoice/settings`; merge into `Mutex<Settings>` managed state; `hasKey` flags from secure-store presence | allow cmd; store:allow-get if store-backed | medium |
| `settings:set` (`setSettings(patch)`) | `(patch)` → `{ok}|{ok:false,error}` | `settings_set`; encrypt+strip key fields → stronghold/keyring; PUT remote; **re-register shortcuts on hotkey change; emit `appearance:changed` on theme/widgetStyle change** | global-shortcut:allow-register/unregister; allow cmd | medium |
| `settings:open` (`openSettings()`) | `()` → `void` | `open_settings`; focus existing `settings` window or build from `settings.html` (singleton focus) | core:webview:allow-create-webview-window / window:allow-show+set-focus | low |
| `window:close` (`closeWindow()`) | `()` → `void` | `getCurrentWindow().close()` (frontend-direct) | core:window:allow-close | low |
| `window:minimize` (`minimizeWindow()`) | `()` → `void` | `getCurrentWindow().hide()` — **HIDE, not minimize** (preserve semantic) | core:window:allow-hide | low |
| `transcribe` (`transcribe(b64,durMs?)`) | `(audioBase64, durationMs?)` → `string` (throws→reject w/ `.message`) | `transcribe`; port whole `pipeline.ts` to Rust (reqwest multipart + whisper-rs); apply dictionary; POST history | allow cmd | high |
| `paste` (`paste(text)`) | `(text)` → `void` | `paste`; per-OS injection (Win SendInput + foreground detection; mac osascript; linux xdotool) | clipboard-manager:allow-read/write-text; allow cmd; macOS Accessibility | high |
| `history:get` (`getHistory()`) | `()` → `{text,ts,words,durationMs}[]` | `history_get`; reqwest GET `?limit=200` | allow cmd | low |
| `history:clear` (`clearHistory()`) | `()` → `void` | `history_clear`; DELETE + clear local `refinedCache` | allow cmd; store:allow-clear | low |
| `history:refine` (`refineText(text,ts)`) | `(text, ts)` → `string` (throws if no key; `.message`) | `history_refine`; provider-select+fallback (openai/groq/openrouter/custom) chat completion; cache by `ts` | allow cmd | medium |
| `history:getRefinedCache` (`getRefinedCache()`) | `()` → `Record<number,string>` | `history_get_refined_cache`; `HashMap<i64,String>` from store | allow cmd; store:allow-get | low |
| `stats:get` (`getStats()`) | `()` → `{recordings,words,durationMs}` | `stats_get`; reqwest GET; default struct on error | allow cmd | low |
| `whisper:listModels` (`whisperListModels()`) | `()` → `{name,size,downloaded}[]` | `whisper_list_models`; scan `app_data_dir/whisper-models` | allow cmd; fs scope | low |
| `whisper:downloadModel` (`whisperDownloadModel(name)`) | `(name)` → `{ok,error?}` (emits `whisper:downloadProgress`) | `whisper_download_model`; reqwest streamed HF download + `app.emit` (throttle to integer pct) | allow cmd; fs write scope | medium |
| `whisper:deleteModel` (`whisperDeleteModel(name)`) | `(name)` → `boolean` | `whisper_delete_model`; `std::fs::remove_file` | allow cmd; fs remove scope | low |

### Push events (main → renderer; emit → `listen`)

| Event (`window.api` member) | Payload → cb | Tauri equivalent | Permission | Effort |
|---|---|---|---|---|
| `hotkey:toggle` (`onHotkey`) | none → `()=>void` | global-shortcut handler → `emit_to('micbar','hotkey:toggle')` | global-shortcut:allow-register; core:event | medium |
| `hotkey:ptt-start` (`onPttStart`) | none → `()=>void` | global-shortcut `ShortcutState::Pressed` → emit (**deletes PowerShell GetAsyncKeyState poll**) | global-shortcut:allow-register; core:event | medium |
| `hotkey:ptt-stop` (`onPttStop`) | none → `()=>void` | global-shortcut `ShortcutState::Released` → emit (native, no PS) | global-shortcut:allow-register; core:event | medium |
| `mode:changed` (`onModeChange`) **exposed, no call site in MicBar/Settings** | `{id,name,color,prompt}` → `(mode)=>void` | cycle handler: advance `activeMode` in state, push remote, emit | global-shortcut:allow-register; core:event | low |
| `appearance:changed` (`onAppearanceChange`) | `{theme,widgetStyle}` → `(d)=>void` | inside `settings_set` → `emit_to('micbar','appearance:changed', payload)` | core:event:allow-listen | low |
| `auth:changed` (`onAuthChange`) | `{active,loggedIn,email}` → `(d)=>void` | after login callback → `emit_to('login','auth:changed', payload)` | core:event:allow-listen | low |
| `update:progress` (`onUpdateProgress`) **no call site in 2 files** | `{percent,transferred,total}` → `(d)=>void` | updater `on_chunk` → `emit_to('update','update:progress', payload)` | updater:allow-download; core:event | medium |
| `update:downloaded` (`onUpdateDownloaded`) **exposed, no call site** | none → `()=>void` | after `download()` completes → emit; native fallback `tauri-plugin-dialog ask()` | updater:allow-download; dialog:allow-ask | medium |
| `whisper:downloadProgress` (`onWhisperDownloadProgress`) | `{model,pct}` → `(d)=>void` | inside `whisper_download_model` → `app.emit` | core:event:allow-listen | low |

**Critical contract:** all 9 subscription members must return a **synchronous** unsubscribe function. Tauri's `listen()` returns `Promise<UnlistenFn>` — the shim must defer: `onX(cb){ const p = listen('x', e=>cb(e.payload)); return () => { p.then(u=>u()); }; }`. `Settings.tsx:181` does `return off;` directly as effect cleanup — a returned Promise would silently leak + double-fire.

---

## Native & OS APIs

| Electron API | Used for | Tauri plugin / crate | Permission | Risk |
|---|---|---|---|---|
| `globalShortcut` | 3 hotkeys (toggle/PTT/cycle) | `tauri-plugin-global-shortcut` | global-shortcut:allow-register/unregister/unregister-all | Accelerator string format differs; PTT uses press/release `ShortcutState` (v2) → **deletes PS key-poll** |
| `Tray` + `Menu` | system tray + mic-bar context menu | core `TrayIconBuilder` + `tauri::menu` (Cargo feature `tray-icon`) | none (Rust-built); core:tray/menu if JS-driven | Menu must be kept alive in app state or dropped; left/right-click + popup anchoring re-test |
| `nativeImage` | tray icon load + 16×16 resize | core `tauri::image::Image` / `include_bytes!` | none | **No runtime resize** — ship a pre-sized 16×16 PNG |
| `safeStorage` | encrypt auth token + 4 provider keys | `tauri-plugin-stronghold` **or** `keyring` crate | stronghold:default (if stronghold) | **HIGHEST-RISK.** DPAPI blobs won't decrypt under new scheme → re-login/re-key migration; stronghold needs password/salt strategy |
| `clipboard` | paste staging (save/stage/restore) | `tauri-plugin-clipboard-manager` | clipboard-manager:allow-read-text/write-text | Restore-race already fragile in Electron; same caveats |
| `systemPreferences.askForMediaAccess` | macOS mic permission only | Info.plist `NSMicrophoneUsageDescription` (bundle); optional `tauri-plugin-macos-permissions` | OS-level (not a Tauri capability) | Windows (primary target) needs nothing; macOS crashes on mic access without the usage string |
| `screen` | mic-bar placement (work area) | core `app.primary_monitor()` / `current_monitor()` | core:window default | Work-area vs full-size differs per platform; taskbar-offset math may need adjustment |
| `shell.openExternal` | auth/subscribe/register browser links | `tauri-plugin-opener` (preferred) or `tauri-plugin-shell` | opener:allow-open-url scoped to bizgrowhub origin + localhost dev | Open denied unless URL scoped in allowlist |
| `dialog.showMessageBox` | update "Restart Now/Later" prompt | `tauri-plugin-dialog` `ask()`/`message()` | dialog:allow-ask/allow-message | Coupled to updater migration |
| `session` (onHeadersReceived / permission handlers) | COOP/COEP for SharedArrayBuffer (ONNX/vad-web); mic grant | `tauri.conf.json` `app.security.headers` or `on_web_resource_request` | none (config-level) | **SharedArrayBuffer parity risk** — COOP `same-origin` + COEP `credentialless` must be served on the asset protocol or local whisper/VAD breaks; permission handler unneeded (OS prompt) |
| `http.createServer` | loopback OAuth token handoff | `tiny_http`/`axum` on tokio task | none (Rust networking) | Replicate ephemeral port-0 bind + `randomBytes(16)` state CSRF + one-shot lifecycle |
| `child_process exec` — PTT key poll PS (`GetAsyncKeyState`) | detect PTT key release | `std::process::Command` (script unchanged) **or removed** via plugin Released event | shell:allow-execute if plugin-shell; none for std | Script generated at runtime to `%TEMP%`, not bundled; **best deleted** in favor of native Released event |
| `child_process exec` — mute PS (Core Audio COM) | `audio:mute` toggle | `std::process::Command` (PS unchanged) **or** `windows` crate native COM | none for std | Per-toggle PowerShell startup cost; native COM is fiddly (apartments/GUIDs) |
| `child_process spawn` — long-lived SendInput typer (~230 ln C#/PS) | `paste` per-app strategy | `std::process::Command` w/ piped stdin (script unchanged) **or future** `enigo`+`windows` crate | shell:allow-spawn if sidecar; none for std | **Largest native asset**, runtime-generated; lifecycle (restart on crash, kill on quit) + per-editor timing must be reproduced |
| `osascript` (mac) / `xdotool` (linux) | paste on non-Windows | `std::process::Command` | none | xdotool is an external dep the user must install |
| `launchOnStartup` | autostart toggle — **STORED BUT NEVER APPLIED** (no `setLoginItemSettings`) | `tauri-plugin-autostart` `enable()/disable()/is_enabled()` | autostart:allow-enable/disable/is-enabled | Currently a no-op bug; porting **fixes** it — confirm desired. Windows Run key vs macOS LaunchAgent; launch with `--hidden` to match tray-resident model |

**Note on runtime-generated PowerShell:** the three PS scripts (`PTT_POLL_SCRIPT` line 618, mute line 766, `TYPE_PS_SCRIPT` line 1113) are TypeScript string literals written to `%TEMP%` at runtime — they do **not** exist on disk to bundle. Their bodies are pure Win32/COM and port unchanged; keep them as Rust string constants written to a temp path, or bundle as Tauri resources.

---

## Windows & lifecycle

The app creates **four windows** + a tray, all gated behind a BizGrowHub login/license. There is **no auth popup window** (sign-in is browser-based via loopback). Lifecycle is **tray-resident**: `window-all-closed` is intentionally a no-op. The existing scaffold declares only one placeholder `main` window pointing at `login.html` — all four real windows still need to be defined.

### Per-window spec

| Window | Geometry / flags (Electron) | Tauri definition | Permissions | Effort |
|---|---|---|---|---|
| **micBar (overlay) — HIGHEST RISK** | 210×46 (`MICBAR_IDLE`); `frame:false, transparent:true, resizable:false, hasShadow:false, focusable:false, skipTaskbar:true, alwaysOnTop:true`; pinned `'screen-saver'` level + `moveTop()` re-asserted every **1500 ms**; centered, y = bottom−12; lazy-created in `onLicensed()`; loads `index.html` | Window in conf: `decorations:false, transparent:true, resizable:false, shadow:false, focus:false, skipTaskbar:true, alwaysOnTop:true`; **Cargo `transparent` feature**; re-assert loop via `std::thread`/tokio interval calling `set_always_on_top(true)` + raw `SetWindowPos(HWND_TOPMOST, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)` (`windows` crate, since Tauri has no `moveTop()`); ensure `WS_EX_NOACTIVATE` | core:window allow-set-always-on-top, set-position, set-skip-taskbar, show, hide, start-dragging | **high** |
| **settingsWin** | 720×580, min 600×420; `frame:false, resizable:true, movable:true, bg #1a1a1f`; JS singleton (focus if exists); loads `settings.html`; `window:minimize` actually **hides** | `WebviewWindowBuilder('settings', App('settings.html'))` 720×580, `min_inner_size(600,420)`, `decorations:false, resizable:true`; "focus existing" = `get_webview_window('settings').set_focus()`; drag via `data-tauri-drag-region` | core:window allow-close, hide, set-focus, start-dragging; webview allow-create | medium |
| **loginWin** | 720×560, `frame:false, resizable:false, bg #0c0c11`; opened when license inactive; closed in `onLicensed()`; receives `auth:changed` | `WebviewWindowBuilder('login', App('login.html'))` 720×560, `decorations:false, resizable:false`; natural startup window | core:window allow-create/close/set-focus; core:event allow-emit/listen | medium |
| **updateWin** | 520×560, `frame:false, resizable:false, bg #0c0c11`; opened by updater `update-available`; receives `update:progress`/`update:downloaded`; native dialog fallback | `WebviewWindowBuilder('update', App('update.html'))` 520×560; whole stack → `tauri-plugin-updater`; events via `window.emit`; fallback → `tauri-plugin-dialog` | updater:default; dialog:allow-message/ask; process:allow-restart; core:window allow-create/close | high |

### Lifecycle items

| Item | Electron | Tauri | Notes |
|---|---|---|---|
| `app.whenReady` startup | COOP/COEP headers, mic grant, whisper dir, pre-warm typer (`ensureTypeProc`), `createTray`, `checkLicense`→`onLicensed`/`openLogin`, update check +3 s & hourly, license re-verify every 6 h | `Builder::setup(...)` closure; COOP/COEP via `security.headers`; timers via `tauri::async_runtime::spawn` tokio intervals | SharedArrayBuffer requires cross-origin isolation on the `tauri://` protocol |
| `window-all-closed` (no-op, stay in tray) | intentional no-op | `RunEvent::ExitRequested { api } => api.prevent_exit()` | Tauri quits by default — must explicitly prevent exit |
| `will-quit` / `before-quit` | `globalShortcut.unregisterAll()`; `typeProc.kill()` | `RunEvent::Exit`: unregister shortcuts; kill spawned helper + loopback server | Avoid orphan processes |
| **Single-instance — ABSENT today** | none (`requestSingleInstanceLock` never called) | **ADD** `tauri-plugin-single-instance` (register FIRST in Builder); focus mic bar if licensed else login | New capability, not a port |
| **micBar position — NOT persisted** | re-centered every launch; `micBar:getPos/setPos` drive drag for mic bar **and** settings | Keep re-center (build position) OR **improve** with `tauri-plugin-window-state`; drag via `data-tauri-drag-region` | Window-state plugin **changes** behavior (remembers drag) — confirm desired |
| **launchOnStartup — no-op today** | stored/synced, never applied | `tauri-plugin-autostart` (makes it functional) | Behavior change/fix |
| Dev vs packaged loading | `VITE_DEV_SERVER_URL` branch; `loadFile(dist/*.html)`; `app.isPackaged` gates updater + `BIZGROWHUB_API` | `devUrl`/`frontendDist` (already in conf); `WebviewUrl::App('x.html')` auto-resolves; `cfg!(debug_assertions)`/`is_dev()` for API switch | Keep Vite 4-entry rollup input (micBar=`index.html`, settings, login, update) so `dist/` has all 4 HTML files |

---

## Transcription core

### A. Local whisper (`whisper-rs`)

| Concern | Status / decision |
|---|---|
| **Model compatibility** | `whisper-rs` consumes the **same** `ggml-*.bin` models (9-model catalog, tiny→large-v3-turbo) downloaded from HF `ggerganov/whisper.cpp`. **No re-download.** Keep on-disk filenames byte-identical; move models dir from `app.getPath('userData')/whisper-models` → `app_data_dir/whisper-models` (migrate or keep stable path). |
| **Inference** | Already prototyped in `lib.rs:59-115` (gated `whisper-spike` feature). `WhisperContext::new_with_params` + `create_state`; `FullParams` Greedy; `set_language(Some(iso))` unless `auto`; join `full_get_segment_text` segments. **Move off the optional feature for shipping.** Add a context cache (`Mutex<Option<(PathBuf,WhisperContext)>>`) mirroring load-once/release-on-switch — the spike re-loads the (up to ~1.5 GB) model every call. |
| **Build** | `whisper-rs` needs **libclang (bindgen) + C/MSVC toolchain** — confirm on CI/build machines (noted in `Cargo.toml`). |
| **Download** | `whisper_download_model`: `reqwest` streamed (`bytes_stream` + `StreamExt`), `.tmp` + `std::fs::rename` atomicity, emit `whisper:downloadProgress {model,pct}` **throttled to integer pct** (per-chunk floods the WebView). Rust streaming-to-disk is an improvement over Electron's buffer-whole-file. |
| **⚠ AUDIO RESAMPLING — HIGHEST RISK** | `whisper-rs` does **NOT** resample; it requires 16 kHz mono f32. The MediaRecorder path records `audio/wav` at the mic's **native rate** (`MicBar.tsx:194` requests no `sampleRate` → typically 44.1/48 kHz); only the VAD path is fixed at 16 kHz (`MicBar.tsx:320`). The `lib.rs` spike decodes + downmixes but **omits resampling** → silent mis-transcription of any 44.1/48 kHz recording. **Mandatory fix:** add `rubato` (`SincFixedIn`) keyed on `spec.sample_rate != 16000`, **OR** pin the recorder to 16 kHz. **Recommended: resample in Rust** so the recorder/UI is untouched (cloud STT is unaffected — providers accept any rate). |

### B. Cloud pipeline — **decision: port to Rust `reqwest`, NOT frontend `fetch`**

**Recommendation: keep all cloud calls in Rust.** API keys are decrypted in the Electron **main** process via `safeStorage` (`index.ts:1089-1100`) and **never reach the renderer**. Moving STT/refine to WebView `fetch` would force plaintext keys into the JS context, breaking that boundary. Decrypt with stronghold/keyring in Rust and build requests with `reqwest` (features `multipart, json, rustls-tls, stream`). The language-seed/ISO/prompt builders (`pipeline.ts:9-47,110-116`) are pure string maps and port verbatim.

| Pipeline path | Electron | Rust equivalent | Notes |
|---|---|---|---|
| OpenAI / custom STT | `openai` SDK `audio.transcriptions.create`, 30 s timeout, multipart | reqwest multipart POST `{base}/audio/transcriptions`, `Bearer`, `.timeout(30s)` | filename must look like real audio; custom uses `customBaseUrl` + parsed `customHeaders` |
| Groq STT | same shape, `api.groq.com/openai/v1`; model = `whisper-large-v3` if langSeed else `-turbo` | shared reqwest fn parametrized by base/key/model | **Keep the langSeed→model branch** (Bangla accuracy) |
| BizGrowHub Bangla proxy | raw fetch `/api/bizvoice/transcribe-indic`, **60 s** timeout, **1× 503 retry after 6 s** | reqwest multipart `.timeout(60s)` + explicit single 503 retry (`tokio::time::sleep`) | **Do NOT collapse** into the generic STT helper — distinct timeout/retry; gated by `lang==bangla` |
| GPT format / refine | `buildGptClient` (openai/groq/openrouter/custom), `chat.completions`, temp 0.2, 20 s; OpenRouter adds `HTTP-Referer`+`X-Title` | reqwest JSON POST `{base}/chat/completions`, per-provider base + OpenRouter headers as a Rust `match`, `.timeout(20s)` | Two near-dup paths (pipeline GPT + `history:refine` at `index.ts:881-918`) — unify in one Rust module to avoid drift; replicate refine fallback order |
| Dictionary + history | `applyDictionary` regex pass; `recordTranscription` POST `/api/bizvoice/history` | `regex` crate; reqwest POST | port verbatim |

**Audio transport:** `audioBase64` over invoke is large (~33 % overhead). Acceptable to keep, but consider a binary command (bytes) or temp-file handoff to cut serialization cost.

---

## Frontend shim strategy

**Goal: zero design/JSX diff.** All 37 runtime call sites (+3 type-only refs) touch only `window.api`. Create **one** file, `src/lib/api.ts`, that rebuilds the exact `window.api` shape on top of `@tauri-apps/api` and assigns it as a **side effect**, imported once at the very top of each renderer entry (`main.tsx`) **before React mounts** (`MicBar.tsx:102` / `Settings.tsx:180` call it on mount).

> **Status:** implemented as `src/lib/tauriApi.ts` and wired into all 4 `main.tsx` entries. Reconcile the filename (`api.ts` vs `tauriApi.ts`) and apply the refinements below.

**Imports:** `invoke` from `@tauri-apps/api/core`; `listen` from `@tauri-apps/api/event`; `getCurrentWindow`, `PhysicalPosition` from `@tauri-apps/api/window`; `getVersion` from `@tauri-apps/api/app`. Move the ambient `declare global { interface Window { api: {...} } }` block (preload `index.ts:79-123`) into the shim so it stays valid.

**One generic sync-unsubscribe helper** implements all 9 subscriptions in a single place:
```ts
const sub = (event, map = e => e.payload) => cb => {
  const p = listen(event, e => cb(map(e)));
  return () => { p.then(u => u()); };   // immediate fn; defers actual unlisten
};
```
This preserves the synchronous-unsubscribe contract that `MicBar.tsx:117-134`, `Settings.tsx:181-183`, and `Settings.tsx:236-239` depend on.

**Mapping rules:**
- Request/response → `member: (...args) => invoke('cmd', { namedArgs })` (Tauri wraps args in an object: `setSettings(patch) → invoke('settings_set', { patch })`).
- Window-local ops can skip the Rust round-trip: `getWinPos`/`setWinPos`/`closeWindow` call `getCurrentWindow()` directly.
- **DPI caveat:** Electron `getPosition`/`setPosition` use logical (DIP) px; Tauri `outerPosition` returns **physical** px. The drag math (`MicBar.tsx:382-388`, `Settings.tsx:416-419`) mixes window position with raw mouse `screenX/Y` (CSS px). **Standardize on one unit in the shim** (use `PhysicalPosition` on both get + set, or convert via `scaleFactor`) to avoid drag drift on HiDPI.
- Implement **all** members (including the no-call-site ones: `onModeChange`, `onUpdateProgress`, `onUpdateDownloaded`, `updateLater`, `minimizeWindow`, `openSettings`, `cancelBrowserLogin`, `openSubscribe`, `openRegister`) so `window.api` is a complete drop-in.

**Verdict:** Choice (a) — keep the `window.api` global — over (b) rewriting 37 sites to `import { invoke }`. Choice (a) is zero-churn and zero design risk. Defer the `import { invoke }` refactor to Phase 6 polish only where a native invoke is genuinely cleaner.

---

## Build, release & auto-update

### Bundler config

| Item | Electron (`package.json`) | Tauri (`tauri.conf.json` / `Cargo.toml`) | Effort |
|---|---|---|---|
| build block | appId, productName, asar, compression max, `release/${version}` | `identifier`→appId, productName/version; **sync 3 version files**: `Cargo.toml:3` + `tauri.conf.json:5` (currently both `0.1.25`); output `bundle/nsis/` | low |
| NSIS | `oneClick:false`, `allowToChangeInstallationDirectory`, shortcuts, x64 | `bundle.windows.nsis` (currently only targets+icon set): `installMode` (oneClick→), `perMachine`/`both` (dir change), shortcuts default, `nsis.languages` | low |
| asar / asarUnpack `dist/vad` | unpacks WASM+onnx | No asar; served via asset protocol (no-op). **Verify WebView2 COOP/COEP for SharedArrayBuffer** (CSP currently `null`) | low |
| file include/exclude | excludes onnxruntime-web, hugeicons, whisper CUDA/Vulkan, vad-web, media-recorder | Obsolete (no node_modules shipped); runtime files → `resources`/`externalBin` (sidecar target-triple `.exe`) | medium |
| Icons | single `assets/icon.ico` | `bundle.icon` full set under `src-tauri/icons/` (already present); `icon.ico` covers installer+exe | low |

### Auto-update — signing required (high effort)

`electron-updater` (GitHub provider `webdevarif/BizVoice`, `autoDownload:false`, hourly, `update.html`, `quitAndInstall`, **unsigned `latest.yml`**) → `tauri-plugin-updater` (**not yet added** to `Cargo.toml`). Tauri **mandates** an ed25519 signature + a static `latest.json` feed.

- Add `tauri-plugin-updater` + `plugins.updater { pubkey, endpoints }` + `createUpdaterArtifacts: true` + the updater capability.
- HTML release-notes parsing (`parseReleaseNotes`) must be reimplemented or dropped (Tauri bodies are markdown).
- `autoInstallOnAppQuit` has **no direct equivalent** — rework. Split `download()` and `install()` to preserve the two-step "ask before download / ask before restart" UX.

### New build-time env vars

| Var | Why |
|---|---|
| **`TAURI_SIGNING_PRIVATE_KEY`** | Required before `tauri build` to sign update artifacts. **Lost key bricks updates.** |
| **`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`** | Passphrase for the signing key |
| `GH_TOKEN` | Kept (GitHub release upload) |
| `WINDOWS_CERTIFICATE` / `_PASSWORD` | Optional code-signing |

### Script changes

- `scripts/release.mjs`: replace `vite build` + `electron-builder --publish always` with `npx tauri build` (signs); load `TAURI_SIGNING_*` first; separate `gh release` upload step; version bump must sync `Cargo.toml:3` **and** `tauri.conf.json:5`.
- `scripts/publish-to-bizgrowhub.mjs`: repoint installer path → `src-tauri/target/release/bundle/nsis/<productName>_<version>_x64-setup.exe`; the manifest can become the updater endpoint if it gains `platforms.windows-x86_64 { signature, url }` + the `.sig` file.

---

## Risk register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Local whisper mis-transcribes 44.1/48 kHz (spike omits resampling) | **High** | High | Add `rubato` resampler in Rust keyed on sample rate, OR pin recorder to 16 kHz; verify before shipping local STT |
| `safeStorage` blobs can't migrate to stronghold/keyring (DPAPI incompatible) | **High** | High | Force one-time re-login + re-enter 4 provider keys on first Tauri launch, OR write a one-time DPAPI→new-store re-encrypt migration |
| Mic-bar overlay parity (transparent + non-focusable + screen-saver re-pin) | Medium | High | Cargo `transparent` feature; `WS_EX_NOACTIVATE`; raw `SetWindowPos(HWND_TOPMOST)` 1500 ms loop via `windows` crate; full visual test against notifications/fullscreen/UAC |
| Paste per-app strategy breaks in terminals/TUIs/browsers | Medium | High | Keep the existing PS SendInput helper initially (script unchanged via `std::process` + piped stdin); per-editor test (Cursor/VS Code/terminals/browsers) before replacing with `enigo`+`windows` |
| SharedArrayBuffer disabled (COOP/COEP not served on asset protocol) | Medium | High | Set `app.security.headers` COOP `same-origin` + COEP `credentialless`; verify WebView2 honors them; local whisper/VAD smoke test |
| Updater signing key mismanaged / lost | Low | High | Generate + back up `TAURI_SIGNING_PRIVATE_KEY` securely; document recovery; CI secret store |
| `transcribe` pipeline port introduces provider/Bangla regressions | Medium | Medium | Unify GPT+refine into one Rust module; preserve 60 s/503-retry Bangla path separately; preserve langSeed→Groq model branch |
| PTT `Released` event unreliable on Windows via plugin | Medium | Medium | Validate `tauri-plugin-global-shortcut` v2 Released on Windows; **fallback:** keep the PS `GetAsyncKeyState` poll spawned from Rust |
| Dead `micBar:resize` channel surfaces as a rejected invoke | Low | Low | Confirm renderer doesn't await it; delete the shim member (Electron silently no-op'd; Tauri rejects) |
| HiDPI drag drift (physical vs logical px) | Medium | Low | Standardize the shim on `PhysicalPosition` for both get/set; test on a scaled display |
| `whisper-rs` build fails (no libclang/MSVC on CI) | Medium | Medium | Provision libclang + C/MSVC toolchain on build machines; fallback to `whisper-cli` sidecar |
| Build size exceeds target / WebView2 missing on clean machine | Low | Medium | Bundle evergreen WebView2 bootstrapper (~2 MB, auto-installs); cold-start + clean-machine check in Phase 6 |

---

## Ordered task checklist

> Grouped by `TAURI_MIGRATION.md` phases 0–6. Exhaustive enough to execute the port. The `src-tauri/` scaffold, `commands.rs` skeleton, and `capabilities/default.json` already exist.

### Phase 0 — Spike / de-risk
- [ ] Confirm React UI renders in WebView2 with zero CSS changes (login.html already wired).
- [ ] Verify `whisper-spike` build: load a `ggml-*.bin` model + transcribe one WAV (`lib.rs:59-115`). Decide `whisper-rs` vs `whisper-cli` sidecar.
- [ ] **Reproduce the resampling failure**: transcribe a 48 kHz WAV through the spike, confirm garbage output, validate `rubato` fix.
- [ ] Verify `spike_paste` types into the foreground app from a Rust-spawned process; validate timing in VS Code + a terminal.
- [ ] Validate `tauri-plugin-global-shortcut` v2 emits `ShortcutState::Released` on Windows for a held chord (decides PS-poll deletion).
- [ ] Confirm WebView2 honors COOP `same-origin` + COEP `credentialless` via `security.headers` (SharedArrayBuffer / ONNX / vad-web smoke test).

### Phase 1 — Project scaffold & build pipeline
- [ ] Replace the single placeholder `main` window in `tauri.conf.json` with the four-window model (micBar transparent overlay, settings, login, update — most created lazily in Rust).
- [ ] Enable Cargo features `["tray-icon", "transparent"]` (currently `[]`).
- [ ] Move `whisper-rs` + `hound` off the optional `whisper-spike` feature for shipping; add `rubato`.
- [ ] Add dependencies: `reqwest` (multipart/json/rustls-tls/stream), `serde`, `tokio`, `regex`, `rand`, `windows` (CoreAudio + SetWindowPos), `futures-util`, `tiny_http`/`axum`; plugins `global-shortcut`, `clipboard-manager`, `store`, `updater`, `process`, `dialog`, `opener`/`shell`, `single-instance`, `autostart`, `stronghold` or `keyring`.
- [ ] Keep the Vite 4-entry rollup input so `dist/` emits `index.html`/`settings.html`/`login.html`/`update.html`; remove `vite-plugin-electron`.
- [ ] Configure NSIS bundler (`oneClick`→installMode, dir change, shortcuts); verify `tauri build` produces a runnable shell.
- [ ] Bundle/temp-write the three PowerShell helper scripts (PTT poll, mute, SendInput typer) as Rust string consts or Tauri resources.
- [ ] Set `BIZGROWHUB_API` switch via `cfg!(debug_assertions)`/`is_dev()` (localhost:8080 dev vs `https://bizgrowhub.shop`).
- [ ] Set COOP/COEP in `app.security.headers`; pre-size a 16×16 tray PNG.

### Phase 2 — Settings, store & secrets
- [ ] Define the `Settings` serde struct mirroring the full DTO (~30 fields) + `hasKey`/`hasGroqKey`/`hasOpenrouterKey`/`hasCustomKey` booleans; hold it in `Mutex<Settings>` managed state.
- [ ] Port `settings_get`: reqwest GET `/api/bizvoice/settings`, merge cache, `normLang` (en/bn→English/Bangla), derive `hasKey` flags from secure-store presence; offline-keeps-cache.
- [ ] Port `settings_set`: encrypt+strip `openaiKey`/`groqKey`/`openrouterKey`/`customKey` to stronghold/keyring (never synced remote); merge rest; PUT remote; re-register shortcuts on hotkey change; `emit_to('micbar','appearance:changed', {theme,widgetStyle})` on appearance change; return `{ok}`/`{ok:false,error}`.
- [ ] Implement secret storage (stronghold OR keyring) for token + 4 keys; decide stronghold password/salt strategy.
- [ ] Decide + implement secrets migration: force re-login/re-key OR one-time DPAPI re-encrypt.
- [ ] Persist non-secret data via `tauri-plugin-store`: `licenseOkAt`, `bizgrowhubEmail`, `refinedCache`.
- [ ] Port `history_get`/`history_clear` (+ clear `refinedCache`)/`history_refine` (provider select+fallback, cache by `ts`)/`history_get_refined_cache` (`HashMap<i64,String>`)/`stats_get`.
- [ ] Reconcile `src/lib/tauriApi.ts` shim with spec: generic `sub()` helper for all 9 events; window-local members via `getCurrentWindow()`; standardize on `PhysicalPosition`.

### Phase 3 — Windows, tray & hotkeys
- [ ] Build micBar window: transparent + non-focusable + always-on-top; lazy-create in `auth_status`/`onLicensed`; show/hide.
- [ ] Implement the 1500 ms re-pin loop via `windows` crate `SetWindowPos(HWND_TOPMOST, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)` + `WS_EX_NOACTIVATE`.
- [ ] Build settings/login/update windows via `WebviewWindowBuilder`; singleton-focus for settings; `data-tauri-drag-region` for frameless drag.
- [ ] `window:minimize` → `hide()` (preserve HIDE semantic).
- [ ] Build tray (`TrayIconBuilder` + `tauri::menu`): Show Mic Bar / Settings / Check for Updates / Quit; left-click shows mic bar or login; keep menu alive in app state.
- [ ] Port `micbar_context_menu` (Settings/Hide/Quit popup at cursor) — wire Quit→`app.exit(0)`, Settings→open window.
- [ ] Register 3 hotkeys via `tauri-plugin-global-shortcut`; translate accelerator strings; `emit_to('micbar', ...)` for `hotkey:toggle`, `mode:changed` (advance `activeMode`, push remote).
- [ ] PTT: emit `hotkey:ptt-start` on Pressed, `hotkey:ptt-stop` on Released; **delete** the PS `GetAsyncKeyState` poll if Released is reliable (else keep PS spawn as fallback).
- [ ] Port `audio_mute`: `windows` crate `IAudioEndpointVolume::SetMute` (or PS unchanged); keep debounce flag in managed state.
- [ ] Add `tauri-plugin-single-instance` (register FIRST); focus mic bar if licensed else login.
- [ ] Add `tauri-plugin-autostart`; wire `launchOnStartup` toggle in `settings_set` (launch `--hidden`).
- [ ] Decide mic-bar position: re-center each launch (parity) vs `tauri-plugin-window-state` (remember drag).
- [ ] `RunEvent::ExitRequested`→`prevent_exit()`; `RunEvent::Exit`→unregister shortcuts + kill helper + loopback server.

### Phase 4 — Transcription core
- [ ] Port `MODEL_META` (9 models) + `whisper_list_models`/`whisper_delete_model` over `app_data_dir/whisper-models` (byte-identical filenames).
- [ ] Port `whisper_download_model`: reqwest streamed HF download, `.tmp`+rename, emit `whisper:downloadProgress {model,pct}` throttled to integer pct.
- [ ] Promote `whisper-rs` inference; add `WhisperContext` cache in managed state (load-once/release-on-switch).
- [ ] **Add `rubato` resampling** (or pin recorder to 16 kHz) — keyed on `spec.sample_rate != 16000`.
- [ ] Port language seed / ISO map / `buildPrompt` / `languageInstruction` verbatim to Rust.
- [ ] Port cloud STT (OpenAI/Groq/custom) as one shared reqwest multipart fn (30 s); keep langSeed→Groq model branch.
- [ ] Port BizGrowHub Bangla proxy separately (multipart, 60 s timeout, 1× 503 retry after 6 s, `lang==bangla` gate).
- [ ] Port GPT format + `history:refine` into one Rust chat-completions module (per-provider base + OpenRouter headers, 20 s, temp 0.2, fallback order).
- [ ] Port `applyDictionary` (regex) + `recordTranscription` POST.
- [ ] Wire `transcribe` command end-to-end (base64 in → text out; reject with `.message` on failure). Decide base64 vs binary transport.
- [ ] Port `paste`: keep persistent SendInput PS helper (piped stdin from Rust, `SMART:<b64>`) with per-app strategy unchanged; clipboard stage/restore via `tauri-plugin-clipboard-manager`; mac osascript / linux xdotool via `std::process`.
- [ ] **Delete dead `micBar:resize`** member after confirming no renderer awaits it.

### Phase 5 — Auth & updates
- [ ] Port `start_browser_login`: Rust loopback HTTP (`tiny_http`/`axum` on 127.0.0.1:0) + `rand` 16-byte state CSRF; open browser via opener/shell; on `/callback` validate state, store token, fetch `/api/auth/me`, `checkLicense`, run `onLicensed`, `emit_to('login','auth:changed', payload)`. (Or `tauri-plugin-deep-link`.)
- [ ] Port `cancel_browser_login` (shutdown listener handle), `logout` (clear token, reset license, hide bar, emit `auth:changed`), `auth_status` (license + 7-day offline grace + `onLicensed` side-effect).
- [ ] Port `open_subscribe`/`open_register` via opener (scoped allowlist).
- [ ] Add `tauri-plugin-updater` + `plugins.updater { pubkey, endpoints }` + `createUpdaterArtifacts`; port `update_info` (check + `getVersion`, cache `Update`), `update_download` (split download; emit `update:progress` {percent,transferred,total}), `update_install` (install + `app.restart()`), `update_later` (close window).
- [ ] Emit `update:downloaded` after download; native fallback `tauri-plugin-dialog ask()`.
- [ ] Reimplement or drop `parseReleaseNotes` (markdown body).

### Phase 6 — Polish, parity & release
- [ ] Search all of `src/` for `onModeChange`/`onUpdateProgress`/`onUpdateDownloaded`/`updateLater`/`cancelBrowserLogin`/`openSubscribe`/`openRegister` consumers; ensure the shim implements every member.
- [ ] Verify appearance/theme live-update, single-instance focus routing, autostart `--hidden`.
- [ ] Author `src-tauri/capabilities/*.json` for all per-window/per-plugin permissions listed above (extend `default.json`).
- [ ] HiDPI drag test (physical px) on a scaled display.
- [ ] Per-editor paste test (Cursor / VS Code / terminals / browsers); macOS Accessibility grant; confirm `xdotool` presence note for Linux.
- [ ] Generate + back up `TAURI_SIGNING_PRIVATE_KEY` (+ password); update `scripts/release.mjs` (`tauri build` + `gh release`) and `publish-to-bizgrowhub.mjs` (bundle path + updater manifest `.sig`).
- [ ] Sync version across `Cargo.toml:3` + `tauri.conf.json:5` in the bump step.
- [ ] Size check (< 30 MB), cold-start check, bundle WebView2 evergreen bootstrapper, clean-machine install test.
