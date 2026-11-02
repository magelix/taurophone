#[cfg(target_os = "linux")]
use arboard::{GetExtLinux, LinuxClipboardKind, SetExtLinux};
use arboard::{Clipboard, ImageData};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// Time the target application gets to read the clipboard before it is restored.
pub const RESTORE_DELAY: Duration = Duration::from_millis(800);

/// Clipboard content saved before pasting, so it can be put back afterwards.
pub struct SavedClipboard {
    content: Option<SavedContent>,
    #[cfg(target_os = "linux")]
    primary: Option<String>,
}

enum SavedContent {
    Files(Vec<PathBuf>),
    Text(String),
    Image(ImageData<'static>),
}

/// Reads the current clipboard content.
/// Files come first because macOS also exposes a copied file as plain text.
pub fn save_clipboard(clipboard: &Mutex<Clipboard>) -> Option<SavedClipboard> {
    let mut cb = clipboard.lock().ok()?;

    let content = if let Ok(files) = cb.get().file_list() {
        Some(SavedContent::Files(files))
    } else if let Ok(text) = cb.get_text() {
        Some(SavedContent::Text(text))
    } else {
        cb.get_image().ok().map(SavedContent::Image)
    };

    Some(SavedClipboard {
        content,
        #[cfg(target_os = "linux")]
        primary: cb.get().clipboard(LinuxClipboardKind::Primary).text().ok(),
    })
}

/// Puts saved content back, unless the clipboard changed since `pasted_text` was set.
pub fn restore_clipboard(
    clipboard: &Mutex<Clipboard>,
    pasted_text: &str,
    saved: SavedClipboard,
) -> Result<(), String> {
    let mut cb = clipboard.lock().map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    if let Some(text) = saved.primary {
        let current = cb.get().clipboard(LinuxClipboardKind::Primary).text();
        if current.ok().as_deref() == Some(pasted_text) {
            cb.set()
                .clipboard(LinuxClipboardKind::Primary)
                .text(text)
                .map_err(|e| e.to_string())?;
        }
    }

    if cb.get_text().ok().as_deref() != Some(pasted_text) {
        return Ok(());
    }

    match saved.content {
        Some(SavedContent::Files(files)) => restore_files(&mut cb, files),
        Some(SavedContent::Text(text)) => cb.set_text(text),
        Some(SavedContent::Image(image)) => cb.set_image(image),
        None => Ok(()),
    }
    .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "linux"))]
fn restore_files(cb: &mut Clipboard, files: Vec<PathBuf>) -> Result<(), arboard::Error> {
    cb.set().file_list(&files)
}

/// Linux file managers only paste a format arboard cannot offer, so GTK provides the files.
#[cfg(target_os = "linux")]
fn restore_files(_cb: &mut Clipboard, files: Vec<PathBuf>) -> Result<(), arboard::Error> {
    gtk::glib::MainContext::default().invoke(move || {
        if let Err(e) = offer_files(&files) {
            log::error!("Failed to restore files: {}", e);
        }
    });
    Ok(())
}

/// Must run on the GTK main thread.
#[cfg(target_os = "linux")]
fn offer_files(files: &[PathBuf]) -> Result<(), String> {
    use gtk::{TargetEntry, TargetFlags};

    const COPIED_FILES: u32 = 0;
    const URI_LIST: u32 = 1;

    let uris = files
        .iter()
        .map(|file| gtk::glib::filename_to_uri(file, None).map(String::from))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .join("\n");

    let targets = [
        TargetEntry::new("x-special/gnome-copied-files", TargetFlags::empty(), COPIED_FILES),
        TargetEntry::new("text/uri-list", TargetFlags::empty(), URI_LIST),
    ];

    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    let offered = clipboard.set_with_data(&targets, move |_, data, info| {
        let content = match info {
            COPIED_FILES => format!("copy\n{}", uris),
            _ => uris.clone(),
        };
        data.set(&data.target(), 8, content.as_bytes());
    });

    if offered {
        Ok(())
    } else {
        Err("GTK could not take over the clipboard".to_string())
    }
}

/// Puts the text where the paste shortcut reads it.
/// Linux terminals paste Shift+Insert from the primary selection, other apps from the clipboard.
fn set_paste_text(cb: &mut Clipboard, text: &str) -> Result<(), String> {
    cb.set_text(text).map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    cb.set()
        .clipboard(LinuxClipboardKind::Primary)
        .text(text)
        .map_err(|e| e.to_string())?;

    Ok(())
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
        set_paste_text(&mut cb, text)?;
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
    set_paste_text(&mut clipboard, text)?;
    thread::sleep(Duration::from_millis(200));
    simulate_paste()?;
    // Don't drop clipboard immediately on macOS — sleep to let paste complete
    thread::sleep(Duration::from_millis(500));
    Ok(())
}

/// Simulates Shift+Insert on Linux using xdotool.
/// Ctrl+V is no paste shortcut in terminals, Shift+Insert works in terminals and other apps.
#[cfg(target_os = "linux")]
fn simulate_paste() -> Result<(), String> {
    let result = Command::new("xdotool")
        .args(["key", "--clearmodifiers", "shift+Insert"])
        .output();

    match result {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                log::warn!("xdotool failed: {}", stderr);
                return Err(format!(
                    "Failed to simulate paste: {}. The text is in the clipboard.",
                    stderr.trim()
                ));
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

    if !crate::permissions::can_post_events() {
        return Err("Accessibility permission missing. The text is in the clipboard.".to_string());
    }

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
