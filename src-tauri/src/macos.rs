// macOS-only helpers: paste with a synthetic Cmd+V, and the Accessibility
// permission that macOS requires before an app may send keystrokes to other
// apps. Without that permission CGEventPost silently does nothing, so the
// check must come first or a "successful" paste would insert nothing.
//
// Cmd+V works in every macOS app, Terminal and iTerm included, so there is no
// per-app strategy like the Windows worker's Shift+Insert for terminals.

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::CFString;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::time::Duration;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceFlagsState(state_id: i32) -> u64;
}

/// kCGEventSourceStateCombinedSessionState
const COMBINED_SESSION_STATE: i32 = 0;
/// Shift | Control | Option | Command
const MODIFIER_MASK: u64 = 0x0002_0000 | 0x0004_0000 | 0x0008_0000 | 0x0010_0000;
/// kVK_ANSI_V
const KEY_V: u16 = 9;

/// Whether BizVoice may send keystrokes. With `prompt`, macOS shows its
/// "BizVoice would like to control this computer" dialog (once per launch
/// while the permission is missing) that links to System Settings.
pub fn accessibility_trusted(prompt: bool) -> bool {
    unsafe {
        if !prompt {
            return AXIsProcessTrusted();
        }
        let key = CFString::from_static_string("AXTrustedCheckOptionPrompt");
        let opts = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef())
    }
}

/// The dictation hotkey (Option+Q by default) is often still held when the
/// paste fires; Option+Cmd+V is "Move" in Finder and does nothing in most
/// editors. Wait up to 400ms for the user to let go.
fn wait_modifiers_released() {
    for _ in 0..40 {
        if unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) } & MODIFIER_MASK == 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Press Cmd+V in the frontmost app. Err("accessibility") when not allowed.
pub fn cmd_v() -> Result<(), String> {
    if !accessibility_trusted(false) {
        return Err("accessibility".into());
    }
    wait_modifiers_released();
    // Private state: the event carries only the flags set here, not the keys
    // the user is physically holding.
    let src = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| "CGEventSource failed".to_string())?;
    for down in [true, false] {
        let ev = CGEvent::new_keyboard_event(src.clone(), KEY_V, down).map_err(|_| "CGEvent failed".to_string())?;
        ev.set_flags(CGEventFlags::CGEventFlagCommand);
        ev.post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(8));
    }
    Ok(())
}
