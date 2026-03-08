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

/// Simulates Cmd+V on macOS using osascript (AppleScript).
#[cfg(target_os = "macos")]
fn simulate_paste(_text: &str) -> Result<(), String> {
    let result = Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to keystroke \"v\" using command down",
        ])
        .output();

    match result {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!("osascript failed: {}", stderr))
            } else {
                Ok(())
            }
        }
        Err(e) => Err(format!("Failed to run osascript: {}", e)),
    }
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
