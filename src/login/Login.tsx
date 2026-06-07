import { useEffect, useState } from 'react';

interface AuthStatus { loggedIn: boolean; active: boolean; email: string; offline?: boolean }
interface BizApi {
  startBrowserLogin: () => Promise<{ ok: boolean; error?: string }>;
  cancelBrowserLogin: () => Promise<{ ok: boolean }>;
  logout: () => Promise<{ ok: boolean }>;
  authStatus: () => Promise<AuthStatus>;
  openSubscribe: () => Promise<void>;
  openRegister: () => Promise<void>;
  onAuthChange: (cb: (d: { active: boolean; loggedIn: boolean; email: string }) => void) => () => void;
  closeWindow: () => Promise<void>;
}
const api = (window as unknown as { api: BizApi }).api;

// Subtle film-grain noise texture for the backdrop.
const NOISE_BG =
  "url(\"data:image/svg+xml,%3Csvg viewBox='0 0 256 256' xmlns='http://www.w3.org/2000/svg'%3E%3Cfilter id='noise'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23noise)'/%3E%3C/svg%3E\")";

const EQ_BARS = [0.5, 0.9, 0.35, 1, 0.6, 0.85, 0.45, 0.95, 0.4];

type View = 'checking' | 'welcome' | 'waiting' | 'subscribe';

export function Login() {
  const [view, setView] = useState<View>('checking');
  const [email, setEmail] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    api.authStatus()
      .then((s) => {
        if (s.active) return; // main shows the mic bar + closes this window
        setEmail(s.email || '');
        setView(s.loggedIn ? 'subscribe' : 'welcome');
      })
      .catch(() => setView('welcome'));

    const off = api.onAuthChange((d) => {
      setEmail(d.email || '');
      if (d.active) return;
      setView(d.loggedIn ? 'subscribe' : 'welcome');
    });
    return off;
  }, []);

  async function signIn() {
    setError('');
    setView('waiting');
    const r = await api.startBrowserLogin();
    if (!r.ok) { setError(r.error || 'Could not open the browser.'); setView('welcome'); }
  }

  function goBack() {
    api.cancelBrowserLogin();
    setError('');
    setView('welcome');
  }

  async function recheck() {
    setError('');
    const s = await api.authStatus();
    if (s.active) return;
    setError(s.offline ? 'Could not reach BizGrowHub — check your connection.' : 'No active subscription found yet.');
  }

  return (
    <div className="drag relative w-full h-full flex bg-[#0c0c11] text-white overflow-hidden">
      {/* Noise backdrop */}
      <div aria-hidden className="pointer-events-none absolute inset-0 opacity-[0.05] mix-blend-soft-light" style={{ backgroundImage: NOISE_BG }} />
      {/* Accent glow */}
      <div aria-hidden className="pointer-events-none absolute -top-24 -left-24 w-72 h-72 rounded-full bg-accent/20 blur-[90px]" />

      {/* Close */}
      <button
        onClick={() => api.closeWindow()}
        className="no-drag absolute top-3 right-4 z-30 text-white/40 hover:text-white text-xl leading-none"
        aria-label="Close"
      >×</button>

      {/* ── Left brand panel ── */}
      <div className="relative w-[42%] shrink-0 flex flex-col justify-between p-8 border-r border-white/[0.06] bg-gradient-to-br from-accent/[0.18] via-accent/[0.04] to-transparent">
        <div className="flex items-center gap-2.5">
          <img src="./logo.svg" alt="BizVoice" className="w-9 h-9 object-contain" draggable={false} />
          <span className="text-lg font-bold tracking-tight">Biz<span className="text-accent">Voice</span></span>
        </div>

        {/* Animated equalizer */}
        <div className="flex items-end gap-[5px] h-20" aria-hidden>
          {EQ_BARS.map((h, i) => (
            <span
              key={i}
              className="eq-bar w-[5px] rounded-full bg-gradient-to-t from-accent to-sky-300"
              style={{ height: `${h * 100}%`, animationDelay: `${i * 0.11}s` }}
            />
          ))}
        </div>

        <div>
          <p className="text-[26px] font-extrabold leading-[1.1] mb-2.5">Speak.<br />It types itself.</p>
          <p className="text-[13px] text-white/55 leading-relaxed">
            Privacy-first voice dictation — runs on-device and drops your words wherever your cursor is, in any app.
          </p>
        </div>
      </div>

      {/* ── Right content panel ── */}
      <div className="relative flex-1 flex flex-col justify-center px-9">
        {view === 'checking' && (
          <p className="text-white/50 text-sm">Checking your subscription…</p>
        )}

        {view === 'welcome' && (
          <div className="max-w-[300px]">
            <h1 className="text-2xl font-bold mb-1.5">Get started</h1>
            <p className="text-sm text-white/50 mb-7 leading-relaxed">
              Sign in with your BizGrowHub account to activate BizVoice on this PC.
            </p>
            {error && <p className="text-xs text-red-400 mb-3">{error}</p>}
            <button
              onClick={signIn}
              className="no-drag w-full bg-accent hover:bg-accent/90 rounded-xl py-3 text-sm font-semibold transition-colors flex items-center justify-center gap-2"
            >
              <span>🌐</span> Sign in with Browser
            </button>
            <button
              onClick={() => api.openRegister()}
              className="no-drag w-full mt-2.5 border border-white/12 hover:bg-white/[0.05] rounded-xl py-3 text-sm font-medium transition-colors"
            >
              Create an account
            </button>
            <p className="text-[11px] text-white/35 mt-6">By continuing you agree to our Terms &amp; Privacy.</p>
          </div>
        )}

        {view === 'waiting' && (
          <div className="max-w-[300px]">
            <h1 className="text-xl font-bold mb-1.5">Finish in your browser</h1>
            <p className="text-sm text-white/50 mb-6 leading-relaxed">
              We opened BizGrowHub in your browser. Approve the sign-in there and you&apos;ll land back here automatically.
            </p>
            {error && <p className="text-xs text-red-400 mb-3">{error}</p>}
            <div className="flex items-center gap-2">
              <button onClick={goBack} className="no-drag px-4 py-2.5 rounded-xl border border-white/12 text-sm text-white/70 hover:bg-white/5 transition-colors">
                ← Back
              </button>
              <button onClick={signIn} className="no-drag px-4 py-2.5 rounded-xl bg-white/[0.06] border border-white/12 text-sm hover:bg-white/10 transition-colors">
                Open browser again
              </button>
            </div>
          </div>
        )}

        {view === 'subscribe' && (
          <div className="max-w-[320px]">
            <h1 className="text-xl font-bold mb-2">One step left</h1>
            <p className="text-sm text-white/75 mb-1.5">
              Signed in{email ? <> as <span className="font-medium">{email}</span></> : ''}.
            </p>
            <p className="text-xs text-white/50 mb-6 leading-relaxed">
              BizVoice isn&apos;t active on your account yet. Subscribe for <span className="text-white font-semibold">৳250/month</span> in BizGrowHub, then continue.
            </p>
            {error && <p className="text-xs text-red-400 mb-3">{error}</p>}
            <button
              onClick={() => api.openSubscribe()}
              className="no-drag w-full bg-accent hover:bg-accent/90 rounded-xl py-3 text-sm font-semibold transition-colors"
            >
              Subscribe in BizGrowHub
            </button>
            <button
              onClick={recheck}
              className="no-drag w-full mt-2.5 border border-white/12 hover:bg-white/5 rounded-xl py-3 text-sm transition-colors"
            >
              I&apos;ve subscribed — continue
            </button>
            <button
              onClick={() => { api.logout(); setView('welcome'); }}
              className="no-drag text-xs text-white/40 hover:text-white mt-4"
            >
              Use a different account
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
