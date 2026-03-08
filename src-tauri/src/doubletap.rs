use rdev::{listen, Event, EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::settings::HotkeyMode;
use crate::{emit_status, process_transcription, start_recording_with_settings, AppState, Status};

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
            HotkeyMode::KeyCombination => return,
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
                        let is_target = is_target_key(&target_key, &key);

                        if is_target && !s.key_down {
                            s.key_down = true;
                            let now = Instant::now();

                            if let Some(last) = s.last_press {
                                if now.duration_since(last)
                                    < Duration::from_millis(DOUBLE_TAP_THRESHOLD_MS)
                                {
                                    s.last_press = None;
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
                        if is_target_key(&target_key, &key) {
                            s.key_down = false;
                        }
                    }
                    _ => {}
                }
            };

            if let Err(e) = listen(callback) {
                log::error!("Failed to start rdev listener: {:?}", e);
                if cfg!(target_os = "macos") {
                    log::error!(
                        "On macOS, rdev requires Accessibility permission. \
                         Go to System Settings → Privacy & Security → Accessibility \
                         and add this application."
                    );
                }
            }
        });
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

/// Checks if the pressed key matches the target (including left/right variants).
fn is_target_key(target: &Key, pressed: &Key) -> bool {
    matches!(
        (target, pressed),
        (Key::MetaLeft, Key::MetaLeft)
            | (Key::MetaLeft, Key::MetaRight)
            | (Key::ControlLeft, Key::ControlLeft)
            | (Key::ControlLeft, Key::ControlRight)
            | (Key::ShiftLeft, Key::ShiftLeft)
            | (Key::ShiftLeft, Key::ShiftRight)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_target_key_meta_left() {
        assert!(is_target_key(&Key::MetaLeft, &Key::MetaLeft));
        assert!(is_target_key(&Key::MetaLeft, &Key::MetaRight));
        assert!(!is_target_key(&Key::MetaLeft, &Key::ControlLeft));
        assert!(!is_target_key(&Key::MetaLeft, &Key::ShiftLeft));
    }

    #[test]
    fn test_is_target_key_ctrl() {
        assert!(is_target_key(&Key::ControlLeft, &Key::ControlLeft));
        assert!(is_target_key(&Key::ControlLeft, &Key::ControlRight));
        assert!(!is_target_key(&Key::ControlLeft, &Key::MetaLeft));
        assert!(!is_target_key(&Key::ControlLeft, &Key::ShiftLeft));
    }

    #[test]
    fn test_is_target_key_shift() {
        assert!(is_target_key(&Key::ShiftLeft, &Key::ShiftLeft));
        assert!(is_target_key(&Key::ShiftLeft, &Key::ShiftRight));
        assert!(!is_target_key(&Key::ShiftLeft, &Key::MetaLeft));
        assert!(!is_target_key(&Key::ShiftLeft, &Key::ControlLeft));
    }

    #[test]
    fn test_is_target_key_unrelated_keys() {
        assert!(!is_target_key(&Key::MetaLeft, &Key::KeyA));
        assert!(!is_target_key(&Key::ControlLeft, &Key::Space));
        assert!(!is_target_key(&Key::ShiftLeft, &Key::Return));
    }

    #[test]
    fn test_double_tap_threshold_is_reasonable() {
        assert!(DOUBLE_TAP_THRESHOLD_MS >= 200, "Threshold too short");
        assert!(DOUBLE_TAP_THRESHOLD_MS <= 800, "Threshold too long");
    }
}

async fn trigger_toggle(app: &AppHandle) {
    let state = app.state::<AppState>();

    if state.recorder.is_recording() {
        let recorder = state.recorder.clone();

        match tokio::task::spawn_blocking(move || recorder.stop_recording()).await {
            Ok(Ok(audio_data)) => {
                process_transcription(app.clone(), audio_data).await;
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
        match start_recording_with_settings(&state) {
            Ok(()) => emit_status(app, Status::Recording),
            Err(e) => {
                log::error!("Start recording failed: {}", e);
                let _ = app.emit("transcription-error", e.to_string());
            }
        }
    }
}
