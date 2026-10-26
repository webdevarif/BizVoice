// Live transcription over the ElevenLabs realtime STT WebSocket.
//
// The batch path in pipeline.rs sends one clip and waits. This one holds a
// socket open while the user talks: audio goes up as 16 kHz mono PCM16, and the
// server sends `partial_transcript` (a rewritable guess at the current phrase)
// and `committed_transcript` (a finished phrase) back.
//
// Mode 1 — live PREVIEW. Partials are shown in the mic bar and nothing else;
// the committed segments are joined and returned when the session stops, so the
// existing refine + single paste path downstream is untouched. Nothing is typed
// into the user's document until they stop talking, which is what keeps this
// change safe: there is no already-injected text to retract when a partial is
// revised or a key fails over.
//
// Protocol shapes below were verified against the live service, not just docs:
//   -> {"message_type":"input_audio_chunk","audio_base_64":"<b64>"}
//   <- {"message_type":"session_started",     ...}
//   <- {"message_type":"partial_transcript",  "text": "..."}
//   <- {"message_type":"committed_transcript","text": "..."}
//   <- {"message_type":"quota_exceeded" | "auth_error" | "rate_limited" | ...,
//       "error": "..."}   (errors arrive as messages, not as a failed handshake)
//
// Reliability: every PCM chunk is also kept in memory for the whole session.
// When the socket fails for any reason, recording carries on, and at stop the
// full audio goes through the batch path (next key, then the fallback
// provider). The live preview can die; the dictation cannot.

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const WS_BASE: &str = "wss://api.elevenlabs.io/v1/speech-to-text/realtime";

/// `scribe_v2_realtime_turbo` measured ~0.8s faster to first partial than
/// `scribe_v2_realtime` and produced identical committed text on the probe
/// clip. (`scribe_v2_realtime_lite` appears in the docs but the service rejects
/// it with invalid_request, so it is not offered.)
const MODEL: &str = "scribe_v2_realtime_turbo";

/// Seconds of silence that end a phrase. Shorter commits sooner — which matters
/// because the joined committed text is what finally gets pasted — but chops
/// mid-sentence pauses into separate segments.
const VAD_SILENCE_SECS: &str = "0.7";

/// Events pushed to the mic bar. Kept distinct from VoiceEngine's
/// `transcript:partial` / `transcript:final` so the two streaming paths can
/// coexist without one window reacting to the other's session.
const EV_PARTIAL: &str = "live:partial";
const EV_COMMITTED: &str = "live:committed";
const EV_ERROR: &str = "live:error";

/// Sample rate of the PCM the mic bar streams.
pub const SAMPLE_RATE: u32 = 16_000;

enum Out {
    Audio(Vec<u8>),
    /// Final chunk: ask the server to commit whatever it still holds.
    Commit,
}

pub struct LiveSession {
    audio_tx: mpsc::UnboundedSender<Out>,
    /// Every byte of PCM pushed this session, kept even after the socket dies
    /// so the batch path can transcribe the whole dictation.
    audio: Arc<Mutex<Vec<u8>>>,
    /// Committed segments in arrival order — this is what gets refined + pasted.
    committed: Arc<Mutex<Vec<String>>>,
    /// Set when the socket dies early, so `finish` can explain why rather than
    /// silently returning a short transcript.
    failure: Arc<Mutex<Option<String>>>,
    /// Resolves when the reader loop ends — i.e. the socket is closed and the
    /// server has sent everything it is going to send. `finish` waits on this
    /// rather than on the segment count, which cannot tell "the tail already
    /// arrived" apart from "the tail is still coming".
    reader: tauri::async_runtime::JoinHandle<()>,
    /// Set by `finish` before it closes anything, so the reader can tell an
    /// expected end-of-session from the socket dying under it.
    stopping: Arc<AtomicBool>,
}

/// What a stopped session produced.
pub struct LiveResult {
    pub text: String,
    /// The full session audio, 16 kHz mono PCM16.
    pub pcm: Vec<u8>,
    /// Present when the socket died mid-dictation. With text alongside it the
    /// transcript is truncated — everything spoken after the death is gone — so
    /// the caller pastes what survived *and* tells the user.
    pub failure: Option<String>,
}

/// Map the app's language label to the ISO-639-3 the realtime API wants.
/// Returning None lets the server auto-detect, which costs accuracy on short
/// Bangla clips, so a known language is always worth passing.
fn lang_code(lang: &str) -> Option<&'static str> {
    match lang.to_lowercase().as_str() {
        "bangla" | "bn" | "ben" | "banglish" => Some("ben"),
        "english" | "en" | "eng" => Some("eng"),
        "hindi" | "hi" | "hin" => Some("hin"),
        "urdu" | "ur" | "urd" => Some("urd"),
        "arabic" | "ar" | "ara" => Some("ara"),
        _ => None,
    }
}

fn socket_url(lang: &str, vocabulary: &str) -> String {
    let mut url = format!(
        "{WS_BASE}?model_id={MODEL}&audio_format=pcm_16000&commit_strategy=vad&vad_silence_threshold_secs={VAD_SILENCE_SECS}"
    );
    if let Some(code) = lang_code(lang) {
        url.push_str(&format!("&language_code={code}"));
        // Bangla dictation is habitually mixed with English technical words;
        // naming the secondary language keeps identification from flip-flopping.
        if code == "ben" {
            url.push_str("&secondary_languages=eng");
        }
    }
    // Realtime allows up to 50 keyterms of under 20 characters each.
    for t in crate::pipeline::vocab_terms(vocabulary, 20, 50) {
        url.push_str("&keyterms=");
        url.push_str(&urlencode(&t));
    }
    url
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Open a live session, trying keys in health order until one connects.
///
/// A key whose account is spent often still completes the handshake and only
/// then sends `quota_exceeded`. That case is handled by the reader: it marks
/// the key as resting (so the next dictation starts on a good key) and the
/// session's audio is recovered through the batch path at stop.
pub async fn start(
    app: AppHandle,
    keys: Vec<(String, String)>,
    input_lang: String,
    vocabulary: String,
) -> Result<LiveSession, String> {
    use crate::keys as kh;
    if keys.is_empty() {
        return Err("No ElevenLabs key configured".into());
    }

    let url = socket_url(&input_lang, &vocabulary);
    let mut worst: Option<kh::Failure> = None;
    let mut last = String::from("no key accepted the connection");

    for (label, key) in kh::order(&keys) {
        let req = build_request(&url, key)?;
        let connect = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio_tungstenite::connect_async(req),
        )
        .await;
        match connect {
            Ok(Ok((stream, _))) => return Ok(spawn_pump(app, stream, label.clone())),
            Ok(Err(e)) => {
                let msg = e.to_string();
                let kind = kh::classify_handshake(&msg);
                eprintln!("[live] key {label:?} failed to connect ({kind:?}): {msg}");
                kh::mark_bad(label, kind);
                worst = Some(kh::worst(worst, kind));
                last = format!("{label}: {msg}");
                if !kind.try_next_key() {
                    return Err(format!("Live transcription failed to start: {msg}"));
                }
            }
            Err(_) => {
                worst = Some(kh::worst(worst, kh::Failure::Transient));
                last = format!("{label}: connect timed out");
            }
        }
    }

    Err(match worst {
        Some(f) => kh::explain(f, &last),
        None => last,
    })
}

fn build_request(
    url: &str,
    key: &str,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request, String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    req.headers_mut().insert(
        "xi-api-key",
        key.parse().map_err(|_| "invalid API key header".to_string())?,
    );
    Ok(req)
}

type Sock =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn chunk_frame(pcm: &[u8], commit: bool) -> String {
    use base64::Engine;
    serde_json::json!({
        "message_type": "input_audio_chunk",
        "audio_base_64": base64::engine::general_purpose::STANDARD.encode(pcm),
        "commit": commit,
        "sample_rate": SAMPLE_RATE,
    })
    .to_string()
}

/// Split the socket and run both directions on the Tauri runtime: audio from an
/// unbounded channel goes out, transcripts come in and are emitted to the mic
/// bar. The channel is unbounded on purpose: the audio callback must never
/// block waiting on the network.
fn spawn_pump(app: AppHandle, stream: Sock, label: String) -> LiveSession {
    let (mut writer, mut reader) = stream.split();
    let (audio_tx, mut audio_rx) = mpsc::unbounded_channel::<Out>();

    let committed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // Outbound: PCM -> base64 -> JSON frame. A dead socket is not an error
    // here: the reader reports why, and the audio is still in the buffer.
    tauri::async_runtime::spawn(async move {
        let mut alive = true;
        while let Some(out) = audio_rx.recv().await {
            if !alive {
                continue; // drain so senders never see a closed channel
            }
            let frame = match out {
                Out::Audio(pcm) => chunk_frame(&pcm, false),
                Out::Commit => chunk_frame(&[], true),
            };
            if writer.send(Message::Text(frame.into())).await.is_err() {
                alive = false;
            }
        }
        let _ = writer.close().await;
    });

    // Inbound: transcripts -> window events + the committed buffer.
    let committed_in = committed.clone();
    let failure_in = failure.clone();
    let stopping = Arc::new(AtomicBool::new(false));
    let stopping_in = stopping.clone();
    let reader = tauri::async_runtime::spawn(async move {
        let fail = |why: String| {
            let first = failure_in
                .lock()
                .map(|mut slot| {
                    let first = slot.is_none();
                    if first {
                        *slot = Some(why.clone());
                    }
                    first
                })
                .unwrap_or(false);
            if first {
                let _ = app.emit_to("micbar", EV_ERROR, why);
            }
        };

        while let Some(msg) = reader.next().await {
            let text = match msg {
                Ok(Message::Text(t)) => t.to_string(),
                Ok(Message::Close(frame)) => {
                    // A close carrying a reason is the service rejecting us
                    // (bad model, spent quota); a bare close is just the end.
                    if let Some(f) = frame {
                        if !f.reason.is_empty() {
                            let why = f.reason.to_string();
                            if let Some(kind) = crate::keys::classify_ws("error", &why) {
                                crate::keys::mark_bad(&label, kind);
                            }
                            fail(why);
                        }
                    }
                    break;
                }
                Ok(_) => continue,
                Err(e) => {
                    fail(e.to_string());
                    break;
                }
            };

            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let kind = v.get("message_type").and_then(|x| x.as_str()).unwrap_or("");
            let body = v.get("text").and_then(|x| x.as_str()).unwrap_or("");

            match kind {
                "session_started" => crate::keys::mark_good(&label),
                "partial_transcript" => {
                    let _ = app.emit_to("micbar", EV_PARTIAL, body);
                }
                "committed_transcript" => {
                    if !body.trim().is_empty() {
                        if let Ok(mut c) = committed_in.lock() {
                            c.push(body.trim().to_string());
                        }
                    }
                    let _ = app.emit_to("micbar", EV_COMMITTED, body);
                }
                other => {
                    let err = v.get("error").and_then(|x| x.as_str()).unwrap_or("");
                    if let Some(f) = crate::keys::classify_ws(other, err) {
                        eprintln!("[live] key {label:?} server error {other} ({f:?}): {err}");
                        crate::keys::mark_bad(&label, f);
                        fail(if err.is_empty() { other.to_string() } else { format!("{other}: {err}") });
                    }
                }
            }
        }

        // Reaching here means the socket is closed. If nobody asked for that,
        // it dropped on its own, and the dictation will be recovered at stop.
        if !stopping_in.load(Ordering::SeqCst) {
            fail(String::from("connection closed"));
        }
    });

    LiveSession {
        audio_tx,
        audio: Arc::new(Mutex::new(Vec::new())),
        committed,
        failure,
        reader,
        stopping,
    }
}

impl LiveSession {
    /// Queue one chunk of 16 kHz mono PCM16. Never blocks. The chunk is kept
    /// in the session buffer whether or not the socket is still alive.
    pub fn push_audio(&self, pcm: Vec<u8>) {
        if let Ok(mut a) = self.audio.lock() {
            a.extend_from_slice(&pcm);
        }
        let _ = self.audio_tx.send(Out::Audio(pcm));
    }

    /// Stop sending and return the joined committed transcript plus the full
    /// session audio.
    ///
    /// Sends a final `commit: true` so the server finalises the phrase it is
    /// still holding, then closes. The reader ends when the server closes back
    /// (normally ~0.3s); 3s is only a backstop for a server that never does.
    pub async fn finish(self) -> LiveResult {
        let LiveSession {
            audio_tx,
            audio,
            committed,
            failure,
            reader,
            stopping,
        } = self;
        // Before anything closes, so the reader never mistakes our own shutdown
        // for the socket dying.
        stopping.store(true, Ordering::SeqCst);
        let _ = audio_tx.send(Out::Commit);
        drop(audio_tx);

        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), reader).await;

        let text = committed
            .lock()
            .map(|c| c.join(" "))
            .unwrap_or_default()
            .trim()
            .to_string();
        let failure = failure.lock().ok().and_then(|f| f.clone());
        let pcm = audio.lock().map(|mut a| std::mem::take(&mut *a)).unwrap_or_default();
        LiveResult { text, pcm, failure }
    }
}

/// Wrap 16 kHz mono PCM16 in a WAV header so the batch endpoints accept it.
pub fn pcm16_to_wav(pcm: &[u8]) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let byte_rate = SAMPLE_RATE * 2;
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    w.extend_from_slice(&byte_rate.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    w.extend_from_slice(pcm);
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_valid() {
        let pcm = vec![0u8; 32_000]; // 1s
        let w = pcm16_to_wav(&pcm);
        assert_eq!(w.len(), 44 + 32_000);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 36 + 32_000);
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 32_000);
    }

    #[test]
    fn frame_has_required_fields() {
        let f: Value = serde_json::from_str(&chunk_frame(&[1, 2], true)).unwrap();
        assert_eq!(f["message_type"], "input_audio_chunk");
        assert_eq!(f["commit"], true);
        assert_eq!(f["sample_rate"], 16_000);
        assert_eq!(f["audio_base_64"], "AQI=");
    }

    #[test]
    fn keyterms_are_encoded_into_url() {
        let u = socket_url("bangla", "Next.js, Surfacly, a very long term that exceeds twenty");
        assert!(u.contains("language_code=ben"));
        assert!(u.contains("&keyterms=Next.js"));
        assert!(u.contains("&keyterms=Surfacly"));
        assert!(!u.contains("exceeds"));
    }
}
