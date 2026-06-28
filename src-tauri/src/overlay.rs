// Phase 3: mic-bar overlay parity (Windows).
//
// Electron pinned the mic bar with `setAlwaysOnTop(true, 'screen-saver')` +
// `moveTop()` re-asserted every 1500 ms, because Windows demotes HWND_TOPMOST
// windows when other topmost windows (notifications, fullscreen apps, UAC)
// compete for the slot. Tauri has no `moveTop()`, so we re-assert via a raw
// `SetWindowPos(HWND_TOPMOST, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)` and mark the
// window `WS_EX_NOACTIVATE` so it never steals focus.

#[cfg(windows)]
pub fn start_micbar_pin(app: &tauri::AppHandle) {
    use std::time::Duration;
    use tauri::Manager;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_NOACTIVATE,
    };

    let app = app.clone();
    std::thread::spawn(move || {
        // Apply WS_EX_NOACTIVATE once, as soon as the mic bar exists.
        let mut styled = false;
        loop {
            if let Some(w) = app.get_webview_window("micbar") {
                if let Ok(hwnd) = w.hwnd() {
                    unsafe {
                        if !styled {
                            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                            let _ = SetWindowLongPtrW(
                                hwnd,
                                GWL_EXSTYLE,
                                ex | WS_EX_NOACTIVATE.0 as isize,
                            );
                            styled = true;
                        }
                        // Re-assert topmost without moving/resizing or activating.
                        let _ = SetWindowPos(
                            hwnd,
                            Some(HWND_TOPMOST),
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                        );
                    }
                }
                let _ = w.set_always_on_top(true);
            }
            std::thread::sleep(Duration::from_millis(1500));
        }
    });
}

#[cfg(not(windows))]
pub fn start_micbar_pin(_app: &tauri::AppHandle) {
    // Topmost overlay re-pin is Windows-specific; other platforms rely on the
    // window manager honouring alwaysOnTop from tauri.conf.json.
}
