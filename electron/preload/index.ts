import { contextBridge, ipcRenderer } from 'electron';

contextBridge.exposeInMainWorld('api', {
  getSettings: () => ipcRenderer.invoke('settings:get'),
  setSettings: (patch: any) => ipcRenderer.invoke('settings:set', patch),
  openSettings: () => ipcRenderer.invoke('settings:open'),
  closeWindow: () => ipcRenderer.invoke('window:close'),
  minimizeWindow: () => ipcRenderer.invoke('window:minimize'),
  transcribe: (audioBase64: string, durationMs?: number) => ipcRenderer.invoke('transcribe', audioBase64, durationMs),
  paste: (text: string) => ipcRenderer.invoke('paste', text),
  onHotkey: (cb: () => void) => {
    ipcRenderer.on('hotkey:toggle', cb);
    return () => { ipcRenderer.removeListener('hotkey:toggle', cb); };
  },
  onPttStart: (cb: () => void) => {
    ipcRenderer.on('hotkey:ptt-start', cb);
    return () => { ipcRenderer.removeListener('hotkey:ptt-start', cb); };
  },
  onPttStop: (cb: () => void) => {
    ipcRenderer.on('hotkey:ptt-stop', cb);
    return () => { ipcRenderer.removeListener('hotkey:ptt-stop', cb); };
  },
  onModeChange: (cb: (mode: { id: string; name: string; color: string; prompt: string }) => void) => {
    const handler = (_e: any, mode: any) => cb(mode);
    ipcRenderer.on('mode:changed', handler);
    return () => { ipcRenderer.removeListener('mode:changed', handler); };
  },
  micBarContextMenu: () => ipcRenderer.invoke('micBar:contextMenu'),
  resizeMicBar: (active: boolean) => ipcRenderer.invoke('micBar:resize', active),
  getWinPos: () => ipcRenderer.invoke('micBar:getPos') as Promise<[number, number]>,
  setWinPos: (x: number, y: number) => ipcRenderer.invoke('micBar:setPos', x, y),
  muteSystem: (mute: boolean) => ipcRenderer.invoke('audio:mute', mute),
  getHistory: () => ipcRenderer.invoke('history:get') as Promise<{ text: string; ts: number; words: number; durationMs: number }[]>,
  clearHistory: () => ipcRenderer.invoke('history:clear'),
  refineText: (text: string, ts: number) => ipcRenderer.invoke('history:refine', text, ts) as Promise<string>,
  getRefinedCache: () => ipcRenderer.invoke('history:getRefinedCache') as Promise<Record<number, string>>,
  getStats: () => ipcRenderer.invoke('stats:get') as Promise<{ recordings: number; words: number; durationMs: number }>,
  whisperListModels: () => ipcRenderer.invoke('whisper:listModels'),
  whisperDownloadModel: (name: string) => ipcRenderer.invoke('whisper:downloadModel', name),
  whisperDeleteModel: (name: string) => ipcRenderer.invoke('whisper:deleteModel', name),
  onWhisperDownloadProgress: (cb: (data: { model: string; pct: number }) => void) => {
    const handler = (_e: any, data: any) => cb(data);
    ipcRenderer.on('whisper:downloadProgress', handler);
    return () => { ipcRenderer.removeListener('whisper:downloadProgress', handler); };
  },
  onAppearanceChange: (cb: (data: { theme: string; widgetStyle: string }) => void) => {
    const handler = (_e: any, data: any) => cb(data);
    ipcRenderer.on('appearance:changed', handler);
    return () => { ipcRenderer.removeListener('appearance:changed', handler); };
  },
  // BizGrowHub auth + BizVoice license (browser-based token handoff)
  startBrowserLogin: () => ipcRenderer.invoke('auth:startBrowserLogin'),
  cancelBrowserLogin: () => ipcRenderer.invoke('auth:cancelBrowserLogin'),
  logout: () => ipcRenderer.invoke('auth:logout'),
  authStatus: () => ipcRenderer.invoke('auth:status'),
  openSubscribe: () => ipcRenderer.invoke('auth:openSubscribe'),
  openRegister: () => ipcRenderer.invoke('auth:openRegister'),
  onAuthChange: (cb: (data: { active: boolean; loggedIn: boolean; email: string }) => void) => {
    const handler = (_e: any, data: any) => cb(data);
    ipcRenderer.on('auth:changed', handler);
    return () => { ipcRenderer.removeListener('auth:changed', handler); };
  },
  // App updates (GitHub Releases via electron-updater)
  updateInfo: () => ipcRenderer.invoke('update:info'),
  updateDownload: () => ipcRenderer.invoke('update:download'),
  updateInstall: () => ipcRenderer.invoke('update:install'),
  updateLater: () => ipcRenderer.invoke('update:later'),
  onUpdateProgress: (cb: (data: { percent: number; transferred: number; total: number }) => void) => {
    const handler = (_e: any, data: any) => cb(data);
    ipcRenderer.on('update:progress', handler);
    return () => { ipcRenderer.removeListener('update:progress', handler); };
  },
  onUpdateDownloaded: (cb: () => void) => {
    ipcRenderer.on('update:downloaded', cb);
    return () => { ipcRenderer.removeListener('update:downloaded', cb); };
  },
});

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
      updateInfo: () => Promise<{ current: string; latest: { version: string; notes?: string[]; releasedAt?: string } | null; updateAvailable: boolean; downloaded: boolean }>;
      updateDownload: () => Promise<void>;
      updateInstall: () => Promise<void>;
      updateLater: () => Promise<void>;
      onUpdateProgress: (cb: (data: { percent: number; transferred: number; total: number }) => void) => () => void;
      onUpdateDownloaded: (cb: () => void) => () => void;
    };
  }
}
