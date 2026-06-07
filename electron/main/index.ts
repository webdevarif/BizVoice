import { app, BrowserWindow, globalShortcut, ipcMain, Tray, Menu, nativeImage, safeStorage, clipboard, session, systemPreferences, screen, shell, dialog } from 'electron';
import { autoUpdater } from 'electron-updater';
import type OpenAI from 'openai';
import { join } from 'path';
import { exec, spawn, type ChildProcess } from 'child_process';
import { writeFileSync } from 'fs';
import { createServer, type Server } from 'http';
import { randomBytes } from 'crypto';
import Store from 'electron-store';
import { runPipeline } from './pipeline';
// Local Whisper — lazy loaded to avoid bundling the native addon
const getLocalWhisper = (() => {
  let mod: typeof import('./localWhisper') | null = null;
  return async () => { if (!mod) mod = await import('./localWhisper'); return mod; };
})();

const VITE_DEV_SERVER_URL = process.env.VITE_DEV_SERVER_URL;

// BizGrowHub backend — the auth + BizVoice-subscription/license authority.
// Dev (running under Vite) talks to the local dashboard; packaged builds talk
// to production. Override anytime with the BIZGROWHUB_API env var.
const BIZGROWHUB_API =
  process.env.BIZGROWHUB_API ||
  (VITE_DEV_SERVER_URL ? 'http://localhost:8080' : 'https://bizgrowhub.shop');

type Lang = string;

type Mode = {
  id: string;
  name: string;
  color: string;
  prompt: string;
};

type GptProvider = 'openai' | 'groq' | 'openrouter' | 'custom';

type Settings = {
  openaiKeyEncrypted: string;
  groqKeyEncrypted: string;
  openrouterKeyEncrypted: string;
  customKeyEncrypted: string;
  customBaseUrl: string;
  customChatModel: string;
  customHeaders: string;  // JSON string, e.g. '{"HTTP-Referer":"..."}'
  hotkey: string;
  pttHotkey: string;
  cycleHotkey: string;
  inputLang: Lang;
  outputLang: Lang;
  gptModel: string;       // any string now (model name varies by provider)
  gptProvider: GptProvider;
  sttModel: string;
  sttProvider: 'openai' | 'groq';
  skipGpt: boolean;
  launchOnStartup: boolean;
  micDeviceId: string;
  vocabulary: string;
  modes: Mode[];
  activeMode: string;
  silenceMs: number;
  autoStop: boolean;
  useLocalWhisper: boolean;
  localModel: string;
  micFallbackId: string;
  muteWhileRecording: boolean;
  dictionary: { from: string; to: string; category?: string; ts?: number }[];
  theme: 'dark' | 'black' | 'light';
  widgetStyle: 'logoText' | 'logo' | 'mono';
  instructions: string;
  history: { text: string; ts: number; words: number; durationMs: number }[];
  stats: { recordings: number; words: number; durationMs: number };
  // BizGrowHub auth/licensing
  bizgrowhubTokenEncrypted: string;
  bizgrowhubEmail: string;
  licenseOkAt: number;
};

const DEFAULT_MODES: Mode[] = [
  {
    id: 'transcript', name: 'Transcript', color: 'sky',
    prompt: 'You are a transcription corrector. Fix ONLY spelling and grammar mistakes. Keep the speaker\'s EXACT words, sentence structure, and meaning. Do NOT rephrase, rewrite, summarize, or add anything. Remove filler words (um, uh, like) and stutters only. Output ONLY the corrected text.',
  },
  {
    id: 'ai', name: 'AI Prompt', color: 'purple',
    prompt: 'Convert raw voice dictation into a concise, direct AI prompt for a coding assistant. Imperative tone, technical, no filler. Remove repetitions and stutters. Keep code terms exact. Output ONLY the prompt text.',
  },
  {
    id: 'client', name: 'Client', color: 'emerald',
    prompt: 'Convert raw voice dictation into a polite, professional client message. Warm but concise tone. Fix grammar, remove filler and stutters. Output ONLY the message text.',
  },
  {
    id: 'clean', name: 'Rewrite', color: 'amber',
    prompt: 'Rewrite raw voice dictation into polished, well-structured written text. Improve clarity and flow while preserving the core meaning. Remove fillers, fix grammar. Output ONLY the rewritten text.',
  },
];

const store = new Store<Settings>({
  defaults: {
    openaiKeyEncrypted: '',
    groqKeyEncrypted: '',
    openrouterKeyEncrypted: '',
    customKeyEncrypted: '',
    customBaseUrl: '',
    customChatModel: '',
    customHeaders: '',
    hotkey: 'CommandOrControl+Shift+Space',
    pttHotkey: 'CommandOrControl+Space',
    cycleHotkey: 'CommandOrControl+Shift+M',
    inputLang: 'auto',
    outputLang: 'auto',
    gptModel: 'gpt-4o-mini',
    gptProvider: 'openai' as const,
    sttModel: 'whisper-1',
    sttProvider: 'openai' as const,
    skipGpt: true,
    launchOnStartup: false,
    micDeviceId: 'default',
    vocabulary: '',
    modes: DEFAULT_MODES,
    activeMode: 'transcript',
    silenceMs: 1500,
    autoStop: false,
    useLocalWhisper: false,
    localModel: 'base',
    micFallbackId: 'default',
    muteWhileRecording: false,
    dictionary: [],
    theme: 'dark' as const,
    widgetStyle: 'logoText' as const,
    instructions: '',
    history: [],
    stats: { recordings: 0, words: 0, durationMs: 0 },
    bizgrowhubTokenEncrypted: '',
    bizgrowhubEmail: '',
    licenseOkAt: 0,
  },
});

// Migrate: ensure Transcript mode exists and add missing default modes
(function migrateToModes() {
  const raw = store.store as any;
  if (raw.styleAI && (!raw.modes || !raw.modes.length)) {
    store.set('modes', DEFAULT_MODES);
    store.set('activeMode', 'transcript');
    store.delete('stylePreset' as any);
    store.delete('styleAI' as any);
    store.delete('styleClient' as any);
    store.delete('styleClean' as any);
  }
  const modes = store.get('modes') || [];
  if (modes.length > 0 && !modes.find((m: Mode) => m.id === 'transcript')) {
    modes.unshift(DEFAULT_MODES[0]);
    store.set('modes', modes);
    store.set('activeMode', 'transcript');
  }
})();

// ── Remote (BizGrowHub) settings ────────────────────────────────────────────
// All app data lives in BizGrowHub now. `S` is an in-memory mirror so hotkey /
// transcribe code can read settings synchronously; it's loaded after sign-in
// and pushed back whenever something changes. Only the auth token + the user's
// OpenAI/Groq keys remain in electron-store.
let S: Settings = { ...store.store };

const DATA_KEYS: (keyof Settings)[] = [
  'hotkey', 'pttHotkey', 'cycleHotkey', 'inputLang', 'outputLang', 'gptModel', 'sttModel',
  'sttProvider', 'gptProvider', 'skipGpt', 'launchOnStartup', 'micDeviceId', 'vocabulary', 'modes', 'activeMode',
  'silenceMs', 'autoStop', 'useLocalWhisper', 'localModel', 'micFallbackId', 'muteWhileRecording',
  'dictionary', 'theme', 'widgetStyle', 'instructions',
  'customBaseUrl', 'customChatModel', 'customHeaders',
];

function bghAuthFetch(path: string, init: RequestInit = {}) {
  const token = getAuthToken();
  return fetch(`${BIZGROWHUB_API}${path}`, {
    ...init,
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}`, ...(init.headers || {}) },
  });
}

/** Pull settings into the cache. Returns false when the user has none yet. */
async function loadRemoteSettings(): Promise<boolean> {
  try {
    const res = await bghAuthFetch('/api/bizvoice/settings');
    if (!res.ok) return false;
    const data = await res.json().catch(() => ({} as Record<string, unknown>));
    const remote = ((data as { settings?: Partial<Settings> }).settings ?? {}) as Partial<Settings>;
    for (const k of DATA_KEYS) {
      if (remote[k] !== undefined) (S as Record<string, unknown>)[k] = remote[k];
    }
    return Object.keys(remote).length > 0;
  } catch { return false; /* offline → keep current cache */ }
}

async function pushRemoteSettings() {
  try {
    const data: Record<string, unknown> = {};
    for (const k of DATA_KEYS) data[k] = (S as Record<string, unknown>)[k];
    await bghAuthFetch('/api/bizvoice/settings', { method: 'PUT', body: JSON.stringify({ settings: data }) });
  } catch { /* ignore */ }
}

function getActiveMode(): Mode {
  const modes = S.modes;
  const activeId = S.activeMode;
  return modes.find(m => m.id === activeId) || modes[0] || DEFAULT_MODES[0];
}

let micBar: BrowserWindow | null = null;
let settingsWin: BrowserWindow | null = null;
let tray: Tray | null = null;

function logEvent(level: 'info' | 'warn' | 'error', msg: string, data?: any) {
  console.log(`[bizvoice][${level}] ${msg}`, data ?? '');
}

const MICBAR_IDLE  = { w: 210, h: 46 };


function createMicBar() {
  const { width: sw, height: sh } = screen.getPrimaryDisplay().workAreaSize;
  const { w, h } = MICBAR_IDLE;
  micBar = new BrowserWindow({
    width: w,
    height: h,
    x: Math.round((sw - w) / 2),
    y: sh - h - 12,
    frame: false,
    transparent: true,
    resizable: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    hasShadow: false,
    focusable: false,
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  // Re-assert periodically — Windows demotes HWND_TOPMOST windows when other
  // topmost windows (notifications, fullscreen apps, UAC) compete for the slot.
  const pinOnTop = () => {
    if (!micBar || micBar.isDestroyed()) return;
    micBar.setAlwaysOnTop(true, 'screen-saver');
    micBar.moveTop();
  };
  pinOnTop();
  micBar.on('show', pinOnTop);
  const pinInterval = setInterval(pinOnTop, 1500);
  micBar.on('closed', () => clearInterval(pinInterval));

  if (VITE_DEV_SERVER_URL) {
    micBar.loadURL(VITE_DEV_SERVER_URL);
  } else {
    micBar.loadFile(join(__dirname, '../../dist/index.html'));
  }
}

function openSettings() {
  if (settingsWin) {
    settingsWin.focus();
    return;
  }
  settingsWin = new BrowserWindow({
    width: 720,
    height: 580,
    minWidth: 600,
    minHeight: 420,
    frame: false,
    resizable: true,
    movable: true,
    backgroundColor: '#1a1a1f',
    icon: join(__dirname, '../../assets/icon.ico'),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      contextIsolation: true,
    },
  });
  settingsWin.on('closed', () => (settingsWin = null));

  if (VITE_DEV_SERVER_URL) {
    settingsWin.loadURL(`${VITE_DEV_SERVER_URL}settings.html`);
  } else {
    settingsWin.loadFile(join(__dirname, '../../dist/settings.html'));
  }
}

// ── BizGrowHub auth + BizVoice license ──────────────────────────────────────
let loginWin: BrowserWindow | null = null;

function getAuthToken(): string {
  const enc = store.get('bizgrowhubTokenEncrypted');
  if (!enc) return '';
  try {
    return safeStorage.isEncryptionAvailable()
      ? safeStorage.decryptString(Buffer.from(enc, 'base64'))
      : Buffer.from(enc, 'base64').toString();
  } catch { return ''; }
}

function setAuthToken(token: string) {
  if (!token) { store.set('bizgrowhubTokenEncrypted', ''); return; }
  const enc = safeStorage.isEncryptionAvailable()
    ? safeStorage.encryptString(token).toString('base64')
    : Buffer.from(token).toString('base64');
  store.set('bizgrowhubTokenEncrypted', enc);
}

type LicenseStatus = { loggedIn: boolean; active: boolean; email: string; offline?: boolean };

async function checkLicense(): Promise<LicenseStatus> {
  const token = getAuthToken();
  const email = store.get('bizgrowhubEmail') || '';
  if (!token) return { loggedIn: false, active: false, email: '' };
  try {
    const res = await fetch(`${BIZGROWHUB_API}/api/bizvoice/license`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    if (res.status === 401) { setAuthToken(''); return { loggedIn: false, active: false, email: '' }; }
    const data = await res.json().catch(() => ({} as Record<string, unknown>));
    const active = !!(data as { active?: boolean }).active;
    if (active) store.set('licenseOkAt', Date.now());
    return { loggedIn: true, active, email };
  } catch {
    // Offline grace: trust the last successful check for 7 days so a paying
    // user isn't locked out by a flaky connection.
    const okAt = store.get('licenseOkAt') || 0;
    const active = okAt > 0 && (Date.now() - okAt) < 7 * 24 * 60 * 60 * 1000;
    return { loggedIn: true, active, email, offline: true };
  }
}

function openLogin() {
  if (loginWin) { loginWin.focus(); return; }
  loginWin = new BrowserWindow({
    width: 720,
    height: 560,
    frame: false,
    resizable: false,
    backgroundColor: '#0c0c11',
    icon: join(__dirname, '../../assets/icon.ico'),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      contextIsolation: true,
    },
  });
  loginWin.on('closed', () => (loginWin = null));
  if (VITE_DEV_SERVER_URL) loginWin.loadURL(`${VITE_DEV_SERVER_URL}login.html`);
  else loginWin.loadFile(join(__dirname, '../../dist/login.html'));
}

/** User is authenticated AND licensed — pull their settings, reveal the app. */
async function onLicensed() {
  if (loginWin) { loginWin.close(); loginWin = null; }
  const hadSettings = await loadRemoteSettings();
  if (!hadSettings) await pushRemoteSettings(); // seed defaults so the dashboard isn't empty
  if (!micBar) createMicBar();
  else micBar.show();
  registerHotkey();
}

ipcMain.handle('auth:status', async () => {
  const s = await checkLicense();
  if (s.active) onLicensed();
  return s;
});

// ── Browser-based sign-in (token handoff via a localhost loopback) ──────────
let authServer: Server | null = null;
let authState = '';

function stopAuthServer() {
  if (authServer) { try { authServer.close(); } catch { /* ignore */ } authServer = null; }
}

function notifyAuth(payload: { active: boolean; loggedIn: boolean; email: string }) {
  loginWin?.webContents.send('auth:changed', payload);
}

/**
 * Open BizGrowHub in the user's browser to sign in. We spin up a one-shot
 * loopback server on 127.0.0.1; the /desktop-auth web page hands the JWT back
 * to it once the user is authenticated. No password is ever typed in the app.
 */
function startBrowserLogin(): Promise<{ ok: boolean; error?: string }> {
  stopAuthServer();
  authState = randomBytes(16).toString('hex');

  return new Promise((resolve) => {
    authServer = createServer(async (req, res) => {
      try {
        const url = new URL(req.url || '', 'http://127.0.0.1');
        if (url.pathname !== '/callback') { res.writeHead(404); res.end(); return; }

        const token = url.searchParams.get('token') || '';
        const state = url.searchParams.get('state') || '';
        const reply = (body: string, code = 200) => {
          res.writeHead(code, { 'Content-Type': 'text/html; charset=utf-8' });
          res.end(`<!doctype html><html><head><meta charset="utf-8"></head><body style="font-family:system-ui;background:#0A0A0F;color:#fff;display:flex;align-items:center;justify-content:center;height:100vh;margin:0"><div style="text-align:center">${body}</div></body></html>`);
        };

        if (!token || state !== authState) {
          reply('<h2>Invalid sign-in</h2><p>Please retry from the BizVoice app.</p>', 400);
          return;
        }

        setAuthToken(token);
        // Best-effort: grab the email for display.
        try {
          const me = await fetch(`${BIZGROWHUB_API}/api/auth/me`, { headers: { Authorization: `Bearer ${token}` } });
          const data = await me.json().catch(() => ({} as Record<string, unknown>));
          const email = ((data as { user?: { email?: string }; email?: string }).user?.email
            || (data as { email?: string }).email || '') as string;
          if (email) store.set('bizgrowhubEmail', email);
        } catch { /* ignore */ }

        const lic = await checkLicense();
        reply('<h2>BizVoice connected ✓</h2><p>You can close this tab and return to the app.</p>');
        stopAuthServer();
        if (lic.active) onLicensed();
        notifyAuth({ active: lic.active, loggedIn: true, email: store.get('bizgrowhubEmail') || '' });
      } catch {
        try { res.writeHead(500); res.end('Error'); } catch { /* ignore */ }
      }
    });

    authServer.on('error', (err) => resolve({ ok: false, error: err.message }));
    authServer.listen(0, '127.0.0.1', () => {
      const addr = authServer?.address();
      const port = addr && typeof addr === 'object' ? addr.port : 0;
      if (!port) { resolve({ ok: false, error: 'Could not start local sign-in server' }); return; }
      shell.openExternal(`${BIZGROWHUB_API}/desktop-auth?port=${port}&state=${authState}`);
      resolve({ ok: true });
    });
  });
}

ipcMain.handle('auth:startBrowserLogin', () => startBrowserLogin());
ipcMain.handle('auth:cancelBrowserLogin', () => { stopAuthServer(); return { ok: true }; });

ipcMain.handle('auth:logout', () => {
  setAuthToken('');
  store.set('licenseOkAt', 0);
  micBar?.hide();
  return { ok: true };
});

ipcMain.handle('auth:openSubscribe', () => {
  shell.openExternal(`${BIZGROWHUB_API}/marketplace`);
});

ipcMain.handle('auth:openRegister', () => {
  shell.openExternal(`${BIZGROWHUB_API}/register`);
});

// ── App updates (GitHub Releases via electron-updater) ──────────────────────
// On launch + every hour we ask GitHub for the latest published release.
// If it's newer than the running version, a popup window asks the user
// whether to download. On download-complete we ask whether to restart.
interface UpdateRelease { version: string; notes?: string[]; releasedAt?: string }
let updateWin: BrowserWindow | null = null;
let latestUpdate: UpdateRelease | null = null;
let updateDownloaded = false;

autoUpdater.autoDownload = false;        // Ask the user before pulling the binary
autoUpdater.autoInstallOnAppQuit = true; // Apply pending update on next quit
autoUpdater.logger = {
  info:  (m: any) => logEvent('info',  `updater: ${typeof m === 'string' ? m : JSON.stringify(m)}`),
  warn:  (m: any) => logEvent('warn',  `updater: ${typeof m === 'string' ? m : JSON.stringify(m)}`),
  error: (m: any) => logEvent('error', `updater: ${typeof m === 'string' ? m : JSON.stringify(m)}`),
  debug: () => {},
} as any;

// GitHub returns release notes as Markdown-rendered HTML, not raw text —
// convert <p>/<br>/<li>/etc. into line breaks, strip any remaining tags,
// decode common HTML entities, and return a clean array of bullet lines.
function parseReleaseNotes(notes: unknown): string[] {
  if (!notes) return [];
  let raw: string;
  if (typeof notes === 'string') raw = notes;
  else if (Array.isArray(notes)) raw = notes.map((n: any) => (typeof n === 'string' ? n : n?.note ?? '')).join('\n');
  else return [];

  const cleaned = raw
    .replace(/<\/p>\s*<p[^>]*>/gi, '\n\n')                     // paragraph break
    .replace(/<br\s*\/?>/gi, '\n')                              // <br>
    .replace(/<\/?(p|div|li|section|article|h[1-6])[^>]*>/gi, '\n')  // block tags
    .replace(/<[^>]+>/g, '')                                    // strip remaining tags
    .replace(/&amp;/g, '&')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&nbsp;/g, ' ');

  return cleaned
    .split(/\n+/)
    .map((l) => l.trim().replace(/^[-*+]\s*/, ''))
    .filter((l) => l.length > 0);
}

autoUpdater.on('update-available', (info) => {
  latestUpdate = {
    version: info.version,
    notes: parseReleaseNotes(info.releaseNotes),
    releasedAt: info.releaseDate,
  };
  updateDownloaded = false;
  openUpdateWindow();
});

autoUpdater.on('update-not-available', () => {
  latestUpdate = null;
});

autoUpdater.on('error', (err) => {
  logEvent('warn', 'update check failed', { error: err?.message });
});

autoUpdater.on('download-progress', (p) => {
  updateWin?.webContents.send('update:progress', { percent: p.percent, transferred: p.transferred, total: p.total });
});

autoUpdater.on('update-downloaded', () => {
  updateDownloaded = true;
  if (updateWin && !updateWin.isDestroyed()) {
    updateWin.webContents.send('update:downloaded');
  } else {
    void promptInstall();
  }
});

async function promptInstall() {
  const { response } = await dialog.showMessageBox({
    type: 'info',
    title: 'Update Ready',
    message: `BizVoice ${latestUpdate?.version ?? ''} is downloaded.`,
    detail: 'The app needs to restart to apply the update.',
    buttons: ['Restart Now', 'Later'],
    defaultId: 0,
    cancelId: 1,
  });
  if (response === 0) autoUpdater.quitAndInstall();
}

function openUpdateWindow() {
  if (updateWin) { updateWin.focus(); return; }
  updateWin = new BrowserWindow({
    width: 520,
    height: 560,
    frame: false,
    resizable: false,
    backgroundColor: '#0c0c11',
    icon: join(__dirname, '../../assets/icon.ico'),
    webPreferences: { preload: join(__dirname, '../preload/index.js'), contextIsolation: true },
  });
  updateWin.on('closed', () => (updateWin = null));
  if (VITE_DEV_SERVER_URL) updateWin.loadURL(`${VITE_DEV_SERVER_URL}update.html`);
  else updateWin.loadFile(join(__dirname, '../../dist/update.html'));
}

async function checkForUpdate() {
  if (!app.isPackaged) return; // electron-updater no-ops in dev anyway, but skip the network call
  try { await autoUpdater.checkForUpdates(); }
  catch (err: any) { logEvent('warn', 'update check threw', { error: err?.message }); }
}

ipcMain.handle('update:info', async (_e, forceCheck?: boolean) => {
  // About page passes forceCheck=true so the user-facing "Check for updates"
  // button reflects a live GitHub query, not stale cached state. The popup
  // (Update.tsx) doesn't need it — it only opens after an update-available
  // event fires, so latestUpdate is already populated.
  if (forceCheck && app.isPackaged) {
    try { await autoUpdater.checkForUpdates(); }
    catch (err: any) { logEvent('warn', 'forced update check failed', { error: err?.message }); }
  }
  return {
    current: app.getVersion(),
    latest: latestUpdate,
    updateAvailable: !!latestUpdate,
    downloaded: updateDownloaded,
  };
});
ipcMain.handle('update:download', async () => {
  try { await autoUpdater.downloadUpdate(); }
  catch (err: any) { logEvent('error', 'update download failed', { error: err?.message }); }
});
ipcMain.handle('update:install', () => {
  if (updateDownloaded) autoUpdater.quitAndInstall();
});
ipcMain.handle('update:later', () => { updateWin?.close(); });

function createTray() {
  // public/ isn't bundled; dist/ is (Vite copies public assets there at build).
  const logoPath = app.isPackaged
    ? join(__dirname, '../../dist/logo.png')
    : join(__dirname, '../../public/logo.png');
  const icon = nativeImage.createFromPath(logoPath).resize({ width: 16, height: 16 });
  tray = new Tray(icon);
  tray.setToolTip('BizVoice');
  const menu = Menu.buildFromTemplate([
    { label: 'Show Mic Bar', click: () => (micBar ? micBar.show() : openLogin()) },
    { label: 'Settings', click: openSettings },
    { label: 'Check for Updates', click: () => { void checkForUpdate(); } },
    { type: 'separator' },
    { label: 'Quit', click: () => app.quit() },
  ]);
  tray.setContextMenu(menu);
  tray.on('click', () => (micBar ? micBar.show() : openLogin()));
}

let pttRecording = false;

const PTT_POLL_SCRIPT_PATH = join(app.getPath('temp'), 'bizvoice-ptt-poll.ps1');
const PTT_POLL_SCRIPT = `
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
public class KeyPoll {
  [DllImport("user32.dll")] static extern short GetAsyncKeyState(int vKey);
  public static void WaitForRelease(int vk) {
    Thread.Sleep(100);
    while ((GetAsyncKeyState(vk) & 0x8000) != 0) { Thread.Sleep(30); }
  }
}
'@
[KeyPoll]::WaitForRelease(0x20)
`;

function registerHotkey() {
  globalShortcut.unregisterAll();
  const hotkey = S.hotkey;
  const pttHotkey = S.pttHotkey;
  const cycleHotkey = S.cycleHotkey;

  if (hotkey) {
    globalShortcut.register(hotkey, () => {
      micBar?.webContents.send('hotkey:toggle');
    });
  }

  if (pttHotkey) {
    writeFileSync(PTT_POLL_SCRIPT_PATH, PTT_POLL_SCRIPT, 'utf-8');
    globalShortcut.register(pttHotkey, () => {
      if (pttRecording) return;
      pttRecording = true;
      micBar?.webContents.send('hotkey:ptt-start');
      logEvent('info', 'PTT: started, polling for Space release...');

      exec(
        `powershell -NoProfile -ExecutionPolicy Bypass -File "${PTT_POLL_SCRIPT_PATH}"`,
        { windowsHide: true },
        () => {
          if (pttRecording) {
            pttRecording = false;
            micBar?.webContents.send('hotkey:ptt-stop');
            logEvent('info', 'PTT: Space released, stopping');
          }
        }
      );
    });
  }

  if (cycleHotkey) {
    globalShortcut.register(cycleHotkey, () => {
      const modes = S.modes;
      if (!modes.length) return;
      const curId = S.activeMode;
      const idx = modes.findIndex(m => m.id === curId);
      const next = modes[(idx + 1) % modes.length];
      S.activeMode = next.id;
      void pushRemoteSettings();
      logEvent('info', `mode cycled → ${next.name}`);
      micBar?.webContents.send('mode:changed', next);
    });
  }
}

app.whenReady().then(async () => {
  // Enable SharedArrayBuffer (required by ONNX Runtime used in @ricky0123/vad-web).
  // credentialless COEP is less strict than require-corp and does not break external fetches.
  session.defaultSession.webRequest.onHeadersReceived((details, callback) => {
    callback({
      responseHeaders: {
        ...details.responseHeaders,
        'Cross-Origin-Opener-Policy': ['same-origin'],
        'Cross-Origin-Embedder-Policy': ['credentialless'],
      },
    });
  });

  // Grant microphone permission for renderer
  session.defaultSession.setPermissionRequestHandler((_wc, _permission, callback) => {
    callback(true);
  });
  session.defaultSession.setPermissionCheckHandler(() => true);
  try {
    if (process.platform === 'darwin') {
      await systemPreferences.askForMediaAccess('microphone');
    }
  } catch {}

  getLocalWhisper().then(lw => lw.setModelsDir(join(app.getPath('userData'), 'whisper-models'))).catch(() => {});

  // Pre-warm the typing helper so the first paste skips PowerShell+Add-Type startup.
  // Windows-only — macOS/Linux use clipboard+shortcut, no helper needed.
  if (process.platform === 'win32') ensureTypeProc();

  createTray();

  // Gate the app behind a BizGrowHub login + active BizVoice subscription.
  const lic = await checkLicense();
  if (lic.active) await onLicensed();
  else openLogin();

  // Check for a newer release shortly after launch (non-blocking).
  setTimeout(() => { void checkForUpdate(); }, 3000);

  // Hourly update check — fast enough to catch fresh releases without
  // hammering GitHub. License re-verification runs on its own slower cadence.
  setInterval(() => { void checkForUpdate(); }, 60 * 60 * 1000);

  // Re-verify license periodically so a lapsed/cancelled subscription locks
  // the app and a freshly-activated one unlocks it without a restart.
  setInterval(async () => {
    const l = await checkLicense();
    if (!l.active && micBar) { micBar.hide(); openLogin(); }
    else if (l.active && !micBar) onLicensed();
  }, 6 * 60 * 60 * 1000);
});

ipcMain.handle('micBar:contextMenu', () => {
  const menu = Menu.buildFromTemplate([
    { label: 'Settings', click: openSettings },
    { label: 'Hide', click: () => micBar?.hide() },
    { type: 'separator' },
    { label: 'Quit BizVoice', click: () => app.quit() },
  ]);
  menu.popup({ window: micBar ?? undefined });
});

app.on('will-quit', () => globalShortcut.unregisterAll());
app.on('window-all-closed', () => { /* keep running in tray */ });

// ---- Drag ---- Generic: operates on whichever window invoked the call,
// so both the mic bar and the settings window can drive their own position.
ipcMain.handle('micBar:getPos', (e) => {
  const w = BrowserWindow.fromWebContents(e.sender);
  return w ? w.getPosition() : [0, 0];
});
ipcMain.handle('micBar:setPos', (e, x: number, y: number) => {
  const w = BrowserWindow.fromWebContents(e.sender);
  w?.setPosition(Math.round(x), Math.round(y));
});

// ---- System audio mute (explicit set via Core Audio COM) ----
let systemMuted = false;
const muteScriptPath = join(app.getPath('temp'), 'bizvoice-mute.ps1');
ipcMain.handle('audio:mute', (_e, mute: boolean) => {
  if (mute === systemMuted) return;
  systemMuted = mute;
  const ps = `
Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
[Guid("5CDF2C82-841E-4546-9722-0CF74078229A"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IAudioEndpointVolume {
  int f();int g();int h();int i();
  int SetMasterVolumeLevel(float a,System.Guid b);int j();
  int GetMasterVolumeLevelScalar(out float a);int k();int l();int m();int n();
  int SetMute([MarshalAs(UnmanagedType.Bool)]bool a,System.Guid b);
  int GetMute(out bool a);
}
[Guid("D666063F-1587-4E43-81F1-B948E807363F"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDevice { int Activate(ref System.Guid id,int ctx,System.IntPtr a,[MarshalAs(UnmanagedType.IUnknown)]out object o); }
[Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDeviceEnumerator { int f();int GetDefaultAudioEndpoint(int dataFlow,int role,out IMMDevice ep); }
[ComImport,Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumeratorComObject {}
public class Audio {
  public static void SetMute(bool m){
    var e=(IMMDeviceEnumerator)(new MMDeviceEnumeratorComObject());
    IMMDevice dev;e.GetDefaultAudioEndpoint(0,1,out dev);
    var iid=typeof(IAudioEndpointVolume).GUID;object o;dev.Activate(ref iid,1,System.IntPtr.Zero,out o);
    ((IAudioEndpointVolume)o).SetMute(m,System.Guid.Empty);
  }
}
'@
[Audio]::SetMute(${mute ? '$true' : '$false'})
`;
  try {
    writeFileSync(muteScriptPath, ps, 'utf-8');
    exec(`powershell -NoProfile -ExecutionPolicy Bypass -File "${muteScriptPath}"`, { windowsHide: true }, () => {});
  } catch {}
});

// ---- IPC ----
const LEGACY_LANG: Record<string, string> = { en: 'English', bn: 'Bangla' };
const normLang = (v: string) => LEGACY_LANG[v] ?? v;

ipcMain.handle('settings:get', async () => {
  // Settings come from BizGrowHub; refresh the cache on open. Keys stay local.
  await loadRemoteSettings();
  return {
    hotkey: S.hotkey,
    pttHotkey: S.pttHotkey,
    cycleHotkey: S.cycleHotkey,
    inputLang: normLang(S.inputLang),
    outputLang: normLang(S.outputLang),
    gptModel: S.gptModel,
    sttModel: S.sttModel,
    launchOnStartup: S.launchOnStartup,
    micDeviceId: S.micDeviceId,
    vocabulary: S.vocabulary,
    modes: S.modes,
    activeMode: S.activeMode,
    silenceMs: S.silenceMs,
    autoStop: S.autoStop,
    useLocalWhisper: S.useLocalWhisper,
    localModel: S.localModel,
    sttProvider: S.sttProvider,
    skipGpt: S.skipGpt,
    micFallbackId: S.micFallbackId,
    muteWhileRecording: S.muteWhileRecording,
    dictionary: S.dictionary,
    theme: S.theme,
    widgetStyle: S.widgetStyle,
    instructions: S.instructions,
    gptProvider: S.gptProvider,
    customBaseUrl: S.customBaseUrl,
    customChatModel: S.customChatModel,
    customHeaders: S.customHeaders,
    hasKey:           !!store.get('openaiKeyEncrypted'),
    hasGroqKey:       !!store.get('groqKeyEncrypted'),
    hasOpenrouterKey: !!store.get('openrouterKeyEncrypted'),
    hasCustomKey:     !!store.get('customKeyEncrypted'),
  };
});

ipcMain.handle('history:get', async () => {
  try {
    const res = await bghAuthFetch('/api/bizvoice/history?limit=200');
    if (!res.ok) return [];
    const data = await res.json().catch(() => ({ history: [] }));
    return data.history ?? [];
  } catch { return []; }
});
ipcMain.handle('history:clear', async () => {
  try { await bghAuthFetch('/api/bizvoice/history', { method: 'DELETE' }); } catch { /* ignore */ }
  store.set('refinedCache' as any, {});
});

// ── History refine ─────────────────────────────────────────────────────────
// On-demand grammar-correction of a past transcription. The refined output is
// cached locally (keyed by original entry timestamp) so subsequent views don't
// re-hit the API. History entries themselves live in BizGrowHub; only the
// cached corrections live here.
//
// Honours the user's gptProvider preference; falls back through other
// configured keys when the selected provider has no key.
const REFINE_SYSTEM_PROMPT =
  "You correct grammar, spelling, and punctuation in the user's transcribed speech. " +
  "Keep the original meaning, tone, and language. Do NOT translate. " +
  "Do NOT add greetings, explanations, or commentary. " +
  "Return ONLY the corrected sentence — nothing else.";

type RefineClient = { client: OpenAI; model: string; label: string };

async function buildRefineClient(): Promise<RefineClient> {
  const OpenAI = (await import('openai')).default;
  const want = S.gptProvider ?? 'openai';

  const openaiKey     = getApiKey();
  const groqKey       = getGroqKey();
  const openrouterKey = getOpenrouterKey();
  const customKey     = getCustomKey();

  const tryBuild = (p: GptProvider): RefineClient | null => {
    if (p === 'openai'     && openaiKey)     return { client: new OpenAI({ apiKey: openaiKey }), model: S.gptModel || 'gpt-4o-mini', label: 'openai' };
    if (p === 'groq'       && groqKey)       return { client: new OpenAI({ apiKey: groqKey, baseURL: 'https://api.groq.com/openai/v1' }), model: 'llama-3.1-8b-instant', label: 'groq' };
    if (p === 'openrouter' && openrouterKey) return { client: new OpenAI({ apiKey: openrouterKey, baseURL: 'https://openrouter.ai/api/v1', defaultHeaders: { 'HTTP-Referer': 'https://github.com/webdevarif/BizVoice', 'X-Title': 'BizVoice' } }), model: S.gptModel || 'meta-llama/llama-3.1-8b-instruct', label: 'openrouter' };
    if (p === 'custom'     && customKey && S.customBaseUrl) {
      let headers: Record<string, string> = {};
      if (S.customHeaders) { try { headers = JSON.parse(S.customHeaders); } catch {} }
      return { client: new OpenAI({ apiKey: customKey, baseURL: S.customBaseUrl, defaultHeaders: headers }), model: S.customChatModel || S.gptModel || 'auto', label: 'custom' };
    }
    return null;
  };

  // First: try the user's chosen provider. Then fall back to any other
  // configured key in a sensible order (cheap/fast first).
  const order: GptProvider[] = [want, ...(['groq', 'openai', 'openrouter', 'custom'] as GptProvider[]).filter((p) => p !== want)];
  for (const p of order) {
    const built = tryBuild(p);
    if (built) return built;
  }
  throw new Error('Refine needs an API key (OpenAI, Groq, OpenRouter, or Custom). Open Settings → Transcription.');
}

async function callRefine(text: string): Promise<string> {
  const { client, model, label } = await buildRefineClient();
  logEvent('info', `refine via ${label} (${model})`);
  const completion = await client.chat.completions.create({
    model,
    temperature: 0.2,
    max_tokens: Math.min(1024, Math.ceil(text.length * 1.5) + 64),
    messages: [
      { role: 'system', content: REFINE_SYSTEM_PROMPT },
      { role: 'user',   content: text },
    ],
  });
  const refined = completion.choices[0]?.message?.content?.trim();
  if (!refined) throw new Error('Empty refinement result');
  return refined;
}

ipcMain.handle('history:refine', async (_e, text: string, ts: number) => {
  const cache = (store.get('refinedCache' as any) as Record<string, string>) || {};
  if (cache[String(ts)]) return cache[String(ts)];
  const refined = await callRefine(text);
  cache[String(ts)] = refined;
  store.set('refinedCache' as any, cache);
  return refined;
});

ipcMain.handle('history:getRefinedCache', () => {
  const cache = (store.get('refinedCache' as any) as Record<string, string>) || {};
  // Convert string keys back to number keys for the renderer's Record<number, string>
  const out: Record<number, string> = {};
  for (const [k, v] of Object.entries(cache)) out[Number(k)] = v;
  return out;
});
ipcMain.handle('stats:get', async () => {
  try {
    const res = await bghAuthFetch('/api/bizvoice/stats');
    if (!res.ok) return { recordings: 0, words: 0, durationMs: 0 };
    return await res.json();
  } catch { return { recordings: 0, words: 0, durationMs: 0 }; }
});

// Encrypt-or-clear a single key field. Centralises the safeStorage pattern
// used for every provider key (OpenAI, Groq, OpenRouter, Custom).
function persistEncryptedKey(storeKey: keyof Settings, raw: string) {
  if (raw === '') { store.set(storeKey, '' as any); return; }
  const enc = safeStorage.isEncryptionAvailable()
    ? safeStorage.encryptString(raw).toString('base64')
    : Buffer.from(raw).toString('base64');
  store.set(storeKey, enc as any);
}

const KEY_PATCH_FIELDS = ['openaiKey', 'groqKey', 'openrouterKey', 'customKey'] as const;

ipcMain.handle('settings:set', (_e, patch: Partial<Settings & { openaiKey: string; groqKey: string; openrouterKey: string; customKey: string }>) => {
  try {
    if (patch.openaiKey     !== undefined) { persistEncryptedKey('openaiKeyEncrypted',     patch.openaiKey);     delete (patch as any).openaiKey; }
    if (patch.groqKey       !== undefined) { persistEncryptedKey('groqKeyEncrypted',       patch.groqKey);       delete (patch as any).groqKey; }
    if (patch.openrouterKey !== undefined) { persistEncryptedKey('openrouterKeyEncrypted', patch.openrouterKey); delete (patch as any).openrouterKey; }
    if (patch.customKey     !== undefined) { persistEncryptedKey('customKeyEncrypted',     patch.customKey);     delete (patch as any).customKey; }
    // Non-key fields update the in-memory cache and sync to BizGrowHub.
    for (const [k, v] of Object.entries(patch)) {
      if (v !== undefined && !(KEY_PATCH_FIELDS as readonly string[]).includes(k)) {
        (S as Record<string, unknown>)[k] = v;
      }
    }
    void pushRemoteSettings();
    if (patch.hotkey || (patch as any).pttHotkey || patch.cycleHotkey) registerHotkey();
    // Notify mic bar of appearance changes live
    if (patch.theme !== undefined || patch.widgetStyle !== undefined) {
      micBar?.webContents.send('appearance:changed', {
        theme: S.theme,
        widgetStyle: S.widgetStyle,
      });
    }
    return { ok: true };
  } catch (err: any) {
    console.error('settings:set failed', err);
    return { ok: false, error: err?.message ?? 'Save failed' };
  }
});

ipcMain.handle('settings:open', () => openSettings());
ipcMain.handle('window:close', (e) => BrowserWindow.fromWebContents(e.sender)?.close());
ipcMain.handle('window:minimize', (e) => BrowserWindow.fromWebContents(e.sender)?.hide());

// ---- Whisper model management ----
ipcMain.handle('whisper:listModels', async () => {
  const lw = await getLocalWhisper();
  return lw.listModels();
});
ipcMain.handle('whisper:deleteModel', async (_e, name: string) => {
  const lw = await getLocalWhisper();
  return lw.deleteModel(name);
});
ipcMain.handle('whisper:downloadModel', async (e, name: string) => {
  const sender = e.sender;
  try {
    const lw = await getLocalWhisper();
    await lw.downloadModel(name, (pct: number) => {
      sender.send('whisper:downloadProgress', { model: name, pct });
    });
    return { ok: true };
  } catch (err: any) {
    return { ok: false, error: err?.message ?? 'Download failed' };
  }
});

ipcMain.handle('transcribe', async (_e, audioBase64: string, durationMs: number = 0) => {
  logEvent('info', `transcribe: received audio (${audioBase64.length} base64 chars)`);

  const inputLang = S.inputLang;
  const outputLang = S.outputLang;
  const gptModel = S.gptModel;
  const sttModel = S.sttModel;
  const sttProvider = S.sttProvider;
  const skipGpt = S.skipGpt;
  const vocabulary = S.vocabulary;
  const mode = getActiveMode();
  const useLocal = S.useLocalWhisper;
  const localModel = S.localModel;
  const instructions = S.instructions;

  // Custom instructions get appended to the active style prompt
  const stylePrompt = instructions?.trim()
    ? `${mode.prompt}\n\nAdditional user instructions: ${instructions.trim()}`
    : mode.prompt;

  logEvent('info', `pipeline start`, { inputLang, outputLang, sttProvider, sttModel, mode: mode.name, useLocal, skipGpt });

  try {
    const text = await runPipeline({
      audioBase64,
      apiKey: getApiKey(),
      groqKey: getGroqKey(),
      openrouterKey: getOpenrouterKey(),
      customKey: getCustomKey(),
      customBaseUrl: S.customBaseUrl,
      customHeaders: S.customHeaders,
      inputLang,
      outputLang,
      gptModel: S.gptProvider === 'custom' ? (S.customChatModel || gptModel) : gptModel,
      gptProvider: S.gptProvider,
      sttModel,
      sttProvider,
      skipGpt,
      vocabulary,
      styleSystemPrompt: stylePrompt,
      useLocalWhisper: useLocal,
      localModel,
      log: (msg, data) => logEvent('info', msg, data),
    });
    const finalText = applyDictionary(text);
    if (finalText) void recordTranscription(finalText, durationMs);
    logEvent('info', 'pipeline done', { text: finalText });
    return finalText;
  } catch (err: any) {
    logEvent('error', 'pipeline failed', { error: err?.message });
    throw err;
  }
});

// History + stats live in BizGrowHub — post each transcription there.
async function recordTranscription(text: string, durationMs: number) {
  const words = text.trim().split(/\s+/).filter(Boolean).length;
  try {
    await bghAuthFetch('/api/bizvoice/history', {
      method: 'POST',
      body: JSON.stringify({ text, words, durationMs }),
    });
  } catch { /* offline — entry is lost; acceptable for a dictation log */ }
}

function applyDictionary(text: string): string {
  const dict = S.dictionary || [];
  let out = text;
  for (const { from, to } of dict) {
    if (!from?.trim()) continue;
    const escaped = from.trim().replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    out = out.replace(new RegExp(`\\b${escaped}\\b`, 'gi'), to);
  }
  return out;
}

function decryptStoredKey(storeKey: keyof Settings): string {
  const enc = store.get(storeKey) as string;
  if (!enc) return '';
  return safeStorage.isEncryptionAvailable()
    ? safeStorage.decryptString(Buffer.from(enc, 'base64'))
    : Buffer.from(enc, 'base64').toString();
}

function getApiKey():        string { return decryptStoredKey('openaiKeyEncrypted'); }
function getGroqKey():       string { return decryptStoredKey('groqKeyEncrypted'); }
function getOpenrouterKey(): string { return decryptStoredKey('openrouterKeyEncrypted'); }
function getCustomKey():     string { return decryptStoredKey('customKeyEncrypted'); }

// -- Text injection helper (Windows) --
// Two strategies, both via Win32 SendInput:
//   PASTE  → clipboard already populated by Node; PS sends Ctrl+V (or
//            Ctrl+Shift+V for terminals it detects). INSTANT for any text
//            length — the apps process one shortcut, not N character events.
//   TYPE   → fallback that types characters one-by-one with KEYEVENTF_UNICODE.
//            Slow but works when paste-shortcut isn't supported.
// A long-running PowerShell process hosts the helper so each invocation
// skips PowerShell+Add-Type startup cost.
const TYPE_SCRIPT_PATH = join(app.getPath('temp'), 'bizvoice-type.ps1');

const TYPE_PS_SCRIPT = [
  "$ErrorActionPreference = 'Continue'",
  '$OutputEncoding = [Text.UTF8Encoding]::new()',
  '[Console]::InputEncoding = [Text.UTF8Encoding]::new()',
  'Add-Type -TypeDefinition @"',
  'using System;',
  'using System.Runtime.InteropServices;',
  'using System.Text;',
  'using System.Threading;',
  '',
  'public static class Typer {',
  '    [StructLayout(LayoutKind.Sequential)]',
  '    public struct INPUT { public int type; public InputUnion U; }',
  '',
  '    [StructLayout(LayoutKind.Explicit)]',
  '    public struct InputUnion {',
  '        [FieldOffset(0)] public MOUSEINPUT mi;',
  '        [FieldOffset(0)] public KEYBDINPUT ki;',
  '        [FieldOffset(0)] public HARDWAREINPUT hi;',
  '    }',
  '',
  '    [StructLayout(LayoutKind.Sequential)]',
  '    public struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }',
  '',
  '    [StructLayout(LayoutKind.Sequential)]',
  '    public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }',
  '',
  '    [StructLayout(LayoutKind.Sequential)]',
  '    public struct HARDWAREINPUT { public uint uMsg; public ushort wParamL; public ushort wParamH; }',
  '',
  '    [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint n, INPUT[] inputs, int cbSize);',
  '    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();',
  '    [DllImport("user32.dll", CharSet = CharSet.Auto)] public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);',
  '',
  '    public const int INPUT_KEYBOARD = 1;',
  '    public const uint KEYEVENTF_KEYUP = 0x0002;',
  '    public const uint KEYEVENTF_UNICODE = 0x0004;',
  '    public const ushort VK_RETURN  = 0x0D;',
  '    public const ushort VK_CONTROL = 0x11;',
  '    public const ushort VK_SHIFT   = 0x10;',
  '    public const ushort VK_V       = 0x56;',
  '',
  '    public const int KEY_DELAY_MS = 3;  // for the TYPE fallback only',
  '',
  '    // Single-shortcut paste — auto-detects terminals and uses Ctrl+Shift+V there.',
  '    public static void Paste() {',
  '        var sb = new StringBuilder(256);',
  '        GetClassName(GetForegroundWindow(), sb, sb.Capacity);',
  '        string cls = sb.ToString();',
  '        bool useShift = IsTerminal(cls);',
  '        int n = useShift ? 6 : 4;',
  '        INPUT[] inputs = new INPUT[n];',
  '        int i = 0;',
  '        inputs[i++] = MakeVkInput(VK_CONTROL, false);',
  '        if (useShift) inputs[i++] = MakeVkInput(VK_SHIFT, false);',
  '        inputs[i++] = MakeVkInput(VK_V, false);',
  '        inputs[i++] = MakeVkInput(VK_V, true);',
  '        if (useShift) inputs[i++] = MakeVkInput(VK_SHIFT, true);',
  '        inputs[i++] = MakeVkInput(VK_CONTROL, true);',
  '        SendInput((uint)n, inputs, Marshal.SizeOf(typeof(INPUT)));',
  '    }',
  '',
  '    public static bool IsTerminal(string cls) {',
  '        if (string.IsNullOrEmpty(cls)) return false;',
  '        if (cls == "CASCADIA_HOSTING_WINDOW_CLASS") return true;  // Windows Terminal',
  '        if (cls == "ConsoleWindowClass") return true;             // legacy conhost (cmd/PowerShell)',
  '        if (cls == "mintty") return true;                         // Git Bash / Cygwin',
  '        if (cls.IndexOf("Console",  StringComparison.OrdinalIgnoreCase) >= 0) return true;',
  '        if (cls.IndexOf("Terminal", StringComparison.OrdinalIgnoreCase) >= 0) return true;',
  '        return false;',
  '    }',
  '',
  '    public static bool IsTerminalNow() {',
  '        var sb = new StringBuilder(256);',
  '        GetClassName(GetForegroundWindow(), sb, sb.Capacity);',
  '        return IsTerminal(sb.ToString());',
  '    }',
  '',
  '    static INPUT MakeVkInput(ushort vk, bool keyUp) {',
  '        INPUT inp = new INPUT();',
  '        inp.type = INPUT_KEYBOARD;',
  '        inp.U.ki.wVk = vk;',
  '        if (keyUp) inp.U.ki.dwFlags = KEYEVENTF_KEYUP;',
  '        return inp;',
  '    }',
  '',
  '    // Fallback: type characters one-by-one (KEYEVENTF_UNICODE).',
  '    public static void Type(string text) {',
  '        foreach (char c in text) {',
  "            if (c == '\\r') { continue; }",
  "            if (c == '\\n') { SendVk(VK_RETURN); }",
  '            else { SendUnicode(c); }',
  '            if (KEY_DELAY_MS > 0) Thread.Sleep(KEY_DELAY_MS);',
  '        }',
  '    }',
  '',
  '    static void SendUnicode(char c) {',
  '        int size = Marshal.SizeOf(typeof(INPUT));',
  '        INPUT[] inputs = new INPUT[2];',
  '        inputs[0].type = INPUT_KEYBOARD;',
  '        inputs[0].U.ki.wScan = (ushort)c;',
  '        inputs[0].U.ki.dwFlags = KEYEVENTF_UNICODE;',
  '        inputs[1].type = INPUT_KEYBOARD;',
  '        inputs[1].U.ki.wScan = (ushort)c;',
  '        inputs[1].U.ki.dwFlags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP;',
  '        SendInput(2, inputs, size);',
  '    }',
  '',
  '    static void SendVk(ushort vk) {',
  '        int size = Marshal.SizeOf(typeof(INPUT));',
  '        INPUT[] inputs = new INPUT[2];',
  '        inputs[0].type = INPUT_KEYBOARD;',
  '        inputs[0].U.ki.wVk = vk;',
  '        inputs[1].type = INPUT_KEYBOARD;',
  '        inputs[1].U.ki.wVk = vk;',
  '        inputs[1].U.ki.dwFlags = KEYEVENTF_KEYUP;',
  '        SendInput(2, inputs, size);',
  '    }',
  '}',
  '"@',
  '',
  'while (($line = [Console]::In.ReadLine()) -ne $null) {',
  '    try {',
  '        if ([string]::IsNullOrEmpty($line)) { continue }',
  "        if ($line -eq 'PASTE') {",
  '            [Typer]::Paste()',
  "        } elseif ($line.StartsWith('TYPE:')) {",
  '            $b64 = $line.Substring(5)',
  '            $bytes = [Convert]::FromBase64String($b64)',
  '            $text = [Text.Encoding]::UTF8.GetString($bytes)',
  '            [Typer]::Type($text)',
  "        } elseif ($line.StartsWith('SMART:')) {",
  '            # PS-side decision: terminals (incl. TUIs like Claude Code, vim,',
  '            # tmux) get character typing so the TUI sees real key events',
  '            # instead of an intercepted paste shortcut. Everything else gets',
  '            # the instant clipboard-paste shortcut.',
  '            $b64 = $line.Substring(6)',
  '            $bytes = [Convert]::FromBase64String($b64)',
  '            $text = [Text.Encoding]::UTF8.GetString($bytes)',
  '            if ([Typer]::IsTerminalNow()) {',
  '                [Typer]::Type($text)',
  '            } else {',
  '                [Typer]::Paste()',
  '            }',
  '        }',
  '    } catch {',
  '        # ignore line errors so the worker keeps running',
  '    }',
  '}',
].join('\n');

let typeProc: ChildProcess | null = null;

function ensureTypeProc() {
  if (typeProc && !typeProc.killed && typeProc.exitCode === null) return;
  try {
    writeFileSync(TYPE_SCRIPT_PATH, TYPE_PS_SCRIPT, 'utf-8');
    typeProc = spawn('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', TYPE_SCRIPT_PATH,
    ], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    typeProc.on('exit', (code) => {
      logEvent('warn', 'type proc exited', { code });
      typeProc = null;
    });
    typeProc.stdout?.on('data', () => { /* drain */ });
    typeProc.stderr?.on('data', (d) => logEvent('warn', 'type proc stderr', d.toString()));
    logEvent('info', 'type proc started');
  } catch (err: any) {
    logEvent('error', 'type proc spawn failed', { error: err?.message });
    typeProc = null;
  }
}

function pasteWindows(text: string) {
  ensureTypeProc();
  if (!typeProc || !typeProc.stdin || typeProc.stdin.destroyed) {
    logEvent('error', 'type proc not available');
    return;
  }
  // SMART: helper inspects the foreground window. Terminal windows get
  // character-typed (works in TUIs like Claude Code / vim / tmux that
  // intercept Ctrl+V for their own purposes). Everything else gets the
  // instant clipboard-paste shortcut. Clipboard is staged either way so
  // the paste branch has data ready.
  const prev = clipboard.readText();
  clipboard.writeText(text);
  const b64 = Buffer.from(text, 'utf-8').toString('base64');
  setTimeout(() => { typeProc?.stdin?.write(`SMART:${b64}\n`); }, 50);
  // Restore original clipboard once the paste/type has landed. Big terminals
  // can take ~1s for long text via char-typing, so wait a bit longer.
  setTimeout(() => { try { clipboard.writeText(prev); } catch {} }, 2000);
}

// macOS: Cmd+V works in Terminal.app, iTerm2, VS Code terminal, and every app —
// no character-typing fallback needed. Requires Accessibility permission for the
// keystroke step (user is prompted on first run).
function pasteMac(text: string) {
  const prev = clipboard.readText();
  clipboard.writeText(text);
  setTimeout(() => {
    exec(`osascript -e 'tell application "System Events" to keystroke "v" using command down'`, (err: any) => {
      if (err) logEvent('error', 'mac paste failed', { error: err.message });
    });
    setTimeout(() => clipboard.writeText(prev), 800);
  }, 50);
}

// Linux: matches macOS pattern — clipboard + Ctrl+V via xdotool. Terminal
// behaviour varies by emulator; most modern ones (gnome-terminal, konsole)
// accept Ctrl+Shift+V — pass that flavour where needed in a future revision.
function pasteLinux(text: string) {
  const prev = clipboard.readText();
  clipboard.writeText(text);
  setTimeout(() => {
    exec(`xdotool key --clearmodifiers ctrl+v`, (err: any) => {
      if (err) logEvent('error', 'linux paste failed', { error: err.message });
    });
    setTimeout(() => clipboard.writeText(prev), 800);
  }, 50);
}

ipcMain.handle('paste', (_e, text: string) => {
  logEvent('info', `paste/type [${process.platform}]: "${text.slice(0, 80)}"`);
  if (process.platform === 'win32')      pasteWindows(text);
  else if (process.platform === 'darwin') pasteMac(text);
  else                                    pasteLinux(text);
});

app.on('before-quit', () => {
  try { typeProc?.kill(); } catch { /* ignore */ }
  typeProc = null;
});
