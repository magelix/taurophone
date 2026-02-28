# Taurophone 🎤

System-wide speech-to-text desktop app powered by OpenAI Whisper API.

Built with **Tauri 2.0** (Rust) + **React** + **TypeScript**.

## Features

- 🎙️ Global hotkey or double-tap trigger (Ctrl, Shift, Super/Meta)
- 📋 Transcribed text injected directly into any active text field
- 📜 Transcription history with copy-to-clipboard
- 🌍 Multi-language support (DE, EN, FR, IT)
- 🎨 Dark theme UI
- 🔧 Configurable microphone, hotkey, and language settings

## Requirements

- OpenAI API key (for Whisper API)
- Linux (X11) — macOS/Windows support planned

## Build

```bash
npm install
cargo tauri build
```

## Development

```bash
cargo tauri dev
```

## License

MIT
