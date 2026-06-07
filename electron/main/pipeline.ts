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

export async function runPipeline(opts: {
  audioBase64: string;
  apiKey: string;
  groqKey?: string;
  openrouterKey?: string;
  customKey?: string;
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
    } else if (opts.sttProvider === 'groq' && opts.groqKey) {
      // ── Groq Whisper (10-20x faster) ──
      log(`calling Groq STT (whisper-large-v3-turbo)...`);
      const groq = new OpenAI({
        apiKey: opts.groqKey,
        baseURL: 'https://api.groq.com/openai/v1',
      });
      const isoLang = toISO639(opts.inputLang);
      const sttAbort = AbortSignal.timeout(STT_TIMEOUT);
      const transcription = await groq.audio.transcriptions.create(
        {
          file: createReadStream(tmpPath) as any,
          model: 'whisper-large-v3-turbo',
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
      const vocabPrompt = opts.vocabulary?.trim()
        ? `Domain vocabulary: ${opts.vocabulary.trim()}`
        : undefined;
      const isoLang = toISO639(opts.inputLang);
      const sttAbort = AbortSignal.timeout(STT_TIMEOUT);
      const transcription = await openai.audio.transcriptions.create(
        {
          file: createReadStream(tmpPath) as any,
          model: opts.sttModel,
          ...(vocabPrompt ? { prompt: vocabPrompt } : {}),
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
