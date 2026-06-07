# NeuroVoice — AI Voice Typing for Windows

An installable Windows desktop app that works **exactly like Microsoft Voice Typing (Win+H)** but enhanced with AI. Press a hotkey anywhere, speak, and cleanly formatted text is auto-pasted into the active field. Supports Bangla and English with AI translation and formatting.

---

## 1. Goals

- Feel identical to Microsoft Voice Typing: press hotkey → floating mic bar appears → speak → text is inserted at cursor in any app (browser, Word, chat, code editor).
- AI layer on top: cleans grammar, adds punctuation, translates Bangla ↔ English, formats output.
- System tray icon with right-click menu → **Settings** window.
- Settings window: API keys, hotkey picker, language mode, startup-on-boot.
- Fully installable `.exe` via `electron-builder` (NSIS installer).

---

## 2. Tech Stack

| Layer | Choice | Reason |
|---|---|---|
| Shell | **Electron** (already installed) | Beautiful UI, tray, hotkeys, installer tooling |
| UI | React + Tailwind CSS | Fast dev, matches NeuroCode ecosystem |
| STT | **OpenAI Whisper API** (`openai` SDK, `whisper-1`) | Best multilingual accuracy, great Bangla support |
| AI Formatting | **OpenAI GPT API** (`openai` SDK, `gpt-4o-mini` / `gpt-4o`) | Fast, cheap, strong Bangla ↔ English translation |
| Global Hotkey | Electron `globalShortcut` | Built-in, system-wide |
| Auto-paste | `clipboardy` + `robotjs` (or `@nut-tree/nut-js`) | Copy → simulate Ctrl+V into active window |
| Tray | Electron `Tray` API | Native Windows tray icon |
| Storage | `electron-store` | Encrypted settings (API keys) |
| Installer | `electron-builder` (NSIS) | One-click `.exe` installer, auto-update ready |

---

## 3. User Flow (Identical to MS Voice Typing)

1. User installs `NeuroVoice-Setup.exe` → app starts on boot → tray icon appears.
2. User clicks in any text field (browser, Word, Slack, VS Code).
3. User presses configured hotkey (default: **`Ctrl+Shift+Space`**, same vibe as `Win+H`).
4. **Floating mic bar** slides up from bottom-center of screen:
   - Pulsing mic icon (recording indicator)
   - Live waveform
   - Timer
   - Mode badge (e.g. "BN → EN")
   - Settings gear icon ⚙️
   - Close ✕
5. User speaks → live partial transcript shown in the bar.
6. User presses hotkey again (or clicks stop) → recording ends.
7. Pipeline runs:
   - Audio → OpenAI Whisper → raw transcript
   - Raw transcript → OpenAI GPT (mode-specific prompt) → cleaned text
8. Cleaned text copied to clipboard → `Ctrl+V` simulated → **text appears at cursor** in original app.
9. Floating bar fades out.

---

## 4. Modes (Selectable in Settings & Mic Bar)

| Mode | Input Lang | Output Lang | GPT Prompt |
|---|---|---|---|
| `bn-clean` | Bangla | Bangla | Fix grammar & punctuation, keep in Bangla. |
| `bn-to-en` | Bangla | English | Translate to natural professional English. |
| `en-clean` | English | English | Fix grammar, punctuation, formatting. |
| `en-to-bn` | English | Bangla | Translate to natural Bangla. |
| `auto` | Any | Match input | Detect language, clean, keep same language. |

Quick mode switch: **`Ctrl+Shift+M`** cycles modes without opening settings.

---

## 5. Floating Mic Bar (Frameless Electron Window)

- **Size:** 420×90 px, rounded corners, dark blur background
- **Position:** Bottom-center, always-on-top, click-through when idle
- **States:** idle / recording / processing / success / error
- **Components:**
  - Mic icon (animated pulse when recording)
  - Waveform visualizer (Web Audio API `AnalyserNode`)
  - Live partial transcript
  - Timer `00:03`
  - Mode badge
  - ⚙️ settings button → opens Settings window
  - ✕ close
- **Keyboard:** `Esc` cancels, hotkey toggles, `Tab` switches mode

---

## 6. Settings Window

Opened from: tray icon menu, mic bar gear ⚙️, or `Ctrl+Shift+,`

### Tabs

**General**
- Launch on Windows startup (toggle)
- Show mic bar position (bottom-center / top-center / cursor)
- Play start/stop sound (toggle)
- Theme (dark / light / system)

**Hotkeys**
- Record toggle — default `Ctrl+Shift+Space`
- Cycle mode — default `Ctrl+Shift+M`
- Open settings — default `Ctrl+Shift+,`
- Cancel recording — `Esc`
- *(Click field → press keys → captured)*

**Languages & Modes**
- Default mode dropdown (bn-clean / bn-to-en / en-clean / en-to-bn / auto)
- Input language dropdown (Auto / Bangla / English / + more later)
- Output language dropdown
- Custom GPT prompt per mode (advanced, collapsible)

**API Keys** (stored encrypted via `electron-store` + `safeStorage`)
- OpenAI API key (password field + test button) — used for both Whisper & GPT
- Model selector: `gpt-4o-mini` (fast/cheap) / `gpt-4o` (best quality)

**Audio**
- Input device dropdown
- Mic gain slider
- Noise suppression toggle
- Silence auto-stop (seconds)

**About**
- Version, update check button, links

---

## 7. Tray Icon Menu

Right-click tray icon:
- 🎙️ Start Recording
- 🔄 Mode: `BN → EN` ▸ (submenu to switch)
- ⚙️ Settings
- ⏸️ Pause Hotkeys
- ❓ Help
- 🚪 Quit

Left-click → opens Settings.

---

## 8. Project Structure

```
neurovoice/
├── package.json
├── electron-builder.yml
├── src/
│   ├── main/                    # Electron main process
│   │   ├── index.ts             # app bootstrap, tray, hotkeys
│   │   ├── tray.ts
│   │   ├── hotkeys.ts
│   │   ├── windows/
│   │   │   ├── micBar.ts        # floating bar window
│   │   │   └── settings.ts      # settings window
│   │   ├── recording/
│   │   │   ├── recorder.ts      # mic capture
│   │   │   ├── whisper.ts       # OpenAI Whisper STT client
│   │   │   └── gpt.ts           # OpenAI GPT formatting
│   │   ├── paste/
│   │   │   └── autoPaste.ts     # clipboard + Ctrl+V injection
│   │   └── store.ts             # electron-store wrapper
│   ├── preload/
│   │   ├── micBar.ts
│   │   └── settings.ts
│   └── renderer/                # React UI
│       ├── micBar/
│       │   ├── App.tsx
│       │   ├── Waveform.tsx
│       │   └── styles.css
│       └── settings/
│           ├── App.tsx
│           ├── tabs/
│           │   ├── General.tsx
│           │   ├── Hotkeys.tsx
│           │   ├── Languages.tsx
│           │   ├── ApiKeys.tsx
│           │   ├── Audio.tsx
│           │   └── About.tsx
│           └── styles.css
├── assets/
│   ├── icon.ico
│   ├── tray-idle.png
│   └── tray-recording.png
├── build/                       # electron-builder output
└── docs/
    └── plan.md                  # this file
```

---

## 9. Build Phases

### Phase 1 — MVP (Week 1)
- [ ] Electron + React + Tailwind scaffold
- [ ] Tray icon + menu
- [ ] Global hotkey registration
- [ ] Floating mic bar window (frameless, always-on-top)
- [ ] Mic recording (Web Audio API)
- [ ] OpenAI Whisper STT integration
- [ ] OpenAI GPT formatting (3 modes: bn-clean, bn-to-en, en-clean)
- [ ] Clipboard copy + auto-paste (`Ctrl+V` simulation)
- [ ] Settings window: API keys tab + Hotkeys tab + Languages tab
- [ ] `electron-store` for persistence

### Phase 2 — Polish (Week 2)
- [ ] Waveform visualizer
- [ ] Chunked recording for faster perceived latency
- [ ] All settings tabs complete
- [ ] Launch-on-startup
- [ ] Start/stop sounds
- [ ] Mode cycle hotkey
- [ ] Error toasts

### Phase 3 — Installer (Week 2-3)
- [ ] App icon + branding
- [ ] `electron-builder` NSIS config
- [ ] Code signing (optional)
- [ ] `NeuroVoice-Setup.exe` output
- [ ] Auto-update via `electron-updater` (GitHub releases)

---

## 10. Key Technical Notes

- **Auto-paste reliability:** Save user's current clipboard → set new text → send `Ctrl+V` → restore clipboard after 500ms. Prevents clobbering their clipboard history.
- **Focus preservation:** Mic bar must NOT steal focus from the target app. Use `BrowserWindow` with `focusable: false` + `setIgnoreMouseEvents` toggled on hover.
- **API key security:** Use Electron's `safeStorage.encryptString()` before writing to `electron-store`. Never log keys.
- **Whisper model:** `whisper-1` via OpenAI API — auto-detects language; optionally pass `language: 'bn'` / `'en'` for higher accuracy.
- **GPT model:** `gpt-4o-mini` default (fast + cheap), `gpt-4o` optional for best translation quality.
- **Single API key:** Both Whisper and GPT use the same OpenAI key — only one field in settings.
- **Latency target:** <2s from stop-speaking to text-pasted for short utterances.

---

## 11. Open Questions

1. Do you want **push-to-talk** (hold hotkey) or **toggle** (press once start, press again stop)? — *Default plan: toggle, like MS Voice Typing.*
2. Should the mic bar show the **final text preview** before pasting, or paste instantly? — *Default: instant paste for speed.*
3. OpenAI-only, or also add Claude/Gemini as optional providers later? — *Default: OpenAI-only for MVP.*
4. App name confirmed as **NeuroVoice**?

---

## 12. Next Step

Once you approve this plan, I'll scaffold the Electron + React + Tailwind project and build Phase 1 MVP starting with: tray icon → hotkey → mic bar → Whisper → GPT → auto-paste.
