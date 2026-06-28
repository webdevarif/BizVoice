# BizVoice — Electron → Tauri Migration Plan

> Goal: cut installer from **~170–215 MB → ~25–30 MB** by replacing the bundled
> Chromium with Windows' system **WebView2**, **without any change to the UI/design**.
> The React + Tailwind frontend (`src/`, 9 files) renders identically in WebView2.
>
> The work is a real backend port: ~2000 lines of TypeScript main-process code
> (`electron/`, mostly `index.ts` @ 1429 lines) becomes Rust (`src-tauri/`).

---

## Current architecture (what we're porting)

| Layer | Files | Notes |
|---|---|---|
| Renderer (UI) | `src/**` (9 files) | **No design change.** Only the `window.api` bridge changes. |
| Bridge | `electron/preload/index.ts` (123 ln) | `contextBridge` → becomes `@tauri-apps/api` invoke + event shim |
| Main | `electron/main/index.ts` (1429 ln) | windows, tray, hotkeys, auth, updater, paste, settings, history |
| Whisper | `electron/main/localWhisper.ts` (144 ln) | **native Node addon `@fugood/whisper.node`** — hardest part |
| Pipeline | `electron/main/pipeline.ts` (306 ln) | OpenAI/Groq/OpenRouter transcription + refine |

**IPC surface to port:** ~28 `ipcMain.handle` channels + 8 push events (hotkey,
update, auth, appearance, whisper progress).

**Native / OS bits:** global hotkeys, system tray, multi-window mic bar
(always-on-top, screen-saver level), PowerShell helpers (PTT key poll, system
mute, SendInput paste injection), local HTTP auth server, `safeStorage` encryption.

---

## Tauri mapping (drop-in replacements)

| Electron | Tauri | Effort |
|---|---|---|
| `globalShortcut` | `tauri-plugin-global-shortcut` | low |
| `Tray` / `Menu` | built-in tray API | low |
| `electron-store` | `tauri-plugin-store` | low |
| `electron-updater` | `tauri-plugin-updater` | medium |
| `clipboard` | `tauri-plugin-clipboard-manager` | low |
| `safeStorage` | `tauri-plugin-stronghold` or OS keychain | medium |
| `shell.openExternal` | `tauri-plugin-shell` / opener | low |
| `dialog` | `tauri-plugin-dialog` | low |
| HTTP auth server (`http.createServer`) | Rust `axum`/`tiny-http` or `tauri-plugin-oauth` | medium |
| PowerShell helpers (`exec`/`spawn`) | Rust `std::process::Command` — **PS scripts unchanged** | low |
| `@fugood/whisper.node` (native addon) | `whisper-rs` **or** `whisper-cli` sidecar | **high** |
| OpenAI/Groq/OpenRouter (`openai` SDK) | Rust `reqwest` **or** keep in frontend `fetch` | medium |
| onnxruntime-web + VAD (renderer) | **unchanged** — runs in WebView2 WASM | none |

---

## Phases

### Phase 0 — Spike / de-risk (½–1 day)
Prove the riskiest pieces before committing. Throwaway code is fine.
- [ ] Scaffold `npm create tauri-app` alongside, point it at existing `dist/` build output.
- [ ] Confirm the React UI loads & renders in WebView2 with **zero CSS changes**.
- [ ] Spike **whisper in Rust**: load a `ggml` model with `whisper-rs` and transcribe one WAV. Decide: `whisper-rs` vs `whisper-cli` sidecar.
- [ ] Spike **paste injection**: spawn the existing SendInput PowerShell script from Rust `Command`; verify it still types into the foreground app.
- **Exit criteria:** UI renders + one transcription + one paste works.

### Phase 1 — Project scaffold & build pipeline (1 day)
- [ ] Add `src-tauri/` (Cargo.toml, `tauri.conf.json`, `main.rs`, `lib.rs`).
- [ ] Wire Vite as the Tauri frontend (`beforeDevCommand` / `beforeBuildCommand`); remove `vite-plugin-electron`.
- [ ] Configure windows in `tauri.conf.json`: main, mic bar (transparent, always-on-top, no-decorations), settings, update.
- [ ] Set up NSIS/MSI bundler + icon. Verify a `tauri build` produces a runnable shell.
- [ ] Bundle PowerShell helper scripts + whisper runtime as Tauri **resources**.

### Phase 2 — Settings, store & secrets (1 day)
Foundation everything else depends on.
- [ ] Port `Settings` type + `settings:get` / `settings:set` to `tauri-plugin-store`.
- [ ] Port API-key encryption (`safeStorage`) → stronghold / keychain.
- [ ] Port `history:*` and `stats:*` channels.
- [ ] Build a thin frontend shim so `window.api.getSettings()` etc. keep working with minimal edits to `Settings.tsx` / `MicBar.tsx`.

### Phase 3 — Windows, tray & hotkeys (1–2 days)
- [ ] Recreate mic bar window: position persistence (`micBar:getPos/setPos`), resize, always-on-top "screen-saver" level, context menu.
- [ ] Tray icon + menu (`createTray`).
- [ ] Global hotkeys: toggle, push-to-talk start/stop, cycle mode → `tauri-plugin-global-shortcut`.
- [ ] PTT key-state poll PowerShell script spawn + the `hotkey:*` push events to the renderer.
- [ ] System-audio mute helper (`audio:mute`).

### Phase 4 — Transcription core (2–3 days) ← biggest risk
- [ ] Port `localWhisper.ts`: model list / download (with `whisper:downloadProgress` events) / delete.
- [ ] Implement chosen whisper backend (whisper-rs or sidecar) behind a clean Rust command.
- [ ] Port `pipeline.ts`: cloud transcription + GPT refine for openai/groq/openrouter/custom (Rust `reqwest`).
- [ ] Port the `transcribe` command end-to-end (audio base64 in → text out).
- [ ] Port paste injection (`paste` command) — clipboard staging + SendInput PS, editor-specific Shift+Insert logic.

### Phase 5 — Auth & updates (1–2 days)
- [ ] Local browser-login HTTP server (`auth:startBrowserLogin` / cancel / status / logout) → Rust axum, talking to `BIZGROWHUB_API`.
- [ ] `auth:changed` push events; subscribe/register external-link opens.
- [ ] `tauri-plugin-updater`: `update:info/download/install/later` + progress/downloaded events. Generate update signing keys, wire GitHub release feed.

### Phase 6 — Polish, parity & release (1–2 days)
- [ ] Sweep all 41 `window.api.*` call sites; remove the shim where a native invoke is cleaner.
- [ ] Verify appearance/theme change events, single-instance lock, autostart.
- [ ] Size check (target < 30 MB), cold-start check, WebView2-runtime bootstrap on a clean machine.
- [ ] Update `scripts/release.mjs` + `publish-to-bizgrowhub.mjs` for Tauri artifact paths (`src-tauri/target/release/bundle/...`).
- [ ] Update `.env` notes if any new build vars appear (e.g. `TAURI_SIGNING_PRIVATE_KEY`).

---

## Estimate
**~9–14 working days** for full parity. Frontend/design untouched.
Front-load Phase 0 — if whisper-in-Rust or paste injection blocks, that's the
decision point on whether a Node sidecar (smaller savings, easier port) is the
pragmatic fallback.

## Known risks
1. **Whisper native addon** has no Rust equivalent — must re-implement (whisper-rs) or sidecar.
2. **Mic bar** always-on-top "screen-saver" overlay level + transparency parity in WebView2.
3. **Paste/SendInput** timing differs slightly under a Rust-spawned process; needs per-editor testing.
4. **WebView2 runtime** must exist on target machines — bundle the evergreen bootstrapper (adds ~2 MB, auto-installs).
