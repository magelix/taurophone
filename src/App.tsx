import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import Settings from './Settings';
import History from './History';

type AppStatus = 'idle' | 'recording' | 'transcribing';

interface AppSettings {
  api_key: string;
  hotkey: string;
  hotkey_mode: 'double_tap_super' | 'double_tap_ctrl' | 'double_tap_shift' | 'key_combination';
  language: string;
  microphone: string;
}

interface HistoryEntry {
  id: number;
  text: string;
  timestamp: string;
}

function App() {
  const [status, setStatus] = useState<AppStatus>('idle');
  const [showSettings, setShowSettings] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const [settings, setSettings] = useState<AppSettings>({
    api_key: '',
    hotkey: 'Ctrl+Shift+Space',
    hotkey_mode: 'key_combination',
    language: 'de',
    microphone: 'default',
  });
  const [microphones, setMicrophones] = useState<string[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [toast, setToast] = useState<string | null>(null);

  const loadHistory = async () => {
    try {
      const entries = await invoke<HistoryEntry[]>('get_history');
      setHistory(entries);
    } catch (e) {
      console.error('Failed to load history:', e);
    }
  };

  useEffect(() => {
    // Load settings on mount
    invoke<AppSettings>('get_settings')
      .then(setSettings)
      .catch(console.error);

    // Get available microphones
    invoke<string[]>('list_microphones')
      .then(setMicrophones)
      .catch(console.error);

    // Load history
    loadHistory();

    // Listen for status changes from backend
    const unlistenStatus = listen<string>('status-changed', (event) => {
      setStatus(event.payload as AppStatus);
    });

    // Listen for transcription results
    const unlistenTranscription = listen<string>('transcription-result', (event) => {
      showToast(`Inserted: "${event.payload.substring(0, 50)}${event.payload.length > 50 ? '...' : ''}"`);
      // Reload history when new transcription arrives
      loadHistory();
    });

    // Listen for errors
    const unlistenError = listen<string>('transcription-error', (event) => {
      showToast(`Error: ${event.payload}`);
    });

    return () => {
      unlistenStatus.then((fn) => fn());
      unlistenTranscription.then((fn) => fn());
      unlistenError.then((fn) => fn());
    };
  }, []);

  const showToast = (message: string) => {
    setToast(message);
    setTimeout(() => setToast(null), 3000);
  };

  const handleSaveSettings = async (newSettings: AppSettings) => {
    try {
      await invoke('save_settings', { settings: newSettings });
      setSettings(newSettings);
      setShowSettings(false);
      showToast('Settings saved');
    } catch (error) {
      console.error('Failed to save settings:', error);
      showToast('Failed to save settings');
    }
  };

  const handleCopyHistory = async (text: string) => {
    try {
      await invoke('copy_to_clipboard', { text });
      showToast('Copied to clipboard');
    } catch (error) {
      console.error('Failed to copy:', error);
      showToast('Failed to copy');
    }
  };

  const handleClearHistory = async () => {
    try {
      await invoke('clear_history');
      setHistory([]);
      showToast('History cleared');
    } catch (error) {
      console.error('Failed to clear history:', error);
      showToast('Failed to clear history');
    }
  };

  const getStatusIcon = () => {
    switch (status) {
      case 'recording':
        return '🎤';
      case 'transcribing':
        return '⏳';
      default:
        return '🎙️';
    }
  };

  const getHotkeyHint = () => {
    switch (settings.hotkey_mode) {
      case 'double_tap_super':
        return 'Double-tap Super/Meta key';
      case 'double_tap_ctrl':
        return 'Double-tap Ctrl key';
      case 'double_tap_shift':
        return 'Double-tap Shift key';
      default:
        return settings.hotkey;
    }
  };

  return (
    <div className="app">
      <header className="header">
        <h1>Taurophone</h1>
        <div className="header-actions">
          <button
            className="icon-btn"
            onClick={() => setShowHistory(true)}
            title="History"
          >
            📜
          </button>
          <button className="settings-btn" onClick={() => setShowSettings(true)}>
            ⚙️ Settings
          </button>
        </div>
      </header>

      <main className="status-container">
        <div
          className={`status-indicator ${status}`}
          onClick={async () => {
            try {
              await invoke('toggle_recording');
            } catch (e) {
              console.error('Toggle recording failed:', e);
            }
          }}
          style={{ cursor: 'pointer' }}
        >
          {getStatusIcon()}
        </div>
        <p className="status-text">{status}</p>
        <div className="hotkey-hint">
          Press <kbd>{getHotkeyHint()}</kbd> to start/stop recording
        </div>
      </main>

      {showSettings && (
        <Settings
          settings={settings}
          microphones={microphones}
          onSave={handleSaveSettings}
          onClose={() => setShowSettings(false)}
        />
      )}

      {showHistory && (
        <History
          entries={history}
          onCopy={handleCopyHistory}
          onClear={handleClearHistory}
          onClose={() => setShowHistory(false)}
        />
      )}

      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}

export default App;
