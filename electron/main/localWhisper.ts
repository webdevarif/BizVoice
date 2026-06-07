import { join } from 'path';
import { existsSync, mkdirSync, writeFileSync, unlinkSync, renameSync } from 'fs';

let modelsDir = '';

export function setModelsDir(dir: string) {
  modelsDir = dir;
}

function getModelsDir(): string {
  if (!modelsDir) throw new Error('modelsDir not set — call setModelsDir() first');
  return modelsDir;
}

const MODEL_META: Record<string, { file: string; size: string }> = {
  'tiny':        { file: 'ggml-tiny.bin',        size: '75 MB' },
  'tiny.en':     { file: 'ggml-tiny.en.bin',     size: '75 MB' },
  'base':        { file: 'ggml-base.bin',        size: '142 MB' },
  'base.en':     { file: 'ggml-base.en.bin',     size: '142 MB' },
  'small':       { file: 'ggml-small.bin',       size: '466 MB' },
  'small.en':    { file: 'ggml-small.en.bin',    size: '466 MB' },
  'medium':      { file: 'ggml-medium.bin',      size: '1.5 GB' },
  'medium.en':   { file: 'ggml-medium.en.bin',   size: '1.5 GB' },
  'large-v3-turbo': { file: 'ggml-large-v3-turbo.bin', size: '1.5 GB' },
};

const HF_BASE = 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main';

export function getModelPath(modelName: string): string {
  const meta = MODEL_META[modelName];
  if (!meta) throw new Error(`Unknown model: ${modelName}`);
  return join(getModelsDir(), meta.file);
}

export function isModelDownloaded(modelName: string): boolean {
  try {
    return existsSync(getModelPath(modelName));
  } catch {
    return false;
  }
}

export function listModels(): { name: string; size: string; downloaded: boolean }[] {
  return Object.entries(MODEL_META).map(([name, meta]) => ({
    name,
    size: meta.size,
    downloaded: isModelDownloaded(name),
  }));
}

export function deleteModel(modelName: string): boolean {
  try {
    const p = getModelPath(modelName);
    if (existsSync(p)) { unlinkSync(p); return true; }
    return false;
  } catch { return false; }
}

export async function downloadModel(
  modelName: string,
  onProgress: (pct: number) => void,
): Promise<string> {
  const meta = MODEL_META[modelName];
  if (!meta) throw new Error(`Unknown model: ${modelName}`);

  const dir = getModelsDir();
  mkdirSync(dir, { recursive: true });
  const dest = join(dir, meta.file);

  if (existsSync(dest)) return dest;

  const url = `${HF_BASE}/${meta.file}`;
  const tmpDest = dest + '.tmp';

  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`Download failed: HTTP ${resp.status}`);

  const total = parseInt(resp.headers.get('content-length') || '0', 10);
  let received = 0;
  const chunks: Uint8Array[] = [];

  const reader = resp.body!.getReader();
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    received += value.length;
    if (total > 0) onProgress(Math.round((received / total) * 100));
  }

  const buffer = Buffer.concat(chunks);
  writeFileSync(tmpDest, buffer);
  try { renameSync(tmpDest, dest); } catch {}
  onProgress(100);
  return dest;
}

let whisperContext: any = null;
let loadedModelPath: string | null = null;

export async function transcribeLocal(opts: {
  audioPath: string;
  modelName: string;
  language?: string;
  log?: (msg: string) => void;
}): Promise<string> {
  const modelPath = getModelPath(opts.modelName);
  if (!existsSync(modelPath)) {
    throw new Error(`Model not downloaded: ${opts.modelName}. Download it in Settings → API & Models.`);
  }

  const { initWhisper } = await import('@fugood/whisper.node');

  if (loadedModelPath !== modelPath) {
    if (whisperContext) {
      try { await whisperContext.release(); } catch {}
    }
    opts.log?.(`loading whisper model: ${opts.modelName}`);
    whisperContext = await initWhisper({ filePath: modelPath });
    loadedModelPath = modelPath;
  }

  opts.log?.('transcribing locally...');
  const options: Record<string, any> = {};
  if (opts.language && opts.language !== 'auto') {
    options.language = opts.language;
  }
  const handle = await whisperContext.transcribe(opts.audioPath, options);
  const result = await handle.promise;

  opts.log?.(`whisper result keys: ${Object.keys(result || {})}`);

  let text = '';
  if (typeof result === 'string') {
    text = result.trim();
  } else if (Array.isArray(result)) {
    text = result.map((s: any) => s.text || '').join(' ').trim();
  } else if (result?.segments) {
    text = result.segments.map((s: any) => s.text).join(' ').trim();
  }

  opts.log?.(`local STT result: ${text || '(empty)'}`);
  return text;
}
