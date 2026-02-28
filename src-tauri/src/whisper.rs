use reqwest::multipart;

pub async fn transcribe(
    api_key: &str,
    audio_data: Vec<u8>,
    language: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let part = multipart::Part::bytes(audio_data)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;

    let form = multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1")
        .text("language", language.to_string());

    let response = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API error {}: {}", status, body));
    }

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    result["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No text in response".to_string())
}
