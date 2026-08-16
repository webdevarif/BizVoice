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

pub struct LiveSession {
    audio_tx: mpsc::UnboundedSender<Vec<u8>>,
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

fn socket_url(lang: &str) -> String {
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
    url
}

/// True when a handshake failure is worth retrying on a different key rather
/// than reporting. Mirrors the batch failover in pipeline.rs.
fn key_is_spent(err: &str) -> bool {
    let l = err.to_lowercase();
    l.contains("quota_exceeded")
        || l.contains("insufficient")
        || l.contains("401")
        || l.contains("403")
        || l.contains("429")
}

/// Open a live session, trying each key in turn until one connects.
///
/// Failover happens only at handshake. Mid-stream the protocol is terminal —
/// the server reports and closes — and replaying buffered audio onto a fresh
/// socket risks duplicating text, so a mid-stream death ends the session and
/// `finish` returns whatever was committed before it, plus the error.
pub async fn start(
    app: AppHandle,
    keys: Vec<(String, String)>,
    input_lang: String,
) -> Result<LiveSession, String> {
    if keys.is_empty() {
        return Err("No ElevenLabs key configured".into());
    }

    let url = socket_url(&input_lang);
    let mut last = String::from("no key accepted the connection");

    for (label, key) in &keys {
        let req = build_request(&url, key)?;

        match tokio_tungstenite::connect_async(req).await {
            Ok((stream, _)) => return Ok(spawn_pump(app, stream)),
            Err(e) => {
                let msg = e.to_string();
                #[cfg(debug_assertions)]
                eprintln!("[live] key {label:?} failed to connect: {msg}");
                if !key_is_spent(&msg) {
                    // A malformed URL or a network outage will fail identically
                    // on every key; only spend the round trips when it's the
                    // key that's at fault.
                    return Err(format!("Live transcription failed to start: {msg}"));
                }
                last = format!("{label}: {msg}");
            }
        }
    }

    Err(format!("All ElevenLabs keys failed. Last error — {last}"))
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

/// Split the socket and run both directions on the Tauri runtime: audio from an
/// unbounded channel goes out, transcripts come in and are emitted to the mic
/// bar. The channel is unbounded on purpose — the audio callback must never
/// block waiting on the network.
fn spawn_pump(app: AppHandle, stream: Sock) -> LiveSession {
    let (mut writer, mut reader) = stream.split();
    let (audio_tx, mut audio_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let committed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // Outbound: PCM -> base64 -> JSON frame.
    tauri::async_runtime::spawn(async move {
        use base64::Engine;
        while let Some(pcm) = audio_rx.recv().await {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&pcm);
            let frame = serde_json::json!({
                "message_type": "input_audio_chunk",
                "audio_base_64": b64,
            });
            if writer
                .send(Message::Text(frame.to_string().into()))
                .await
                .is_err()
            {
                break; // reader task reports the reason
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
        while let Some(msg) = reader.next().await {
            let text = match msg {
                Ok(Message::Text(t)) => t.to_string(),
                Ok(Message::Close(frame)) => {
                    // A close carrying a reason is the service rejecting us
                    // (bad model, spent quota); a bare close is just the end.
                    if let Some(f) = frame {
                        if !f.reason.is_empty() {
                            let why = f.reason.to_string();
                            if let Ok(mut slot) = failure_in.lock() {
                                *slot = Some(why.clone());
                            }
                            let _ = app.emit_to("micbar", EV_ERROR, why);
                        }
                    }
                    break;
                }
                Ok(_) => continue,
                Err(e) => {
                    let why = e.to_string();
                    if let Ok(mut slot) = failure_in.lock() {
                        *slot = Some(why.clone());
                    }
                    let _ = app.emit_to("micbar", EV_ERROR, why);
                    break;
                }
            };

            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let kind = v.get("message_type").and_then(|x| x.as_str()).unwrap_or("");
            let body = v.get("text").and_then(|x| x.as_str()).unwrap_or("");

            match kind {
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
                _ => {}
            }
        }

        // Reaching here means the socket is closed. If nobody asked for that,
        // it dropped on its own — and a bare close or a vanished connection
        // carries no reason for the arms above to record, so without this the
        // dictation would be truncated with nothing anywhere saying why.
        if !stopping_in.load(Ordering::SeqCst) {
            let recorded = failure_in.lock().ok().and_then(|mut slot| {
                if slot.is_some() {
                    return None; // already reported, with a better reason
                }
                let why = String::from("connection closed");
                *slot = Some(why.clone());
                Some(why)
            });
            if let Some(why) = recorded {
                let _ = app.emit_to("micbar", EV_ERROR, why);
            }
        }
    });

    LiveSession {
        audio_tx,
        committed,
        failure,
        reader,
        stopping,
    }
}

impl LiveSession {
    /// Queue one chunk of 16 kHz mono PCM16. Never blocks; a closed socket just
    /// drops the audio, which `finish` then reports.
    pub fn push_audio(&self, pcm: Vec<u8>) {
        let _ = self.audio_tx.send(pcm);
    }

    /// Stop sending and return the joined committed transcript.
    ///
    /// Waits for the tail: when the user releases the hotkey the last phrase —
    /// sometimes the last *two* — is still in flight, and dropping it would
    /// silently truncate their sentence.
    ///
    /// The wait ends when the socket closes, not when a segment count changes.
    /// Counting cannot distinguish "one tail commit landed, another is still
    /// coming" from "everything has arrived", and it cannot tell "the tail
    /// already landed before the user stopped" from "the tail is late" — so it
    /// both lost text and stalled a fixed 3s on the commonest stop pattern.
    ///
    /// Measured against the live service: after the client close the server
    /// flushes its pending commit at +253ms and closes at +276ms. So this
    /// normally costs ~0.3s, not the 3s the old loop spent.
    pub async fn finish(self) -> Result<LiveResult, String> {
        // Closing the audio channel ends the writer task, which closes the
        // socket and prompts the server to flush its final commits and close
        // back — which is what ends the reader.
        let LiveSession {
            audio_tx,
            committed,
            failure,
            reader,
            stopping,
        } = self;
        // Before anything closes, so the reader never mistakes our own shutdown
        // for the socket dying.
        stopping.store(true, Ordering::SeqCst);
        drop(audio_tx);

        // Backstop only: a server that neither flushes nor closes must not hang
        // the paste. A socket that already died resolves immediately.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(4), reader).await;

        let text = committed
            .lock()
            .map(|c| c.join(" "))
            .unwrap_or_default()
            .trim()
            .to_string();

        let failure = failure.lock().ok().and_then(|f| f.clone());

        // Nothing survived — the failure is the whole story, so report it.
        if text.is_empty() {
            if let Some(err) = failure {
                return Err(format!("Live transcription failed: {err}"));
            }
        }
        Ok(LiveResult { text, failure })
    }
}
