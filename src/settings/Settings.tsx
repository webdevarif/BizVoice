import { useEffect, useRef, useState, useCallback } from 'react';
import Lenis from 'lenis';
import ReactSelect, { type StylesConfig } from 'react-select';

type Tab = 'overview' | 'history' | 'dictionary' | 'instructions' | 'general' | 'shortcuts' | 'languages' | 'audio' | 'appearance' | 'about';
type Lang = string;
type DictEntry = { from: string; to: string; category?: string; ts?: number };
type HistEntry = { text: string; ts: number; words: number; durationMs: number };

const LANGS: { value: Lang; label: string }[] = [
  { value: 'auto', label: 'Auto detect' },
  { value: 'English', label: 'English' },
  { value: 'Bangla', label: 'Bangla (বাংলা)' },
  { value: 'Hindi', label: 'Hindi (हिन्दी)' },
  { value: 'Urdu', label: 'Urdu (اردو)' },
  { value: 'Arabic', label: 'Arabic (العربية)' },
  { value: 'Spanish', label: 'Spanish (Español)' },
  { value: 'French', label: 'French (Français)' },
  { value: 'German', label: 'German (Deutsch)' },
  { value: 'Italian', label: 'Italian (Italiano)' },
  { value: 'Portuguese', label: 'Portuguese (Português)' },
  { value: 'Russian', label: 'Russian (Русский)' },
  { value: 'Chinese', label: 'Chinese (中文)' },
  { value: 'Japanese', label: 'Japanese (日本語)' },
  { value: 'Korean', label: 'Korean (한국어)' },
  { value: 'Turkish', label: 'Turkish (Türkçe)' },
  { value: 'Dutch', label: 'Dutch (Nederlands)' },
  { value: 'Polish', label: 'Polish (Polski)' },
  { value: 'Vietnamese', label: 'Vietnamese (Tiếng Việt)' },
  { value: 'Thai', label: 'Thai (ไทย)' },
  { value: 'Indonesian', label: 'Indonesian (Bahasa)' },
];

const ic = (p: JSX.Element) => (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">{p}</svg>
);

const COG = ic(<><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></>);
const NAV: { id: Tab; label: string; icon: JSX.Element }[] = [
  { id: 'overview',   label: 'Overview',   icon: ic(<><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/></>) },
  { id: 'history',    label: 'History',    icon: ic(<><path d="M3 3v5h5"/><path d="M3.05 13A9 9 0 1 0 6 5.3L3 8"/><path d="M12 7v5l4 2"/></>) },
  { id: 'dictionary', label: 'Dictionary', icon: ic(<><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></>) },
  { id: 'instructions', label: 'Instructions', icon: ic(<><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><path d="M14 2v6h6M16 13H8M16 17H8M10 9H8"/></>) },
  { id: 'general',    label: 'Transcription', icon: COG },
  { id: 'shortcuts',  label: 'Shortcuts',  icon: ic(<><rect x="2" y="4" width="20" height="16" rx="2"/><path d="M6 8h.01M10 8h.01M14 8h.01M18 8h.01M6 12h.01M10 12h.01M14 12h.01M18 12h.01M7 16h10"/></>) },
  { id: 'languages',  label: 'Languages',  icon: ic(<><circle cx="12" cy="12" r="10"/><path d="M2 12h20M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></>) },
  { id: 'audio',      label: 'Audio',      icon: ic(<><rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10a7 7 0 0 0 14 0"/><line x1="12" y1="19" x2="12" y2="22"/></>) },
  { id: 'appearance', label: 'Appearance', icon: ic(<><circle cx="13.5" cy="6.5" r=".5"/><circle cx="17.5" cy="10.5" r=".5"/><circle cx="8.5" cy="7.5" r=".5"/><circle cx="6.5" cy="12.5" r=".5"/><path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 12 2z"/></>) },
  { id: 'about',      label: 'About',      icon: ic(<><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></>) },
];

function fmtDuration(ms: number): string {
  if (!ms || !isFinite(ms) || ms < 0) return '0s';
  const totalSec = Math.round(ms / 1000);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const sec = totalSec % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${sec}s`;
  return `${sec}s`;
}

const THEMES = [
  { value: 'dark',  label: 'Dark',  desc: 'Cool dark with blue accent',          win: '#15171f', bar: 'rgba(255,255,255,0.18)', accent: '#3b82f6', pill: '#0b0e13', pillText: '#fff' },
  { value: 'black', label: 'Black', desc: 'Pure OLED black, zero distraction',    win: '#000000', bar: 'rgba(255,255,255,0.85)', accent: 'rgba(255,255,255,0.3)', pill: '#000000', pillText: '#fff' },
  { value: 'light', label: 'Light', desc: 'Clean and bright, sharp contrast',     win: '#ffffff', bar: 'rgba(0,0,0,0.18)', accent: '#3b82f6', pill: '#1a1a1a', pillText: '#fff' },
];

function ThemePreview({ t }: { t: typeof THEMES[number] }) {
  return (
    <div className="w-full rounded-lg p-2.5 mb-2.5 border" style={{ background: t.win, borderColor: 'rgba(255,255,255,0.08)' }}>
      {/* window dots */}
      <div className="flex gap-1 mb-2">
        <span className="w-1.5 h-1.5 rounded-full" style={{ background: t.bar, opacity: 0.5 }} />
        <span className="w-1.5 h-1.5 rounded-full" style={{ background: t.bar, opacity: 0.5 }} />
        <span className="w-1.5 h-1.5 rounded-full" style={{ background: t.bar, opacity: 0.5 }} />
      </div>
      {/* content bars */}
      <div className="space-y-1.5 mb-2.5">
        <div className="h-1.5 rounded-full" style={{ width: '70%', background: t.bar }} />
        <div className="flex gap-1.5">
          <div className="h-1.5 rounded-full" style={{ width: '35%', background: t.accent }} />
          <div className="h-1.5 rounded-full" style={{ width: '25%', background: t.bar, opacity: 0.5 }} />
        </div>
      </div>
      {/* mini pill */}
      <div className="flex items-center gap-1 rounded-full px-1.5 py-1 w-fit mx-auto" style={{ background: t.pill, border: '1px solid rgba(255,255,255,0.1)' }}>
        <NeuroLogo size={9} />
        <span className="text-[7px] font-bold" style={{ color: t.pillText }}>BizVoice</span>
      </div>
    </div>
  );
}

const WIDGET_STYLES = [
  { value: 'logoText', label: 'Logo & Text', desc: 'Logo plus "BizVoice" label' },
  { value: 'logo',     label: 'Logo',        desc: 'Compact symbol — takes less space' },
  { value: 'mono',     label: 'Monochrome',  desc: 'Compact symbol, black & white' },
];

function NeuroLogo({ size = 20 }: { size?: number }) {
  return <img src="./logo.svg" width={size} height={size} draggable={false} className="neuro-logo" style={{ objectFit: 'contain' }} />;
}

export function Settings() {
  const [tab, setTab] = useState<Tab>('overview');
  const [apiKey, setApiKey] = useState('');
  const [hasKey, setHasKey] = useState(false);
  const [groqKey, setGroqKey] = useState('');
  const [hasGroqKey, setHasGroqKey] = useState(false);
  const [hotkey, setHotkey] = useState('CommandOrControl+Shift+Space');
  const [pttHotkey, setPttHotkey] = useState('CommandOrControl+Space');
  const [inputLang, setInputLang] = useState<Lang>('auto');
  const [gptModel, setGptModel] = useState('gpt-4o-mini');
  const [sttModel, setSttModel] = useState('gpt-4o-mini-transcribe');
  const [micDeviceId, setMicDeviceId] = useState('default');
  const [micFallbackId, setMicFallbackId] = useState('default');
  const [mics, setMics] = useState<{ deviceId: string; label: string }[]>([]);
  const [activeMicLabel, setActiveMicLabel] = useState('');
  const [vocabulary, setVocabulary] = useState('');
  const [silenceMs, setSilenceMs] = useState(1500);
  const [autoStop, setAutoStop] = useState(false);
  const [useLocalWhisper, setUseLocalWhisper] = useState(false);
  const [localModel, setLocalModel] = useState('base');
  const [whisperModels, setWhisperModels] = useState<{ name: string; size: string; downloaded: boolean }[]>([]);
  const [downloading, setDownloading] = useState<string | null>(null);
  const [downloadPct, setDownloadPct] = useState(0);
  // sttProvider value isn't read in this component — the unified picker drives
  // gptProvider, and sttProvider is set as a derived backend hint via setSttProvider.
  const [, setSttProvider] = useState<'openai' | 'groq'>('openai');
  const [gptProvider, setGptProvider] = useState<'openai' | 'groq' | 'openrouter' | 'custom'>('openai');
  const [openrouterKey, setOpenrouterKey] = useState('');
  const [hasOpenrouterKey, setHasOpenrouterKey] = useState(false);
  const [customKey, setCustomKey] = useState('');
  const [hasCustomKey, setHasCustomKey] = useState(false);
  const [customBaseUrl, setCustomBaseUrl] = useState('');
  const [customChatModel, setCustomChatModel] = useState('');
  const [customHeaders, setCustomHeaders] = useState('');
  const [useBetterBangla, setUseBetterBangla] = useState(false);
  const [skipGpt, setSkipGpt] = useState(false);
  const [muteWhileRecording, setMuteWhileRecording] = useState(false);
  const [dictionary, setDictionary] = useState<DictEntry[]>([]);
  const [theme, setTheme] = useState('dark');
  const [widgetStyle, setWidgetStyle] = useState('logoText');
  const [instructions, setInstructions] = useState('');
  const [history, setHistory] = useState<HistEntry[]>([]);
  const [historyPage, setHistoryPage] = useState(0);
  const [refined, setRefined] = useState<Record<number, string>>({});
  const [refining, setRefining] = useState<Record<number, boolean>>({});
  const HISTORY_PER_PAGE = 10;
  const [stats, setStats] = useState<{ recordings: number; words: number; durationMs: number }>({ recordings: 0, words: 0, durationMs: 0 });
  const [dictSearch, setDictSearch] = useState('');
  const [newTerm, setNewTerm] = useState({ from: '', to: '', category: '' });
  const [editingDict, setEditingDict] = useState<number | null>(null);
  const [toast, setToast] = useState<{ kind: 'ok' | 'err'; msg: string } | null>(null);
  const [auth, setAuth] = useState<{ loggedIn: boolean; active: boolean; email: string; offline?: boolean }>({ loggedIn: false, active: false, email: '' });
  const [upd, setUpd] = useState<{ current: string; latest: { version: string; notes?: string[]; releasedAt?: string } | null; updateAvailable: boolean; downloaded: boolean } | null>(null);
  const [checking, setChecking] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Lenis smooth scroll for the settings content
  useEffect(() => {
    const wrapper = scrollRef.current;
    if (!wrapper) return;
    const lenis = new Lenis({
      wrapper,
      content: wrapper.firstElementChild as HTMLElement || wrapper,
      duration: 0.8,
      easing: (t: number) => 1 - Math.pow(1 - t, 3),
      smoothWheel: true,
    });
    let raf = 0;
    const loop = (time: number) => { lenis.raf(time); raf = requestAnimationFrame(loop); };
    raf = requestAnimationFrame(loop);
    return () => { cancelAnimationFrame(raf); lenis.destroy(); };
  }, []);

  // Logged-in user shown in the sidebar footer
  useEffect(() => {
    window.api.authStatus().then(setAuth).catch(() => {});
    const off = window.api.onAuthChange((d) => setAuth((p) => ({ ...p, ...d })));
    return off;
  }, []);

  // App version + live update check (About tab).
  // First mount pulls cached state instantly so the version + "checking…"
  // both render right away; the button below triggers a fresh GitHub query.
  async function checkUpdates(force = true) {
    setChecking(true);
    try { setUpd(await window.api.updateInfo(force)); } catch { /* offline */ }
    setChecking(false);
  }
  useEffect(() => { void checkUpdates(false); }, []);
  useEffect(() => { if (tab === 'about') void checkUpdates(true); }, [tab]);

  const LEGACY_LANG_MAP: Record<string, string> = { en: 'English', bn: 'Bangla' };
  const normalizeLang = (v: string) => LEGACY_LANG_MAP[v] ?? v ?? 'auto';

  useEffect(() => {
    window.api.getSettings().then((s) => {
      setHotkey(s.hotkey);
      setPttHotkey(s.pttHotkey || 'CommandOrControl+Space');
      setInputLang(normalizeLang(s.inputLang));
      setGptModel(s.gptModel);
      setSttModel(s.sttModel || 'gpt-4o-mini-transcribe');
      setMicDeviceId(s.micDeviceId || 'default');
      setMicFallbackId(s.micFallbackId || 'default');
      setVocabulary(s.vocabulary || '');
      setSilenceMs(s.silenceMs ?? 1500);
      setAutoStop(s.autoStop ?? false);
      setUseLocalWhisper(s.useLocalWhisper ?? false);
      setLocalModel(s.localModel || 'base');
      setSttProvider(s.sttProvider || 'openai');
      setGptProvider(s.gptProvider || 'openai');
      setCustomBaseUrl(s.customBaseUrl || '');
      setCustomChatModel(s.customChatModel || '');
      setCustomHeaders(s.customHeaders || '');
      setHasOpenrouterKey(!!s.hasOpenrouterKey);
      if (s.hasOpenrouterKey) setOpenrouterKey('••••••••••••••••••••');
      setHasCustomKey(!!s.hasCustomKey);
      if (s.hasCustomKey) setCustomKey('••••••••••••••••••••');
      setUseBetterBangla(s.useBetterBangla ?? false);
      setSkipGpt(s.skipGpt ?? false);
      setMuteWhileRecording(s.muteWhileRecording ?? false);
      setDictionary(s.dictionary || []);
      setTheme(s.theme || 'dark');
      setWidgetStyle(s.widgetStyle || 'logoText');
      setInstructions(s.instructions || '');
      setHasGroqKey(!!s.hasGroqKey);
      if (s.hasGroqKey) setGroqKey('••••••••••••••••••••');
      setHasKey(s.hasKey);
      if (s.hasKey) setApiKey('••••••••••••••••••••');
      loadWhisperModels(s.localModel || 'base');
    });
    loadMics();
    const removeProgress = window.api.onWhisperDownloadProgress(({ model, pct }) => {
      if (model) setDownloadPct(pct);
    });
    return () => removeProgress();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (tab === 'history' || tab === 'overview') {
      window.api.getHistory().then(async (h) => {
        setHistory(h);
        const cached = await window.api.getRefinedCache();
        setRefined(cached);
      });
      setHistoryPage(0);
    }
    if (tab === 'overview') window.api.getStats().then(setStats);
  }, [tab]);

  async function refineEntry(entry: HistEntry) {
    if (refining[entry.ts] || refined[entry.ts]) return;
    setRefining((m) => ({ ...m, [entry.ts]: true }));
    try {
      const r = await window.api.refineText(entry.text, entry.ts);
      if (r) setRefined((m) => ({ ...m, [entry.ts]: r }));
    } catch (e: any) {
      flash('err', e?.message || 'Refine failed');
    } finally {
      setRefining((m) => { const c = { ...m }; delete c[entry.ts]; return c; });
    }
  }

  async function loadMics() {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      stream.getTracks().forEach((t) => t.stop());
      const devices = await navigator.mediaDevices.enumerateDevices();
      const list = devices
        .filter((d) => d.kind === 'audioinput' && d.deviceId !== 'communications')
        .map((d) => ({ deviceId: d.deviceId, label: d.label || `Microphone ${d.deviceId.slice(0, 6)}` }));
      setMics(list);
      // Determine active mic (the default the browser actually uses)
      const def = devices.find((d) => d.kind === 'audioinput' && d.deviceId === 'default');
      setActiveMicLabel(def?.label?.replace(/^Default - /, '') || list[0]?.label || 'System default');
    } catch {}
  }

  async function loadWhisperModels(currentModel?: string) {
    try {
      const models = await window.api.whisperListModels();
      setWhisperModels(models);
      const active = currentModel || localModel;
      const downloaded = models.filter((m: any) => m.downloaded);
      if (downloaded.length > 0 && !downloaded.some((m: any) => m.name === active)) {
        setLocalModel(downloaded[0].name);
        window.api.setSettings({ localModel: downloaded[0].name });
      }
    } catch {}
  }

  async function downloadWhisperModel(name: string) {
    setDownloading(name);
    setDownloadPct(0);
    const res = await window.api.whisperDownloadModel(name);
    setDownloading(null);
    if (res.ok) {
      setToast({ kind: 'ok', msg: `Model "${name}" downloaded` });
      setLocalModel(name);
      save({ localModel: name });
      loadWhisperModels();
    } else {
      setToast({ kind: 'err', msg: res.error || 'Download failed' });
    }
    setTimeout(() => setToast(null), 2000);
  }

  async function deleteWhisperModel(name: string) {
    await window.api.whisperDeleteModel(name);
    loadWhisperModels();
    setToast({ kind: 'ok', msg: `Model "${name}" deleted` });
    setTimeout(() => setToast(null), 2000);
  }

  async function save(patch: any, msg = 'Saved') {
    try {
      const res: any = await window.api.setSettings(patch);
      if (res?.ok === false) throw new Error(res.error || 'Save failed');
      setToast({ kind: 'ok', msg });
    } catch (err: any) {
      setToast({ kind: 'err', msg: err?.message ?? 'Save failed' });
    }
    setTimeout(() => setToast(null), 2000);
  }

  async function saveApiKey() {
    const t = apiKey.trim();
    if (!t || t.startsWith('•')) { flash('err', 'Please enter a key'); return; }
    await save({ openaiKey: t }, 'API key saved');
    setHasKey(true); setApiKey('••••••••••••••••••••');
  }
  async function saveGroqKey() {
    const t = groqKey.trim();
    if (!t || t.startsWith('•')) { flash('err', 'Please enter a key'); return; }
    await save({ groqKey: t } as any, 'Groq key saved');
    setHasGroqKey(true); setGroqKey('••••••••••••••••••••');
  }
  // Unified per-provider save. Collects every editable field for the chosen
  // provider into one patch and persists with a single toast — replaces the
  // old "save on each blur" pattern. Key fields are masked once stored, so
  // a value starting with • means "unchanged" and is skipped.
  async function saveOpenrouterConfig() {
    const patch: Record<string, unknown> = { gptModel };
    const k = openrouterKey.trim();
    if (k && !k.startsWith('•')) {
      patch.openrouterKey = k;
    } else if (!hasOpenrouterKey) {
      flash('err', 'Please enter an API key');
      return;
    }
    await save(patch, 'OpenRouter settings saved');
    if (patch.openrouterKey) { setHasOpenrouterKey(true); setOpenrouterKey('••••••••••••••••••••'); }
  }
  async function saveCustomConfig() {
    const patch: Record<string, unknown> = { customBaseUrl, customChatModel, customHeaders };
    const k = customKey.trim();
    if (k && !k.startsWith('•')) {
      patch.customKey = k;
    } else if (!hasCustomKey) {
      flash('err', 'Please enter an API key');
      return;
    }
    if (!customBaseUrl.trim()) { flash('err', 'Base URL is required'); return; }
    await save(patch, 'Custom settings saved');
    if (patch.customKey) { setHasCustomKey(true); setCustomKey('••••••••••••••••••••'); }
  }
  function flash(kind: 'ok' | 'err', msg: string) {
    setToast({ kind, msg });
    setTimeout(() => setToast(null), 2000);
  }

  function captureKey(e: React.KeyboardEvent, field: 'hotkey' | 'pttHotkey') {
    e.preventDefault();
    const parts: string[] = [];
    if (e.ctrlKey) parts.push('CommandOrControl');
    if (e.shiftKey) parts.push('Shift');
    if (e.altKey) parts.push('Alt');
    const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
    if (!['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) {
      parts.push(key);
      const combo = parts.join('+');
      (field === 'hotkey' ? setHotkey : setPttHotkey)(combo);
      save({ [field]: combo }, 'Hotkey updated');
    }
  }
  const prettyHotkey = (h: string) => h.replace('CommandOrControl', 'Ctrl').split('+').join(' + ');

  // Dictionary helpers
  function persistDict(updated: DictEntry[]) {
    setDictionary(updated);
    save({ dictionary: updated.filter(d => d.from.trim()) }, 'Dictionary saved');
  }
  function addTerm() {
    if (!newTerm.from.trim() || !newTerm.to.trim()) { flash('err', 'Enter both term and replacement'); return; }
    persistDict([{ from: newTerm.from.trim(), to: newTerm.to.trim(), category: newTerm.category.trim() || undefined, ts: Date.now() }, ...dictionary]);
    setNewTerm({ from: '', to: '', category: '' });
  }
  function saveEditedTerm(i: number, patch: Partial<DictEntry>) {
    persistDict(dictionary.map((d, idx) => idx === i ? { ...d, ...patch, ts: Date.now() } : d));
    setEditingDict(null);
  }
  function deleteTerm(i: number) {
    persistDict(dictionary.filter((_, idx) => idx !== i));
  }

  const filteredDict = dictionary.filter(d =>
    !dictSearch.trim() || (d.from + d.to + (d.category || '')).toLowerCase().includes(dictSearch.toLowerCase())
  );

  const onTitleBarMouseDown = useCallback(async (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('button')) return;
    const [wx, wy] = await window.api.getWinPos();
    const start = { x: e.screenX, y: e.screenY, wx, wy };
    const onMove = (ev: MouseEvent) => {
      window.api.setWinPos(start.wx + (ev.screenX - start.x), start.wy + (ev.screenY - start.y));
    };
    const onUp = () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'grabbing';
  }, []);

  return (
    <div className="h-screen flex flex-col text-white" style={{ background: 'linear-gradient(135deg, #0f0f15 0%, #15151d 100%)' }}>
      {/* Title bar — JS-based drag via IPC (same pattern as MicBar). */}
      <div
        className="flex items-center justify-between px-5 py-3 border-b border-white/5 select-none cursor-grab active:cursor-grabbing"
        onMouseDown={onTitleBarMouseDown}
      >
        <div className="flex items-center gap-2.5 pointer-events-none">
          <NeuroLogo size={18} />
          <div className="font-semibold text-sm tracking-wide">BizVoice</div>
          <div className="text-xs text-white/30">Settings</div>
        </div>
        <button
          className="w-7 h-7 flex items-center justify-center rounded-md hover:bg-white/10 text-white/50 hover:text-white transition-colors cursor-pointer"
          onClick={() => window.api.closeWindow()}
        >&#10005;</button>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* Sidebar */}
        <div className="w-48 border-r border-white/5 flex flex-col">
          <div className="flex-1 overflow-y-auto py-3 px-2">
            {NAV.map((n) => (
              <button key={n.id} onClick={() => setTab(n.id)}
                className={`w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-sm mb-0.5 transition-all ${
                  tab === n.id ? 'bg-blue-600/20 text-white border border-blue-500/30' : 'text-white/60 hover:text-white hover:bg-white/5 border border-transparent'
                }`}>
                <span className={tab === n.id ? 'text-blue-400' : 'text-white/40'}>{n.icon}</span>
                {n.label}
              </button>
            ))}
          </div>

          {/* Logged-in user */}
          <div className="border-t border-white/5 p-2.5">
            {auth.loggedIn ? (
              <div className="flex items-center gap-2.5 px-1">
                <div className="w-8 h-8 rounded-full bg-blue-600/30 text-blue-300 flex items-center justify-center text-xs font-bold uppercase shrink-0">
                  {auth.email ? auth.email[0] : '?'}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="text-xs font-medium text-white truncate" title={auth.email}>{auth.email || 'Signed in'}</div>
                  <div className="flex items-center gap-1 text-[10px] text-white/40">
                    <span className={`w-1.5 h-1.5 rounded-full ${auth.offline ? 'bg-amber-500' : auth.active ? 'bg-green-500' : 'bg-white/30'}`} />
                    {auth.offline ? 'Offline' : auth.active ? 'Active' : 'Inactive'}
                  </div>
                </div>
                <button onClick={() => window.api.logout()} title="Log out"
                  className="text-white/40 hover:text-red-400 p-1 rounded hover:bg-white/5 transition-colors shrink-0">
                  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" y1="12" x2="9" y2="12"/></svg>
                </button>
              </div>
            ) : (
              <button onClick={() => window.api.startBrowserLogin()}
                className="w-full flex items-center justify-center gap-2 px-3 py-2 rounded-md text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white transition-colors">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"/><polyline points="10 17 15 12 10 7"/><line x1="15" y1="12" x2="3" y2="12"/></svg>
                Sign in
              </button>
            )}
          </div>
        </div>

        {/* Content */}
        <div ref={scrollRef} className="flex-1 overflow-y-auto p-6">
          <div>
          {/* ── OVERVIEW ── */}
          {tab === 'overview' && (
            <Section title="Overview" description="Your BizVoice performance snapshot, recent activity, and productivity trends.">
              <div className="grid grid-cols-2 gap-3">
                <StatCard label="Words transcribed" value={stats.words.toLocaleString()} accent="blue" />
                <StatCard label="Total speaking time" value={fmtDuration(stats.durationMs)} accent="cyan" />
                <StatCard label="Total recordings" value={stats.recordings.toLocaleString()} accent="purple" />
                <StatCard label="Average pace" value={`${stats.durationMs && stats.durationMs > 0 ? Math.round(stats.words / (stats.durationMs / 60000)) : 0} wpm`} accent="emerald" hint="words per minute" />
              </div>

              <div className="flex items-center justify-between mt-6 mb-2">
                <div>
                  <div className="text-sm font-semibold text-white">Recent Activity</div>
                  <div className="text-[11px] text-white/40">Your latest transcriptions from this device.</div>
                </div>
                {history.length > 0 && <button onClick={() => setTab('history')} className="text-[11px] text-blue-400 hover:text-blue-300">View Full History</button>}
              </div>
              {history.length === 0 ? (
                <div className="text-xs text-white/40 bg-white/[0.02] rounded-lg px-4 py-8 border border-white/5 text-center">No activity yet. Press your hotkey and start dictating!</div>
              ) : (
                <div className="space-y-2">
                  {history.slice(0, 6).map((h, i) => (
                    <div key={i} className="bg-white/[0.02] border border-white/5 rounded-lg px-4 py-3">
                      <div className="text-sm text-white/85 leading-snug line-clamp-2">{h.text}</div>
                      <div className="flex items-center gap-2 mt-2 text-[10px] text-white/35">
                        <span>{new Date(h.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>
                        <span className="px-1.5 py-0.5 rounded bg-white/5">{h.words || 0} words</span>
                        {h.durationMs > 0 && <span className="px-1.5 py-0.5 rounded bg-white/5">{fmtDuration(h.durationMs)}</span>}
                        <button onClick={() => { setNewTerm({ from: h.text.split(/\s+/)[0] || '', to: '', category: '' }); setTab('dictionary'); }}
                          className="ml-auto text-white/40 hover:text-blue-400 flex items-center gap-1">
                          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></svg>Add to Dictionary
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </Section>
          )}

          {/* ── GENERAL ── */}
          {tab === 'general' && (
            <Section title="Transcription" description="Choose how BizVoice converts your speech to text.">
              {/* Mode cards */}
              <div className="grid grid-cols-2 gap-3">
                <ModeCard
                  active={useLocalWhisper}
                  onClick={() => { setUseLocalWhisper(true); save({ useLocalWhisper: true }, 'Local mode'); }}
                  icon={ic(<><rect x="4" y="4" width="16" height="16" rx="2"/><rect x="9" y="9" width="6" height="6"/><path d="M9 1v3M15 1v3M9 20v3M15 20v3M20 9h3M20 14h3M1 9h3M1 14h3"/></>)}
                  title="Local" tags={['Private', 'Offline']}
                  desc="On-device transcription — fully private, no internet needed"
                />
                <ModeCard
                  active={!useLocalWhisper}
                  onClick={() => { setUseLocalWhisper(false); save({ useLocalWhisper: false }, 'Cloud mode'); }}
                  icon={ic(<path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"/>)}
                  title="Cloud" tags={['100+ Languages', 'Fast', 'Accurate']}
                  desc="More accurate, faster, supports 100+ languages"
                />
              </div>

              {/* LOCAL settings */}
              {useLocalWhisper && (
                <Card>
                  <Label>Local Model</Label>
                  <HelpText>Bigger models are more accurate but slower. Download once, use offline forever.</HelpText>
                  <div className="space-y-1.5 mt-2">
                    {whisperModels.map(m => (
                      <div key={m.name}
                        onClick={() => { if (m.downloaded) { setLocalModel(m.name); save({ localModel: m.name }, `Model: ${m.name}`); } }}
                        className={`flex items-center justify-between py-2 px-3 rounded-lg border transition-all ${
                          localModel === m.name && m.downloaded ? 'border-blue-500/50 bg-blue-600/10' : 'border-white/5 bg-white/[0.02]'
                        } ${m.downloaded ? 'cursor-pointer hover:border-white/15' : ''}`}>
                        <div className="flex items-center gap-2">
                          <span className="text-xs font-semibold text-white capitalize">{m.name.replace(/\.en$/, ' (English)')}</span>
                          <span className="text-[10px] text-white/30">{m.size}</span>
                        </div>
                        <div className="flex items-center gap-2">
                          {downloading === m.name ? (
                            <div className="flex items-center gap-2">
                              <div className="w-16 h-1.5 bg-white/10 rounded-full overflow-hidden"><div className="h-full bg-blue-500 transition-all" style={{ width: `${downloadPct}%` }} /></div>
                              <span className="text-[10px] text-blue-400 w-7">{downloadPct}%</span>
                            </div>
                          ) : m.downloaded ? (
                            localModel === m.name
                              ? <span className="w-4 h-4 rounded-full bg-blue-500 flex items-center justify-center"><svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="3"><path d="M5 13l4 4L19 7"/></svg></span>
                              : <button onClick={(e) => { e.stopPropagation(); deleteWhisperModel(m.name); }} className="text-[10px] text-red-400/60 hover:text-red-400">Delete</button>
                          ) : (
                            <button onClick={(e) => { e.stopPropagation(); downloadWhisperModel(m.name); }} className="text-[10px] text-blue-400 hover:text-blue-300">Download</button>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                </Card>
              )}

              {/* CLOUD settings — one provider picker, used for STT + AI Formatting + Refine.
                  STT-only-capable providers (OpenRouter, Custom) auto-fall back to
                  whatever OpenAI/Groq key is set for the Whisper step. */}
              {!useLocalWhisper && (
                <>
                  <Card>
                    <Label>Provider</Label>
                    <HelpText>One provider powers transcription, AI formatting, and refine. OpenRouter & Custom are chat-only — STT falls back to your OpenAI or Groq key.</HelpText>
                    <div className="grid grid-cols-4 gap-2 mt-1">
                      {(['openai', 'groq', 'openrouter', 'custom'] as const).map((p) => {
                        const active = gptProvider === p;
                        const accent = p === 'groq'
                          ? 'bg-orange-600 border-orange-500 text-white'
                          : p === 'openrouter'
                          ? 'bg-purple-600 border-purple-500 text-white'
                          : p === 'custom'
                          ? 'bg-slate-600 border-slate-500 text-white'
                          : 'bg-blue-600 border-blue-500 text-white';
                        const label = p === 'openai' ? 'OpenAI' : p === 'groq' ? 'Groq' : p === 'openrouter' ? 'OpenRouter' : 'Custom';
                        return (
                          <button key={p}
                            onClick={() => {
                              setGptProvider(p);
                              // Whisper-capable providers sync sttProvider too. Chat-only
                              // ones (openrouter/custom) leave sttProvider as a Whisper fallback.
                              if (p === 'openai' || p === 'groq') {
                                setSttProvider(p);
                                save({ gptProvider: p, sttProvider: p }, `Provider: ${label}`);
                              } else {
                                save({ gptProvider: p }, `Provider: ${label}`);
                              }
                            }}
                            className={`px-2 py-2 rounded-md text-[12px] font-semibold border transition-all ${
                              active ? accent : 'bg-[#1f1f28] border-white/10 text-white/60 hover:text-white'
                            }`}>{label}</button>
                        );
                      })}
                    </div>
                  </Card>

                  {/* OpenAI: API key + STT model picker (Whisper variants) */}
                  {gptProvider === 'openai' && (
                    <>
                      <KeyCard label="OpenAI API Key" has={hasKey} val={apiKey} setVal={setApiKey} onSave={saveApiKey} prefix="sk-"
                        onChange={() => setApiKey('')} onRemove={() => { save({ openaiKey: '' } as any, 'Key removed'); setHasKey(false); setApiKey(''); }} />
                      <Card>
                        <Label>Transcription Model</Label>
                        <Select
                          value={sttModel}
                          onChange={(v) => { setSttModel(v); save({ sttModel: v }); }}
                          options={[
                            { value: 'gpt-4o-mini-transcribe', label: 'gpt-4o-mini-transcribe — fast' },
                            { value: 'gpt-4o-transcribe',      label: 'gpt-4o-transcribe — best accuracy' },
                            { value: 'whisper-1',              label: 'whisper-1 — legacy' },
                          ]}
                        />
                      </Card>
                    </>
                  )}

                  {/* Groq: API key only — one STT model (whisper-large-v3-turbo) */}
                  {gptProvider === 'groq' && (
                    <KeyCard label="Groq API Key" has={hasGroqKey} val={groqKey} setVal={setGroqKey} onSave={saveGroqKey} prefix="gsk_"
                      onChange={() => setGroqKey('')} onRemove={() => { save({ groqKey: '' } as any, 'Key removed'); setHasGroqKey(false); setGroqKey(''); }} />
                  )}

                  {/* OpenRouter: key + chat model, ONE Save for both. */}
                  {gptProvider === 'openrouter' && (
                    <Card>
                      <Label>OpenRouter Configuration</Label>
                      <HelpText>One Save persists key + model. STT silently falls back to your OpenAI or Groq key.</HelpText>
                      <div className="space-y-3 mt-2">
                        <div>
                          <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">API Key {hasOpenrouterKey && <span className="text-green-400/80 normal-case ml-1">✓ saved</span>}</div>
                          <input type="password" value={openrouterKey} onChange={(e) => setOpenrouterKey(e.target.value)}
                            onFocus={() => { if (openrouterKey.startsWith('•')) setOpenrouterKey(''); }}
                            placeholder="sk-or-..." className={inputCls} />
                        </div>
                        <div>
                          <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">Chat Model</div>
                          <input type="text" value={gptModel} onChange={(e) => setGptModel(e.target.value)}
                            placeholder="e.g. meta-llama/llama-3.1-8b-instruct"
                            className={inputCls} />
                          <HelpText>Find model IDs at openrouter.ai/models</HelpText>
                        </div>
                        <div className="flex items-center justify-between gap-2 pt-1">
                          {hasOpenrouterKey ? (
                            <button onClick={() => { save({ openrouterKey: '' } as any, 'Key removed'); setHasOpenrouterKey(false); setOpenrouterKey(''); }}
                              className="px-3 py-1.5 rounded-md text-[11px] text-red-400/80 hover:text-red-400 hover:bg-red-500/10 border border-red-500/20">Remove key</button>
                          ) : <span />}
                          <button onClick={saveOpenrouterConfig}
                            className="px-5 py-1.5 rounded-md text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white transition-colors">
                            Save Configuration
                          </button>
                        </div>
                      </div>
                      {!hasKey && !hasGroqKey && (
                        <div className="text-[11px] text-amber-400 bg-amber-500/5 rounded-lg px-3 py-2 border border-amber-500/20 mt-3">
                          OpenRouter doesn't transcribe audio. Add an OpenAI or Groq key for STT fallback, or switch to Local mode.
                        </div>
                      )}
                    </Card>
                  )}

                  {/* Custom: full config (key + base URL + model + headers), ONE Save. */}
                  {gptProvider === 'custom' && (
                    <Card>
                      <Label>Custom API Configuration</Label>
                      <HelpText>OpenAI-compatible endpoint. Fill all fields and click Save once.</HelpText>
                      <div className="space-y-3 mt-2">
                        <div>
                          <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">API Key {hasCustomKey && <span className="text-green-400/80 normal-case ml-1">✓ saved</span>}</div>
                          <input type="password" value={customKey} onChange={(e) => setCustomKey(e.target.value)}
                            onFocus={() => { if (customKey.startsWith('•')) setCustomKey(''); }}
                            placeholder="your-api-key" className={inputCls} />
                        </div>
                        <div>
                          <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">Base URL</div>
                          <input type="text" value={customBaseUrl} onChange={(e) => setCustomBaseUrl(e.target.value)}
                            placeholder="https://freellmapi.webdevarif.com/v1"
                            className={inputCls} />
                          <HelpText>OpenAI-compatible endpoint root. The /chat/completions path is appended automatically.</HelpText>
                        </div>
                        <div>
                          <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">Chat Model</div>
                          <input type="text" value={customChatModel} onChange={(e) => setCustomChatModel(e.target.value)}
                            placeholder="e.g. auto, gpt-4o-mini, llama-3.1-70b"
                            className={inputCls} />
                        </div>
                        <div>
                          <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">Extra Headers (JSON, optional)</div>
                          <textarea value={customHeaders} onChange={(e) => setCustomHeaders(e.target.value)}
                            placeholder='{"HTTP-Referer":"https://example.com"}'
                            rows={2}
                            className={inputCls + ' resize-none font-mono text-xs'} />
                          <HelpText>Optional. Provider-specific headers as JSON.</HelpText>
                        </div>
                        <div className="flex items-center justify-between gap-2 pt-1">
                          {hasCustomKey ? (
                            <button onClick={() => { save({ customKey: '' } as any, 'Key removed'); setHasCustomKey(false); setCustomKey(''); }}
                              className="px-3 py-1.5 rounded-md text-[11px] text-red-400/80 hover:text-red-400 hover:bg-red-500/10 border border-red-500/20">Remove key</button>
                          ) : <span />}
                          <button onClick={saveCustomConfig}
                            className="px-5 py-1.5 rounded-md text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white transition-colors">
                            Save Configuration
                          </button>
                        </div>
                      </div>
                      {!hasKey && !hasGroqKey && (
                        <div className="text-[11px] text-amber-400 bg-amber-500/5 rounded-lg px-3 py-2 border border-amber-500/20 mt-3">
                          Most custom proxies don't transcribe audio. Add an OpenAI or Groq key for STT fallback, or switch to Local mode.
                        </div>
                      )}
                    </Card>
                  )}
                </>
              )}

              {/* AI Formatting — single toggle. Uses whichever provider is set above
                  (or any cloud-key fallback when Local mode is on). */}
              <Card>
                <div className="flex items-center justify-between">
                  <div>
                    <Label>AI Formatting</Label>
                    <HelpText>
                      {skipGpt
                        ? 'Off — pastes raw transcription instantly (fastest)'
                        : useLocalWhisper
                        ? 'On — uses your cloud key (OpenAI/Groq/OpenRouter/Custom) for grammar cleanup'
                        : `On — cleans up grammar & formatting via ${gptProvider === 'openrouter' ? 'OpenRouter' : gptProvider === 'custom' ? 'your custom endpoint' : gptProvider === 'groq' ? 'Groq' : 'OpenAI'}`}
                    </HelpText>
                  </div>
                  <Toggle on={!skipGpt} onClick={() => { const v = !skipGpt; setSkipGpt(v); save({ skipGpt: v }, v ? 'Fast mode' : 'AI formatting on'); }} />
                </div>

                {/* Quick model swap for built-in providers (OpenAI/Groq).
                    OpenRouter/Custom set their model in the Provider config above. */}
                {!skipGpt && !useLocalWhisper && gptProvider === 'openai' && (
                  <div className="mt-3">
                    <Select
                      value={gptModel}
                      onChange={(v) => { setGptModel(v); save({ gptModel: v }); }}
                      options={[
                        { value: 'gpt-4o-mini', label: 'gpt-4o-mini — fast, cheap' },
                        { value: 'gpt-4o',      label: 'gpt-4o — best quality' },
                      ]}
                    />
                  </div>
                )}
                {!skipGpt && !useLocalWhisper && gptProvider === 'groq' && (
                  <div className="mt-3">
                    <Select
                      value={gptModel}
                      onChange={(v) => { setGptModel(v); save({ gptModel: v }); }}
                      options={[
                        { value: 'llama-3.1-8b-instant',    label: 'llama-3.1-8b-instant — fastest' },
                        { value: 'llama-3.3-70b-versatile', label: 'llama-3.3-70b-versatile — best quality' },
                        { value: 'mixtral-8x7b-32768',      label: 'mixtral-8x7b — long context' },
                      ]}
                    />
                  </div>
                )}
              </Card>
            </Section>
          )}

          {/* ── SHORTCUTS ── */}
          {tab === 'shortcuts' && (
            <Section title="Shortcuts" description="Global hotkeys work anywhere in Windows.">
              <Card>
                <Label>Toggle Record</Label>
                <HelpText>Press once to start, press again to stop.</HelpText>
                <input readOnly value={prettyHotkey(hotkey)} onKeyDown={(e) => captureKey(e, 'hotkey')} className={inputCls + ' text-center font-mono cursor-pointer'} />
              </Card>
              <Card>
                <Label>Push-to-Talk</Label>
                <HelpText>Hold to record, release to stop. Fastest workflow.</HelpText>
                <input readOnly value={prettyHotkey(pttHotkey)} onKeyDown={(e) => captureKey(e, 'pttHotkey')} className={inputCls + ' text-center font-mono cursor-pointer'} />
              </Card>
              <Card>
                <div className="flex items-center justify-between">
                  <div><Label>Auto-stop on silence</Label><HelpText>Stop recording automatically after silence.</HelpText></div>
                  <Toggle on={autoStop} onClick={() => { const v = !autoStop; setAutoStop(v); save({ autoStop: v }); }} />
                </div>
                {autoStop && (
                  <>
                    <div className="flex items-center justify-between mt-3 mb-1"><span className="text-[11px] text-white/50">Silence threshold</span><span className="text-xs text-blue-400 font-mono">{silenceMs}ms</span></div>
                    <input type="range" min={500} max={4000} step={100} value={silenceMs} onChange={(e) => setSilenceMs(Number(e.target.value))} onMouseUp={() => save({ silenceMs })} className="w-full accent-blue-500" />
                  </>
                )}
              </Card>
            </Section>
          )}

          {/* ── LANGUAGES ── */}
          {tab === 'languages' && (
            <Section title="Languages" description="Set the language you speak for the most accurate transcription.">
              <Card>
                <Label>Spoken Language</Label>
                <HelpText>Choosing a specific language improves accuracy over Auto-detect.</HelpText>
                <Select
                  value={inputLang}
                  onChange={(v) => { setInputLang(v); save({ inputLang: v }); }}
                  options={LANGS.map((l) => ({ value: l.value, label: l.label }))}
                  isSearchable
                />
              </Card>
              <Card>
                <Label>Custom Vocabulary</Label>
                <HelpText>Comma-separated terms (names, jargon) for better recognition.</HelpText>
                <textarea value={vocabulary} onChange={(e) => setVocabulary(e.target.value)} onBlur={() => save({ vocabulary }, 'Vocabulary saved')} rows={2} placeholder="React, Next.js, Supabase, Prisma" className={inputCls + ' font-mono text-[11px] resize-none'} />
              </Card>

              {/* Better Bangla — routes Bangla audio through a server-side
                  ASR model on BizGrowHub. No additional account or token
                  needed; works with the same login users already have. */}
              <Card>
                <div className="flex items-center justify-between">
                  <div>
                    <Label>Improved Bangla transcription</Label>
                    <HelpText>
                      Routes Bangla audio through a Bangla-tuned ASR model on the BizGrowHub backend. No extra token needed — works with your existing login. Far more accurate than default Whisper for Bangla.
                    </HelpText>
                  </div>
                  <Toggle on={useBetterBangla} onClick={() => { const v = !useBetterBangla; setUseBetterBangla(v); save({ useBetterBangla: v }, v ? 'Improved Bangla enabled' : 'Default Bangla'); }} />
                </div>
                {useBetterBangla && (
                  <div className="text-[11px] text-white/40 mt-2">
                    Bangla dictation will route to the BizGrowHub Bangla model. Other languages keep using your selected provider above. First request may take 10–30s (cold start); subsequent ones are fast.
                  </div>
                )}
              </Card>
            </Section>
          )}

          {/* ── AUDIO ── */}
          {tab === 'audio' && (
            <Section title="Audio" description="Choose the microphone BizVoice uses.">
              <Card>
                <div className="flex items-center justify-between mb-2">
                  <Label>Input Device</Label>
                  <button onClick={loadMics} className="text-[11px] text-blue-400 hover:text-blue-300">&#8635; Refresh</button>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">Primary</div>
                    <Select
                      value={micDeviceId}
                      onChange={(v) => { setMicDeviceId(v); save({ micDeviceId: v }, 'Microphone updated'); }}
                      options={[
                        { value: 'default', label: 'System default' },
                        ...mics.map((m) => ({ value: m.deviceId, label: m.label })),
                      ]}
                      isSearchable
                    />
                  </div>
                  <div>
                    <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">Fallback</div>
                    <Select
                      value={micFallbackId}
                      onChange={(v) => { setMicFallbackId(v); save({ micFallbackId: v }, 'Fallback updated'); }}
                      options={[
                        { value: 'default', label: 'System default' },
                        ...mics.map((m) => ({ value: m.deviceId, label: m.label })),
                      ]}
                      isSearchable
                    />
                  </div>
                </div>
                {activeMicLabel && (
                  <div className="flex items-center gap-2 mt-3 text-[11px] text-white/50">
                    <span className="w-2 h-2 rounded-full bg-green-500 shadow-[0_0_6px_rgba(34,197,94,0.7)]" />
                    <span className="uppercase tracking-wider text-[9px] text-white/35">Active now</span>
                    <span className="text-white/70 truncate">{activeMicLabel}</span>
                  </div>
                )}
              </Card>
              {mics.length === 0 && (
                <div className="text-xs text-amber-400 bg-amber-500/5 rounded-lg px-4 py-3 border border-amber-500/20">No microphones detected. Click Refresh and allow mic access.</div>
              )}
              <Card>
                <div className="flex items-center justify-between">
                  <div><Label>Mute system audio while recording</Label><HelpText>Silences speakers so the mic doesn't pick up videos/music. Restored when recording stops.</HelpText></div>
                  <Toggle on={muteWhileRecording} onClick={() => { const v = !muteWhileRecording; setMuteWhileRecording(v); save({ muteWhileRecording: v }); }} />
                </div>
              </Card>
            </Section>
          )}

          {/* ── DICTIONARY ── */}
          {tab === 'dictionary' && (
            <Section title="Dictionary" description="Define custom replacements so BizVoice produces clean, consistent language.">
              {/* Add Term */}
              <Card>
                <Label>Add Term</Label>
                <HelpText>Add an original phrase and choose how it should be output.</HelpText>
                <div className="grid grid-cols-2 gap-3 mt-1">
                  <div>
                    <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">Original Term</div>
                    <input value={newTerm.from} onChange={(e) => setNewTerm({ ...newTerm, from: e.target.value })} placeholder="e.g. webhook" className={inputCls} />
                  </div>
                  <div>
                    <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">Replacement</div>
                    <input value={newTerm.to} onChange={(e) => setNewTerm({ ...newTerm, to: e.target.value })} placeholder="e.g. WebHook" className={inputCls} />
                  </div>
                </div>
                <div className="mt-3">
                  <div className="text-[10px] uppercase tracking-wider text-white/40 mb-1">Category (optional)</div>
                  <input value={newTerm.category} onChange={(e) => setNewTerm({ ...newTerm, category: e.target.value })} placeholder="e.g. product, team, acronym" className={inputCls} />
                </div>
                <button onClick={addTerm} className="mt-3 px-4 py-1.5 bg-blue-600 hover:bg-blue-500 rounded-md text-xs font-medium flex items-center gap-1.5">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>Add Term
                </button>
              </Card>

              {/* Saved Terms */}
              <Card>
                <div className="flex items-center justify-between mb-3">
                  <div>
                    <Label>Saved Terms ({dictionary.length})</Label>
                    <HelpText>Search and manage existing dictionary entries.</HelpText>
                  </div>
                  <input value={dictSearch} onChange={(e) => setDictSearch(e.target.value)} placeholder="Search terms" className={inputCls + ' w-40 py-1.5'} />
                </div>
                {filteredDict.length === 0 ? (
                  <div className="text-xs text-white/40 py-6 text-center">{dictionary.length === 0 ? 'No terms yet. Add one above.' : 'No matches.'}</div>
                ) : (
                  <div className="space-y-2">
                    {filteredDict.map((d) => {
                      const i = dictionary.indexOf(d);
                      const isEdit = editingDict === i;
                      return (
                        <div key={i} className="bg-white/[0.02] border border-white/5 rounded-lg px-3 py-2.5">
                          {isEdit ? (
                            <div className="flex items-center gap-2">
                              <input defaultValue={d.from} onChange={(e) => (d.from = e.target.value)} className={inputCls + ' flex-1 py-1.5'} />
                              <span className="text-white/30">&rarr;</span>
                              <input defaultValue={d.to} onChange={(e) => (d.to = e.target.value)} className={inputCls + ' flex-1 py-1.5'} />
                              <button onClick={() => saveEditedTerm(i, { from: d.from, to: d.to })} className="text-[11px] text-blue-400 hover:text-blue-300 px-2">Save</button>
                            </div>
                          ) : (
                            <div className="flex items-center justify-between">
                              <div className="min-w-0">
                                <div className="text-sm text-white/90 truncate"><span className="text-white/60">{d.from}</span> <span className="text-white/30">&rarr;</span> <span className="font-medium">{d.to}</span></div>
                                <div className="flex items-center gap-2 mt-1">
                                  <span className="text-[9px] uppercase tracking-wider bg-white/5 text-white/50 px-1.5 py-0.5 rounded">{d.category || 'Uncategorized'}</span>
                                  {d.ts && <span className="text-[10px] text-white/30">Updated {new Date(d.ts).toLocaleDateString()}</span>}
                                </div>
                              </div>
                              <div className="flex items-center gap-1 shrink-0">
                                <button onClick={() => setEditingDict(i)} className="text-[11px] text-white/50 hover:text-white px-2 py-1 rounded hover:bg-white/5 flex items-center gap-1"><svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 20h9M16.5 3.5a2.121 2.121 0 1 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>Edit</button>
                                <button onClick={() => deleteTerm(i)} className="text-[11px] text-red-400/60 hover:text-red-400 px-2 py-1 rounded hover:bg-white/5 flex items-center gap-1"><svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>Delete</button>
                              </div>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </Card>
            </Section>
          )}

          {/* ── APPEARANCE ── */}
          {tab === 'appearance' && (
            <Section title="Appearance" description="Customize how the floating widget looks.">
              <Card>
                <Label>Widget Style</Label>
                <HelpText>How the pill displays when idle.</HelpText>
                <div className="grid grid-cols-3 gap-2 mt-2">
                  {WIDGET_STYLES.map(w => (
                    <button key={w.value} onClick={() => { setWidgetStyle(w.value); save({ widgetStyle: w.value }, w.label); }}
                      className={`p-3 rounded-lg border text-left transition-all ${widgetStyle === w.value ? 'border-blue-500/50 bg-blue-600/10' : 'border-white/5 bg-white/[0.02] hover:border-white/15'}`}>
                      <div className="text-xs font-semibold text-white mb-1">{w.label}</div>
                      <div className="text-[10px] text-white/40 leading-tight">{w.desc}</div>
                    </button>
                  ))}
                </div>
              </Card>
              <Card>
                <Label>Theme</Label>
                <HelpText>Color style of the floating widget.</HelpText>
                <div className="grid grid-cols-3 gap-2.5 mt-2">
                  {THEMES.map(t => (
                    <button key={t.value} onClick={() => { setTheme(t.value); save({ theme: t.value }, t.label); }}
                      className={`relative p-2.5 rounded-xl border text-left transition-all ${theme === t.value ? 'border-blue-500/60 bg-blue-600/10' : 'border-white/5 bg-white/[0.02] hover:border-white/15'}`}>
                      {theme === t.value && <span className="absolute top-2 right-2 w-4 h-4 rounded-full bg-blue-500 flex items-center justify-center z-10"><svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="3.5"><path d="M5 13l4 4L19 7"/></svg></span>}
                      <ThemePreview t={t} />
                      <div className="text-xs font-bold text-white px-0.5">{t.label}</div>
                      <div className="text-[10px] text-white/40 leading-tight px-0.5 mt-0.5">{t.desc}</div>
                    </button>
                  ))}
                </div>
              </Card>
            </Section>
          )}

          {/* ── HISTORY ── */}
          {tab === 'history' && (() => {
            const totalPages = Math.max(1, Math.ceil(history.length / HISTORY_PER_PAGE));
            const page = Math.min(historyPage, totalPages - 1);
            const start = page * HISTORY_PER_PAGE;
            const pageItems = history.slice(start, start + HISTORY_PER_PAGE);
            return (
              <Section title="History" description="Your recent transcriptions. Click the original to copy, or refine to see a grammar-corrected version.">
                {history.length > 0 && (
                  <button onClick={async () => { await window.api.clearHistory(); setHistory([]); setRefined({}); flash('ok', 'History cleared'); }}
                    className="text-[11px] text-red-400/70 hover:text-red-400 mb-1">Clear all history</button>
                )}
                {history.length === 0 ? (
                  <div className="text-xs text-white/40 bg-white/[0.02] rounded-lg px-4 py-8 border border-white/5 text-center">No transcriptions yet. Start dictating!</div>
                ) : (
                  <>
                    <div className="space-y-2">
                      {pageItems.map((h) => {
                        const ref = refined[h.ts];
                        const isRefining = !!refining[h.ts];
                        return (
                          <div key={h.ts}
                            className="bg-white/[0.02] border border-white/5 rounded-lg px-4 py-3 hover:border-white/15 transition-all group">
                            {/* Original (top) */}
                            <div className="flex items-start gap-2">
                              <span className="text-[9px] font-semibold text-white/35 tracking-wider mt-1 shrink-0">YOU</span>
                              <div onClick={() => { navigator.clipboard.writeText(h.text); flash('ok', 'Original copied'); }}
                                className="text-sm text-white/85 leading-snug cursor-pointer flex-1">{h.text}</div>
                            </div>

                            {/* Refined (bottom) — visible if cached, button if not */}
                            {ref ? (
                              <div className="flex items-start gap-2 mt-2 pt-2 border-t border-white/5">
                                <span className="text-[9px] font-semibold text-blue-400/80 tracking-wider mt-1 shrink-0">REFINED</span>
                                <div onClick={() => { navigator.clipboard.writeText(ref); flash('ok', 'Refined copied'); }}
                                  className="text-sm text-blue-100/85 leading-snug cursor-pointer flex-1">{ref}</div>
                              </div>
                            ) : (
                              <button onClick={() => refineEntry(h)} disabled={isRefining}
                                className="mt-2 text-[11px] text-blue-400/70 hover:text-blue-400 disabled:opacity-50 disabled:cursor-wait">
                                {isRefining ? 'Refining…' : '✨ Refine sentence'}
                              </button>
                            )}

                            <div className="flex items-center gap-3 mt-2 text-[10px] text-white/35">
                              <span>{new Date(h.ts).toLocaleString([], { hour: '2-digit', minute: '2-digit', month: 'short', day: 'numeric' })}</span>
                              <span className="px-1.5 py-0.5 rounded bg-white/5">{h.words || h.text.trim().split(/\s+/).filter(Boolean).length} words</span>
                              {h.durationMs > 0 && <span className="px-1.5 py-0.5 rounded bg-white/5">{fmtDuration(h.durationMs)}</span>}
                            </div>
                          </div>
                        );
                      })}
                    </div>

                    {/* Pagination controls */}
                    {totalPages > 1 && (
                      <div className="flex items-center justify-between mt-4 text-xs">
                        <div className="text-white/40">
                          Showing {start + 1}–{Math.min(start + HISTORY_PER_PAGE, history.length)} of {history.length}
                        </div>
                        <div className="flex items-center gap-1">
                          <button onClick={() => setHistoryPage(Math.max(0, page - 1))} disabled={page === 0}
                            className="px-2.5 py-1 rounded border border-white/10 text-white/70 hover:bg-white/5 disabled:opacity-40 disabled:cursor-not-allowed">Prev</button>
                          {Array.from({ length: totalPages }, (_, i) => i)
                            .filter((i) => i === 0 || i === totalPages - 1 || Math.abs(i - page) <= 1)
                            .map((i, idx, arr) => (
                              <span key={i} className="flex items-center">
                                {idx > 0 && arr[idx - 1] !== i - 1 && <span className="px-1 text-white/30">…</span>}
                                <button onClick={() => setHistoryPage(i)}
                                  className={`min-w-[28px] px-2 py-1 rounded border text-center ${i === page ? 'bg-blue-600/30 border-blue-500/40 text-white' : 'border-white/10 text-white/70 hover:bg-white/5'}`}>{i + 1}</button>
                              </span>
                            ))}
                          <button onClick={() => setHistoryPage(Math.min(totalPages - 1, page + 1))} disabled={page >= totalPages - 1}
                            className="px-2.5 py-1 rounded border border-white/10 text-white/70 hover:bg-white/5 disabled:opacity-40 disabled:cursor-not-allowed">Next</button>
                        </div>
                      </div>
                    )}
                  </>
                )}
              </Section>
            );
          })()}

          {/* ── INSTRUCTIONS ── */}
          {tab === 'instructions' && (
            <Section title="Custom Instructions" description="Provide style preferences or instructions for your transcriptions.">
              <Card>
                <HelpText>Tell BizVoice how to format your transcriptions. For example: "Use all lowercase in Slack," or "Break text into paragraphs."</HelpText>
                <textarea value={instructions} maxLength={2000} onChange={(e) => setInstructions(e.target.value)}
                  rows={8} placeholder="Always use proper capitalization and punctuation. Format variable and function names in camelCase. Keep technical terms lowercase unless they are acronyms (e.g. API, SDK, UI)..." className={inputCls + ' resize-none leading-relaxed'} />
                <div className="flex items-center justify-between mt-3">
                  <span className="text-[11px] text-white/35">{instructions.length} / 2,000 characters</span>
                  <button onClick={() => save({ instructions }, 'Instructions saved')} className="px-4 py-1.5 bg-blue-600 hover:bg-blue-500 rounded-md text-xs font-medium flex items-center gap-1.5">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/><polyline points="17 21 17 13 7 13 7 21"/><polyline points="7 3 7 8 15 8"/></svg>Save Instructions
                  </button>
                </div>
              </Card>
              {skipGpt && (
                <div className="text-xs text-amber-400 bg-amber-500/5 rounded-lg px-4 py-3 border border-amber-500/20">
                  AI Formatting is currently off (Fast Mode). Instructions only apply when AI Formatting is enabled in Transcription.
                </div>
              )}
            </Section>
          )}

          {/* ── ABOUT ── */}
          {tab === 'about' && (
            <Section title="" description="">
              <Card>
                <div className="flex flex-col items-center py-4">
                  <NeuroLogo size={48} />
                  <div className="text-lg font-bold text-white mt-3">BizVoice</div>
                  <div className="text-xs text-white/40 mt-1">Version {upd?.current ?? '…'}</div>
                  <div className="text-sm text-white/60 mt-4 leading-relaxed text-center max-w-sm">AI voice typing for Windows. Press your hotkey anywhere to dictate into any app.</div>

                  {/* Updates */}
                  <div className="mt-5 flex flex-col items-center gap-2 w-full max-w-sm">
                    {checking ? (
                      <div className="text-[11px] text-white/50 flex items-center gap-2">
                        <span className="inline-block w-3 h-3 border-2 border-white/20 border-t-blue-400 rounded-full animate-spin" />
                        Checking GitHub for updates…
                      </div>
                    ) : upd?.updateAvailable && upd.latest ? (
                      <>
                        <div className="text-xs font-medium text-blue-400 text-center">
                          New version available: v{upd.latest.version}
                        </div>
                        <div className="text-[10px] text-white/40">
                          You&apos;re on v{upd.current}
                        </div>
                        {upd.downloaded ? (
                          <button
                            onClick={() => window.api.updateInstall()}
                            className="mt-1 px-4 py-2 rounded-md text-xs font-semibold bg-green-600 hover:bg-green-500 text-white transition-colors"
                          >
                            Restart to install
                          </button>
                        ) : (
                          <button
                            onClick={() => window.api.updateDownload()}
                            className="mt-1 px-4 py-2 rounded-md text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white transition-colors"
                          >
                            Download &amp; update
                          </button>
                        )}
                        <button
                          onClick={() => checkUpdates(true)}
                          className="text-[10px] text-white/40 hover:text-white/70 transition-colors"
                        >
                          Re-check
                        </button>
                      </>
                    ) : (
                      <>
                        {upd && <div className="text-[11px] text-green-400/80">You&apos;re on the latest version.</div>}
                        <button
                          onClick={() => checkUpdates(true)}
                          disabled={checking}
                          className="px-4 py-2 rounded-md text-xs font-medium bg-white/5 hover:bg-white/10 border border-white/10 text-white/70 transition-colors disabled:opacity-50"
                        >
                          Check for updates
                        </button>
                      </>
                    )}
                  </div>
                </div>
              </Card>
            </Section>
          )}
          </div>
        </div>
      </div>

      {toast && (
        <div className={`absolute bottom-4 left-1/2 -translate-x-1/2 px-4 py-2 rounded-lg text-xs font-medium shadow-xl border ${
          toast.kind === 'ok' ? 'bg-green-600/20 text-green-300 border-green-500/30' : 'bg-red-600/20 text-red-300 border-red-500/30'
        }`}>{toast.kind === 'ok' ? '✓' : '✕'} {toast.msg}</div>
      )}
    </div>
  );
}

// ---------- UI helpers ----------
const inputCls = 'w-full bg-[#1a1a22] border border-white/10 rounded-md px-3 py-2 text-sm outline-none focus:border-blue-500 focus:bg-[#1f1f28] transition-colors';

// ---------- Themed dropdown (replaces native <select>) ----------
type SelOpt = { value: string; label: string };

const selectStyles: StylesConfig<SelOpt, false> = {
  control: (base, state) => ({
    ...base,
    background: state.isFocused ? '#1f1f28' : '#1a1a22',
    borderColor: state.isFocused ? '#3b82f6' : 'rgba(255,255,255,0.1)',
    boxShadow: 'none',
    minHeight: '38px',
    borderRadius: '6px',
    fontSize: '14px',
    cursor: 'pointer',
    transition: 'all 150ms',
    '&:hover': { borderColor: state.isFocused ? '#3b82f6' : 'rgba(255,255,255,0.2)' },
  }),
  valueContainer: (base) => ({ ...base, padding: '2px 12px' }),
  singleValue:    (base) => ({ ...base, color: 'rgba(255,255,255,0.9)' }),
  placeholder:    (base) => ({ ...base, color: 'rgba(255,255,255,0.4)' }),
  input:          (base) => ({ ...base, color: 'rgba(255,255,255,0.9)' }),
  indicatorSeparator: () => ({ display: 'none' }),
  dropdownIndicator: (base, state) => ({
    ...base,
    color: state.isFocused ? '#3b82f6' : 'rgba(255,255,255,0.5)',
    padding: '6px 8px',
    '&:hover': { color: 'white' },
  }),
  menu: (base) => ({
    ...base,
    background: '#1a1a22',
    border: '1px solid rgba(255,255,255,0.1)',
    borderRadius: '6px',
    boxShadow: '0 8px 24px rgba(0,0,0,0.5)',
    overflow: 'hidden',
    marginTop: '4px',
  }),
  menuPortal: (base) => ({ ...base, zIndex: 9999 }),
  menuList:   (base) => ({ ...base, padding: '4px' }),
  option: (base, state) => ({
    ...base,
    background: state.isSelected
      ? 'rgba(59,130,246,0.25)'
      : state.isFocused
      ? 'rgba(255,255,255,0.05)'
      : 'transparent',
    color: state.isSelected ? 'white' : 'rgba(255,255,255,0.85)',
    fontSize: '13px',
    padding: '8px 10px',
    borderRadius: '4px',
    cursor: 'pointer',
    '&:active': { background: 'rgba(59,130,246,0.35)' },
  }),
  noOptionsMessage: (base) => ({ ...base, color: 'rgba(255,255,255,0.4)', fontSize: '13px' }),
};

function Select({ value, onChange, options, placeholder, isSearchable = false }: {
  value: string;
  onChange: (v: string) => void;
  options: SelOpt[];
  placeholder?: string;
  isSearchable?: boolean;
}) {
  const selected = options.find((o) => o.value === value) ?? null;
  return (
    <ReactSelect<SelOpt>
      value={selected}
      onChange={(o) => o && onChange(o.value)}
      options={options}
      placeholder={placeholder}
      isSearchable={isSearchable}
      menuPortalTarget={typeof document !== 'undefined' ? document.body : undefined}
      styles={selectStyles}
    />
  );
}

function Section({ title, description, children }: { title: string; description: string; children: React.ReactNode }) {
  return (
    <div className="max-w-2xl">
      {title && <h2 className="text-lg font-semibold text-white mb-1">{title}</h2>}
      {description && <p className="text-xs text-white/50 mb-5">{description}</p>}
      <div className="space-y-3">{children}</div>
    </div>
  );
}
function Card({ children }: { children: React.ReactNode }) {
  return <div className="bg-white/[0.02] border border-white/5 rounded-lg px-4 py-3.5">{children}</div>;
}
function Label({ children, colorClass = 'text-white/85' }: { children: React.ReactNode; colorClass?: string }) {
  return <div className={`text-xs font-semibold ${colorClass} mb-1`}>{children}</div>;
}
function HelpText({ children, success }: { children: React.ReactNode; success?: boolean }) {
  return <div className={`text-[11px] mb-2 ${success ? 'text-green-400' : 'text-white/40'}`}>{children}</div>;
}
function StatCard({ label, value, hint, accent }: { label: string; value: string; hint?: string; accent?: 'blue' | 'cyan' | 'purple' | 'emerald' }) {
  const dot = accent === 'cyan' ? 'text-cyan-400' : accent === 'purple' ? 'text-purple-400' : accent === 'emerald' ? 'text-emerald-400' : 'text-blue-400';
  return (
    <div className="bg-white/[0.02] border border-white/5 rounded-xl px-4 py-5">
      <div className="flex items-center justify-between">
        <div className="text-[10px] uppercase tracking-wider text-white/40">{label}</div>
        {accent && <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className={dot}><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>}
      </div>
      <div className="text-2xl font-bold text-white mt-2">{value}</div>
      {hint && <div className="text-[10px] text-white/30 mt-0.5">{hint}</div>}
    </div>
  );
}
function Toggle({ on, onClick }: { on: boolean; onClick: () => void }) {
  return (
    <button onClick={onClick} className={`relative w-11 h-6 rounded-full transition-colors shrink-0 ${on ? 'bg-blue-600' : 'bg-white/10'}`}>
      <span className={`absolute top-1 w-4 h-4 bg-white rounded-full shadow transition-all ${on ? 'left-6' : 'left-1'}`} />
    </button>
  );
}
function ModeCard({ active, onClick, icon, title, tags, desc }: { active: boolean; onClick: () => void; icon: JSX.Element; title: string; tags: string[]; desc: string }) {
  return (
    <button onClick={onClick} className={`relative p-4 rounded-xl border text-left transition-all ${active ? 'border-blue-500/50 bg-blue-600/10' : 'border-white/5 bg-white/[0.02] hover:border-white/15'}`}>
      {active && <span className="absolute top-3 right-3 w-5 h-5 rounded-full bg-blue-500 flex items-center justify-center"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="3"><path d="M5 13l4 4L19 7"/></svg></span>}
      <div className={`w-10 h-10 rounded-lg flex items-center justify-center mb-3 ${active ? 'bg-blue-600/20 text-blue-400' : 'bg-white/5 text-white/50'}`}>{icon}</div>
      <div className="text-sm font-bold text-white mb-1">{title}</div>
      <div className="text-[11px] text-white/45 leading-snug mb-2">{desc}</div>
      <div className="flex flex-wrap gap-1">
        {tags.map(t => <span key={t} className="text-[9px] px-1.5 py-0.5 rounded bg-white/5 text-white/50">{t}</span>)}
      </div>
    </button>
  );
}
function KeyCard({ label, has, val, setVal, onSave, prefix, onChange, onRemove }: { label: string; has: boolean; val: string; setVal: (v: string) => void; onSave: () => void; prefix: string; onChange: () => void; onRemove: () => void }) {
  return (
    <Card>
      <Label>{label}</Label>
      <HelpText success={has}>{has ? '✓ Key saved' : `Paste your key (${prefix}...)`}</HelpText>
      {has && !val.startsWith(prefix) ? (
        <div className="flex items-center gap-2">
          <div className={inputCls + ' font-mono text-white/40 flex-1 flex items-center'}>••••••••••••••••••••</div>
          <button className="px-3 py-2 bg-white/5 hover:bg-white/10 border border-white/10 rounded-md text-xs text-white/60" onClick={onChange}>Change</button>
          <button className="px-3 py-2 bg-red-500/10 hover:bg-red-500/20 border border-red-500/20 rounded-md text-xs text-red-400" onClick={onRemove}>Remove</button>
        </div>
      ) : (
        <>
          <input type="text" value={val} onChange={(e) => setVal(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && onSave()} placeholder={`${prefix}...`} className={inputCls + ' font-mono'} />
          <button className="mt-3 px-4 py-1.5 bg-blue-600 hover:bg-blue-500 rounded-md text-xs font-medium" onClick={onSave}>Save Key</button>
        </>
      )}
    </Card>
  );
}
