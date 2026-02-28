use rdev::{listen, Event, EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::settings::HotkeyMode;
use crate::AppState;

const DOUBLE_TAP_THRESHOLD_MS: u64 = 400;

struct DoubleTapState {
    last_press: Option<Instant>,
    key_down: bool,
}

impl Default for DoubleTapState {
    fn default() -> Self {
        Self {
            last_press: None,
            key_down: false,
        }
    }
}

pub struct DoubleTapListener {
    running: Arc<AtomicBool>,
}

impl DoubleTapListener {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(&self, app: AppHandle, target_mode: HotkeyMode) {
        if self.running.load(Ordering::SeqCst) {
            return;
        }

        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();

        let target_key = match target_mode {
            HotkeyMode::DoubleTapSuper => Key::MetaLeft,
            HotkeyMode::DoubleTapCtrl => Key::ControlLeft,
            HotkeyMode::DoubleTapShift => Key::ShiftLeft,
            HotkeyMode::KeyCombination => return, // Don't start listener for combo mode
        };

        std::thread::spawn(move || {
            let state = Arc::new(Mutex::new(DoubleTapState::default()));
            let state_clone = state.clone();
            let app_clone = app.clone();

            let callback = move |event: Event| {
                if !running.load(Ordering::SeqCst) {
                    return;
                }

                let mut s = state_clone.lock().unwrap();

                match event.event_type {
                    EventType::KeyPress(key) => {
                        // Check for both left and right variants
                        let is_target = matches!(
                            (&target_key, &key),
                            (Key::MetaLeft, Key::MetaLeft) |
                            (Key::MetaLeft, Key::MetaRight) |
                            (Key::ControlLeft, Key::ControlLeft) |
                            (Key::ControlLeft, Key::ControlRight) |
                            (Key::ShiftLeft, Key::ShiftLeft) |
                            (Key::ShiftLeft, Key::ShiftRight)
                        );

                        if is_target && !s.key_down {
                            s.key_down = true;
                            let now = Instant::now();

                            if let Some(last) = s.last_press {
                                if now.duration_since(last) < Duration::from_millis(DOUBLE_TAP_THRESHOLD_MS) {
                                    // Double tap detected!
                                    log::info!("Double tap detected for {:?}", target_key);
                                    s.last_press = None; // Reset to avoid triple-tap

                                    // Trigger toggle_recording
                                    let app_c = app_clone.clone();
                                    tauri::async_runtime::spawn(async move {
                                        trigger_toggle(&app_c).await;
                                    });
                                } else {
                                    s.last_press = Some(now);
                                }
                            } else {
                                s.last_press = Some(now);
                            }
                        }
                    }
                    EventType::KeyRelease(key) => {
                        let is_target = matches!(
                            (&target_key, &key),
                            (Key::MetaLeft, Key::MetaLeft) |
                            (Key::MetaLeft, Key::MetaRight) |
                            (Key::ControlLeft, Key::ControlLeft) |
                            (Key::ControlLeft, Key::ControlRight) |
                            (Key::ShiftLeft, Key::ShiftLeft) |
                            (Key::ShiftLeft, Key::ShiftRight)
                        );

                        if is_target {
                            s.key_down = false;
                        }
                    }
                    _ => {}
                }
            };

            if let Err(e) = listen(callback) {
                log::error!("Failed to start rdev listener: {:?}", e);
            }
        });
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

async fn trigger_toggle(app: &AppHandle) {
    use crate::{emit_status, Status};

    let state = app.state::<AppState>();
    let is_recording = state.recorder.is_recording();

    log::info!("Double-tap handler: is_recording={}", is_recording);

    if is_recording {
        // Clone recorder for the blocking task
        let recorder = state.recorder.clone();

        // Run stop_recording in a blocking thread pool
        let stop_result = tokio::task::spawn_blocking(move || {
            recorder.stop_recording()
        }).await;

        match stop_result {
            Ok(Ok(audio_data)) => {
                emit_status(app, Status::Transcribing);
                let settings = state.settings.lock().unwrap().clone();
                if settings.api_key.is_empty() {
                    emit_status(app, Status::Idle);
                    let _ = app.emit("transcription-error", "API key not configured");
                    return;
                }
                let app_c = app.clone();
                tokio::spawn(async move {
                    match crate::whisper::transcribe(&settings.api_key, audio_data, &settings.language).await {
                        Ok(text) => {
                            log::info!("Transcription: {}", text);

                            // Add to history
                            {
                                let state = app_c.state::<AppState>();
                                let mut history = state.history.lock().unwrap();
                                history.add_entry(text.clone());
                                if let Err(e) = crate::settings::save_history(&history) {
                                    log::error!("Failed to save history: {}", e);
                                }
                            }

                            if let Err(e) = crate::text_inject::inject_text(&text) {
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
                emit_status(app, Status::Idle);
            }
            Err(e) => {
                log::error!("Stop recording task join failed: {}", e);
                emit_status(app, Status::Idle);
            }
        }
    } else {
        let microphone = state.settings.lock().unwrap().microphone.clone();
        let mic_ref = if microphone == "default" { None } else { Some(microphone.as_str()) };
        match state.recorder.start_recording(mic_ref.as_deref()) {
            Ok(()) => emit_status(app, Status::Recording),
            Err(e) => {
                log::error!("Start recording failed: {}", e);
                let _ = app.emit("transcription-error", e.to_string());
            }
        }
    }
}
