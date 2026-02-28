use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{WavSpec, WavWriter};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// Thread-safe audio recording state.
/// The cpal Stream is not Send, so it lives in a dedicated thread.
#[derive(Clone)]
pub struct AudioState {
    samples: Arc<Mutex<Vec<i16>>>,
    sample_rate: Arc<Mutex<u32>>,
    is_recording: Arc<AtomicBool>,
    stop_signal: Arc<Mutex<bool>>,
    thread_handle: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            samples: Arc::new(Mutex::new(Vec::new())),
            sample_rate: Arc::new(Mutex::new(16000)),
            is_recording: Arc::new(AtomicBool::new(false)),
            stop_signal: Arc::new(Mutex::new(false)),
            thread_handle: Arc::new(Mutex::new(None)),
        }
    }

    pub fn list_devices() -> Vec<String> {
        let host = cpal::default_host();
        host.input_devices()
            .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
            .unwrap_or_default()
    }

    pub fn start_recording(&self, device_name: Option<&str>) -> Result<(), String> {
        if self.is_recording.load(Ordering::SeqCst) {
            return Err("Already recording".to_string());
        }

        let host = cpal::default_host();

        let device = match device_name {
            Some("default") | None => host.default_input_device(),
            Some(name) => host
                .input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().ok().as_deref() == Some(name)),
        };

        let device = device.ok_or("No input device available")?;
        log::info!("Using input device: {}", device.name().unwrap_or_default());

        // Find a config close to 16kHz mono (Whisper prefers this)
        let supported_config = device
            .supported_input_configs()
            .map_err(|e| e.to_string())?
            .filter(|c| c.channels() == 1 || c.channels() == 2)
            .min_by_key(|c| {
                let rate = c.min_sample_rate().0.max(c.max_sample_rate().0.min(16000));
                (rate as i32 - 16000).abs()
            })
            .ok_or("No suitable audio config found")?;

        let sample_rate = supported_config
            .min_sample_rate()
            .0
            .max(supported_config.max_sample_rate().0.min(16000));

        let config = supported_config.with_sample_rate(cpal::SampleRate(sample_rate));
        *self.sample_rate.lock().unwrap() = sample_rate;

        log::info!(
            "Audio config: {} Hz, {} channels",
            sample_rate,
            config.channels()
        );

        self.samples.lock().unwrap().clear();
        *self.stop_signal.lock().unwrap() = false;
        self.is_recording.store(true, Ordering::SeqCst);

        let samples = self.samples.clone();
        let stop_signal = self.stop_signal.clone();
        let is_recording = self.is_recording.clone();
        let channels = config.channels() as usize;

        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();

        let handle = thread::spawn(move || {
            let err_fn = |err| log::error!("Audio stream error: {}", err);

            let stream_result = match config.sample_format() {
                cpal::SampleFormat::I16 => {
                    let samples = samples.clone();
                    device.build_input_stream(
                        &config.into(),
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            let mut samples = samples.lock().unwrap();
                            if channels == 1 {
                                samples.extend_from_slice(data);
                            } else {
                                for chunk in data.chunks(channels) {
                                    let sum: i32 = chunk.iter().map(|&s| s as i32).sum();
                                    samples.push((sum / channels as i32) as i16);
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::F32 => {
                    let samples = samples.clone();
                    device.build_input_stream(
                        &config.into(),
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            let mut samples = samples.lock().unwrap();
                            if channels == 1 {
                                samples.extend(data.iter().map(|&s| (s * 32767.0) as i16));
                            } else {
                                for chunk in data.chunks(channels) {
                                    let sum: f32 = chunk.iter().sum();
                                    let avg = sum / channels as f32;
                                    samples.push((avg * 32767.0) as i16);
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::U8 => {
                    let samples = samples.clone();
                    device.build_input_stream(
                        &config.into(),
                        move |data: &[u8], _: &cpal::InputCallbackInfo| {
                            let mut samples = samples.lock().unwrap();
                            if channels == 1 {
                                samples.extend(data.iter().map(|&s| ((s as i16) - 128) * 256));
                            } else {
                                for chunk in data.chunks(channels) {
                                    let sum: i32 =
                                        chunk.iter().map(|&s| ((s as i32) - 128) * 256).sum();
                                    samples.push((sum / channels as i32) as i16);
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::U16 => {
                    let samples = samples.clone();
                    device.build_input_stream(
                        &config.into(),
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            let mut samples = samples.lock().unwrap();
                            if channels == 1 {
                                samples.extend(data.iter().map(|&s| (s as i32 - 32768) as i16));
                            } else {
                                for chunk in data.chunks(channels) {
                                    let sum: i32 = chunk.iter().map(|&s| s as i32 - 32768).sum();
                                    samples.push((sum / channels as i32) as i16);
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                format => {
                    is_recording.store(false, Ordering::SeqCst);
                    let _ = tx.send(Err(format!("Unsupported sample format: {:?}", format)));
                    return;
                }
            };

            let stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Failed to build stream: {}", e);
                    is_recording.store(false, Ordering::SeqCst);
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };

            if let Err(e) = stream.play() {
                log::error!("Failed to play stream: {}", e);
                is_recording.store(false, Ordering::SeqCst);
                let _ = tx.send(Err(e.to_string()));
                return;
            }

            let _ = tx.send(Ok(()));

            // Keep the stream alive until stop signal
            loop {
                thread::sleep(std::time::Duration::from_millis(50));
                if *stop_signal.lock().unwrap() {
                    break;
                }
            }
        });

        match rx.recv_timeout(std::time::Duration::from_secs(3)) {
            Ok(Ok(())) => {
                *self.thread_handle.lock().unwrap() = Some(handle);
                Ok(())
            }
            Ok(Err(e)) => {
                self.is_recording.store(false, Ordering::SeqCst);
                Err(e)
            }
            Err(_) => {
                self.is_recording.store(false, Ordering::SeqCst);
                Err("Timeout starting recording".to_string())
            }
        }
    }

    pub fn stop_recording(&self) -> Result<Vec<u8>, String> {
        *self.stop_signal.lock().unwrap() = true;
        self.is_recording.store(false, Ordering::SeqCst);

        if let Some(h) = self.thread_handle.lock().unwrap().take() {
            let _ = h.join();
        }

        let samples = self.samples.lock().unwrap();
        let sample_rate = *self.sample_rate.lock().unwrap();

        if samples.is_empty() {
            return Err("No audio recorded".to_string());
        }

        log::info!("Recorded {} samples at {} Hz", samples.len(), sample_rate);

        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = WavWriter::new(&mut cursor, spec).map_err(|e| e.to_string())?;
            for &sample in samples.iter() {
                writer.write_sample(sample).map_err(|e| e.to_string())?;
            }
            writer.finalize().map_err(|e| e.to_string())?;
        }

        Ok(cursor.into_inner())
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }
}

unsafe impl Send for AudioState {}
unsafe impl Sync for AudioState {}
