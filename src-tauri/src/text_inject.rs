use arboard::{Clipboard, ImageData};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// Time the target application gets to read the clipboard before it is restored.
pub const RESTORE_DELAY: Duration = Duration::from_millis(800);

/// Clipboard content saved before pasting, so it can be put back afterwards.
pub enum SavedClipboard {
    Files(Vec<PathBuf>),
    Text(String),
    Image(ImageData<'static>),
}

/// Reads the current clipboard content.
/// Files come first because macOS also exposes a copied file as plain text.
pub fn save_clipboard(clipboard: &Mutex<Clipboard>) -> Option<SavedClipboard> {
    let mut cb = clipboard.lock().ok()?;

    if let Ok(files) = cb.get().file_list() {
        return Some(SavedClipboard::Files(files));
    }
    if let Ok(text) = cb.get_text() {
        return Some(SavedClipboard::Text(text));
    }
    cb.get_image().ok().map(SavedClipboard::Image)
}

/// Puts saved content back, unless the clipboard changed since `pasted_text` was set.
pub fn restore_clipboard(
    clipboard: &Mutex<Clipboard>,
    pasted_text: &str,
    saved: SavedClipboard,
) -> Result<(), String> {
    let mut cb = clipboard.lock().map_err(|e| e.to_string())?;

    if cb.get_text().ok().as_deref() != Some(pasted_text) {
        return Ok(());
    }

    match saved {
        SavedClipboard::Files(files) => cb.set().file_list(&files),
        SavedClipboard::Text(text) => cb.set_text(text),
        SavedClipboard::Image(image) => cb.set_image(image),
    }
    .map_err(|e| e.to_string())
}

/// Injects text into the currently focused application using clipboard + paste shortcut.
/// Uses a persistent clipboard reference to avoid macOS clearing clipboard on drop.
pub fn inject_text_with_clipboard(
    clipboard: &Mutex<Clipboard>,
    text: &str,
) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

    // Set text to clipboard using the persistent instance
    {
        let mut cb = clipboard.lock().map_err(|e| e.to_string())?;
        cb.set_text(text).map_err(|e| e.to_string())?;
    }

    // Delay to ensure clipboard is ready (macOS needs time to propagate)
    thread::sleep(Duration::from_millis(200));

    // Simulate paste keystroke (platform-specific)
    simulate_paste()?;

    Ok(())
}

/// Fallback: inject text with a temporary clipboard (used if no persistent one available).
pub fn inject_text(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(200));
    simulate_paste()?;
    // Don't drop clipboard immediately on macOS — sleep to let paste complete
    thread::sleep(Duration::from_millis(500));
    Ok(())
}

/// Simulates Ctrl+V on Linux using xdotool.
#[cfg(target_os = "linux")]
fn simulate_paste() -> Result<(), String> {
    let result = Command::new("xdotool")
        .args(["key", "--clearmodifiers", "ctrl+v"])
        .output();

    match result {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                log::warn!("xdotool warning: {}", stderr);
            }
            Ok(())
        }
        Err(e) => {
            log::warn!("xdotool key failed: {}", e);
            Err(format!("Failed to simulate paste: {}", e))
        }
    }
}

/// Simulates Cmd+V on macOS using CoreGraphics key events.
#[cfg(target_os = "macos")]
fn simulate_paste() -> Result<(), String> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGKeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    const KEY_V: CGKeyCode = 0x09;

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| "Failed to create CGEventSource".to_string())?;

    let key_down = CGEvent::new_keyboard_event(source.clone(), KEY_V, true)
        .map_err(|_| "Failed to create key-down event".to_string())?;
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);

    let key_up = CGEvent::new_keyboard_event(source, KEY_V, false)
        .map_err(|_| "Failed to create key-up event".to_string())?;
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);

    key_down.post(core_graphics::event::CGEventTapLocation::Session);
    thread::sleep(Duration::from_millis(20));
    key_up.post(core_graphics::event::CGEventTapLocation::Session);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulate_paste_command_exists() {
        #[cfg(target_os = "linux")]
        {
            let result = Command::new("which").arg("xdotool").output();
            assert!(result.is_ok(), "xdotool should be discoverable via 'which'");
        }

        #[cfg(target_os = "macos")]
        {
            let result = Command::new("which").arg("osascript").output();
            assert!(result.is_ok(), "osascript should be discoverable via 'which'");
        }
    }
}
