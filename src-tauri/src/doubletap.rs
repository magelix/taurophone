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
            HotkeyMode::DoubleTapSuper => TargetKey::Super,
            HotkeyMode::DoubleTapCtrl => TargetKey::Ctrl,
            HotkeyMode::DoubleTapShift => TargetKey::Shift,
            HotkeyMode::KeyCombination => return,
        };

        std::thread::spawn(move || {
            platform::run_listener(running, app, target_key);
        });
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum TargetKey {
    Super,
    Ctrl,
    Shift,
}

/// Common double-tap logic shared across platforms.
/// Returns true if a double-tap was detected.
fn check_double_tap(state: &mut DoubleTapState, is_press: bool, is_target: bool) -> bool {
    if !is_target {
        return false;
    }

    if is_press && !state.key_down {
        state.key_down = true;
        let now = Instant::now();

        if let Some(last) = state.last_press {
            if now.duration_since(last) < Duration::from_millis(DOUBLE_TAP_THRESHOLD_MS) {
                state.last_press = None;
                return true;
            }
        }
        state.last_press = Some(now);
    } else if !is_press && is_target {
        state.key_down = false;
    }

    false
}

// ─── Linux: use rdev ────────────────────────────────────────────
#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use rdev::{listen, Event, EventType, Key};

    pub fn run_listener(running: Arc<AtomicBool>, app: AppHandle, target_key: TargetKey) {
        let state = Arc::new(Mutex::new(DoubleTapState::default()));

        let callback = move |event: Event| {
            if !running.load(Ordering::SeqCst) {
                return;
            }

            let (is_press, key) = match event.event_type {
                EventType::KeyPress(k) => (true, k),
                EventType::KeyRelease(k) => (false, k),
                _ => return,
            };

            let is_target = match target_key {
                TargetKey::Super => matches!(key, Key::MetaLeft | Key::MetaRight),
                TargetKey::Ctrl => matches!(key, Key::ControlLeft | Key::ControlRight),
                TargetKey::Shift => matches!(key, Key::ShiftLeft | Key::ShiftRight),
            };

            let mut s = state.lock().unwrap();
            if check_double_tap(&mut s, is_press, is_target) {
                let app_c = app.clone();
                tauri::async_runtime::spawn(async move {
                    trigger_toggle(&app_c).await;
                });
            }
        };

        if let Err(e) = listen(callback) {
            log::error!("Failed to start rdev listener: {:?}", e);
        }
    }
}

// ─── macOS: use CGEventTap (avoids TSM main-thread crash) ───────
#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventType, EventField,
    };

    // macOS virtual key codes for modifier keys
    const KC_LEFT_SHIFT: i64 = 0x38;   // 56
    const KC_RIGHT_SHIFT: i64 = 0x3C;  // 60
    const KC_LEFT_CTRL: i64 = 0x3B;    // 59
    const KC_RIGHT_CTRL: i64 = 0x3E;   // 62
    const KC_LEFT_CMD: i64 = 0x37;     // 55
    const KC_RIGHT_CMD: i64 = 0x36;    // 54

    pub fn run_listener(running: Arc<AtomicBool>, app: AppHandle, target_key: TargetKey) {
        let state = Arc::new(Mutex::new(DoubleTapState::default()));

        // We listen for flagsChanged events (modifier key presses/releases)
        let tap = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::FlagsChanged],
            move |_proxy, event_type, event| -> Option<CGEvent> {
                if !running.load(Ordering::SeqCst) {
                    return None;
                }

                if event_type != CGEventType::FlagsChanged {
                    return None;
                }

                let keycode = event.get_integer_value_field(
                    EventField::KEYBOARD_EVENT_KEYCODE,
                );

                let flags = event.get_flags();

                let is_target = match target_key {
                    TargetKey::Super => keycode == KC_LEFT_CMD || keycode == KC_RIGHT_CMD,
                    TargetKey::Ctrl => keycode == KC_LEFT_CTRL || keycode == KC_RIGHT_CTRL,
                    TargetKey::Shift => keycode == KC_LEFT_SHIFT || keycode == KC_RIGHT_SHIFT,
                };

                if !is_target {
                    return None;
                }

                // For FlagsChanged, check if the modifier flag is set (press) or cleared (release)
                let is_press = match target_key {
                    TargetKey::Super => flags.contains(CGEventFlags::CGEventFlagCommand),
                    TargetKey::Ctrl => flags.contains(CGEventFlags::CGEventFlagControl),
                    TargetKey::Shift => flags.contains(CGEventFlags::CGEventFlagShift),
                };

                let mut s = state.lock().unwrap();
                if check_double_tap(&mut s, is_press, true) {
                    let app_c = app.clone();
                    tauri::async_runtime::spawn(async move {
                        trigger_toggle(&app_c).await;
                    });
                }

                None // ListenOnly: don't modify events
            },
        );

        match tap {
            Ok(tap) => unsafe {
                let loop_source = tap
                    .mach_port
                    .create_runloop_source(0)
                    .expect("Failed to create run loop source");
                let runloop = CFRunLoop::get_current();
                runloop.add_source(&loop_source, kCFRunLoopCommonModes);
                tap.enable();
                CFRunLoop::run_current();
            },
            Err(_) => {
                log::error!(
                    "Failed to create CGEventTap. On macOS, you need to grant Accessibility \
                     permission: System Settings → Privacy & Security → Accessibility"
                );
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_double_tap_detection() {
        let mut state = DoubleTapState::default();

        // First press — no double tap yet
        assert!(!check_double_tap(&mut state, true, true));
        assert!(state.last_press.is_some());

        // Release
        check_double_tap(&mut state, false, true);
        assert!(!state.key_down);

        // Second press within threshold — should detect double tap
        assert!(check_double_tap(&mut state, true, true));
        assert!(state.last_press.is_none()); // reset after detection
    }

    #[test]
    fn test_double_tap_timeout() {
        let mut state = DoubleTapState::default();

        // First press
        check_double_tap(&mut state, true, true);
        check_double_tap(&mut state, false, true);

        // Wait beyond threshold
        state.last_press = Some(Instant::now() - Duration::from_millis(500));

        // Second press — too slow, no double tap
        assert!(!check_double_tap(&mut state, true, true));
    }

    #[test]
    fn test_non_target_key_ignored() {
        let mut state = DoubleTapState::default();
        assert!(!check_double_tap(&mut state, true, false));
        assert!(state.last_press.is_none());
    }

    #[test]
    fn test_double_tap_threshold_is_reasonable() {
        assert!(DOUBLE_TAP_THRESHOLD_MS >= 200, "Threshold too short");
        assert!(DOUBLE_TAP_THRESHOLD_MS <= 800, "Threshold too long");
    }
}
