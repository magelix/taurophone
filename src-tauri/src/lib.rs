mod audio;
mod doubletap;
mod permissions;
mod realtime;
mod replacements;
mod settings;
mod text_inject;
mod whisper;

use audio::AudioState;
use doubletap::DoubleTapListener;
use permissions::Permissions;
use realtime::RealtimeSession;
use settings::{AppSettings, HistoryEntry, HotkeyMode, TranscriptionHistory, TranscriptionMode};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub struct AppState {
    pub recorder: AudioState,
    pub settings: Mutex<AppSettings>,
    pub history: Mutex<TranscriptionHistory>,
    doubletap_listener: Mutex<Option<DoubleTapListener>>,
    clipboard: Mutex<arboard::Clipboard>,
    realtime: Mutex<Option<RealtimeSession>>,
}

#[derive(Clone, serde::Serialize)]
pub enum Status {
    Idle,
    Recording,
    Transcribing,
}

impl Status {
    fn as_str(&self) -> &str {
        match self {
            Status::Idle => "idle",
            Status::Recording => "recording",
            Status::Transcribing => "transcribing",
        }
    }
}

pub fn emit_status(app: &AppHandle, status: Status) {
    let _ = app.emit("status-changed", status.as_str());
}

fn add_to_history(state: &AppState, text: &str) {
    let mut history = state.history.lock().unwrap();
    history.add_entry(text.to_string());
    if let Err(e) = settings::save_history(&history) {
        log::error!("Failed to save history: {}", e);
    }
}

fn keep_failed_recording(audio_data: &[u8]) {
    if let Err(e) = settings::save_failed_recording(audio_data) {
        log::error!("Failed to keep recording: {}", e);
    }
}

async fn upload_audio(settings: &AppSettings, audio_data: &[u8]) -> Result<String, String> {
    whisper::transcribe(
        &settings.api_key,
        audio_data,
        settings.api_language(),
        &settings.keywords,
    )
    .await
}

/// Falls back to the file upload, so a realtime problem does not lose the dictation.
async fn transcribe_recording(
    settings: &AppSettings,
    realtime: Option<RealtimeSession>,
    audio_data: &[u8],
) -> Result<String, String> {
    let started = Instant::now();

    let text = match realtime {
        Some(session) => match session.finish().await {
            Ok(text) => text,
            Err(e) => {
                log::error!("Realtime failed, uploading the file instead: {}", e);
                upload_audio(settings, audio_data).await?
            }
        },
        None => upload_audio(settings, audio_data).await?,
    };

    log::info!(
        "Transcription took {} ms, mode {:?}",
        started.elapsed().as_millis(),
        settings.transcription_mode
    );

    let text = replacements::apply(&text, &settings.replacements);

    if settings.uses_swiss_spelling() {
        return Ok(replacements::to_swiss_spelling(&text));
    }

    Ok(text)
}

/// Processes audio data: transcribes it, saves to history, and injects text.
/// Shared by both the Tauri command and hotkey/doubletap handlers.
pub async fn process_transcription(app: AppHandle, audio_data: Vec<u8>) {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let realtime = state.realtime.lock().unwrap().take();

    if settings.api_key.is_empty() {
        keep_failed_recording(&audio_data);
        emit_status(&app, Status::Idle);
        let _ = app.emit("transcription-error", "API key not configured");
        return;
    }

    emit_status(&app, Status::Transcribing);

    match transcribe_recording(&settings, realtime, &audio_data).await {
        Ok(text) => {
            log::info!("Transcribed {} characters", text.chars().count());

            add_to_history(&state, &text);

            let saved_clipboard = if settings.restore_clipboard {
                text_inject::save_clipboard(&state.clipboard)
            } else {
                None
            };

            if let Err(e) = text_inject::inject_text_with_clipboard(&state.clipboard, &text) {
                log::error!("Failed to inject text: {}", e);
                let _ = app.emit("transcription-error", e.to_string());
            } else {
                let _ = app.emit("transcription-result", text.clone());

                if let Some(saved) = saved_clipboard {
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(text_inject::RESTORE_DELAY).await;
                        let state = app_clone.state::<AppState>();
                        if let Err(e) =
                            text_inject::restore_clipboard(&state.clipboard, &text, saved)
                        {
                            log::error!("Failed to restore clipboard: {}", e);
                        }
                    });
                }
            }
        }
        Err(e) => {
            log::error!("Transcription failed: {}", e);
            keep_failed_recording(&audio_data);
            let _ = app.emit("transcription-error", e);
        }
    }

    emit_status(&app, Status::Idle);
}

/// Starts recording with the configured microphone.
pub fn start_recording_with_settings(state: &AppState) -> Result<(), String> {
    let settings = state.settings.lock().unwrap().clone();
    let mic_ref = if settings.microphone == "default" {
        None
    } else {
        Some(settings.microphone.as_str())
    };
    state.recorder.start_recording(mic_ref)?;

    if settings.transcription_mode != TranscriptionMode::File && !settings.api_key.is_empty() {
        let session = RealtimeSession::start(&settings, state.recorder.clone());
        *state.realtime.lock().unwrap() = Some(session);
    }

    Ok(())
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> AppSettings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    settings: AppSettings,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let old_settings = state.settings.lock().unwrap().clone();

    settings::save_settings(&settings)?;
    *state.settings.lock().unwrap() = settings.clone();

    if old_settings.hotkey_mode != settings.hotkey_mode || old_settings.hotkey != settings.hotkey {
        update_hotkey_listener(&app, &settings)?;
    }

    Ok(())
}

fn update_hotkey_listener(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let state = app.state::<AppState>();

    if let Some(listener) = state.doubletap_listener.lock().unwrap().take() {
        listener.stop();
    }

    let _ = app.global_shortcut().unregister_all();

    match &settings.hotkey_mode {
        HotkeyMode::KeyCombination => {
            register_shortcut(app, &settings.hotkey)?;
        }
        mode => start_doubletap_listener(app, mode.clone()),
    }

    Ok(())
}

fn start_doubletap_listener(app: &AppHandle, mode: HotkeyMode) {
    let listener = DoubleTapListener::new();
    listener.start(app.clone(), mode);
    *app.state::<AppState>().doubletap_listener.lock().unwrap() = Some(listener);
}

#[tauri::command]
fn list_microphones() -> Vec<String> {
    AudioState::list_devices()
}

#[tauri::command]
fn get_history(state: tauri::State<AppState>) -> Vec<HistoryEntry> {
    state.history.lock().unwrap().entries.clone()
}

#[tauri::command]
fn clear_history(state: tauri::State<AppState>) -> Result<(), String> {
    let mut history = state.history.lock().unwrap();
    history.entries.clear();
    settings::save_history(&history)
}

#[tauri::command]
fn copy_to_clipboard(text: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut clipboard = state.clipboard.lock().unwrap();
    clipboard.set_text(&text).map_err(|e| e.to_string())
}

#[tauri::command]
fn check_permissions() -> Permissions {
    permissions::check()
}

#[tauri::command]
fn has_failed_recording() -> bool {
    settings::has_failed_recording()
}

/// Transcribes the kept recording. The text goes to the history and the clipboard,
/// because the focus is on this window and pasting would miss the original text field.
#[tauri::command]
async fn retry_failed_recording(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let audio_data = settings::load_failed_recording().ok_or("No failed recording")?;
    let settings = state.settings.lock().unwrap().clone();

    let text = transcribe_recording(&settings, None, &audio_data).await?;

    add_to_history(&state, &text);
    state
        .clipboard
        .lock()
        .unwrap()
        .set_text(&text)
        .map_err(|e| e.to_string())?;
    settings::delete_failed_recording()?;

    Ok(text)
}

#[tauri::command]
async fn toggle_recording(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if state.recorder.is_recording() {
        let recorder = state.recorder.clone();
        let audio_data = tokio::task::spawn_blocking(move || recorder.stop_recording())
            .await
            .map_err(|e| format!("Join error: {}", e))??;

        let app_clone = app.clone();
        tokio::spawn(async move {
            process_transcription(app_clone, audio_data).await;
        });
    } else {
        start_recording_with_settings(&state)?;
        emit_status(&app, Status::Recording);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hotkey_ctrl_shift_space() {
        let shortcut = parse_hotkey("Ctrl+Shift+Space").unwrap();
        assert_eq!(shortcut.key, Code::Space);
    }

    #[test]
    fn test_parse_hotkey_super_shift_space() {
        let shortcut = parse_hotkey("Super+Shift+Space").unwrap();
        assert_eq!(shortcut.key, Code::Space);
    }

    #[test]
    fn test_parse_hotkey_cmd_alias() {
        let shortcut = parse_hotkey("Cmd+Shift+Space").unwrap();
        assert_eq!(shortcut.key, Code::Space);
    }

    #[test]
    fn test_parse_hotkey_invalid() {
        assert!(parse_hotkey("InvalidKey").is_none());
    }

    #[test]
    fn test_parse_hotkey_key_r() {
        let shortcut = parse_hotkey("Ctrl+R").unwrap();
        assert_eq!(shortcut.key, Code::KeyR);
    }
}

fn parse_hotkey(hotkey_str: &str) -> Option<Shortcut> {
    let mut modifiers = Modifiers::empty();
    let mut code = None;

    for part in hotkey_str.split('+') {
        let part = part.trim();
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" => modifiers |= Modifiers::ALT,
            "super" | "meta" | "cmd" => modifiers |= Modifiers::SUPER,
            "space" => code = Some(Code::Space),
            "r" => code = Some(Code::KeyR),
            _ => {}
        }
    }

    code.map(|c| Shortcut::new(Some(modifiers), c))
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .map_err(|e| format!("Failed to load tray icon: {}", e))?;

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Taurophone")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "show" => show_main_window(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn register_shortcut(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let shortcut = parse_hotkey(hotkey).ok_or("Invalid hotkey format")?;

    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| format!("Failed to register shortcut: {}", e))?;

    log::info!("Registered global shortcut: {}", hotkey);
    Ok(())
}

fn handle_shortcut_event(
    app: &AppHandle,
    _shortcut: &Shortcut,
    event: tauri_plugin_global_shortcut::ShortcutEvent,
) {
    if event.state != ShortcutState::Pressed {
        return;
    }

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app_clone.state::<AppState>();

        if state.recorder.is_recording() {
            let recorder = state.recorder.clone();

            match tokio::task::spawn_blocking(move || recorder.stop_recording()).await {
                Ok(Ok(audio_data)) => {
                    process_transcription(app_clone, audio_data).await;
                }
                Ok(Err(e)) => {
                    log::error!("Stop recording failed: {}", e);
                    emit_status(&app_clone, Status::Idle);
                }
                Err(e) => {
                    log::error!("Stop recording task join failed: {}", e);
                    emit_status(&app_clone, Status::Idle);
                }
            }
        } else {
            match start_recording_with_settings(&state) {
                Ok(()) => emit_status(&app_clone, Status::Recording),
                Err(e) => {
                    log::error!("Start recording failed: {}", e);
                    let _ = app_clone.emit("transcription-error", e.to_string());
                }
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let settings = settings::load_settings();
    let history = settings::load_history();
    let hotkey = settings.hotkey.clone();
    let hotkey_mode = settings.hotkey_mode.clone();
    let startup_mode = hotkey_mode.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(handle_shortcut_event)
                .build(),
        )
        .manage(AppState {
            recorder: AudioState::new(),
            settings: Mutex::new(settings),
            history: Mutex::new(history),
            doubletap_listener: Mutex::new(None),
            clipboard: Mutex::new(arboard::Clipboard::new().expect("Failed to init clipboard")),
            realtime: Mutex::new(None),
        })
        .setup(move |app| {
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log::LevelFilter::Info)
                    .build(),
            )?;

            if let Err(e) = setup_tray(app.handle()) {
                log::error!("Failed to setup tray: {}", e);
            }

            if hotkey_mode == HotkeyMode::KeyCombination {
                if let Err(e) = register_shortcut(app.handle(), &hotkey) {
                    log::error!("Failed to register global shortcut: {}", e);
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            // Dictation keeps working in the background, only Quit ends the app
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            list_microphones,
            toggle_recording,
            get_history,
            clear_history,
            copy_to_clipboard,
            check_permissions,
            has_failed_recording,
            retry_failed_recording,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app, event| match event {
            // Started here, because on macOS a tap created during setup receives no key events
            tauri::RunEvent::Ready => {
                let uses_double_tap = startup_mode != HotkeyMode::KeyCombination;
                permissions::request_missing(uses_double_tap);

                if uses_double_tap {
                    start_doubletap_listener(app, startup_mode.clone());
                }
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_main_window(app),
            _ => {}
        });
}
