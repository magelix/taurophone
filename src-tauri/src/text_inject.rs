use arboard::Clipboard;
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Injects text into the currently focused application using clipboard + Ctrl+V
/// Linux X11 implementation using xdotool
pub fn inject_text(text: &str) -> Result<(), String> {
    // Store original clipboard content
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    let original = clipboard.get_text().ok();

    // Set new text to clipboard
    clipboard.set_text(text).map_err(|e| e.to_string())?;

    // Small delay to ensure clipboard is ready
    thread::sleep(Duration::from_millis(50));

    // Simulate Ctrl+V using xdotool
    let result = Command::new("xdotool")
        .args(["key", "--clearmodifiers", "ctrl+v"])
        .output();

    match result {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                log::warn!("xdotool warning: {}", stderr);
            }
        }
        Err(e) => {
            // Try alternative: xclip + xdotool type (fallback)
            log::warn!("xdotool key failed: {}, trying alternative", e);

            // xdotool type can also work but may be slower
            let type_result = Command::new("xdotool")
                .args(["type", "--clearmodifiers", "--", text])
                .output();

            if let Err(type_err) = type_result {
                return Err(format!("Failed to inject text: {} / {}", e, type_err));
            }
        }
    }

    // Small delay before restoring clipboard
    thread::sleep(Duration::from_millis(100));

    // Restore original clipboard content
    if let Some(original_text) = original {
        let _ = clipboard.set_text(original_text);
    }

    Ok(())
}
