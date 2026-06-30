// Tauri implementation of the Electron `window.api` preload bridge.
//
// This lets the EXISTING React UI run under Tauri with zero component/design
// changes: every `window.api.*` method is re-implemented on top of
// `@tauri-apps/api`, preserving the exact contract that `electron/preload/index.ts`
// exposes — including the synchronous unsubscribe function returned by every
// `onX(cb)` subscription.
//
// Under Electron this module is a no-op: the real preload bridge stays in charge.
// Under Tauri it installs `window.api` so call sites in MicBar.tsx / Settings.tsx
// work untouched.
//
// CONTRACT NOTE for the Rust side (Phases 2-5):
//   • invoke() command names below are the snake_case Rust `#[tauri::command]`
//     function names that must be implemented.
//   • Argument keys are camelCase; implement commands with
//     `#[tauri::command(rename_all = "camelCase")]` (or matching snake_case params)
//     so e.g. { audioBase64, durationMs } maps to (audio_base64, duration_ms).
//   • Event names (with colons) are emitted from Rust verbatim via app.emit().

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/**
 * Bridge an Electron-style `onX(cb) => unsubscribe` subscription onto Tauri's
 * async `listen()`. Returns a SYNCHRONOUS unsubscribe function — the contract
 * the React cleanup code relies on (`const off = api.onX(cb); return () => off()`).
 * Tears down the listener even if `listen()` resolves after the caller already
 * unsubscribed.
 */
function sub<T>(event: string, cb: (payload: T) => void): () => void {
  let unlisten: (() => void) | null = null;
  let cancelled = false;
  listen<T>(event, (e) => cb(e.payload as T)).then((fn) => {
    if (cancelled) fn();
    else unlisten = fn;
  });
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
    unlisten = null;
  };
}

type Mode = { id: string; name: string; color: string; prompt: string };

const api: Window['api'] = {
  // ── Settings & windows ──
  getSettings: () => invoke('get_settings'),
  setSettings: (patch: any) => invoke('set_settings', { patch }),
  openSettings: () => invoke('open_settings'),
  closeWindow: () => invoke('close_window'),
  minimizeWindow: () => invoke('minimize_window'),

  // ── Core transcribe + paste ──
  transcribe: (audioBase64: string, durationMs?: number) =>
    invoke('transcribe', { audioBase64, durationMs: durationMs ?? 0 }),
  paste: (text: string) => invoke('paste', { text }),

  // ── Hotkey / mode push events ──
  onHotkey: (cb) => sub('hotkey:toggle', cb),
  onPttStart: (cb) => sub('hotkey:ptt-start', cb),
  onPttStop: (cb) => sub('hotkey:ptt-stop', cb),
  onModeChange: (cb) => sub<Mode>('mode:changed', cb),

  // ── Mic bar window ──
  micBarContextMenu: () => invoke('mic_bar_context_menu'),
  // `micBar:resize` has no handler in the original Electron main process (dead
  // channel — Electron silently no-op'd it). MicBar.tsx still calls it fire-and-
  // forget, so keep a no-op here rather than invoking a non-existent command
  // (which Tauri would reject with an unhandled rejection). See PORT_SPEC.md.
  resizeMicBar: (_active: boolean) => Promise.resolve(),
  getWinPos: () => invoke('get_win_pos'),
  setWinPos: (x: number, y: number) => invoke('set_win_pos', { x, y }),
  muteSystem: (mute: boolean) => invoke('mute_system', { mute }),

  // ── History & stats ──
  getHistory: () => invoke('get_history'),
  clearHistory: () => invoke('clear_history'),
  refineText: (text: string, ts: number) => invoke('refine_text', { text, ts }),
  getRefinedCache: () => invoke('get_refined_cache'),
  getStats: () => invoke('get_stats'),

  // ── Local whisper models ──
  whisperListModels: () => invoke('whisper_list_models'),
  whisperDownloadModel: (name: string) => invoke('whisper_download_model', { name }),
  whisperDeleteModel: (name: string) => invoke('whisper_delete_model', { name }),
  onWhisperDownloadProgress: (cb) => sub('whisper:downloadProgress', cb),

  // ── Appearance ──
  onAppearanceChange: (cb) => sub('appearance:changed', cb),

  // ── BizGrowHub auth + license ──
  startBrowserLogin: () => invoke('start_browser_login'),
  cancelBrowserLogin: () => invoke('cancel_browser_login'),
  logout: () => invoke('logout'),
  authStatus: () => invoke('auth_status'),
  openSubscribe: () => invoke('open_subscribe'),
  openRegister: () => invoke('open_register'),
  onAuthChange: (cb) => sub('auth:changed', cb),

  // ── App updates ──
  updateInfo: (forceCheck?: boolean) => invoke('update_info', { forceCheck: forceCheck ?? false }),
  updateDownload: () => invoke('update_download'),
  updateInstall: () => invoke('update_install'),
  updateLater: () => invoke('update_later'),
  onUpdateProgress: (cb) => sub('update:progress', cb),
  onUpdateDownloaded: (cb) => sub('update:downloaded', cb),
};

/** Install `window.api` when running under Tauri. No-op under Electron. */
export function installTauriApi(): void {
  if (!isTauri()) return;
  (window as any).api = api;
  // F12 opens the webview DevTools (works in release too — the tauri `devtools`
  // feature is enabled in Cargo.toml).
  window.addEventListener('keydown', (e) => {
    if (e.key === 'F12') {
      e.preventDefault();
      invoke('open_devtools_cmd').catch(() => {});
    }
  });
}

// Auto-install on import so a single `import './lib/tauriApi'` in each window's
// entry (main.tsx) is enough — no component changes required.
installTauriApi();

// Mirror the preload's ambient type so the React code type-checks under the
// Tauri build (where electron/preload/index.ts is not part of the program).
declare global {
  interface Window {
    api: {
      getSettings: () => Promise<any>;
      setSettings: (patch: any) => Promise<any>;
      openSettings: () => Promise<void>;
      closeWindow: () => Promise<void>;
      minimizeWindow: () => Promise<void>;
      transcribe: (audioBase64: string, durationMs?: number) => Promise<string>;
      paste: (text: string) => Promise<void>;
      onHotkey: (cb: () => void) => () => void;
      onPttStart: (cb: () => void) => () => void;
      onPttStop: (cb: () => void) => () => void;
      onModeChange: (cb: (mode: { id: string; name: string; color: string; prompt: string }) => void) => () => void;
      micBarContextMenu: () => Promise<void>;
      resizeMicBar: (active: boolean) => Promise<void>;
      getWinPos: () => Promise<[number, number]>;
      setWinPos: (x: number, y: number) => Promise<void>;
      muteSystem: (mute: boolean) => Promise<void>;
      getHistory: () => Promise<{ text: string; ts: number; words: number; durationMs: number }[]>;
      clearHistory: () => Promise<void>;
      refineText: (text: string, ts: number) => Promise<string>;
      getRefinedCache: () => Promise<Record<number, string>>;
      getStats: () => Promise<{ recordings: number; words: number; durationMs: number }>;
      whisperListModels: () => Promise<{ name: string; size: string; downloaded: boolean }[]>;
      whisperDownloadModel: (name: string) => Promise<{ ok: boolean; error?: string }>;
      whisperDeleteModel: (name: string) => Promise<boolean>;
      onWhisperDownloadProgress: (cb: (data: { model: string; pct: number }) => void) => () => void;
      onAppearanceChange: (cb: (data: { theme: string; widgetStyle: string }) => void) => () => void;
      startBrowserLogin: () => Promise<{ ok: boolean; error?: string }>;
      cancelBrowserLogin: () => Promise<{ ok: boolean }>;
      logout: () => Promise<{ ok: boolean }>;
      authStatus: () => Promise<{ loggedIn: boolean; active: boolean; email: string; offline?: boolean }>;
      openSubscribe: () => Promise<void>;
      openRegister: () => Promise<void>;
      onAuthChange: (cb: (data: { active: boolean; loggedIn: boolean; email: string }) => void) => () => void;
      updateInfo: (forceCheck?: boolean) => Promise<{ current: string; latest: { version: string; notes?: string[]; releasedAt?: string } | null; updateAvailable: boolean; downloaded: boolean }>;
      updateDownload: () => Promise<void>;
      updateInstall: () => Promise<void>;
      updateLater: () => Promise<void>;
      onUpdateProgress: (cb: (data: { percent: number; transferred: number; total: number }) => void) => () => void;
      onUpdateDownloaded: (cb: () => void) => () => void;
    };
  }
}
