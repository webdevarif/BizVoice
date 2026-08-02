import { useEffect, useRef, useState, useCallback } from 'react';
import { HugeiconsIcon } from '@hugeicons/react';
import { CheckmarkCircle02Icon, Loading03Icon } from '@hugeicons/core-free-icons';
import {
  MediaRecorder as ExtMediaRecorder,
  register,
} from 'extendable-media-recorder';
import { connect } from 'extendable-media-recorder-wav-encoder';

const ic = (p: JSX.Element, size = 18) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth={2}
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    {p}
  </svg>
);
const ICON_MIC = (size = 18) =>
  ic(
    <>
      <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z" />
      <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
      <line x1="12" y1="19" x2="12" y2="22" />
    </>,
    size,
  );
const ICON_MIC_OFF = (size = 18) =>
  ic(
    <>
      <line x1="1" y1="1" x2="23" y2="23" />
      <path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V4a3 3 0 0 0-5.94-.6" />
      <path d="M17 16.95A7 7 0 0 1 5 12v-2m14 0v2a7 7 0 0 1-.11 1.23" />
      <line x1="12" y1="19" x2="12" y2="22" />
    </>,
    size,
  );
const ICON_STOP = (size = 18) =>
  ic(<rect x="6" y="6" width="12" height="12" rx="2" />, size);
const ICON_PAUSE = (size = 18) =>
  ic(
    <>
      <circle cx="12" cy="12" r="10" />
      <line x1="10" y1="15" x2="10" y2="9" />
      <line x1="14" y1="15" x2="14" y2="9" />
    </>,
    size,
  );
const ICON_PLAY = (size = 18) =>
  ic(
    <>
      <circle cx="12" cy="12" r="10" />
      <polygon points="10 8 16 12 10 16 10 8" />
    </>,
    size,
  );
const ICON_COPY = (size = 18) =>
  ic(
    <>
      <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </>,
    size,
  );
const ICON_TRASH = (size = 18) =>
  ic(
    <>
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6l-2 14a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2L5 6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    </>,
    size,
  );
const ICON_SETTINGS = (size = 18) =>
  ic(
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </>,
    size,
  );

type SessionState = 'idle' | 'recording' | 'paused' | 'processing' | 'error';

interface TranscriptSegment {
  id: string;
  text: string;
  isPartial: boolean;
  timestamp: number;
}

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

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onloadend = () => resolve((reader.result as string).split(',')[1] ?? '');
    reader.onerror = reject;
    reader.readAsDataURL(blob);
  });
}

const uid = () =>
  Date.now().toString(36) + Math.random().toString(36).slice(2, 8);

function LogoGlyph({ size = 20 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 1500 1500"
      xmlns="http://www.w3.org/2000/svg"
      style={{ display: 'block', filter: 'drop-shadow(0 1px 2px rgba(0,0,0,0.35))' }}
    >
      <path
        fill="#3a84ef"
        d="M 1500 750 C 1500 1164.332031 1164.332031 1500 750 1500 C 749.320312 1500 748.304688 1500 747.625 1500 C 543.171875 1499.320312 358.046875 1416.929688 223.101562 1283.679688 L 819.507812 1283.679688 C 890.710938 1283.679688 951.742188 1270.796875 1002.597656 1245.367188 C 1053.457031 1219.597656 1092.449219 1185.011719 1119.234375 1140.597656 C 1146.359375 1096.519531 1159.921875 1047.015625 1159.921875 991.75 C 1159.921875 919.53125 1136.867188 858.839844 1090.753906 809.675781 C 1067.019531 784.246094 1037.863281 763.222656 1003.617188 746.269531 C 1027.691406 732.707031 1048.710938 716.773438 1065.664062 698.464844 C 1105.671875 655.742188 1125.339844 603.527344 1125.339844 541.816406 C 1125.339844 493.332031 1113.472656 449.253906 1089.738281 409.585938 C 1066.003906 369.574219 1031.417969 338.042969 986.664062 314.308594 C 961.914062 301.425781 934.109375 291.929688 903.59375 285.828125 L 1400.65625 377.035156 C 1463.71875 486.890625 1500 614.035156 1500 750 Z"
      />
      <path
        fill="#f7b731"
        d="M 920.546875 909.019531 C 932.074219 929.023438 937.5 951.742188 937.5 977.507812 C 937.5 1014.464844 925.292969 1046 900.542969 1071.429688 C 875.792969 1097.195312 841.546875 1109.742188 798.148438 1109.742188 L 91.886719 1109.742188 C 33.226562 1002.9375 0 880.199219 0 750 C 0 729.65625 0.679688 709.3125 2.375 689.308594 C 6.78125 635.058594 16.953125 582.503906 32.210938 531.984375 C 125.792969 224.117188 411.617188 0 750 0 C 985.648438 0 1195.863281 108.5 1333.183594 278.367188 L 831.035156 278.367188 C 828.324219 278.367188 825.269531 278.367188 822.21875 278.367188 L 413.3125 278.367188 L 413.3125 996.835938 L 611.664062 996.835938 L 611.664062 844.9375 L 798.144531 844.9375 C 827.644531 844.9375 852.394531 850.699219 872.738281 861.890625 C 893.421875 873.417969 909.019531 889.015625 920.546875 909.019531 Z M 869.347656 479.769531 C 846.632812 461.121094 816.792969 451.964844 779.835938 451.964844 L 611.664062 451.964844 L 611.664062 675.40625 L 779.835938 675.40625 C 816.792969 675.40625 846.632812 666.253906 869.347656 647.605469 C 892.066406 628.957031 903.59375 600.8125 903.59375 562.839844 C 903.59375 526.21875 892.066406 498.417969 869.347656 479.769531 Z"
      />
    </svg>
  );
}

export function VoiceEngine() {
  const [state, setState] = useState<SessionState>('idle');
  const [error, setError] = useState('');
  const [segments, setSegments] = useState<TranscriptSegment[]>([]);
  const [sessionId, setSessionId] = useState<string>('');
  const [elapsed, setElapsed] = useState(0);
  const [theme, setTheme] = useState<'dark' | 'light'>('dark');
  const [lang, setLang] = useState('auto');
  const [copied, setCopied] = useState(false);

  const mediaRef = useRef<InstanceType<typeof ExtMediaRecorder> | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const recStartRef = useRef<number>(0);
  const pauseOffsetRef = useRef<number>(0);
  const timerRef = useRef<number | null>(null);
  const lastSegmentIdRef = useRef<string>('');
  const audioCtxRef = useRef<AudioContext | null>(null);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rafRef = useRef<number | null>(null);
  const segmentsEndRef = useRef<HTMLDivElement | null>(null);

  const stateRef = useRef<SessionState>('idle');
  stateRef.current = state;

  useEffect(() => {
    segmentsEndRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }, [segments]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const s = await window.api.getSettings();
        if (!cancelled) {
          if (s.theme) setTheme(s.theme as any);
          if (s.inputLang) setLang(s.inputLang);
        }
      } catch {}
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const startTimer = useCallback(() => {
    const start = Date.now() - pauseOffsetRef.current;
    timerRef.current = window.setInterval(() => {
      setElapsed(Math.floor((Date.now() - start) / 1000));
    }, 250);
  }, []);

  const stopTimer = useCallback(() => {
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
    pauseOffsetRef.current = 0;
  }, []);

  const pauseTimer = useCallback(() => {
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const drawWaveform = useCallback(() => {
    const canvas = canvasRef.current;
    const analyser = analyserRef.current;
    if (!canvas || !analyser) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const cssW = canvas.clientWidth || 800;
    const cssH = canvas.clientHeight || 60;
    if (canvas.width !== Math.round(cssW * dpr) || canvas.height !== Math.round(cssH * dpr)) {
      canvas.width = Math.round(cssW * dpr);
      canvas.height = Math.round(cssH * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const W = cssW;
    const H = cssH;
    ctx.clearRect(0, 0, W, H);
    const freqData = new Uint8Array(analyser.frequencyBinCount);
    analyser.getByteFrequencyData(freqData);
    const bars = 64;
    const step = Math.floor(freqData.length / bars);
    const bw = W / bars;
    for (let i = 0; i < bars; i++) {
      let sum = 0;
      for (let j = 0; j < step; j++) sum += freqData[i * step + j];
      const avg = sum / step;
      const norm = Math.min(1, avg / 255);
      const h = Math.max(2, norm * H * 0.9);
      const mid = H / 2;
      const x = i * bw + bw * 0.15;
      const w = bw * 0.7;
      const g = ctx.createLinearGradient(0, mid - h / 2, 0, mid + h / 2);
      if (stateRef.current === 'recording') {
        g.addColorStop(0, '#60a5fa');
        g.addColorStop(0.5, '#3b82f6');
        g.addColorStop(1, '#1d4ed8');
      } else {
        g.addColorStop(0, '#94a3b8');
        g.addColorStop(1, '#64748b');
      }
      ctx.fillStyle = g;
      const r = Math.min(w / 2, 4);
      ctx.beginPath();
      ctx.roundRect(x, mid - h / 2, w, h, r);
      ctx.fill();
    }
    if (stateRef.current === 'recording') {
      rafRef.current = requestAnimationFrame(drawWaveform);
    }
  }, []);

  useEffect(() => {
    if (state === 'recording') {
      rafRef.current = requestAnimationFrame(drawWaveform);
    } else if (rafRef.current) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [state, drawWaveform]);

  const getMicStream = async (s: Awaited<ReturnType<typeof window.api.getSettings>>): Promise<MediaStream> => {
    const primary = s.micDeviceId && s.micDeviceId !== 'default' ? s.micDeviceId : undefined;
    const fallback = s.micFallbackId && s.micFallbackId !== 'default' ? s.micFallbackId : undefined;
    const dsp: MediaTrackConstraints = {
      channelCount: 1,
      sampleRate: 16000,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    };
    const tryGet = (id?: string) =>
      navigator.mediaDevices.getUserMedia({
        audio: id ? { deviceId: { exact: id }, ...dsp } : { ...dsp },
      });
    try {
      return await tryGet(primary);
    } catch {
      try {
        return await tryGet(fallback);
      } catch {
        return await tryGet(undefined);
      }
    }
  };

  const sendAudioChunk = async (blob: Blob, sid: string, isFinal = false) => {
    try {
      const b64 = await blobToBase64(blob);
      if (window.api.streamAudioChunk) {
        await window.api.streamAudioChunk(b64, sid, {
          isFinal,
          sampleRate: 16000,
          channels: 1,
          mimeType: blob.type,
        });
      }
    } catch (err) {
      console.warn('[VoiceEngine] audio_chunk send failed:', err);
    }
  };

  const startSession = async () => {
    try {
      setError('');
      const s = await window.api.getSettings();
      await ensureWavEncoder();

      const sid = uid();
      setSessionId(sid);
      setSegments([]);
      lastSegmentIdRef.current = '';

      if (s.muteWhileRecording) window.api.muteSystem(true);
      const stream = await getMicStream(s);
      streamRef.current = stream;

      const ctx = new AudioContext();
      audioCtxRef.current = ctx;
      const src = ctx.createMediaStreamSource(stream);
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 512;
      src.connect(analyser);
      analyserRef.current = analyser;

      if (window.api.startStreamSession) {
        try {
          await window.api.startStreamSession(sid, {
            language: s.inputLang || 'auto',
            provider: s.sttProvider,
            model: s.sttModel,
          });
        } catch (err) {
          console.warn('[VoiceEngine] startStreamSession failed (continuing):', err);
        }
      }

      const unsubPartial = window.api.onTranscriptPartial?.((ev) => {
        if (ev.sessionId && ev.sessionId !== sid) return;
        setSegments((prev) => {
          const next = [...prev];
          if (lastSegmentIdRef.current && next.length > 0) {
            const last = next[next.length - 1];
            if (last.id === lastSegmentIdRef.current && last.isPartial) {
              next[next.length - 1] = {
                ...last,
                text: ev.text,
                isPartial: true,
                timestamp: Date.now(),
              };
              return next;
            }
          }
          const id = uid();
          lastSegmentIdRef.current = id;
          next.push({ id, text: ev.text, isPartial: true, timestamp: Date.now() });
          return next;
        });
      });

      const unsubFinal = window.api.onTranscriptFinal?.((ev) => {
        if (ev.sessionId && ev.sessionId !== sid) return;
        setSegments((prev) => {
          const next = [...prev];
          if (lastSegmentIdRef.current && next.length > 0) {
            const last = next[next.length - 1];
            if (last.id === lastSegmentIdRef.current && last.isPartial) {
              next[next.length - 1] = {
                ...last,
                text: ev.text || last.text,
                isPartial: false,
                timestamp: Date.now(),
              };
              lastSegmentIdRef.current = '';
              return next;
            }
          }
          const id = uid();
          next.push({ id, text: ev.text, isPartial: false, timestamp: Date.now() });
          return next;
        });
      });

      const rec = new ExtMediaRecorder(stream, { mimeType: 'audio/wav' });
      chunksRef.current = [];

      rec.ondataavailable = (e: BlobEvent) => {
        if (!e.data.size) return;
        chunksRef.current.push(e.data);
        sendAudioChunk(e.data, sid, false);
      };

      rec.onstop = async () => {
        if (unsubPartial) unsubPartial();
        if (unsubFinal) unsubFinal();
        const audioCtx = audioCtxRef.current;
        if (audioCtx) audioCtx.close().catch(() => {});
        audioCtxRef.current = null;
        analyserRef.current = null;
        const blob = new Blob(chunksRef.current, { type: 'audio/wav' });
        if (blob.size > 1000) {
          await sendAudioChunk(blob, sid, true);
        }
        if (window.api.endStreamSession) {
          try {
            await window.api.endStreamSession(sid);
          } catch {}
        }
        stopStream();
        window.api.muteSystem(false);
        setState((prev) => (prev === 'error' ? prev : 'idle'));
        stopTimer();
      };

      rec.start(500);
      mediaRef.current = rec as any;
      recStartRef.current = Date.now();
      pauseOffsetRef.current = 0;
      startTimer();
      setState('recording');
    } catch (err: any) {
      const msg = err?.message || (typeof err === 'string' ? err : '') || 'Mic error';
      setError(msg);
      setState('error');
      setTimeout(() => setState((p) => (p === 'error' ? 'idle' : p)), 4000);
    }
  };

  const stopStream = () => {
    streamRef.current?.getTracks().forEach((t) => t.stop());
    streamRef.current = null;
  };

  const stopSession = () => {
    try {
      if (mediaRef.current && mediaRef.current.state !== 'inactive') {
        (mediaRef.current as any).stop();
      }
    } catch {}
    mediaRef.current = null;
    if (stateRef.current === 'recording' || stateRef.current === 'paused') {
      setState('processing');
      setTimeout(() => {
        setState((p) => (p === 'processing' ? 'idle' : p));
      }, 1200);
    }
    stopTimer();
  };

  const pauseSession = () => {
    try {
      if (mediaRef.current && mediaRef.current.state === 'recording') {
        (mediaRef.current as any).pause();
      }
      pauseOffsetRef.current = Date.now() - recStartRef.current;
      pauseTimer();
      setState('paused');
    } catch (err) {
      console.warn(err);
    }
  };

  const resumeSession = async () => {
    try {
      if (mediaRef.current && mediaRef.current.state === 'paused') {
        (mediaRef.current as any).resume();
        recStartRef.current = Date.now() - pauseOffsetRef.current;
        startTimer();
        setState('recording');
      }
    } catch (err) {
      console.warn(err);
    }
  };

  const clearAll = () => {
    setSegments([]);
    lastSegmentIdRef.current = '';
  };

  const copyAll = async () => {
    const txt = segments
      .map((s) => s.text.trim())
      .filter(Boolean)
      .join(' ');
    if (!txt) return;
    try {
      await navigator.clipboard.writeText(txt);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {}
  };

  const openSettings = () => {
    window.api.openSettings().catch(() => {});
  };

  const fmt = (s: number) => {
    const m = Math.floor(s / 60)
      .toString()
      .padStart(2, '0');
    const sec = (s % 60).toString().padStart(2, '0');
    return `${m}:${sec}`;
  };

  const isDark = theme === 'dark';
  const fullText = segments
    .map((s) => s.text)
    .join(' ')
    .trim();
  const wordCount = fullText ? fullText.split(/\s+/).length : 0;

  return (
    <div
      className="w-full h-full flex flex-col"
      style={{
        background: isDark
          ? 'linear-gradient(180deg, #0a0e1a 0%, #0d111c 40%, #0b0f1a 100%)'
          : 'linear-gradient(180deg, #f8fafc 0%, #f1f5f9 100%)',
        color: isDark ? '#e2e8f0' : '#0f172a',
      }}
    >
      <div
        className="flex items-center justify-between px-5 py-3 border-b select-none"
        style={{
          borderColor: isDark ? 'rgba(255,255,255,0.08)' : 'rgba(0,0,0,0.08)',
          background: isDark ? 'rgba(255,255,255,0.02)' : 'rgba(0,0,0,0.02)',
        }}
      >
        <div className="flex items-center gap-3">
          <LogoGlyph size={22} />
          <div>
            <div className="text-[15px] font-bold leading-tight">
              <span
                style={{
                  background: 'linear-gradient(90deg,#3a84ef 0%,#8b5cf6 50%,#f7b731 100%)',
                  WebkitBackgroundClip: 'text',
                  WebkitTextFillColor: 'transparent',
                }}
              >
                BizVoice Live
              </span>
            </div>
            <div
              className="text-[11px] leading-tight mt-0.5"
              style={{ color: isDark ? '#94a3b8' : '#64748b' }}
            >
              Live Transcript · {lang.toUpperCase()}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-3">
          <div
            className="text-[12px] font-mono px-3 py-1 rounded-full"
            style={{
              background: isDark ? 'rgba(59,130,246,0.12)' : 'rgba(59,130,246,0.10)',
              color: isDark ? '#93c5fd' : '#2563eb',
              border: `1px solid ${isDark ? 'rgba(59,130,246,0.25)' : 'rgba(59,130,246,0.2)'}`,
            }}
          >
            {fmt(elapsed)}
          </div>
          <button
            onClick={openSettings}
            className="nodrag p-2 rounded-lg transition-all hover:scale-105"
            style={{
              background: isDark ? 'rgba(255,255,255,0.05)' : 'rgba(0,0,0,0.04)',
              color: isDark ? '#cbd5e1' : '#475569',
            }}
            title="Settings"
          >
            {ICON_SETTINGS(18)}
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto px-8 py-6">
        <div className="max-w-3xl mx-auto space-y-1">
          {segments.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-[60vh] text-center">
              <div
                className="mb-5 flex items-center justify-center rounded-full"
                style={{
                  width: 88,
                  height: 88,
                  background: isDark
                    ? 'radial-gradient(circle at 40% 30%, rgba(59,130,246,0.18), rgba(139,92,246,0.08) 60%, transparent 80%)'
                    : 'radial-gradient(circle at 40% 30%, rgba(59,130,246,0.14), rgba(139,92,246,0.06) 60%, transparent 80%)',
                }}
              >
                <div style={{ color: isDark ? '#60a5fa' : '#3b82f6' }}>
                  {ICON_MIC(40)}
                </div>
              </div>
              <div
                className="text-[22px] font-semibold mb-2"
                style={{ color: isDark ? '#cbd5e1' : '#1e293b' }}
              >
                Press the mic to start
              </div>
              <div
                className="text-[13px] max-w-md leading-relaxed"
                style={{ color: isDark ? '#64748b' : '#94a3b8' }}
              >
                Speak naturally — your voice will appear here in real time. Click the red mic
                button below or use your keyboard shortcut.
              </div>
            </div>
          ) : (
            segments.map((seg, i) => {
              const prev = segments[i - 1];
              const gap =
                prev && seg.timestamp - prev.timestamp > 5000 ? ' mt-3' : '';
              return (
                <div
                  key={seg.id}
                  className={gap}
                  style={{
                    display: 'inline-block',
                    width: '100%',
                  }}
                >
                  <span
                    className="whitespace-pre-wrap break-words"
                    style={{
                      fontSize: state === 'recording' && i === segments.length - 1 ? '24px' : '20px',
                      lineHeight: 1.55,
                      fontWeight: seg.isPartial ? 500 : 400,
                      color: seg.isPartial
                        ? isDark
                          ? '#60a5fa'
                          : '#2563eb'
                        : isDark
                        ? '#e2e8f0'
                        : '#0f172a',
                      opacity: seg.isPartial ? 1 : 1,
                      transition: 'color 0.2s ease',
                      fontStyle: seg.isPartial ? 'normal' : 'normal',
                      letterSpacing: seg.isPartial ? '0.01em' : '0',
                    }}
                  >
                    {seg.text}
                    {seg.isPartial && (
                      <span
                        style={{
                          display: 'inline-block',
                          width: 2,
                          height: '1em',
                          background: isDark ? '#60a5fa' : '#2563eb',
                          marginLeft: 2,
                          verticalAlign: 'text-bottom',
                          animation: 've-caret 0.8s steps(2, end) infinite',
                          borderRadius: 1,
                        }}
                      />
                    )}
                  </span>
                </div>
              );
            })
          )}
          <div ref={segmentsEndRef} />
        </div>
      </div>

      <div
        className="px-6 pt-2"
        style={{
          borderTop: isDark ? '1px solid rgba(255,255,255,0.06)' : '1px solid rgba(0,0,0,0.06)',
          background: isDark ? 'linear-gradient(0deg, rgba(13,17,28,1) 0%, rgba(13,17,28,0.6) 100%)' : '#fff',
        }}
      >
        <canvas
          ref={canvasRef}
          style={{
            width: '100%',
            height: 56,
            display: 'block',
            opacity: state === 'recording' || state === 'paused' ? 1 : 0.25,
            transition: 'opacity 0.3s',
          }}
        />
      </div>

      <div
        className="flex items-center justify-between px-6 py-4 nodrag"
        style={{
          background: isDark ? 'rgba(10,14,26,0.95)' : '#f8fafc',
          borderTop: isDark ? '1px solid rgba(255,255,255,0.06)' : '1px solid rgba(0,0,0,0.06)',
        }}
      >
        <div className="flex items-center gap-2 min-w-[180px]">
          {segments.length > 0 && (
            <>
              <button
                onClick={copyAll}
                className="p-2 rounded-lg transition-all hover:scale-105"
                style={{
                  background: isDark ? 'rgba(255,255,255,0.05)' : 'rgba(0,0,0,0.04)',
                  color: copied
                    ? '#4ade80'
                    : isDark
                    ? '#cbd5e1'
                    : '#475569',
                }}
                title="Copy all text"
              >
                {copied
                  ? <HugeiconsIcon icon={CheckmarkCircle02Icon} size={18} strokeWidth={2} />
                  : ICON_COPY(18)}
              </button>
              <button
                onClick={clearAll}
                className="p-2 rounded-lg transition-all hover:scale-105"
                style={{
                  background: isDark ? 'rgba(255,255,255,0.05)' : 'rgba(0,0,0,0.04)',
                  color: isDark ? '#f87171' : '#dc2626',
                }}
                title="Clear transcript"
              >
                {ICON_TRASH(18)}
              </button>
              <div
                className="text-[11px] ml-2 font-medium px-2 py-1 rounded"
                style={{
                  color: isDark ? '#64748b' : '#94a3b8',
                }}
              >
                {wordCount} words
              </div>
            </>
          )}
        </div>

        <div className="flex items-center gap-3">
          {state === 'recording' && (
            <button
              onClick={pauseSession}
              className="p-3 rounded-full transition-all hover:scale-110"
              style={{
                background: isDark ? 'rgba(251,191,36,0.15)' : 'rgba(251,191,36,0.12)',
                color: isDark ? '#fbbf24' : '#d97706',
                border: `1px solid ${isDark ? 'rgba(251,191,36,0.3)' : 'rgba(251,191,36,0.25)'}`,
              }}
              title="Pause"
            >
              {ICON_PAUSE(22)}
            </button>
          )}
          {state === 'paused' && (
            <button
              onClick={resumeSession}
              className="p-3 rounded-full transition-all hover:scale-110"
              style={{
                background: isDark ? 'rgba(34,197,94,0.15)' : 'rgba(34,197,94,0.12)',
                color: isDark ? '#4ade80' : '#16a34a',
                border: `1px solid ${isDark ? 'rgba(34,197,94,0.3)' : 'rgba(34,197,94,0.25)'}`,
              }}
              title="Resume"
            >
              {ICON_PLAY(22)}
            </button>
          )}

          <button
            onClick={() => {
              if (state === 'recording' || state === 'paused') stopSession();
              else startSession();
            }}
            className="relative flex items-center justify-center transition-all transform hover:scale-[1.04] active:scale-95"
            style={{
              width: 68,
              height: 68,
              borderRadius: 9999,
              background:
                state === 'recording'
                  ? 'linear-gradient(145deg, #ef4444 0%, #dc2626 100%)'
                  : 'linear-gradient(145deg, #3b82f6 0%, #2563eb 100%)',
              boxShadow:
                state === 'recording'
                  ? '0 0 0 4px rgba(239,68,68,0.18), 0 8px 28px rgba(239,68,68,0.45), inset 0 2px 4px rgba(255,255,255,0.2)'
                  : '0 0 0 4px rgba(59,130,246,0.18), 0 8px 28px rgba(59,130,246,0.45), inset 0 2px 4px rgba(255,255,255,0.25)',
              border: `2px solid ${
                state === 'recording'
                  ? 'rgba(255,255,255,0.35)'
                  : 'rgba(255,255,255,0.45)'
              }`,
            }}
            title={state === 'recording' ? 'Stop' : 'Start recording'}
          >
            {state === 'processing' ? (
              <HugeiconsIcon
                icon={Loading03Icon}
                size={26}
                strokeWidth={2.5}
                className="spin"
                style={{ color: '#fff' }}
              />
            ) : state === 'recording' || state === 'paused' ? (
              <div
                style={{
                  width: 22,
                  height: 22,
                  borderRadius: 5,
                  background: '#fff',
                }}
              />
            ) : (
              <div style={{ color: '#fff' }}>
                {ICON_MIC_OFF(28)}
              </div>
            )}

            {state === 'recording' && (
              <>
                <span
                  className="ve-sonar"
                  style={{
                    position: 'absolute',
                    inset: 0,
                    borderRadius: 9999,
                    border: '2px solid #ef4444',
                    pointerEvents: 'none',
                    animation: 've-sonar 1.8s ease-out infinite',
                  }}
                />
                <span
                  className="ve-sonar"
                  style={{
                    position: 'absolute',
                    inset: 0,
                    borderRadius: 9999,
                    border: '2px solid #ef4444',
                    pointerEvents: 'none',
                    animation: 've-sonar 1.8s ease-out infinite 0.6s',
                  }}
                />
              </>
            )}
          </button>
        </div>

        <div className="flex items-center gap-2 min-w-[180px] justify-end">
          {state === 'idle' && !error && (
            <div
              className="flex items-center gap-2 text-[12px] px-3 py-1.5 rounded-full"
              style={{
                color: isDark ? '#94a3b8' : '#64748b',
                background: isDark ? 'rgba(255,255,255,0.04)' : 'rgba(0,0,0,0.04)',
              }}
            >
              <span
                style={{
                  width: 7,
                  height: 7,
                  borderRadius: 9999,
                  background: '#4ade80',
                  display: 'inline-block',
                  boxShadow: '0 0 8px rgba(74,222,128,0.7)',
                }}
              />
              Ready
            </div>
          )}
          {error && (
            <div
              className="text-[12px] font-medium px-3 py-1.5 rounded-full max-w-[200px] truncate"
              style={{
                color: '#fca5a5',
                background: 'rgba(239,68,68,0.12)',
                border: '1px solid rgba(239,68,68,0.25)',
              }}
              title={error}
            >
              {error}
            </div>
          )}
        </div>
      </div>

      <style>{`
        @keyframes ve-caret {
          0%, 100% { opacity: 1; }
          50% { opacity: 0.1; }
        }
        @keyframes ve-sonar {
          0% { transform: scale(1); opacity: 0.55; }
          100% { transform: scale(1.55); opacity: 0; }
        }
      `}</style>
    </div>
  );
}
