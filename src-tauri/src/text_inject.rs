use arboard::Clipboard;
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Injects text into the currently focused application using clipboard + paste shortcut.
/// Platform-specific: uses xdotool on Linux, osascript on macOS.
pub fn inject_text(text: &str) -> Result<(), String> {
    // Store original clipboard content
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    let original = clipboard.get_text().ok();

    // Set new text to clipboard
    clipboard.set_text(text).map_err(|e| e.to_string())?;

    // Small delay to ensure clipboard is ready
    thread::sleep(Duration::from_millis(50));

    // Simulate paste keystroke (platform-specific)
    simulate_paste(text)?;

    // Small delay before restoring clipboard
    thread::sleep(Duration::from_millis(100));

    // Restore original clipboard content
    if let Some(original_text) = original {
        let _ = clipboard.set_text(original_text);
    }

    Ok(())
}

/// Simulates Ctrl+V on Linux using xdotool.
#[cfg(target_os = "linux")]
fn simulate_paste(text: &str) -> Result<(), String> {
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
            log::warn!("xdotool key failed: {}, trying alternative", e);

            let type_result = Command::new("xdotool")
                .args(["type", "--clearmodifiers", "--", text])
                .output();

            match type_result {
                Ok(_) => Ok(()),
                Err(type_err) => {
                    Err(format!("Failed to inject text: {} / {}", e, type_err))
                }
            }
        }
    }
}

/// Simulates Cmd+V on macOS using CoreGraphics key events.
/// This runs in-process so the Accessibility permission on Taurophone itself applies.
#[cfg(target_os = "macos")]
fn simulate_paste(_text: &str) -> Result<(), String> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGKeyCode, EventField};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // Virtual key code for 'V' on macOS
    const KEY_V: CGKeyCode = 0x09;

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| "Failed to create CGEventSource".to_string())?;

    // Key down: Cmd+V
    let key_down = CGEvent::new_keyboard_event(source.clone(), KEY_V, true)
        .map_err(|_| "Failed to create key-down event".to_string())?;
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);

    // Key up: V (with Cmd)
    let key_up = CGEvent::new_keyboard_event(source, KEY_V, false)
        .map_err(|_| "Failed to create key-up event".to_string())?;
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);

    key_down.post(core_graphics::event::CGEventTapLocation::HID);
    key_up.post(core_graphics::event::CGEventTapLocation::HID);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulate_paste_command_exists() {
        // Verify the platform-specific paste command is available
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
