// Phase 4: Windows text-injection — ported from electron/main/index.ts.
//
// A long-lived PowerShell worker hosts a `Typer` C# class (SendInput + foreground
// detection). Each paste writes a single `SMART:<base64>` line to its stdin; the
// worker picks the injection strategy per foreground app:
//   • code editors (Cursor/VS Code/Windsurf/Trae) + native terminals → Shift+Insert
//   • browsers / Chromium-Gecko hosts (web-hosted TUIs)               → char-type
//   • everything else                                                 → Ctrl+V
// Keeping a resident process skips PowerShell+Add-Type startup on every paste.
//
// The PS/C# body is byte-for-byte the Electron `TYPE_PS_SCRIPT` (pure Win32) so
// behaviour — including per-editor timing — is identical.

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The resident PowerShell typing helper. `child` is kept so we can kill it on
/// quit; `stdin` is the pipe we write `SMART:` lines to.
#[derive(Default)]
pub struct PasteWorker {
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    /// One line per finished command ("DONE"), read from the worker's stdout.
    /// Lets the paste wait until the keystroke really went out before it
    /// restores the clipboard. Without it, the first paste after start (while
    /// PowerShell is still compiling the C# helper, ~1-2s) raced the 2s
    /// restore and pasted the user's OLD clipboard instead of the dictation.
    acks: Mutex<Option<std::sync::mpsc::Receiver<String>>>,
}

const TYPE_PS_SCRIPT: &str = r#"$ErrorActionPreference = 'Continue'
$OutputEncoding = [Text.UTF8Encoding]::new()
[Console]::InputEncoding = [Text.UTF8Encoding]::new()
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class Typer {
    [StructLayout(LayoutKind.Sequential)]
    public struct INPUT { public int type; public InputUnion U; }

    [StructLayout(LayoutKind.Explicit)]
    public struct InputUnion {
        [FieldOffset(0)] public MOUSEINPUT mi;
        [FieldOffset(0)] public KEYBDINPUT ki;
        [FieldOffset(0)] public HARDWAREINPUT hi;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }

    [StructLayout(LayoutKind.Sequential)]
    public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }

    [StructLayout(LayoutKind.Sequential)]
    public struct HARDWAREINPUT { public uint uMsg; public ushort wParamL; public ushort wParamH; }

    [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint n, INPUT[] inputs, int cbSize);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vKey);
    [DllImport("user32.dll", CharSet = CharSet.Auto)] public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr OpenProcess(uint dwDesiredAccess, bool bInheritHandle, uint dwProcessId);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr hObject);
    [DllImport("psapi.dll", CharSet = CharSet.Auto)] public static extern int GetModuleBaseName(IntPtr hProcess, IntPtr hModule, StringBuilder lpBaseName, int nSize);

    public const int INPUT_KEYBOARD = 1;
    public const uint KEYEVENTF_KEYUP = 0x0002;
    public const uint KEYEVENTF_UNICODE = 0x0004;
    public const ushort VK_RETURN  = 0x0D;
    public const ushort VK_CONTROL = 0x11;
    public const ushort VK_SHIFT   = 0x10;
    public const ushort VK_V       = 0x56;
    public const ushort VK_INSERT  = 0x2D;
    public const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;

    public const int CHUNK_SIZE = 50;

    // The dictation hotkey (e.g. Ctrl+Shift+Space, Alt+...) is often still
    // physically held when the paste fires, which turns Ctrl+V into
    // Ctrl+Alt+V or Ctrl+Shift+V and the target app ignores it or does
    // something else. Wait briefly for the user to let go, then release any
    // modifier that is still down.
    static readonly ushort[] MODS = { 0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5 };
    public static void ReleaseModifiers() {
        for (int t = 0; t < 40; t++) {
            bool any = false;
            foreach (var vk in MODS) if ((GetAsyncKeyState(vk) & 0x8000) != 0) { any = true; break; }
            if (!any) return;
            Thread.Sleep(10);
        }
        var ups = new System.Collections.Generic.List<INPUT>();
        foreach (var vk in MODS) if ((GetAsyncKeyState(vk) & 0x8000) != 0) ups.Add(MakeVkInput(vk, true));
        if (ups.Count > 0) SendInput((uint)ups.Count, ups.ToArray(), Marshal.SizeOf(typeof(INPUT)));
    }

    public static void Paste() {
        ReleaseModifiers();
        var sb = new StringBuilder(256);
        GetClassName(GetForegroundWindow(), sb, sb.Capacity);
        string cls = sb.ToString();
        string proc = GetForegroundProcessName();
        bool useShiftInsert = IsTerminal(cls) || IsTerminalProc(proc) || IsCodeEditor(proc);

        INPUT[] inputs = new INPUT[4];
        if (useShiftInsert) {
            inputs[0] = MakeVkInput(VK_SHIFT,  false);
            inputs[1] = MakeVkInput(VK_INSERT, false);
            inputs[2] = MakeVkInput(VK_INSERT, true);
            inputs[3] = MakeVkInput(VK_SHIFT,  true);
        } else {
            inputs[0] = MakeVkInput(VK_CONTROL, false);
            inputs[1] = MakeVkInput(VK_V,       false);
            inputs[2] = MakeVkInput(VK_V,       true);
            inputs[3] = MakeVkInput(VK_CONTROL, true);
        }
        SendInput(4, inputs, Marshal.SizeOf(typeof(INPUT)));
    }

    public static bool IsCodeEditor(string proc) {
        if (string.IsNullOrEmpty(proc)) return false;
        proc = proc.ToLowerInvariant();
        if (proc == "code.exe" || proc == "cursor.exe" || proc == "windsurf.exe" || proc == "trae.exe") return true;
        if (proc.StartsWith("code-")) return true;
        return false;
    }

    // Terminals whose window class does not say "Terminal"/"Console". All of
    // them paste on Shift+Insert; several ignore a plain Ctrl+V.
    public static bool IsTerminalProc(string proc) {
        if (string.IsNullOrEmpty(proc)) return false;
        proc = proc.ToLowerInvariant();
        return proc == "windowsterminal.exe"
            || proc == "wezterm-gui.exe"
            || proc == "alacritty.exe"
            || proc == "mintty.exe"
            || proc == "conemu64.exe"
            || proc == "conemu.exe"
            || proc == "openconsole.exe";
    }

    public static bool IsBrowser(string proc) {
        if (string.IsNullOrEmpty(proc)) return false;
        proc = proc.ToLowerInvariant();
        return proc == "chrome.exe"
            || proc == "msedge.exe"
            || proc == "firefox.exe"
            || proc == "brave.exe"
            || proc == "opera.exe"
            || proc == "vivaldi.exe"
            || proc == "arc.exe"
            || proc == "zen.exe"
            || proc == "librewolf.exe";
    }

    public static bool IsTerminal(string cls) {
        if (string.IsNullOrEmpty(cls)) return false;
        if (cls == "CASCADIA_HOSTING_WINDOW_CLASS") return true;
        if (cls == "ConsoleWindowClass") return true;
        if (cls == "mintty") return true;
        if (cls.IndexOf("Console",  StringComparison.OrdinalIgnoreCase) >= 0) return true;
        if (cls.IndexOf("Terminal", StringComparison.OrdinalIgnoreCase) >= 0) return true;
        return false;
    }

    public static string GetForegroundClassName() {
        var sb = new StringBuilder(256);
        GetClassName(GetForegroundWindow(), sb, sb.Capacity);
        return sb.ToString();
    }

    public static bool IsChromiumOrGeckoHost(string cls) {
        if (string.IsNullOrEmpty(cls)) return false;
        return cls == "Chrome_WidgetWin_1"
            || cls == "MozillaWindowClass";
    }

    public static string GetForegroundProcessName() {
        uint pid;
        GetWindowThreadProcessId(GetForegroundWindow(), out pid);
        IntPtr h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
        if (h == IntPtr.Zero) return "";
        try {
            var sb = new StringBuilder(260);
            GetModuleBaseName(h, IntPtr.Zero, sb, sb.Capacity);
            return sb.ToString();
        } finally {
            CloseHandle(h);
        }
    }

    static INPUT MakeVkInput(ushort vk, bool keyUp) {
        INPUT inp = new INPUT();
        inp.type = INPUT_KEYBOARD;
        inp.U.ki.wVk = vk;
        if (keyUp) inp.U.ki.dwFlags = KEYEVENTF_KEYUP;
        return inp;
    }

    public static void Type(string text) {
        ReleaseModifiers();
        var list = new System.Collections.Generic.List<INPUT>();
        foreach (char c in text) {
            if (c == '\r') { continue; }
            if (c == '\n') {
                INPUT d = new INPUT(); d.type = INPUT_KEYBOARD; d.U.ki.wVk = VK_RETURN; list.Add(d);
                INPUT u = new INPUT(); u.type = INPUT_KEYBOARD; u.U.ki.wVk = VK_RETURN; u.U.ki.dwFlags = KEYEVENTF_KEYUP; list.Add(u);
            } else {
                INPUT d = new INPUT(); d.type = INPUT_KEYBOARD; d.U.ki.wScan = (ushort)c; d.U.ki.dwFlags = KEYEVENTF_UNICODE; list.Add(d);
                INPUT u = new INPUT(); u.type = INPUT_KEYBOARD; u.U.ki.wScan = (ushort)c; u.U.ki.dwFlags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP; list.Add(u);
            }
        }
        int total = list.Count;
        if (total == 0) return;
        int size = Marshal.SizeOf(typeof(INPUT));
        for (int i = 0; i < total; i += CHUNK_SIZE) {
            int count = Math.Min(CHUNK_SIZE, total - i);
            INPUT[] chunk = list.GetRange(i, count).ToArray();
            SendInput((uint)count, chunk, size);
            if (i + CHUNK_SIZE < total) Thread.Sleep(1);
        }
    }
}
"@

while (($line = [Console]::In.ReadLine()) -ne $null) {
    try {
        if ([string]::IsNullOrEmpty($line)) { continue }
        if ($line -eq 'PASTE') {
            [Typer]::Paste()
        } elseif ($line.StartsWith('TYPE:')) {
            $b64 = $line.Substring(5)
            $bytes = [Convert]::FromBase64String($b64)
            $text = [Text.Encoding]::UTF8.GetString($bytes)
            [Typer]::Type($text)
        } elseif ($line.StartsWith('SMART:')) {
            $b64 = $line.Substring(6)
            $bytes = [Convert]::FromBase64String($b64)
            $text = [Text.Encoding]::UTF8.GetString($bytes)
            $proc = [Typer]::GetForegroundProcessName()
            $cls  = [Typer]::GetForegroundClassName()
            $strategy = ""
            # Browsers used to get char-by-char typing. That is slow for long
            # text and can scramble Bangla (each conjunct is several code
            # points typed as separate keys). Clipboard paste is atomic and is
            # what Wispr Flow and Handy do everywhere.
            if ([Typer]::IsCodeEditor($proc) -or [Typer]::IsTerminal($cls) -or [Typer]::IsTerminalProc($proc)) {
                $strategy = "shift-insert"
                [Typer]::Paste()
            } else {
                $strategy = "ctrl-v"
                [Typer]::Paste()
            }
            [Console]::Error.WriteLine("bizvoice-paste: proc=$proc cls=$cls strategy=$strategy")
        }
    } catch {
    } finally {
        [Console]::Out.WriteLine("DONE|$proc|$strategy")
        [Console]::Out.Flush()
    }
}
"#;

/// Spawn the resident PowerShell typing worker if it isn't already running.
/// Mirrors `ensureTypeProc()` in index.ts — writes the script to a temp file and
/// spawns a hidden long-lived process with a piped stdin.
pub fn ensure_worker(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<PasteWorker>();
    let mut child_guard = state.child.lock().map_err(|_| "paste lock poisoned")?;

    let alive = match child_guard.as_mut() {
        Some(c) => matches!(c.try_wait(), Ok(None)),
        None => false,
    };
    if alive {
        return Ok(());
    }

    let script_path = std::env::temp_dir().join("bizvoice-type.ps1");
    std::fs::write(&script_path, TYPE_PS_SCRIPT).map_err(|e| format!("write type script: {e}"))?;

    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn().map_err(|e| format!("spawn type proc: {e}"))?;
    let stdin = child.stdin.take();
    if let Some(out) = child.stdout.take() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        *state.acks.lock().map_err(|_| "paste lock poisoned")? = Some(rx);
    }
    *state.stdin.lock().map_err(|_| "paste lock poisoned")? = stdin;
    *child_guard = Some(child);
    Ok(())
}

/// Write one command line to the worker. On a broken pipe (the worker died:
/// killed by AV, crashed, user ended it) restart it once and try again.
fn send_line(app: &tauri::AppHandle, line: &str) -> bool {
    for attempt in 0..2 {
        if ensure_worker(app).is_err() {
            return false;
        }
        let Some(state) = app.try_state::<PasteWorker>() else { return false };
        // Drop stale acks so the wait below sees this command's.
        if let Ok(g) = state.acks.lock() {
            if let Some(rx) = g.as_ref() {
                while rx.try_recv().is_ok() {}
            }
        }
        let ok = match state.stdin.lock() {
            Ok(mut g) => match g.as_mut() {
                Some(stdin) => writeln!(stdin, "{line}").and_then(|_| stdin.flush()).is_ok(),
                None => false,
            },
            Err(_) => false,
        };
        if ok {
            return true;
        }
        applog!("[paste] worker pipe broken (attempt {attempt}), restarting");
        kill_worker(app);
    }
    false
}

/// Wait until the worker reports the keystroke was sent; returns its report
/// ("DONE|<process>|<strategy>"). The first command
/// after a (re)start includes compiling the C# helper (measured 5.7s on
/// this machine), hence the long cap.
fn wait_ack(app: &tauri::AppHandle, timeout: Duration) -> Option<String> {
    let state = app.try_state::<PasteWorker>()?;
    let g = state.acks.lock().ok()?;
    g.as_ref()?.recv_timeout(timeout).ok()
}

/// Stage `text` on the clipboard, then drive the resident worker to inject it
/// into the foreground app using the per-app strategy, then put the user's
/// previous clipboard text back.
pub fn paste_text(app: &tauri::AppHandle, text: String) -> Result<(), String> {
    use base64::Engine;
    use tauri::Emitter;
    use tauri_plugin_clipboard_manager::ClipboardExt;

    if text.is_empty() {
        return Ok(());
    }
    // Err when the clipboard holds an image or files. Then there is nothing
    // we can restore as text, and writing "" back would just wipe it.
    let prev = app.clipboard().read_text().ok().filter(|p| !p.is_empty());
    app.clipboard()
        .write_text(text.clone())
        .map_err(|e| format!("clipboard write: {e}"))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let app2 = app.clone();
    std::thread::spawn(move || {
        // Small delay lets the clipboard settle before the shortcut fires.
        std::thread::sleep(Duration::from_millis(40));
        let t0 = std::time::Instant::now();
        let ack = if send_line(&app2, &format!("SMART:{b64}")) { wait_ack(&app2, Duration::from_secs(12)) } else { None };
        // "DONE|<process>|<strategy>"; process may be empty, so split on '|'.
        let mut parts = ack.as_deref().unwrap_or("").split('|').skip(1);
        crate::diag::append(serde_json::json!({
            "event": "paste",
            "ok": ack.is_some(),
            "target_process": parts.next().unwrap_or(""),
            "strategy": parts.next().unwrap_or(""),
            "ack_ms": t0.elapsed().as_millis() as u64,
            "chars": text.chars().count(),
        }));
        if ack.is_none() {
            // The text is still on the clipboard, so nothing is lost.
            applog!("[paste] keystroke not confirmed; leaving text on clipboard");
            let _ = app2.emit_to("micbar", "dictation:notice", "Couldn't paste. The text is on your clipboard: press Ctrl+V.");
            return;
        }
        // The target app reads the clipboard asynchronously after the
        // keystroke (Electron apps and terminals can take a few hundred ms).
        std::thread::sleep(Duration::from_millis(700));
        if let Some(prev) = prev {
            // Only restore if the clipboard still holds our text: if the user
            // copied something in the meantime, keep theirs.
            if app2.clipboard().read_text().ok().as_deref() == Some(text.as_str()) {
                let _ = app2.clipboard().write_text(prev);
            }
        }
    });
    Ok(())
}

/// Kill the resident worker (called on app exit — mirrors index.ts `before-quit`).
pub fn kill_worker(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<PasteWorker>() {
        if let Ok(mut guard) = state.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
            }
        }
        if let Ok(mut s) = state.stdin.lock() {
            *s = None;
        }
        if let Ok(mut a) = state.acks.lock() {
            *a = None;
        }
    }
}
