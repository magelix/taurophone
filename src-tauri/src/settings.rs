use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    DoubleTapSuper,
    DoubleTapCtrl,
    DoubleTapShift,
    KeyCombination,
}

impl Default for HotkeyMode {
    fn default() -> Self {
        HotkeyMode::KeyCombination
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub api_key: String,
    pub hotkey: String,
    pub hotkey_mode: HotkeyMode,
    pub language: String,
    pub microphone: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            hotkey: default_hotkey().to_string(),
            hotkey_mode: HotkeyMode::KeyCombination,
            language: "de".to_string(),
            microphone: "default".to_string(),
        }
    }
}

/// Returns the platform-appropriate default hotkey.
/// macOS uses Cmd (Super), Linux uses Ctrl.
pub fn default_hotkey() -> &'static str {
    if cfg!(target_os = "macos") {
        "Super+Shift+Space"
    } else {
        "Ctrl+Shift+Space"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: u64,
    pub text: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranscriptionHistory {
    pub entries: Vec<HistoryEntry>,
    #[serde(default)]
    pub next_id: u64,
}

impl TranscriptionHistory {
    pub fn add_entry(&mut self, text: String) {
        let entry = HistoryEntry {
            id: self.next_id,
            text,
            timestamp: Utc::now(),
        };
        self.next_id += 1;
        self.entries.insert(0, entry);
        // Keep only last 20 entries
        if self.entries.len() > 20 {
            self.entries.truncate(20);
        }
    }
}

fn config_dir() -> PathBuf {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("taurophone");

    fs::create_dir_all(&config_dir).ok();
    config_dir
}

fn config_path() -> PathBuf {
    config_dir().join("settings.json")
}

fn history_path() -> PathBuf {
    config_dir().join("history.json")
}

pub fn load_settings() -> AppSettings {
    let path = config_path();

    if let Ok(content) = fs::read_to_string(&path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        AppSettings::default()
    }
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let path = config_path();
    let content = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

pub fn load_history() -> TranscriptionHistory {
    let path = history_path();

    if let Ok(content) = fs::read_to_string(&path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        TranscriptionHistory::default()
    }
}

pub fn save_history(history: &TranscriptionHistory) -> Result<(), String> {
    let path = history_path();
    let content = serde_json::to_string_pretty(history).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_hotkey_on_linux() {
        // On the current build platform (Linux), the default should use Ctrl
        #[cfg(target_os = "linux")]
        assert_eq!(default_hotkey(), "Ctrl+Shift+Space");

        #[cfg(target_os = "macos")]
        assert_eq!(default_hotkey(), "Super+Shift+Space");
    }

    #[test]
    fn test_default_settings_hotkey_matches_platform() {
        let settings = AppSettings::default();
        assert_eq!(settings.hotkey, default_hotkey());
        assert_eq!(settings.hotkey_mode, HotkeyMode::KeyCombination);
    }

    #[test]
    fn test_history_add_entry() {
        let mut history = TranscriptionHistory::default();
        assert_eq!(history.entries.len(), 0);
        assert_eq!(history.next_id, 0);

        history.add_entry("Hello".to_string());
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].text, "Hello");
        assert_eq!(history.entries[0].id, 0);
        assert_eq!(history.next_id, 1);
    }

    #[test]
    fn test_history_truncates_at_20() {
        let mut history = TranscriptionHistory::default();
        for i in 0..25 {
            history.add_entry(format!("Entry {}", i));
        }
        assert_eq!(history.entries.len(), 20);
        // Most recent entry should be first
        assert_eq!(history.entries[0].text, "Entry 24");
        assert_eq!(history.next_id, 25);
    }

    #[test]
    fn test_history_newest_first() {
        let mut history = TranscriptionHistory::default();
        history.add_entry("First".to_string());
        history.add_entry("Second".to_string());
        assert_eq!(history.entries[0].text, "Second");
        assert_eq!(history.entries[1].text, "First");
    }

    #[test]
    fn test_hotkey_mode_serialization() {
        let mode = HotkeyMode::DoubleTapSuper;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"double_tap_super\"");

        let deserialized: HotkeyMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, HotkeyMode::DoubleTapSuper);
    }

    #[test]
    fn test_settings_roundtrip_serialization() {
        let settings = AppSettings {
            api_key: "test-key".to_string(),
            hotkey: "Ctrl+Shift+Space".to_string(),
            hotkey_mode: HotkeyMode::DoubleTapCtrl,
            language: "en".to_string(),
            microphone: "default".to_string(),
        };

        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.api_key, "test-key");
        assert_eq!(deserialized.hotkey_mode, HotkeyMode::DoubleTapCtrl);
        assert_eq!(deserialized.language, "en");
    }
}
