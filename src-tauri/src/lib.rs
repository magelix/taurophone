mod audio;
mod doubletap;
mod settings;
mod text_inject;
mod whisper;

use audio::AudioState;
use doubletap::DoubleTapListener;
use settings::{AppSettings, HistoryEntry, HotkeyMode, TranscriptionHistory};
use std::sync::Mutex;
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

    // Handle hotkey mode changes
    if old_settings.hotkey_mode != settings.hotkey_mode || old_settings.hotkey != settings.hotkey {
        update_hotkey_listener(&app, &settings)?;
    }

    Ok(())
}

fn update_hotkey_listener(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let state = app.state::<AppState>();

    // Stop existing double-tap listener if any
    if let Some(listener) = state.doubletap_listener.lock().unwrap().take() {
        listener.stop();
    }

    // Unregister existing global shortcut
    let _ = app.global_shortcut().unregister_all();

    match &settings.hotkey_mode {
        HotkeyMode::KeyCombination => {
            // Register the combo hotkey
            register_shortcut(app, &settings.hotkey)?;
        }
        mode => {
            // Start double-tap listener
            let listener = DoubleTapListener::new();
            listener.start(app.clone(), mode.clone());
            *state.doubletap_listener.lock().unwrap() = Some(listener);
        }
    }

    Ok(())
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
    // Use the persistent clipboard instance from AppState
    // (arboard on X11 clears clipboard when Clipboard is dropped)
    let mut clipboard = state.clipboard.lock().unwrap();
    clipboard.set_text(&text).map_err(|e| e.to_string())?;
    log::info!("Copied to clipboard: {} chars", text.len());
    Ok(())
}

#[tauri::command]
async fn toggle_recording(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let is_recording = state.recorder.is_recording();
    log::info!("toggle_recording: is_recording={}", is_recording);

    if is_recording {
        // Clone recorder to move into blocking task (avoids blocking the UI thread)
        let recorder = state.recorder.clone();
        let audio_data = tokio::task::spawn_blocking(move || {
            recorder.stop_recording()
        }).await.map_err(|e| format!("Join error: {}", e))??;
        log::info!("toggle_recording: stopped, {} bytes", audio_data.len());
        emit_status(&app, Status::Transcribing);

        let settings = state.settings.lock().unwrap().clone();

        if settings.api_key.is_empty() {
            emit_status(&app, Status::Idle);
            let _ = app.emit("transcription-error", "API key not configured");
            return Err("API key not configured".to_string());
        }

        // Transcribe in background
        let app_clone = app.clone();
        tokio::spawn(async move {
            match whisper::transcribe(&settings.api_key, audio_data, &settings.language).await {
                Ok(text) => {
                    log::info!("Transcription: {}", text);

                    // Add to history
                    {
                        let state = app_clone.state::<AppState>();
                        let mut history = state.history.lock().unwrap();
                        history.add_entry(text.clone());
                        if let Err(e) = settings::save_history(&history) {
                            log::error!("Failed to save history: {}", e);
                        }
                    }

                    // Inject text
                    if let Err(e) = text_inject::inject_text(&text) {
                        log::error!("Failed to inject text: {}", e);
                        let _ = app_clone.emit("transcription-error", e.to_string());
                    } else {
                        let _ = app_clone.emit("transcription-result", text);
                    }
                }
                Err(e) => {
                    log::error!("Transcription failed: {}", e);
                    let _ = app_clone.emit("transcription-error", e);
                }
            }
            emit_status(&app_clone, Status::Idle);
        });
    } else {
        // Start recording
        log::info!("toggle_recording: starting recording...");
        let microphone = state.settings.lock().unwrap().microphone.clone();
        let mic_ref = if microphone == "default" {
            None
        } else {
            Some(microphone.as_str())
        };

        state.recorder.start_recording(mic_ref.as_deref())?;
        log::info!("toggle_recording: recording started");
        emit_status(&app, Status::Recording);
    }

    log::info!("toggle_recording: COMMAND EXIT");
    Ok(())
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

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    // Use default icon from resources
    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .map_err(|e| format!("Failed to load tray icon: {}", e))?;

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Taurophone")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => {
                app.exit(0);
            }
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
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
    shortcut: &Shortcut,
    event: tauri_plugin_global_shortcut::ShortcutEvent,
) {
    if event.state != ShortcutState::Pressed {
        return;
    }

    log::info!("Hotkey {:?} pressed", shortcut);

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app_clone.state::<AppState>();
        let is_recording = state.recorder.is_recording();

        log::info!("Hotkey handler: is_recording={}", is_recording);

        if is_recording {
            // Clone recorder for the blocking task
            let recorder = state.recorder.clone();

            // Run stop_recording in a blocking thread pool
            let stop_result = tokio::task::spawn_blocking(move || {
                recorder.stop_recording()
            }).await;

            match stop_result {
                Ok(Ok(audio_data)) => {
                    emit_status(&app_clone, Status::Transcribing);
                    let settings = state.settings.lock().unwrap().clone();
                    if settings.api_key.is_empty() {
                        emit_status(&app_clone, Status::Idle);
                        let _ = app_clone.emit("transcription-error", "API key not configured");
                        return;
                    }
                    let app_c = app_clone.clone();
                    tokio::spawn(async move {
                        match whisper::transcribe(&settings.api_key, audio_data, &settings.language)
                            .await
                        {
                            Ok(text) => {
                                log::info!("Transcription: {}", text);

                                // Add to history
                                {
                                    let state = app_c.state::<AppState>();
                                    let mut history = state.history.lock().unwrap();
                                    history.add_entry(text.clone());
                                    if let Err(e) = settings::save_history(&history) {
                                        log::error!("Failed to save history: {}", e);
                                    }
                                }

                                if let Err(e) = text_inject::inject_text(&text) {
                                    log::error!("Failed to inject text: {}", e);
                                    let _ = app_c.emit("transcription-error", e.to_string());
                                } else {
                                    let _ = app_c.emit("transcription-result", text);
                                }
                            }
                            Err(e) => {
                                log::error!("Transcription failed: {}", e);
                                let _ = app_c.emit("transcription-error", e);
                            }
                        }
                        emit_status(&app_c, Status::Idle);
                    });
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
            let microphone = state.settings.lock().unwrap().microphone.clone();
            let mic_ref = if microphone == "default" {
                None
            } else {
                Some(microphone.as_str())
            };
            match state.recorder.start_recording(mic_ref.as_deref()) {
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

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    handle_shortcut_event(app, shortcut, event);
                })
                .build(),
        )
        .manage(AppState {
            recorder: AudioState::new(),
            settings: Mutex::new(settings),
            history: Mutex::new(history),
            doubletap_listener: Mutex::new(None),
            clipboard: Mutex::new(arboard::Clipboard::new().expect("Failed to init clipboard")),
        })
        .setup(move |app| {
            if true {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Setup system tray
            if let Err(e) = setup_tray(app.handle()) {
                log::error!("Failed to setup tray: {}", e);
            }

            // Setup hotkey based on mode
            match hotkey_mode {
                HotkeyMode::KeyCombination => {
                    if let Err(e) = register_shortcut(app.handle(), &hotkey) {
                        log::error!("Failed to register global shortcut: {}", e);
                    }
                }
                mode => {
                    // Start double-tap listener
                    let listener = DoubleTapListener::new();
                    listener.start(app.handle().clone(), mode);
                    let state = app.state::<AppState>();
                    *state.doubletap_listener.lock().unwrap() = Some(listener);
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            list_microphones,
            toggle_recording,
            get_history,
            clear_history,
            copy_to_clipboard,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
