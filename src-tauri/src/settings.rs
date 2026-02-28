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
            hotkey: "Ctrl+Shift+Space".to_string(),
            hotkey_mode: HotkeyMode::KeyCombination,
            language: "de".to_string(),
            microphone: "default".to_string(),
        }
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
