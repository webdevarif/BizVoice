// Phase 4: local whisper inference (whisper-rs / whisper.cpp).
//
// Ported from electron/main/localWhisper.ts transcribeLocal. whisper.cpp wants
// 16 kHz mono f32 PCM, but the recorder captures WAV at the mic's native rate
// (44.1/48 kHz) — so we decode, downmix to mono, and resample to 16 kHz here.
// The model context is cached (load-once / reload-on-switch) so we don't reload
// a up-to-~1.5 GB model on every utterance.
//
// Gated behind the `local-whisper` Cargo feature (see Cargo.toml for why). When
// the feature is off, `WhisperCache` is a zero-cost stub whose `transcribe_local`
// returns an error, so the rest of the app (model download/list/delete, cloud
// STT) compiles and ships unchanged.

#[cfg(not(feature = "local-whisper"))]
mod imp {
    /// Stub used when local whisper isn't built. Keeps the managed-state type and
    /// the `transcribe` local branch compiling without pulling in whisper-rs.
    #[derive(Default, Clone)]
    pub struct WhisperCache;

    impl WhisperCache {
        pub fn transcribe_local(
            &self,
            _model_path: &str,
            _wav_bytes: &[u8],
            _language: Option<&str>,
        ) -> Result<String, String> {
            Err("Local Whisper isn't built in this build (enable the `local-whisper` Cargo feature).".into())
        }
    }
}

#[cfg(feature = "local-whisper")]
mod imp {
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Loaded-model cache, shared via `Arc` so it can live in Tauri managed state.
#[derive(Default, Clone)]
pub struct WhisperCache(Arc<Mutex<Option<(String, WhisperContext)>>>);

/// Decode a WAV byte buffer to mono f32 PCM at the file's native sample rate.
fn decode_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    let mut reader =
        hound::WavReader::new(Cursor::new(bytes)).map_err(|e| format!("open wav: {e}"))?;
    let spec = reader.spec();
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap_or(0) as f32 / max)
                .collect()
        }
    };
    let mono: Vec<f32> = if spec.channels > 1 {
        raw.chunks(spec.channels as usize)
            .map(|c| c.iter().sum::<f32>() / c.len() as f32)
            .collect()
    } else {
        raw
    };
    Ok((mono, spec.sample_rate))
}

/// Linear-interpolation resample of mono f32 to 16 kHz. Lower quality than a sinc
/// resampler (rubato) but dependency-free and adequate for speech — fixes the
/// spike's silent mis-transcription of 44.1/48 kHz audio (see PORT_SPEC.md).
fn resample_16k(input: &[f32], in_rate: u32) -> Vec<f32> {
    if in_rate == 16_000 || input.is_empty() {
        return input.to_vec();
    }
    let ratio = 16_000f64 / in_rate as f64;
    let out_len = (input.len() as f64 * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let idx = src.floor() as usize;
        let frac = (src - idx as f64) as f32;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

impl WhisperCache {
    /// Transcribe a WAV byte buffer with the ggml model at `model_path`.
    /// `language` is an ISO code or "auto"/None for auto-detect.
    pub fn transcribe_local(
        &self,
        model_path: &str,
        wav_bytes: &[u8],
        language: Option<&str>,
    ) -> Result<String, String> {
        let (mono, rate) = decode_wav(wav_bytes)?;
        let samples = resample_16k(&mono, rate);
        if samples.is_empty() {
            return Ok(String::new());
        }

        let mut guard = self.0.lock().map_err(|_| "whisper cache poisoned")?;
        let need_load = !matches!(guard.as_ref(), Some((p, _)) if p == model_path);
        if need_load {
            let ctx =
                WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
                    .map_err(|e| format!("load model: {e}"))?;
            *guard = Some((model_path.to_string(), ctx));
        }
        let ctx = &guard.as_ref().unwrap().1;
        let mut state = ctx.create_state().map_err(|e| format!("create state: {e}"))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        if let Some(lang) = language {
            if lang != "auto" && !lang.is_empty() {
                params.set_language(Some(lang));
            }
        }
        params.set_print_progress(false);
        params.set_print_special(false);
        params.set_print_realtime(false);

        state
            .full(params, &samples)
            .map_err(|e| format!("transcribe: {e}"))?;
        let n = state.full_n_segments().map_err(|e| format!("segments: {e}"))?;
        let mut text = String::new();
        for i in 0..n {
            if let Ok(seg) = state.full_get_segment_text(i) {
                text.push_str(&seg);
            }
        }
        Ok(text.trim().to_string())
    }
}
}

pub use imp::WhisperCache;
