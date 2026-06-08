import OpenAI from 'openai';
import { writeFileSync, unlinkSync, createReadStream } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';
import { transcribeLocal } from './localWhisper';

type Lang = string;

const LANG_TO_ISO: Record<string, string> = {
  english: 'en', bangla: 'bn', hindi: 'hi', urdu: 'ur', arabic: 'ar',
  spanish: 'es', french: 'fr', german: 'de', italian: 'it', portuguese: 'pt',
  russian: 'ru', chinese: 'zh', japanese: 'ja', korean: 'ko', turkish: 'tr',
  dutch: 'nl', polish: 'pl', vietnamese: 'vi', thai: 'th', indonesian: 'id',
};

function toISO639(lang: Lang): string | undefined {
  if (!lang || lang === 'auto') return undefined;
  return LANG_TO_ISO[lang.toLowerCase()] ?? undefined;
}

// Whisper accepts a free-text `prompt` parameter that biases recognition
// toward similar style/vocabulary. For low-resource languages (Bangla,
// Hindi, Urdu, etc.) Whisper's accuracy is poor by default; seeding the
// prompt with a natural sample in the target language nudges the model
// toward correct character shapes, conjunct consonants, and grammar.
const LANG_SEEDS: Record<string, string> = {
  bangla: 'নমস্কার, আমি বাংলায় কথা বলছি। প্রযুক্তি, কাজ, ব্যবসা, এবং দৈনন্দিন কথোপকথন নিয়ে কথা বলব।',
  hindi:  'नमस्ते, मैं हिंदी में बात कर रहा हूँ। टेक्नोलॉजी, काम, व्यवसाय, और रोज़मर्रा की बातचीत के बारे में बोलूँगा।',
  urdu:   'السلام علیکم، میں اردو میں بات کر رہا ہوں۔ ٹیکنالوجی، کام، اور روزمرہ کی گفتگو کے بارے میں۔',
  arabic: 'مرحباً، أنا أتحدث بالعربية. سأتحدث عن التكنولوجيا والعمل والمحادثات اليومية.',
  tamil:  'வணக்கம், நான் தமிழில் பேசுகிறேன். தொழில்நுட்பம், வேலை, மற்றும் தினசரி உரையாடல்கள் பற்றி பேசுவேன்.',
};

function langSeed(inputLang: Lang): string {
  if (!inputLang || inputLang === 'auto') return '';
  return LANG_SEEDS[inputLang.toLowerCase()] ?? '';
}

// Combine the user's custom vocabulary with a language seed (when applicable)
// into a single Whisper `prompt` value. Order matters: the seed goes first so
// the model anchors on the target language's character set before tuning to
// domain terms.
function buildPrompt(opts: { vocabulary?: string; inputLang: Lang }): string {
  const seed  = langSeed(opts.inputLang);
  const vocab = opts.vocabulary?.trim() ? `Domain vocabulary: ${opts.vocabulary.trim()}` : '';
  return [seed, vocab].filter(Boolean).join(' ').trim();
}

// Construct the OpenAI-compatible client for the configured GPT provider.
// Returns null when the chosen provider has no key (caller should skip GPT).
function buildGptClient(opts: {
  apiKey?: string;
  groqKey?: string;
  openrouterKey?: string;
  customKey?: string;
  customBaseUrl?: string;
  customHeaders?: string;
  gptModel: string;
  gptProvider?: GptProvider;
}): { client: OpenAI; model: string; providerLabel: string } | null {
  const provider = opts.gptProvider ?? 'openai';
  if (provider === 'groq') {
    if (!opts.groqKey) return null;
    return {
      client: new OpenAI({ apiKey: opts.groqKey, baseURL: 'https://api.groq.com/openai/v1' }),
      model: opts.gptModel,
      providerLabel: 'Groq',
    };
  }
  if (provider === 'openrouter') {
    if (!opts.openrouterKey) return null;
    return {
      client: new OpenAI({
        apiKey: opts.openrouterKey,
        baseURL: 'https://openrouter.ai/api/v1',
        defaultHeaders: {
          'HTTP-Referer': 'https://github.com/webdevarif/BizVoice',
          'X-Title': 'BizVoice',
        },
      }),
      model: opts.gptModel,
      providerLabel: 'OpenRouter',
    };
  }
  if (provider === 'custom') {
    if (!opts.customKey || !opts.customBaseUrl) return null;
    let extraHeaders: Record<string, string> = {};
    if (opts.customHeaders) {
      try { extraHeaders = JSON.parse(opts.customHeaders); } catch { /* ignore bad JSON */ }
    }
    return {
      client: new OpenAI({
        apiKey: opts.customKey,
        baseURL: opts.customBaseUrl,
        defaultHeaders: extraHeaders,
      }),
      model: opts.gptModel,
      providerLabel: 'Custom',
    };
  }
  // Default: OpenAI
  if (!opts.apiKey) return null;
  return {
    client: new OpenAI({ apiKey: opts.apiKey }),
    model: opts.gptModel,
    providerLabel: 'OpenAI',
  };
}

function languageInstruction(inputLang: Lang, outputLang: Lang): string {
  if (outputLang && outputLang !== 'auto') {
    const inLbl = inputLang && inputLang !== 'auto' ? inputLang : 'the detected input language';
    return `The input is in ${inLbl}. Output MUST be in ${outputLang}. Translate if needed — do not keep any ${inLbl} words in the output.`;
  }
  return `Keep the output in the SAME language as the input.`;
}

export type GptProvider = 'openai' | 'groq' | 'openrouter' | 'custom';

// Better-Bangla path: BizVoice client POSTs the audio to the BizGrowHub
// backend, which holds the developer's own HuggingFace token (or runs a
// self-hosted Bangla ASR model) and returns the transcript. End users
// never need an HF account or token of their own — works out of the box
// for anyone with a valid BizGrowHub login.
export function isBetterBanglaSupported(lang: Lang): boolean {
  return !!lang && lang.toLowerCase() === 'bangla';
}

async function transcribeViaBizGrowHub(opts: {
  bghBaseUrl: string;
  bghToken: string;
  audioPath: string;
  inputLang: Lang;
  log: (msg: string, data?: any) => void;
}): Promise<string> {
  const { bghBaseUrl, bghToken, audioPath, inputLang, log } = opts;
  log(`calling BizGrowHub Bangla STT...`);

  const { readFileSync } = await import('fs');
  const audioBytes = readFileSync(audioPath);
  // @ts-ignore — Node global Blob accepts Buffer
  const blob = new Blob([audioBytes], { type: 'audio/wav' });
  const form = new FormData();
  form.append('file', blob, 'audio.wav');
  form.append('language', inputLang.toLowerCase());

  const doFetch = () =>
    fetch(`${bghBaseUrl}/api/bizvoice/transcribe-indic`, {
      method: 'POST',
      headers: { 'Authorization': `Bearer ${bghToken}` },
      body: form as any,
      signal: AbortSignal.timeout(60_000),
    });

  // The server may warm a HuggingFace model on first call — retry once on
  // 503 with a short wait, then surface the error.
  let res = await doFetch();
  if (res.status === 503) {
    log('bgh model warming up — waiting 6s and retrying once');
    await new Promise((r) => setTimeout(r, 6000));
    res = await doFetch();
  }
  if (!res.ok) {
    const errText = await res.text().catch(() => '');
    throw new Error(`BizGrowHub Bangla STT failed (${res.status}): ${errText.slice(0, 200)}`);
  }
  const data = await res.json().catch(() => ({} as any));
  return String(data.transcript ?? data.text ?? '').trim();
}

export async function runPipeline(opts: {
  audioBase64: string;
  apiKey: string;
  groqKey?: string;
  openrouterKey?: string;
  customKey?: string;
  useBetterBangla?: boolean;
  bghBaseUrl?: string;
  bghToken?: string;
  customBaseUrl?: string;
  customHeaders?: string;  // JSON string
  inputLang: Lang;
  outputLang: Lang;
  gptModel: string;
  gptProvider?: GptProvider;
  sttModel: string;
  sttProvider?: 'openai' | 'groq';
  skipGpt?: boolean;
  vocabulary?: string;
  styleSystemPrompt: string;
  useLocalWhisper?: boolean;
  localModel?: string;
  log?: (msg: string, data?: any) => void;
}): Promise<string> {
  const log = opts.log || (() => {});

  const buffer = Buffer.from(opts.audioBase64, 'base64');
  log(`decoded audio: ${buffer.length} bytes`);
  if (buffer.length < 1000) {
    throw new Error(`Audio too short (${buffer.length} bytes). Try again.`);
  }

  const tmpPath = join(tmpdir(), `bizvoice-${Date.now()}.wav`);
  writeFileSync(tmpPath, buffer);

  const STT_TIMEOUT = 30_000;
  const GPT_TIMEOUT = 20_000;

  try {
    let raw: string;

    if (opts.useLocalWhisper && opts.localModel) {
      raw = await transcribeLocal({
        audioPath: tmpPath,
        modelName: opts.localModel,
        language: toISO639(opts.inputLang),
        log: (msg) => log(msg),
      });
    } else if (opts.useBetterBangla && opts.bghBaseUrl && opts.bghToken && isBetterBanglaSupported(opts.inputLang)) {
      // ── BizGrowHub proxy → server-side Bangla ASR (free for end users) ──
      raw = await transcribeViaBizGrowHub({
        bghBaseUrl: opts.bghBaseUrl,
        bghToken: opts.bghToken,
        audioPath: tmpPath,
        inputLang: opts.inputLang,
        log,
      });
    } else if (opts.sttProvider === 'groq' && opts.groqKey) {
      // ── Groq Whisper (10-20x faster) ──
      // whisper-large-v3 (non-turbo) — slightly higher accuracy, esp. for
      // low-resource languages like Bangla. Worth the speed trade-off when
      // a language seed is in play.
      const groqModel = langSeed(opts.inputLang) ? 'whisper-large-v3' : 'whisper-large-v3-turbo';
      log(`calling Groq STT (${groqModel})...`);
      const groq = new OpenAI({
        apiKey: opts.groqKey,
        baseURL: 'https://api.groq.com/openai/v1',
      });
      const isoLang = toISO639(opts.inputLang);
      const promptHint = buildPrompt(opts);
      const sttAbort = AbortSignal.timeout(STT_TIMEOUT);
      const transcription = await groq.audio.transcriptions.create(
        {
          file: createReadStream(tmpPath) as any,
          model: groqModel,
          ...(promptHint ? { prompt: promptHint } : {}),
          ...(isoLang ? { language: isoLang } : {}),
        },
        { signal: sttAbort },
      );
      raw = transcription.text?.trim() ?? '';
    } else {
      // ── OpenAI Whisper ──
      if (!opts.apiKey) throw new Error('Missing API key. Open Settings.');
      const openai = new OpenAI({ apiKey: opts.apiKey });
      log(`calling OpenAI STT (${opts.sttModel})...`);
      const isoLang = toISO639(opts.inputLang);
      const promptHint = buildPrompt(opts);
      const sttAbort = AbortSignal.timeout(STT_TIMEOUT);
      const transcription = await openai.audio.transcriptions.create(
        {
          file: createReadStream(tmpPath) as any,
          model: opts.sttModel,
          ...(promptHint ? { prompt: promptHint } : {}),
          ...(isoLang ? { language: isoLang } : {}),
        },
        { signal: sttAbort },
      );
      raw = transcription.text?.trim() ?? '';
    }

    log(`STT result`, { text: raw });
    if (!raw) return '';

    // ── Skip GPT? Return raw transcription ──
    if (opts.skipGpt) {
      log('skipping GPT (fast mode)');
      return raw;
    }

    // ── GPT formatting ──
    const gptClient = buildGptClient(opts);
    if (!gptClient) return raw;  // no usable provider/key configured
    const { client, model, providerLabel } = gptClient;
    const systemPrompt = `${opts.styleSystemPrompt}\n\n${languageInstruction(opts.inputLang, opts.outputLang)}`;

    log(`calling GPT via ${providerLabel} (${model})...`);
    const gptAbort = AbortSignal.timeout(GPT_TIMEOUT);
    const completion = await client.chat.completions.create(
      {
        model,
        messages: [
          { role: 'system', content: systemPrompt },
          { role: 'user', content: raw },
        ],
        temperature: 0.2,
      },
      { signal: gptAbort },
    );
    const out = completion.choices[0]?.message?.content?.trim() ?? raw;
    log(`GPT result`, { text: out });
    return out;
  } finally {
    try { unlinkSync(tmpPath); } catch {}
  }
}
