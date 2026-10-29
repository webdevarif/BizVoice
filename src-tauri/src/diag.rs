// Diagnostics: one JSON line per event in %APPDATA%/com.bizvoice.app/diagnostics.jsonl.
//
// Written for an AI agent (or a human) to read later: every dictation records
// what ran (provider, key label, timings, fallbacks), how it ended (ok, empty,
// error + error kind), quality signals (low-confidence words, live/batch
// disagreement, wrong script) and, if the user allows it, the text. Paste
// attempts and user corrections are their own events. The schema is described
// in README.md inside the "Export Diagnostics" zip (see `export`).
//
// Collection uses one global "current dictation" record. Dictations run one at
// a time (the mic bar queues them), so there is no cross-talk in practice; a
// stray note with no open record is simply dropped.

use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

const APP_DIR: &str = "com.bizvoice.app";
const FILE: &str = "diagnostics.jsonl";
const CORRECTIONS: &str = "corrections.json";
const MAX_BYTES: u64 = 2_000_000;
const KEEP_LINES: usize = 3_000;
/// Scribe word logprob below this (probability < ~0.37) is "low confidence".
pub const LOW_CONF_LOGPROB: f64 = -1.0;

struct Current {
    started: Instant,
    rec: Map<String, Value>,
}

static CUR: Mutex<Option<Current>> = Mutex::new(None);
/// Live text + socket failure handed from live_stop to the batch transcribe it
/// triggers, so that one record can compare the two.
static LIVE_STASH: Mutex<Option<(String, Option<String>)>> = Mutex::new(None);
static FILE_LOCK: Mutex<()> = Mutex::new(());

pub fn dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join(APP_DIR))
}

pub fn path() -> Option<PathBuf> {
    dir().map(|d| d.join(FILE))
}

fn now_iso() -> String {
    // Reuse the log timestamp format.
    crate::applog::timestamp()
}

pub fn append(mut v: Value) {
    if let Some(o) = v.as_object_mut() {
        o.entry("ts").or_insert_with(|| Value::String(now_iso()));
        o.entry("version")
            .or_insert_with(|| Value::String(env!("CARGO_PKG_VERSION").to_string()));
    }
    let Some(p) = path() else { return };
    let _g = FILE_LOCK.lock();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if std::fs::metadata(&p).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        if let Ok(all) = std::fs::read_to_string(&p) {
            let lines: Vec<&str> = all.lines().collect();
            let keep = &lines[lines.len().saturating_sub(KEEP_LINES)..];
            let _ = std::fs::write(&p, keep.join("\n") + "\n");
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{}", v);
    }
}

/// Start the record for one dictation.
pub fn begin(mode: &str, audio_ms: u64, lang: &str, provider: &str) {
    let mut rec = Map::new();
    rec.insert("event".into(), json!("dictation"));
    rec.insert("mode".into(), json!(mode));
    rec.insert("audio_ms".into(), json!(audio_ms));
    rec.insert("lang".into(), json!(lang));
    rec.insert("provider".into(), json!(provider));
    if let Ok(mut s) = LIVE_STASH.lock() {
        if let Some((live, fail)) = s.take() {
            rec.insert("mode".into(), json!("live"));
            rec.insert("live_text_words".into(), json!(live.split_whitespace().count()));
            rec.insert("_live_text".into(), json!(live));
            if let Some(f) = fail {
                rec.insert("live_failure".into(), json!(f));
            }
        }
    }
    if let Ok(mut c) = CUR.lock() {
        *c = Some(Current { started: Instant::now(), rec });
    }
}

pub fn stash_live(text: &str, failure: Option<String>) {
    if let Ok(mut s) = LIVE_STASH.lock() {
        *s = Some((text.to_string(), failure));
    }
}

/// Set a field on the current record.
pub fn note(key: &str, v: Value) {
    if let Ok(mut c) = CUR.lock() {
        if let Some(cur) = c.as_mut() {
            cur.rec.insert(key.to_string(), v);
        }
    }
}

/// Append to an array field on the current record.
pub fn push(key: &str, v: Value) {
    if let Ok(mut c) = CUR.lock() {
        if let Some(cur) = c.as_mut() {
            let e = cur.rec.entry(key.to_string()).or_insert_with(|| json!([]));
            if let Some(a) = e.as_array_mut() {
                if a.len() < 40 {
                    a.push(v);
                }
            }
        }
    }
}

/// Close the current record and write it.
pub fn finish(result: &Result<String, String>, include_text: bool) {
    let Some(mut cur) = CUR.lock().ok().and_then(|mut c| c.take()) else { return };
    let r = &mut cur.rec;
    r.insert("total_ms".into(), json!(cur.started.elapsed().as_millis() as u64));
    let live = r.remove("_live_text").and_then(|v| v.as_str().map(String::from));
    match result {
        Ok(t) if t.trim().is_empty() => {
            r.insert("status".into(), json!("empty"));
        }
        Ok(t) => {
            r.insert("status".into(), json!("ok"));
            r.insert("words".into(), json!(t.split_whitespace().count()));
            r.insert("chars".into(), json!(t.chars().count()));
            if let Some(l) = &live {
                let sim = similarity(l, t);
                r.insert("live_vs_final_similarity".into(), json!((sim * 100.0).round() / 100.0));
                if sim < 0.6 && !l.trim().is_empty() {
                    r.insert("flag_live_mismatch".into(), json!(true));
                }
            }
            let script = crate::pipeline::Script::of(r.get("lang").and_then(|v| v.as_str()).unwrap_or(""));
            if !script.accepts(t) {
                r.insert("flag_wrong_script".into(), json!(true));
            }
            if include_text {
                r.insert("text".into(), json!(t));
                if let Some(l) = live {
                    r.insert("live_text".into(), json!(l));
                }
            }
        }
        Err(e) => {
            r.insert("status".into(), json!("error"));
            r.insert("error".into(), json!(e.chars().take(300).collect::<String>()));
            r.insert("error_kind".into(), json!(error_kind(e)));
        }
    }
    append(Value::Object(cur.rec));
}

fn error_kind(e: &str) -> &'static str {
    let l = e.to_lowercase();
    if l.contains("unusual activity") {
        "flagged_free_tier"
    } else if l.contains("credits") || l.contains("quota") {
        "quota"
    } else if l.contains("rejected every key") || l.contains("401") || l.contains("invalid") && l.contains("key") {
        "auth"
    } else if l.contains("rate limit") || l.contains("429") {
        "rate_limited"
    } else if l.contains("timed out") || l.contains("could not reach") || l.contains("request failed") || l.contains("connect") {
        "network"
    } else if l.contains("no ") && l.contains("key") {
        "no_key"
    } else {
        "other"
    }
}

pub fn recent_lines(n: usize) -> Vec<String> {
    let Some(p) = path() else { return vec![] };
    let all = std::fs::read_to_string(p).unwrap_or_default();
    let lines: Vec<String> = all.lines().map(String::from).collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

pub const REVIEW_PROMPT: &str = "You are reviewing diagnostics from BizVoice, a Windows voice-typing app (Bangla/English, ElevenLabs Scribe with Groq fallback). Each line is one JSON event; the schema: event=dictation (status ok|empty|error, error_kind, attempts[] per key, fallback, stt_ms, total_ms, low_confidence[] words with probability, flag_live_mismatch, flag_wrong_script, live_vs_final_similarity), event=paste (ok, target_process, strategy, ack_ms), event=correction (substitutions the user made), event=ui_error, event=panic. Write a short plain report: 1) problems that repeat, with counts and the likely cause, 2) words that are often mis-heard and should go into the vocabulary, 3) one concrete fix per problem. No preamble.";

const README: &str = include_str!("diag_readme.md");

/// Build BizVoice-diagnostics-<ts>.zip in the app data dir: diagnostics.jsonl,
/// bizvoice.log, corrections.json, settings with every secret removed, and a
/// README describing the format. Uses PowerShell's Compress-Archive so the app
/// needs no zip dependency.
pub fn export(settings: &Value) -> Result<PathBuf, String> {
    let d = dir().ok_or("no APPDATA")?;
    let stamp = now_iso().replace([':', ' '], "-");
    let stage = d.join(format!("diag-export-{stamp}"));
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    for f in [FILE, "bizvoice.log", CORRECTIONS] {
        let src = d.join(f);
        if src.exists() {
            let _ = std::fs::copy(&src, stage.join(f));
        }
    }
    let _ = std::fs::write(stage.join("settings.redacted.json"), serde_json::to_string_pretty(&redact(settings)).unwrap_or_default());
    let summary = summarize(&recent_lines(KEEP_LINES));
    let readme = README.replace("{{SUMMARY}}", &summary).replace("{{VERSION}}", env!("CARGO_PKG_VERSION"));
    let _ = std::fs::write(stage.join("README.md"), readme);
    let zip = d.join(format!("BizVoice-diagnostics-{stamp}.zip"));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let st = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!("Compress-Archive -Path '{}\\*' -DestinationPath '{}' -Force", stage.display(), zip.display()))
            .creation_flags(0x0800_0000)
            .status()
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&stage);
        if !st.success() || !zip.exists() {
            return Err("Compress-Archive failed".into());
        }
    }
    Ok(zip)
}

/// Drop every key-ish field; keep the rest so an agent sees the configuration.
pub fn redact(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| {
            let l = k.to_lowercase();
            if l.contains("key") || l.contains("token") || l.contains("secret") || l.contains("header") || l.contains("email") {
                let present = !(x.is_null() || x.as_str() == Some("") || x.as_array().map(|a| a.is_empty()).unwrap_or(false));
                let n = x.as_array().map(|a| a.len());
                (k.clone(), json!(match n { Some(n) => format!("<redacted: {n} item(s)>"), None if present => "<redacted>".to_string(), None => "<empty>".to_string() }))
            } else {
                (k.clone(), redact(x))
            }
        }).collect()),
        Value::Array(a) => Value::Array(a.iter().map(redact).collect()),
        x => x.clone(),
    }
}

/// Counts that let a reader see the picture before opening the JSONL.
pub fn summarize(lines: &[String]) -> String {
    use std::collections::BTreeMap;
    let mut by_status: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_kind: BTreeMap<String, u64> = BTreeMap::new();
    let mut paste: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let (mut n, mut fallbacks, mut mismatch, mut wrong, mut panics, mut ui) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut ms: Vec<u64> = vec![];
    for l in lines {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        match s("event").as_str() {
            "dictation" => {
                n += 1;
                *by_status.entry(s("status")).or_default() += 1;
                if s("status") == "error" { *by_kind.entry(s("error_kind")).or_default() += 1; }
                if v.get("fallback").is_some() { fallbacks += 1; }
                if v.get("flag_live_mismatch").is_some() { mismatch += 1; }
                if v.get("flag_wrong_script").is_some() { wrong += 1; }
                if let Some(t) = v.get("total_ms").and_then(|x| x.as_u64()) { ms.push(t); }
            }
            "paste" => {
                let e = paste.entry(format!("{} ({})", s("target_process"), s("strategy"))).or_default();
                if v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) { e.0 += 1 } else { e.1 += 1 }
            }
            "panic" => panics += 1,
            "ui_error" => ui += 1,
            _ => {}
        }
    }
    ms.sort();
    let med = ms.get(ms.len() / 2).copied().unwrap_or(0);
    let mut o = format!("- Dictations: {n}, median processing {med} ms\n- By status: {by_status:?}\n- Errors by kind: {by_kind:?}\n- Groq fallbacks: {fallbacks}\n- Live/final mismatch flags: {mismatch}\n- Wrong-script flags: {wrong}\n- Panics: {panics}, UI errors: {ui}\n- Paste by app (ok, failed):\n");
    for (k, (a, b)) in paste { o += &format!("  - {k}: {a}, {b}\n"); }
    o
}

// ── text comparison ─────────────────────────────────────────────────────────

fn norm_words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Word-level LCS ratio in [0, 1].
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (norm_words(a), norm_words(b));
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let lcs = lcs_table(&a, &b);
    2.0 * lcs[a.len()][b.len()] as f64 / (a.len() + b.len()) as f64
}

fn lcs_table(a: &[String], b: &[String]) -> Vec<Vec<usize>> {
    let mut t = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            t[i][j] = if a[i - 1] == b[j - 1] {
                t[i - 1][j - 1] + 1
            } else {
                t[i - 1][j].max(t[i][j - 1])
            };
        }
    }
    t
}

/// Replaced spans between the original and the user's corrected text, as
/// (from, to) phrase pairs of at most 3 words each side. Pure insertions and
/// deletions are ignored: they are edits, not mis-heard words.
pub fn substitutions(original: &str, corrected: &str) -> Vec<(String, String)> {
    let raw = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-').to_string())
            .filter(|w| !w.is_empty())
            .collect()
    };
    let (a, b) = (raw(original), raw(corrected));
    let (la, lb): (Vec<String>, Vec<String>) = (
        a.iter().map(|w| w.to_lowercase()).collect(),
        b.iter().map(|w| w.to_lowercase()).collect(),
    );
    let t = lcs_table(&la, &lb);
    // Walk back, collecting aligned (a-run, b-run) gaps between matches.
    let (mut i, mut j) = (a.len(), b.len());
    let mut gaps: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    let (mut ga, mut gb): (Vec<String>, Vec<String>) = (vec![], vec![]);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && la[i - 1] == lb[j - 1] {
            if !ga.is_empty() || !gb.is_empty() {
                gaps.push((std::mem::take(&mut ga), std::mem::take(&mut gb)));
            }
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || t[i][j - 1] >= t[i - 1][j]) {
            gb.insert(0, b[j - 1].clone());
            j -= 1;
        } else {
            ga.insert(0, a[i - 1].clone());
            i -= 1;
        }
    }
    if !ga.is_empty() || !gb.is_empty() {
        gaps.push((ga, gb));
    }
    gaps.into_iter()
        .filter(|(x, y)| !x.is_empty() && !y.is_empty() && x.len() <= 3 && y.len() <= 3)
        .map(|(x, y)| (x.join(" "), y.join(" ")))
        .rev()
        .collect()
}

// ── corrections store ──────────────────────────────────────────────────────

fn corrections_path() -> Option<PathBuf> {
    dir().map(|d| d.join(CORRECTIONS))
}

pub fn load_corrections() -> Map<String, Value> {
    corrections_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub fn save_corrections(m: &Map<String, Value>) {
    if let Some(p) = corrections_path() {
        let _ = std::fs::write(p, serde_json::to_string_pretty(&Value::Object(m.clone())).unwrap_or_default());
    }
}

/// Record the user's fix of a dictation. Returns the substitutions found.
pub fn record_correction(original: &str, corrected: &str) -> Vec<(String, String)> {
    let subs = substitutions(original, corrected);
    let mut m = load_corrections();
    let pairs = m.entry("pairs").or_insert_with(|| json!({}));
    if let Some(p) = pairs.as_object_mut() {
        for (f, t) in &subs {
            let k = format!("{f}\u{2192}{t}");
            let n = p.get(&k).and_then(|v| v.as_u64()).unwrap_or(0) + 1;
            p.insert(k, json!(n));
        }
    }
    save_corrections(&m);
    append(json!({
        "event": "correction",
        "substitutions": subs.iter().map(|(f, t)| json!({"from": f, "to": t})).collect::<Vec<_>>(),
        "similarity": (similarity(original, corrected) * 100.0).round() / 100.0,
    }));
    subs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_bounds() {
        assert_eq!(similarity("a b c", "a b c"), 1.0);
        assert_eq!(similarity("", "a"), 0.0);
        assert!(similarity("আমি GitHub এ build করি", "আমি বাড়িতে থাকি") < 0.6);
    }

    #[test]
    fn substitutions_find_misheard_words() {
        let s = substitutions(
            "আমরা Surfacely dashboard এ কাজ করছি",
            "আমরা Surfacly dashboard এ কাজ করছি",
        );
        assert_eq!(s, vec![("Surfacely".to_string(), "Surfacly".to_string())]);
        // Pure insertion is not a substitution.
        assert!(substitutions("a b", "a x b").is_empty());
        let two = substitutions("one two three four", "one 2 three 4");
        assert_eq!(two.len(), 2);
    }

    #[test]
    fn redact_hides_secrets() {
        let s = json!({"elevenlabsKeys": [{"label":"a","keyEncrypted":"xyz"}], "groqKeyEncrypted": "abc",
                       "bizgrowhubEmail": "a@b.c", "inputLang": "bangla", "nested": {"customHeaders": "Bearer q"}});
        let r = redact(&s).to_string();
        for leak in ["xyz", "abc", "a@b.c", "Bearer q"] { assert!(!r.contains(leak), "{leak} leaked: {r}"); }
        assert!(r.contains("bangla"));
    }

    #[test]
    fn summary_counts() {
        let lines = vec![
            json!({"event":"dictation","status":"ok","total_ms":1000}).to_string(),
            json!({"event":"dictation","status":"error","error_kind":"quota","total_ms":3000}).to_string(),
            json!({"event":"paste","ok":true,"target_process":"code.exe","strategy":"shift-insert"}).to_string(),
            "not json".to_string(),
        ];
        let s = summarize(&lines);
        assert!(s.contains("Dictations: 2"));
        assert!(s.contains("\"quota\": 1"));
        assert!(s.contains("code.exe (shift-insert): 1, 0"));
    }

    #[test]
    fn error_kinds() {
        assert_eq!(error_kind("All ElevenLabs keys are out of credits."), "quota");
        assert_eq!(error_kind("ElevenLabs is rate limiting right now."), "rate_limited");
        assert_eq!(error_kind("Could not reach ElevenLabs: x"), "network");
    }
}
