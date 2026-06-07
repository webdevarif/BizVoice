import { useEffect, useState } from 'react';

interface Release {
  version: string;
  notes?: string[];
  releasedAt?: string;
}
interface UpdateApi {
  updateInfo: () => Promise<{ current: string; latest: Release | null; updateAvailable: boolean; downloaded: boolean }>;
  updateDownload: () => Promise<void>;
  updateInstall: () => Promise<void>;
  updateLater: () => Promise<void>;
  onUpdateProgress: (cb: (data: { percent: number; transferred: number; total: number }) => void) => () => void;
  onUpdateDownloaded: (cb: () => void) => () => void;
}
const api = (window as unknown as { api: UpdateApi }).api;

type Stage = 'prompt' | 'downloading' | 'ready';

export function Update() {
  const [current, setCurrent] = useState('');
  const [rel, setRel] = useState<Release | null>(null);
  const [stage, setStage] = useState<Stage>('prompt');
  const [progress, setProgress] = useState(0);

  useEffect(() => {
    api.updateInfo().then((d) => {
      setCurrent(d.current);
      setRel(d.latest);
      if (d.downloaded) setStage('ready');
    });
    const offProgress = api.onUpdateProgress((p) => {
      setProgress(p.percent);
      setStage('downloading');
    });
    const offDone = api.onUpdateDownloaded(() => {
      setProgress(100);
      setStage('ready');
    });
    return () => { offProgress(); offDone(); };
  }, []);

  const notes = rel?.notes ?? [];

  return (
    <div className="drag relative w-full h-full flex flex-col bg-[#0c0c11] text-white overflow-hidden">
      <div aria-hidden className="pointer-events-none absolute -top-24 left-1/2 -translate-x-1/2 w-72 h-72 rounded-full bg-accent/20 blur-[90px]" />

      <button onClick={() => api.updateLater()} className="no-drag absolute top-3 right-4 z-30 text-white/40 hover:text-white text-xl leading-none" aria-label="Close">×</button>

      <div className="relative z-10 flex-1 overflow-y-auto px-8 py-8">
        <div className="flex flex-col items-center text-center mb-6">
          <img src="./logo.svg" alt="BizVoice" className="w-11 h-11 object-contain mb-3" draggable={false} />
          <div className="text-[11px] font-semibold tracking-wider text-accent uppercase mb-1">
            v{rel?.version ?? ''}{rel?.releasedAt ? ` · ${rel.releasedAt}` : ''}
          </div>
          <h1 className="text-2xl font-extrabold leading-tight">
            {stage === 'ready' ? 'Update ready to install' : stage === 'downloading' ? 'Downloading update' : 'Update available'}
          </h1>
          <p className="text-sm text-white/55 mt-1.5">
            {stage === 'ready'
              ? 'Restart BizVoice to apply the update.'
              : stage === 'downloading'
              ? `Pulling v${rel?.version ?? ''} from GitHub…`
              : `You're on v${current}. A newer BizVoice is ready.`}
          </p>
        </div>

        {stage === 'downloading' && (
          <div className="max-w-[360px] mx-auto mb-6">
            <div className="h-2 w-full rounded-full bg-white/10 overflow-hidden">
              <div className="h-full bg-accent transition-all" style={{ width: `${Math.max(2, Math.round(progress))}%` }} />
            </div>
            <div className="text-xs text-white/40 text-center mt-2">{Math.round(progress)}%</div>
          </div>
        )}

        {stage === 'prompt' && notes.length > 0 && (
          <div className="max-w-[420px] mx-auto space-y-2.5 mb-2">
            {notes.map((n, i) => (
              <div key={i} className="flex items-start gap-2.5">
                <span className="mt-0.5 text-accent shrink-0">✦</span>
                <p className="text-sm text-white/75 leading-relaxed">{n}</p>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="relative z-10 border-t border-white/[0.06] p-4 flex items-center justify-end gap-2 bg-black/20">
        {stage !== 'downloading' && (
          <button onClick={() => api.updateLater()} className="no-drag px-4 py-2.5 rounded-xl border border-white/12 text-sm text-white/70 hover:bg-white/5 transition-colors">
            Later
          </button>
        )}
        {stage === 'prompt' && (
          <button onClick={() => { setStage('downloading'); api.updateDownload(); }} className="no-drag px-5 py-2.5 rounded-xl bg-accent hover:bg-accent/90 text-sm font-semibold transition-colors">
            Download &amp; update
          </button>
        )}
        {stage === 'ready' && (
          <button onClick={() => api.updateInstall()} className="no-drag px-5 py-2.5 rounded-xl bg-accent hover:bg-accent/90 text-sm font-semibold transition-colors">
            Restart now
          </button>
        )}
      </div>
    </div>
  );
}
