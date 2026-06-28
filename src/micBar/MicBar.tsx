import { useEffect, useRef, useState, useCallback } from 'react';
import { HugeiconsIcon } from '@hugeicons/react';
import { Loading03Icon, CheckmarkCircle02Icon, Alert02Icon } from '@hugeicons/core-free-icons';
import {
  MediaRecorder as ExtMediaRecorder,
  register,
} from 'extendable-media-recorder';
import { connect } from 'extendable-media-recorder-wav-encoder';
import toWav from 'audiobuffer-to-wav';
import type { MicVAD as MicVADType } from '@ricky0123/vad-web';

// ── types ────────────────────────────────────────────────────────────────────

type State = 'idle' | 'recording' | 'processing' | 'done' | 'error';

const BAR_COUNT      = 5;
// VAD assets (silero ONNX model + audio worklet) are FETCHED (binary fetch /
// audioWorklet.addModule), so they load fine from /public → '/vad/'.
const VAD_ASSET_PATH = '/vad/';
// ORT is different: it loads its wasm glue via a dynamic import(`${wasmPaths}
// ort-wasm-simd-threaded.mjs`), and Vite DEV forbids importing files out of
// /public ("can only be referenced via HTML tags") — while a relative path
// resolves against the dep at /node_modules/.vite/deps/ (404). So in dev, point
// ORT at its real node_modules dir, which Vite serves as a proper module. The
// production build has everything copied into /vad/ by copyVadAssets.
const ORT_BASE = import.meta.env.DEV ? '/node_modules/onnxruntime-web/dist/' : '/vad/';

// ── brand logo ────────────────────────────────────────────────────────────────
// Inlined SVG so the icon can never fail to load due to path/asar/cache issues.
// Source of truth still lives at public/logo.svg for tray + installer icons.

function LogoGlyph({ size = 18, mono = false }: { size?: number; color?: string; mono?: boolean }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 1500 1500"
      xmlns="http://www.w3.org/2000/svg"
      style={{
        display: 'block',
        filter: mono ? 'grayscale(1) brightness(1.6)' : 'drop-shadow(0 1px 2px rgba(0,0,0,0.35))',
      }}
    >
      <path fill="#3a84ef" d="M 1500 750 C 1500 1164.332031 1164.332031 1500 750 1500 C 749.320312 1500 748.304688 1500 747.625 1500 C 543.171875 1499.320312 358.046875 1416.929688 223.101562 1283.679688 L 819.507812 1283.679688 C 890.710938 1283.679688 951.742188 1270.796875 1002.597656 1245.367188 C 1053.457031 1219.597656 1092.449219 1185.011719 1119.234375 1140.597656 C 1146.359375 1096.519531 1159.921875 1047.015625 1159.921875 991.75 C 1159.921875 919.53125 1136.867188 858.839844 1090.753906 809.675781 C 1067.019531 784.246094 1037.863281 763.222656 1003.617188 746.269531 C 1027.691406 732.707031 1048.710938 716.773438 1065.664062 698.464844 C 1105.671875 655.742188 1125.339844 603.527344 1125.339844 541.816406 C 1125.339844 493.332031 1113.472656 449.253906 1089.738281 409.585938 C 1066.003906 369.574219 1031.417969 338.042969 986.664062 314.308594 C 961.914062 301.425781 934.109375 291.929688 903.59375 285.828125 L 1400.65625 377.035156 C 1463.71875 486.890625 1500 614.035156 1500 750 Z" />
      <path fill="#f7b731" d="M 920.546875 909.019531 C 932.074219 929.023438 937.5 951.742188 937.5 977.507812 C 937.5 1014.464844 925.292969 1046 900.542969 1071.429688 C 875.792969 1097.195312 841.546875 1109.742188 798.148438 1109.742188 L 91.886719 1109.742188 C 33.226562 1002.9375 0 880.199219 0 750 C 0 729.65625 0.679688 709.3125 2.375 689.308594 C 6.78125 635.058594 16.953125 582.503906 32.210938 531.984375 C 125.792969 224.117188 411.617188 0 750 0 C 985.648438 0 1195.863281 108.5 1333.183594 278.367188 L 831.035156 278.367188 C 828.324219 278.367188 825.269531 278.367188 822.21875 278.367188 L 413.3125 278.367188 L 413.3125 996.835938 L 611.664062 996.835938 L 611.664062 844.9375 L 798.144531 844.9375 C 827.644531 844.9375 852.394531 850.699219 872.738281 861.890625 C 893.421875 873.417969 909.019531 889.015625 920.546875 909.019531 Z M 869.347656 479.769531 C 846.632812 461.121094 816.792969 451.964844 779.835938 451.964844 L 611.664062 451.964844 L 611.664062 675.40625 L 779.835938 675.40625 C 816.792969 675.40625 846.632812 666.253906 869.347656 647.605469 C 892.066406 628.957031 903.59375 600.8125 903.59375 562.839844 C 903.59375 526.21875 892.066406 498.417969 869.347656 479.769531 Z" />
      <path fill="#e09717" d="M 903.59375 563.179688 C 903.59375 601.152344 892.066406 629.292969 869.347656 647.941406 C 846.632812 666.589844 816.792969 675.746094 779.835938 675.746094 L 611.664062 675.746094 L 611.664062 452.304688 L 779.835938 452.304688 C 816.792969 452.304688 846.632812 461.460938 869.347656 480.109375 C 892.066406 498.417969 903.59375 526.21875 903.59375 563.179688 Z M 920.546875 909.019531 C 909.019531 889.015625 893.082031 873.417969 872.738281 861.890625 C 852.394531 850.363281 827.304688 844.9375 798.144531 844.9375 L 611.664062 844.9375 L 611.664062 996.835938 L 413.3125 996.835938 L 413.3125 278.367188 L 32.210938 531.984375 C 16.953125 582.164062 6.78125 635.058594 2.375 689.308594 C 0.679688 709.3125 0 729.65625 0 750 C 0 880.539062 33.226562 1002.9375 91.886719 1109.742188 L 798.148438 1109.742188 C 841.886719 1109.742188 876.128906 1096.859375 900.542969 1071.429688 C 925.292969 1045.660156 937.5 1014.464844 937.5 977.507812 C 937.839844 951.742188 932.074219 929.023438 920.546875 909.019531 Z" />
    </svg>
  );
}

// ── one-time WAV encoder registration ───────────────────────────────────────

let wavEncoderPromise: Promise<void> | null = null;
function ensureWavEncoder(): Promise<void> {
  if (!wavEncoderPromise) {
    wavEncoderPromise = connect().then(register).catch((err) => {
      wavEncoderPromise = null;
      throw err;
    });
  }
  return wavEncoderPromise;
}

// ── audio helpers ────────────────────────────────────────────────────────────

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onloadend = () => resolve((reader.result as string).split(',')[1] ?? '');
    reader.onerror   = reject;
    reader.readAsDataURL(blob);
  });
}

async function float32ToWavBase64(audio: Float32Array, sampleRate: number): Promise<string> {
  const offCtx = new OfflineAudioContext(1, audio.length, sampleRate);
  const buf    = offCtx.createBuffer(1, audio.length, sampleRate);
  buf.copyToChannel(new Float32Array(audio), 0);
  return blobToBase64(new Blob([toWav(buf)], { type: 'audio/wav' }));
}

// ── component ────────────────────────────────────────────────────────────────

export function MicBar() {
  const [state,     setState]     = useState<State>('idle');
  const [error,     setError]     = useState('');
  const [levels,    setLevels]    = useState<number[]>(() => Array(BAR_COUNT).fill(0.15));
  const [hasKey,    setHasKey]    = useState(true);
  const [animPhase, setAnimPhase] = useState<'hidden' | 'entering' | 'visible'>('hidden');
  const [theme,       setTheme]       = useState<'dark' | 'black' | 'light'>('dark');
  const [widgetStyle, setWidgetStyle] = useState<'logoText' | 'logo' | 'mono'>('logoText');

  const mediaRef  = useRef<InstanceType<typeof ExtMediaRecorder> | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const streamRef = useRef<MediaStream | null>(null);

  const vadRafRef    = useRef<number | null>(null);
  const audioCtxRef  = useRef<AudioContext | null>(null);
  const analyserRef  = useRef<AnalyserNode | null>(null);

  const recStartRef    = useRef<number>(0);
  const micVADRef      = useRef<MicVADType | null>(null);
  const speechFiredRef = useRef(false);

  const stateRef  = useRef<State>('idle');
  const hasKeyRef = useRef(true);
  stateRef.current  = state;
  hasKeyRef.current = hasKey;

  async function refreshSettings() {
    const s = await window.api.getSettings();
    // Ready if: local mode (no key needed), or the active cloud provider has a key
    const ready = !!s.useLocalWhisper || (s.sttProvider === 'groq' ? !!s.hasGroqKey : !!s.hasKey);
    setHasKey(ready);
    hasKeyRef.current = ready;
    if (s.theme) setTheme(s.theme);
    if (s.widgetStyle) setWidgetStyle(s.widgetStyle);
  }

  useEffect(() => {
    const t = setTimeout(() => setAnimPhase('entering'), 150);
    return () => clearTimeout(t);
  }, []);

  useEffect(() => {
    const removeHotkey   = window.api.onHotkey(() => toggle());
    const removePttStart = window.api.onPttStart(() => {
      if (stateRef.current === 'idle') startRecording();
    });
    const removePttStop  = window.api.onPttStop(() => {
      if (stateRef.current === 'recording') stopRecording();
    });
    const removeAppearance = window.api.onAppearanceChange(({ theme, widgetStyle }) => {
      setTheme(theme as any);
      setWidgetStyle(widgetStyle as any);
    });
    refreshSettings();
    window.addEventListener('focus', refreshSettings);
    return () => {
      removeHotkey();
      removePttStart();
      removePttStop();
      removeAppearance();
      window.removeEventListener('focus', refreshSettings);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ── frequency-bar visualisation ──────────────────────────────────────────

  const startAnalyser = useCallback((stream: MediaStream) => {
    const ctx      = new AudioContext();
    audioCtxRef.current = ctx;
    const source   = ctx.createMediaStreamSource(stream);
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 256;
    source.connect(analyser);
    analyserRef.current = analyser;

    const freqData   = new Uint8Array(analyser.frequencyBinCount);
    const smoothed   = Array(BAR_COUNT).fill(0.08);
    const binsPerBar = Math.floor(freqData.length / BAR_COUNT);
    let lastUpdate   = 0;

    const tick = (now: number) => {
      if (stateRef.current !== 'recording') {
        vadRafRef.current = requestAnimationFrame(tick);
        return;
      }
      if (now - lastUpdate < 50) { vadRafRef.current = requestAnimationFrame(tick); return; }
      lastUpdate = now;
      analyser.getByteFrequencyData(freqData);
      for (let i = 0; i < BAR_COUNT; i++) {
        let sum = 0;
        for (let j = 0; j < binsPerBar; j++) sum += freqData[i * binsPerBar + j];
        const target = Math.min(1, (sum / binsPerBar / 255) * 2.5);
        smoothed[i] += (target - smoothed[i]) * 0.4;
      }
      setLevels(smoothed.map(v => Math.max(0.08, v)));
      vadRafRef.current = requestAnimationFrame(tick);
    };
    vadRafRef.current = requestAnimationFrame(tick);
  }, []);

  function stopAnalyser() {
    if (vadRafRef.current) { cancelAnimationFrame(vadRafRef.current); vadRafRef.current = null; }
    analyserRef.current = null;
    audioCtxRef.current?.close().catch(() => {});
    audioCtxRef.current = null;
    setLevels(Array(BAR_COUNT).fill(0.15));
  }

  function stopStream() {
    streamRef.current?.getTracks().forEach(t => t.stop());
    streamRef.current = null;
  }

  // Resolve mic stream: try primary, fall back to fallback, then system default
  async function getMicStream(s: Awaited<ReturnType<typeof window.api.getSettings>>): Promise<MediaStream> {
    const primary  = s.micDeviceId && s.micDeviceId !== 'default' ? s.micDeviceId : undefined;
    const fallback = s.micFallbackId && s.micFallbackId !== 'default' ? s.micFallbackId : undefined;
    const tryGet = (id?: string) =>
      navigator.mediaDevices.getUserMedia({ audio: id ? { deviceId: { exact: id } } : true });
    try {
      return await tryGet(primary);
    } catch {
      try { return await tryGet(fallback); }
      catch { return await tryGet(undefined); }
    }
  }

  // ── send base64 WAV to main process ──────────────────────────────────────

  async function submitB64(b64: string) {
    window.api.muteSystem(false);
    const durationMs = recStartRef.current ? Date.now() - recStartRef.current : 0;
    // Ignore recordings shorter than 1s — go straight back to idle, no processing
    if (durationMs < 1000) {
      setState('idle');
      return;
    }
    setState('processing');
    try {
      const text = await window.api.transcribe(b64, durationMs);
      if (text) {
        setState('done');
        await window.api.paste(text);
        await new Promise(r => setTimeout(r, 1000));
      }
      setState('idle');
    } catch (err: any) {
      const msg = err?.message || (typeof err === 'string' ? err : '') || 'Transcription failed';
      setError(msg);
      setState('error');
      setTimeout(() => setState('idle'), 3000);
    }
  }

  // ── MediaRecorder path ───────────────────────────────────────────────────

  async function startMediaRecording(s: Awaited<ReturnType<typeof window.api.getSettings>>) {
    await ensureWavEncoder();
    const silenceMs = s.silenceMs ?? 1500;
    const autoStop  = s.autoStop  ?? false;

    if (s.muteWhileRecording) window.api.muteSystem(true);
    const stream = await getMicStream(s);
    streamRef.current = stream;

    const rec = new ExtMediaRecorder(stream, { mimeType: 'audio/wav' });
    chunksRef.current = [];
    rec.ondataavailable = (e: BlobEvent) => { if (e.data.size) chunksRef.current.push(e.data); };
    rec.onstop = async () => {
      stopAnalyser();
      stopStream();
      const durationMs = recStartRef.current ? Date.now() - recStartRef.current : 0;
      const blob = new Blob(chunksRef.current, { type: 'audio/wav' });
      // Too short or no audio — silently return to idle, no processing
      if (durationMs < 1000 || !blob.size) {
        window.api.muteSystem(false);
        setState('idle');
        return;
      }
      await submitB64(await blobToBase64(blob));
    };

    rec.start(250);
    mediaRef.current = rec as any;
    recStartRef.current = Date.now();
    startAnalyser(stream);
    setState('recording');

    if (autoStop) {
      const THRESHOLD = 0.012;
      let lastVoiceAt = Date.now(), hasSpoken = false;
      const td = new Uint8Array(256);
      const check = () => {
        if (stateRef.current !== 'recording') return;
        const an = analyserRef.current;
        if (!an) return;
        an.getByteTimeDomainData(td);
        let sum = 0;
        for (let i = 0; i < td.length; i++) { const v = (td[i] - 128) / 128; sum += v * v; }
        const r = Math.sqrt(sum / td.length), now = Date.now();
        if (r > THRESHOLD) { lastVoiceAt = now; hasSpoken = true; }
        if (hasSpoken && now - lastVoiceAt > silenceMs) { stopMediaRecording(); return; }
        requestAnimationFrame(check);
      };
      check();
    }
  }

  function stopMediaRecording() {
    (mediaRef.current as any)?.stop();
    mediaRef.current = null;
  }

  // ── MicVAD path ─────────────────────────────────────────────────────────

  async function startVADRecording(s: Awaited<ReturnType<typeof window.api.getSettings>>) {
    const silenceMs = s.silenceMs ?? 1500;

    if (s.muteWhileRecording) window.api.muteSystem(true);
    const stream = await getMicStream(s);
    streamRef.current = stream;
    startAnalyser(stream);
    speechFiredRef.current = false;

    const { MicVAD } = await import('@ricky0123/vad-web');
    const vad = await MicVAD.new({
      ortConfig(ort: any) {
        ort.env.wasm.numThreads = 1;
        ort.env.wasm.wasmPaths  = ORT_BASE;
      },
      baseAssetPath:    VAD_ASSET_PATH,
      onnxWASMBasePath: ORT_BASE,
      getStream:    async () => stream,
      pauseStream:  async () => {},
      resumeStream: async () => stream,
      submitUserSpeechOnPause: true,
      redemptionMs:            silenceMs,
      onSpeechEnd: async (audio: Float32Array) => {
        speechFiredRef.current = true;
        const v = micVADRef.current;
        micVADRef.current = null;
        v?.destroy().catch(() => {});
        stopAnalyser();
        stopStream();
        await submitB64(await float32ToWavBase64(audio, 16000));
      },
      onVADMisfire: () => {},
    });
    micVADRef.current = vad;
    await vad.start();
    recStartRef.current = Date.now();
    setState('recording');
  }

  async function stopVADRecording() {
    const vad = micVADRef.current;
    if (!vad) return;
    await vad.pause();
    if (!speechFiredRef.current) {
      micVADRef.current = null;
      vad.destroy().catch(() => {});
      stopAnalyser();
      stopStream();
      window.api.muteSystem(false);
      setState('idle');
    }
  }

  // ── unified API ──────────────────────────────────────────────────────────

  async function startRecording() {
    try {
      setError('');
      const s = await window.api.getSettings();
      if (s.autoStop) await startVADRecording(s);
      else            await startMediaRecording(s);
    } catch (err: any) {
      setError(err?.message ?? 'Mic error');
      setState('error');
      setTimeout(() => setState('idle'), 2500);
    }
  }

  function stopRecording() {
    if (micVADRef.current) stopVADRecording();
    else                   stopMediaRecording();
  }

  function toggle() {
    if (!hasKeyRef.current) {
      setError('No API key — open Settings');
      setState('error');
      setTimeout(() => setState('idle'), 3000);
      return;
    }
    const s = stateRef.current;
    if      (s === 'recording') stopRecording();
    else if (s === 'idle')      startRecording();
  }

  // ── drag ──────────────────────────────────────────────────────────────────

  const dragRef = useRef<{ startX: number; startY: number; winX: number; winY: number } | null>(null);

  const onDragStart = useCallback(async (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('.nodrag')) return;
    const [wx, wy] = await window.api.getWinPos();
    dragRef.current = { startX: e.screenX, startY: e.screenY, winX: wx, winY: wy };
    const onMove = (ev: MouseEvent) => {
      if (!dragRef.current) return;
      const dx = ev.screenX - dragRef.current.startX;
      const dy = ev.screenY - dragRef.current.startY;
      window.api.setWinPos(dragRef.current.winX + dx, dragRef.current.winY + dy);
    };
    const onUp = () => {
      dragRef.current = null;
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }, []);

  // ── render ────────────────────────────────────────────────────────────────

  const noKey = !hasKey && state === 'idle';
  const isRec = state === 'recording';

  const stateLabel =
    isRec                  ? 'Listening'    :
    state === 'processing' ? 'Processing'   :
    state === 'done'       ? 'Pasted'       :
    state === 'error'      ? (error || 'Error') :
    noKey                  ? 'No API Key'   :
    'Transcript';

  const glowClass = isRec ? 'glow-rec'
    : state === 'processing' ? 'glow-proc'
    : state === 'done' ? 'glow-done'
    : state === 'error' ? 'glow-err'
    : '';

  const themeClass = theme === 'light' ? 'theme-light' : theme === 'black' ? 'theme-black' : 'theme-dark';
  const showText = widgetStyle === 'logoText';
  const mono = widgetStyle === 'mono';
  const lightText = theme === 'light';

  return (
    <div className="w-full h-full flex flex-col items-center justify-end p-1" onMouseDown={onDragStart}>
      <div
        className={`glow-wrap ${themeClass} ${glowClass} cursor-grab active:cursor-grabbing ${animPhase === 'entering' ? 'pill-enter' : ''}`}
        onAnimationEnd={() => setAnimPhase('visible')}
        style={{
          ...(animPhase === 'hidden' ? { transform: 'scale(0)', opacity: 0 } : {}),
        } as React.CSSProperties}
        onContextMenu={(e) => { e.preventDefault(); window.api.micBarContextMenu(); }}
      >
        <div className={`glow-inner flex items-center ${showText ? 'gap-2 px-3' : 'px-2'} py-1.5`}>
          {/* Logo glyph — recording starts via shortcut only */}
          <span
            className="relative flex items-center justify-center"
            title={noKey ? 'Set API key — right-click → Settings' : 'Press your shortcut to record'}
          >
            <span key={state} className="icon-swap">
              {state === 'processing' ? (
                <HugeiconsIcon icon={Loading03Icon} size={18} className="spin" strokeWidth={2.5} style={{ color: '#fbbf24' }} />
              ) : state === 'done' ? (
                <HugeiconsIcon icon={CheckmarkCircle02Icon} size={18} strokeWidth={2.5} style={{ color: '#4ade80' }} />
              ) : state === 'error' ? (
                <HugeiconsIcon icon={Alert02Icon} size={18} strokeWidth={2} style={{ color: '#f87171' }} />
              ) : (
                <LogoGlyph size={18} mono={mono} />
              )}
            </span>
          </span>

          {/* Right side: label or waveform — only in logoText mode (or while recording) */}
          {isRec ? (
            <div className={`flex items-center gap-[2px] h-4 ${showText ? 'px-0.5' : 'pl-1.5'}`}>
              {levels.map((lv, i) => {
                const wave = Math.sin(Date.now() / 200 + i * 1.2) * 0.12;
                const h = Math.max(20, (lv + wave) * 100);
                return (
                  <span key={i} className="rounded-full"
                    style={{
                      width: 3,
                      height: `${h}%`,
                      background: 'linear-gradient(to top, #f87171, #fca5a5)',
                      transition: 'height 40ms ease-out',
                      boxShadow: lv > 0.3 ? `0 0 5px rgba(248,113,113,${lv * 0.7})` : 'none',
                    }} />
                );
              })}
            </div>
          ) : showText ? (
            <div key={stateLabel} className={`label-swap text-[13px] font-semibold tracking-wide select-none truncate ${
              state === 'done' ? 'text-green-500' :
              state === 'error' ? 'text-red-500' :
              state === 'processing' ? 'text-amber-500' :
              noKey ? (lightText ? 'text-zinc-400' : 'text-zinc-500') :
              (lightText ? 'text-zinc-800' : 'text-white')
            }`}>
              {state === 'idle' && !noKey ? 'BizVoice' : stateLabel}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
