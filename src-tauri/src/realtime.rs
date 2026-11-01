use crate::audio::AudioState;
use crate::settings::{AppSettings, TranscriptionMode};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

const URL: &str = "wss://api.openai.com/v1/realtime?intent=transcription";
const SAMPLE_RATE: u32 = 24000;
const LIVE_DELAY: &str = "medium";
const SEND_INTERVAL: Duration = Duration::from_millis(200);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FINISH_TIMEOUT: Duration = Duration::from_secs(60);

/// Sends the audio to the API while the user is speaking.
pub struct RealtimeSession {
    stop: oneshot::Sender<()>,
    task: JoinHandle<Result<String, String>>,
}

impl RealtimeSession {
    pub fn start(settings: &AppSettings, recorder: AudioState) -> Self {
        let (stop, stop_received) = oneshot::channel();
        let api_key = settings.api_key.clone();
        let session_update = session_update(settings);
        let mut sent = 0;

        let task = tauri::async_runtime::spawn(async move {
            let next_audio = move || recorder.new_samples(&mut sent, SAMPLE_RATE);
            run_session(URL, &api_key, session_update, next_audio, stop_received).await
        });

        Self { stop, task }
    }

    /// Sends the rest of the audio and waits for the final text.
    pub async fn finish(self) -> Result<String, String> {
        let _ = self.stop.send(());

        let text = self.task.await.map_err(|e| e.to_string())??;

        if text.trim().is_empty() {
            return Err("Empty realtime transcript".to_string());
        }

        Ok(text)
    }
}

fn session_update(settings: &AppSettings) -> Value {
    let live = settings.transcription_mode == TranscriptionMode::RealtimeLive;
    let model = if live {
        "gpt-live-transcribe"
    } else {
        "gpt-transcribe"
    };

    let mut transcription = json!({ "model": model, "languages": [settings.language] });
    if !settings.keywords.is_empty() {
        transcription["keywords"] = json!(settings.keywords);
    }
    if live {
        transcription["delay"] = json!(LIVE_DELAY);
    }

    json!({
        "type": "session.update",
        "session": {
            "type": "transcription",
            "audio": {
                "input": {
                    "format": { "type": "audio/pcm", "rate": SAMPLE_RATE },
                    "transcription": transcription,
                    "turn_detection": null
                }
            }
        }
    })
}

fn append_event(samples: &[i16]) -> Option<Message> {
    if samples.is_empty() {
        return None;
    }

    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let event = json!({ "type": "input_audio_buffer.append", "audio": STANDARD.encode(bytes) });

    Some(Message::text(event.to_string()))
}

/// Returns the event of a server message, or an error if the API reported one.
fn parse_event(message: Message) -> Result<Option<Value>, String> {
    let Message::Text(text) = message else {
        return Ok(None);
    };

    let event: Value = serde_json::from_str(text.as_str()).map_err(|e| e.to_string())?;

    if event["type"] == "error" {
        return Err(format!("Realtime API error: {}", event["error"]["message"]));
    }

    Ok(Some(event))
}

async fn run_session(
    url: &str,
    api_key: &str,
    session_update: Value,
    mut next_audio: impl FnMut() -> Vec<i16>,
    mut stop: oneshot::Receiver<()>,
) -> Result<String, String> {
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    let authorization =
        HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|e| e.to_string())?;
    request.headers_mut().insert("Authorization", authorization);

    let (socket, _) = timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| "Realtime connection timed out".to_string())?
        .map_err(|e| format!("Realtime connection failed: {}", e))?;
    let (mut sink, mut stream) = socket.split();

    sink.send(Message::text(session_update.to_string()))
        .await
        .map_err(|e| e.to_string())?;

    let mut ticker = tokio::time::interval(SEND_INTERVAL);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Some(message) = append_event(&next_audio()) {
                    sink.send(message).await.map_err(|e| e.to_string())?;
                }
            }
            _ = &mut stop => break,
            message = stream.next() => {
                let message = message
                    .ok_or("Realtime connection closed")?
                    .map_err(|e| e.to_string())?;
                parse_event(message)?;
            }
        }
    }

    if let Some(message) = append_event(&next_audio()) {
        sink.send(message).await.map_err(|e| e.to_string())?;
    }

    let commit = json!({ "type": "input_audio_buffer.commit" });
    sink.send(Message::text(commit.to_string()))
        .await
        .map_err(|e| e.to_string())?;

    let wait_for_transcript = async {
        let mut item_id = None;

        while let Some(message) = stream.next().await {
            let Some(event) = parse_event(message.map_err(|e| e.to_string())?)? else {
                continue;
            };

            match event["type"].as_str() {
                Some("input_audio_buffer.committed") => {
                    item_id = event["item_id"].as_str().map(str::to_string);
                }
                Some("conversation.item.input_audio_transcription.completed")
                    if event["item_id"].as_str() == item_id.as_deref() =>
                {
                    return event["transcript"]
                        .as_str()
                        .map(str::to_string)
                        .ok_or("No transcript in response".to_string());
                }
                Some("conversation.item.input_audio_transcription.failed")
                    if event["item_id"].as_str() == item_id.as_deref() =>
                {
                    return Err("Realtime transcription failed".to_string());
                }
                _ => {}
            }
        }

        Err("Realtime connection closed".to_string())
    };

    timeout(FINISH_TIMEOUT, wait_for_transcript)
        .await
        .map_err(|_| "Realtime transcript timed out".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn settings(mode: TranscriptionMode, keywords: Vec<String>) -> AppSettings {
        AppSettings {
            transcription_mode: mode,
            keywords,
            ..AppSettings::default()
        }
    }

    /// Starts a server that collects the audio and answers the commit with `reply`.
    async fn fake_server(reply: Vec<Value>) -> (String, tokio::task::JoinHandle<(Value, usize)>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());

        let server = tokio::spawn(async move {
            let (connection, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(connection).await.unwrap();
            let mut update = Value::Null;
            let mut audio_bytes = 0;

            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let event: Value = serde_json::from_str(text.as_str()).unwrap();

                match event["type"].as_str().unwrap() {
                    "session.update" => update = event,
                    "input_audio_buffer.append" => {
                        let audio = event["audio"].as_str().unwrap();
                        audio_bytes += STANDARD.decode(audio).unwrap().len();
                    }
                    "input_audio_buffer.commit" => {
                        for event in reply {
                            socket.send(Message::text(event.to_string())).await.unwrap();
                        }
                        break;
                    }
                    other => panic!("unexpected event {}", other),
                }
            }

            (update, audio_bytes)
        });

        (url, server)
    }

    fn committed(item_id: &str) -> Value {
        json!({ "type": "input_audio_buffer.committed", "item_id": item_id })
    }

    fn completed(item_id: &str, transcript: &str) -> Value {
        json!({
            "type": "conversation.item.input_audio_transcription.completed",
            "item_id": item_id,
            "transcript": transcript
        })
    }

    #[test]
    fn test_session_update_live_uses_delay_and_keywords() {
        let settings = settings(TranscriptionMode::RealtimeLive, vec!["Taurophone".to_string()]);
        let update = session_update(&settings);
        let transcription = &update["session"]["audio"]["input"]["transcription"];

        assert_eq!(transcription["model"], "gpt-live-transcribe");
        assert_eq!(transcription["languages"][0], "de");
        assert_eq!(transcription["keywords"][0], "Taurophone");
        assert_eq!(transcription["delay"], LIVE_DELAY);
        assert_eq!(update["session"]["type"], "transcription");
        assert!(update["session"]["audio"]["input"]["turn_detection"].is_null());
    }

    #[test]
    fn test_session_update_turn_has_no_delay_or_empty_keywords() {
        let update = session_update(&settings(TranscriptionMode::RealtimeTurn, Vec::new()));
        let transcription = &update["session"]["audio"]["input"]["transcription"];

        assert_eq!(transcription["model"], "gpt-transcribe");
        assert!(transcription.get("delay").is_none());
        assert!(transcription.get("keywords").is_none());
    }

    #[tokio::test]
    async fn test_session_sends_all_audio_and_returns_matching_transcript() {
        let (url, server) = fake_server(vec![
            completed("other", "falscher Text"),
            committed("item_1"),
            completed("other", "falscher Text"),
            completed("item_1", "Hallo Welt"),
        ])
        .await;

        let mut chunks = vec![vec![1, 2], vec![3]].into_iter();
        let next_audio = move || chunks.next().unwrap_or_default();
        let (stop, stop_received) = oneshot::channel();
        stop.send(()).unwrap();

        let update = session_update(&settings(TranscriptionMode::RealtimeLive, Vec::new()));
        let text = run_session(&url, "key", update, next_audio, stop_received).await;

        assert_eq!(text.unwrap(), "Hallo Welt");

        let (update, audio_bytes) = server.await.unwrap();
        assert_eq!(update["type"], "session.update");
        assert!(audio_bytes >= 4, "audio bytes: {}", audio_bytes);
    }

    #[tokio::test]
    async fn test_session_reports_api_error() {
        let error = json!({ "type": "error", "error": { "message": "invalid keyword" } });
        let (url, _server) = fake_server(vec![error]).await;
        let (stop, stop_received) = oneshot::channel();
        stop.send(()).unwrap();

        let update = session_update(&settings(TranscriptionMode::RealtimeTurn, Vec::new()));
        let result = run_session(&url, "key", update, Vec::new, stop_received).await;

        assert!(result.unwrap_err().contains("invalid keyword"));
    }

    #[tokio::test]
    async fn test_session_fails_without_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        drop(listener);

        let (_stop, stop_received) = oneshot::channel();
        let update = session_update(&settings(TranscriptionMode::RealtimeTurn, Vec::new()));
        let result = run_session(&url, "key", update, Vec::new, stop_received).await;

        assert!(result.unwrap_err().contains("connection failed"));
    }
}
