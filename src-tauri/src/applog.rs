// App log: errors, panics and pipeline diagnostics in one file the user can
// open from the tray ("Open Log").
//
// A release build has no console, so eprintln! output was simply lost, and a
// failed dictation or a crash left no trace. Everything that used eprintln!
// now goes through `applog!`, which still prints in dev and also appends a
// timestamped line to %APPDATA%/com.bizvoice.app/bizvoice.log. The file is
// trimmed to its last ~200 KB when it passes 1 MB.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const APP_DIR: &str = "com.bizvoice.app";
const FILE: &str = "bizvoice.log";
const MAX_BYTES: u64 = 1_000_000;
const KEEP_BYTES: usize = 200_000;

static LOCK: Mutex<()> = Mutex::new(());

/// Resolved without a Tauri handle so the panic hook can use it before (and
/// after) the app exists. Matches app_data_dir() for this identifier.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join(APP_DIR).join(FILE))
}

pub fn write(line: &str) {
    eprintln!("{line}");
    let Some(p) = path() else { return };
    let _g = LOCK.lock();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&p).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        if let Ok(all) = std::fs::read(&p) {
            let tail = &all[all.len().saturating_sub(KEEP_BYTES)..];
            let _ = std::fs::write(&p, tail);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} {line}", timestamp());
    }
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // UTC, YYYY-MM-DD HH:MM:SS, without a date crate.
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mo <= 2 { 1 } else { 0 };
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}Z")
}

/// Log panics (with location) before the default handler runs. Release
/// builds unwind instead of aborting, so a panic inside a command or a
/// spawned task fails that one operation instead of closing the app.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        write(&format!("[panic] {msg} at {loc}"));
        default(info);
    }));
}

#[macro_export]
macro_rules! applog {
    ($($arg:tt)*) => { $crate::applog::write(&format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_format() {
        let t = super::timestamp();
        assert_eq!(t.len(), 20);
        assert!(t.starts_with("20"));
        assert!(t.ends_with('Z'));
    }
}
