// ElevenLabs key health: error classification and sticky rotation.
//
// Both the batch path (pipeline.rs) and the live socket (realtime.rs) walk the
// user's key list. Two things matter for speed and reliability:
//
//   1. Start from the key that worked last time. Starting from key #1 on every
//      dictation means a drained first account costs a failed round trip on
//      every single request.
//   2. Remember which keys are spent, and skip them for a while instead of
//      re-trying them each time. They are re-checked after a cool-down, because
//      free credits reset monthly and rate limits clear in seconds.
//
// State is in-memory only: a restart simply re-learns it on the first request.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Why a request with one key failed. Drives whether to try the next key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// Account is out of credits. Next key; skip this one for a long while.
    Quota,
    /// Wrong or revoked key. Next key; skip it until the user edits keys.
    Auth,
    /// ElevenLabs flagged the free tier on this IP ("unusual activity").
    /// Every free key on the same machine is usually hit at once.
    Flagged,
    /// Too many requests right now. Next key; this one is fine again soon.
    RateLimited,
    /// Network blip, timeout or 5xx. Next key (fresh connection).
    Transient,
    /// Bad request, bad audio, missing permission: the same on every key.
    Fatal,
}

impl Failure {
    pub fn try_next_key(self) -> bool {
        !matches!(self, Failure::Fatal)
    }

    fn cooldown(self) -> Option<Duration> {
        match self {
            Failure::Quota => Some(Duration::from_secs(6 * 60 * 60)),
            Failure::Auth => Some(Duration::from_secs(24 * 60 * 60)),
            Failure::Flagged => Some(Duration::from_secs(6 * 60 * 60)),
            Failure::RateLimited => Some(Duration::from_secs(20)),
            Failure::Transient | Failure::Fatal => None,
        }
    }
}

/// Classify an HTTP error from the batch endpoint. ElevenLabs reports quota and
/// bad keys as 400 *or* 401, so the body decides, not the status alone.
pub fn classify_http(status: u16, body: &str) -> Failure {
    let l = body.to_lowercase();
    if is_flagged(&l) {
        return Failure::Flagged;
    }
    if l.contains("quota_exceeded") || l.contains("insufficient") || l.contains("credits") {
        return Failure::Quota;
    }
    if l.contains("missing_permissions") {
        return Failure::Fatal;
    }
    match status {
        401 | 403 => Failure::Auth,
        429 => Failure::RateLimited,
        500..=599 => Failure::Transient,
        _ => Failure::Fatal,
    }
}

/// Classify a realtime `message_type` the server sends instead of transcripts.
/// Returns None for message types that are not errors.
pub fn classify_ws(message_type: &str, error: &str) -> Option<Failure> {
    let l = error.to_lowercase();
    let f = match message_type {
        "quota_exceeded" => Failure::Quota,
        "auth_error" => {
            if is_flagged(&l) {
                Failure::Flagged
            } else {
                Failure::Auth
            }
        }
        "rate_limited" | "commit_throttled" | "queue_overflow" | "resource_exhausted" => {
            Failure::RateLimited
        }
        "transcriber_error" | "error" | "session_time_limit_exceeded" => {
            if is_flagged(&l) {
                Failure::Flagged
            } else if l.contains("quota") || l.contains("credits") {
                Failure::Quota
            } else {
                Failure::Transient
            }
        }
        "unaccepted_terms" | "input_error" | "invalid_request" | "chunk_size_exceeded" => {
            Failure::Fatal
        }
        // Not an error: silence just produced nothing to transcribe.
        "insufficient_audio_activity" => return None,
        _ => return None,
    };
    Some(f)
}

/// Classify a WebSocket handshake error string (tungstenite formats the HTTP
/// status into it).
pub fn classify_handshake(err: &str) -> Failure {
    let l = err.to_lowercase();
    if is_flagged(&l) {
        Failure::Flagged
    } else if l.contains("quota") || l.contains("insufficient") {
        Failure::Quota
    } else if l.contains("401") || l.contains("403") {
        Failure::Auth
    } else if l.contains("429") {
        Failure::RateLimited
    } else if l.contains("url") {
        Failure::Fatal
    } else {
        Failure::Transient
    }
}

fn is_flagged(lower: &str) -> bool {
    lower.contains("unusual activity") || lower.contains("detected_unusual_activity")
}

/// User-facing explanation of the final failure after every key was tried.
pub fn explain(f: Failure, detail: &str) -> String {
    match f {
        Failure::Quota => "All ElevenLabs keys are out of credits.".into(),
        Failure::Auth => "ElevenLabs rejected every key. Check them in Settings.".into(),
        Failure::Flagged => "ElevenLabs disabled the free tier on this network (\"unusual activity\"). Free keys from the same IP are blocked together; use a paid key or another provider.".into(),
        Failure::RateLimited => "ElevenLabs is rate limiting right now. Try again in a few seconds.".into(),
        Failure::Transient => format!("Could not reach ElevenLabs: {detail}"),
        Failure::Fatal => detail.to_string(),
    }
}

#[derive(Default)]
struct Health {
    /// Label of the key that last succeeded.
    last_good: Option<String>,
    /// Label -> skip until this instant.
    resting: HashMap<String, Instant>,
}

fn health() -> &'static Mutex<Health> {
    static H: OnceLock<Mutex<Health>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(Health::default()))
}

/// Keys in the order to try them: last good first, then the rest in the
/// user's order, with resting keys moved to the end (still tried as a last
/// resort, so a wrong cool-down never makes dictation fail outright).
pub fn order<'a>(keys: &'a [(String, String)]) -> Vec<&'a (String, String)> {
    let h = health().lock().ok();
    let now = Instant::now();
    let (last_good, resting) = match &h {
        Some(h) => (h.last_good.clone(), h.resting.clone()),
        None => (None, HashMap::new()),
    };
    order_with(keys, last_good.as_deref(), &resting, now)
}

fn order_with<'a>(
    keys: &'a [(String, String)],
    last_good: Option<&str>,
    resting: &HashMap<String, Instant>,
    now: Instant,
) -> Vec<&'a (String, String)> {
    let is_resting = |label: &str| resting.get(label).map(|t| *t > now).unwrap_or(false);
    let start = last_good
        .and_then(|l| keys.iter().position(|(k, _)| k == l))
        .unwrap_or(0);
    let rotated = keys[start..].iter().chain(keys[..start].iter());
    let (mut fresh, tired): (Vec<_>, Vec<_>) = rotated.partition(|(l, _)| !is_resting(l));
    fresh.extend(tired);
    fresh
}

pub fn mark_good(label: &str) {
    if let Ok(mut h) = health().lock() {
        h.last_good = Some(label.to_string());
        h.resting.remove(label);
    }
}

pub fn mark_bad(label: &str, f: Failure) {
    if let Some(d) = f.cooldown() {
        if let Ok(mut h) = health().lock() {
            h.resting.insert(label.to_string(), Instant::now() + d);
            if h.last_good.as_deref() == Some(label) {
                h.last_good = None;
            }
        }
    }
}

/// Forget everything, e.g. after the user edits their key list.
pub fn reset() {
    if let Ok(mut h) = health().lock() {
        *h = Health::default();
    }
}

/// Of several failures, the one most worth telling the user about.
pub fn worst(a: Option<Failure>, b: Failure) -> Failure {
    let rank = |f: Failure| match f {
        Failure::Flagged => 6,
        Failure::Fatal => 5,
        Failure::Quota => 4,
        Failure::Auth => 3,
        Failure::RateLimited => 2,
        Failure::Transient => 1,
    };
    match a {
        Some(a) if rank(a) >= rank(b) => a,
        _ => b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(labels: &[&str]) -> Vec<(String, String)> {
        labels.iter().map(|l| (l.to_string(), format!("key-{l}"))).collect()
    }

    fn labels(v: Vec<&(String, String)>) -> Vec<String> {
        v.into_iter().map(|(l, _)| l.clone()).collect()
    }

    #[test]
    fn quota_body_wins_over_status() {
        assert_eq!(classify_http(401, r#"{"detail":{"status":"quota_exceeded"}}"#), Failure::Quota);
        assert_eq!(classify_http(400, "quota_exceeded"), Failure::Quota);
    }

    #[test]
    fn flagged_free_tier_is_detected() {
        let body = r#"{"detail":{"status":"detected_unusual_activity","message":"Unusual activity detected. Free Tier usage disabled."}}"#;
        assert_eq!(classify_http(401, body), Failure::Flagged);
        assert_eq!(classify_ws("auth_error", "Unusual activity detected"), Some(Failure::Flagged));
    }

    #[test]
    fn plain_401_is_auth_and_missing_scope_is_fatal() {
        assert_eq!(classify_http(401, "invalid_api_key"), Failure::Auth);
        assert_eq!(classify_http(401, "missing_permissions"), Failure::Fatal);
        assert_eq!(classify_http(429, ""), Failure::RateLimited);
        assert_eq!(classify_http(503, ""), Failure::Transient);
        assert_eq!(classify_http(422, "bad file"), Failure::Fatal);
    }

    #[test]
    fn ws_error_types() {
        assert_eq!(classify_ws("quota_exceeded", ""), Some(Failure::Quota));
        assert_eq!(classify_ws("rate_limited", ""), Some(Failure::RateLimited));
        assert_eq!(classify_ws("input_error", ""), Some(Failure::Fatal));
        assert_eq!(classify_ws("partial_transcript", ""), None);
        assert_eq!(classify_ws("insufficient_audio_activity", ""), None);
    }

    #[test]
    fn order_starts_at_last_good_and_demotes_resting() {
        let keys = k(&["a", "b", "c"]);
        let now = Instant::now();
        let mut resting = HashMap::new();
        assert_eq!(labels(order_with(&keys, None, &resting, now)), ["a", "b", "c"]);
        assert_eq!(labels(order_with(&keys, Some("b"), &resting, now)), ["b", "c", "a"]);
        resting.insert("b".to_string(), now + Duration::from_secs(60));
        assert_eq!(labels(order_with(&keys, Some("b"), &resting, now)), ["c", "a", "b"]);
        // Expired rest is ignored.
        resting.insert("b".to_string(), now - Duration::from_secs(1));
        assert_eq!(labels(order_with(&keys, Some("b"), &resting, now)), ["b", "c", "a"]);
    }

    #[test]
    fn unknown_last_good_falls_back_to_user_order() {
        let keys = k(&["a", "b"]);
        let resting = HashMap::new();
        assert_eq!(labels(order_with(&keys, Some("gone"), &resting, Instant::now())), ["a", "b"]);
    }
}
