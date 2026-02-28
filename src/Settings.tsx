import { useState, useRef, useEffect } from 'react';

interface CustomSelectProps {
  id: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
}

function CustomSelect({ id, value, options, onChange }: CustomSelectProps) {
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const selectedOption = options.find((opt) => opt.value === value);
  const displayLabel = selectedOption?.label || value;

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    }

    if (isOpen) {
      document.addEventListener('mousedown', handleClickOutside);
      return () => document.removeEventListener('mousedown', handleClickOutside);
    }
  }, [isOpen]);

  const handleSelect = (optionValue: string) => {
    onChange(optionValue);
    setIsOpen(false);
  };

  return (
    <div className="custom-select" ref={containerRef} id={id}>
      <div
        className={`custom-select-trigger ${isOpen ? 'open' : ''}`}
        onClick={() => setIsOpen(!isOpen)}
      >
        <span>{displayLabel}</span>
        <span className="custom-select-arrow">▼</span>
      </div>
      {isOpen && (
        <div className="custom-select-options">
          {options.map((option) => (
            <div
              key={option.value}
              className={`custom-select-option ${option.value === value ? 'selected' : ''}`}
              onClick={() => handleSelect(option.value)}
            >
              {option.label}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

interface AppSettings {
  api_key: string;
  hotkey: string;
  hotkey_mode: 'double_tap_super' | 'double_tap_ctrl' | 'double_tap_shift' | 'key_combination';
  language: string;
  microphone: string;
}

interface SettingsProps {
  settings: AppSettings;
  microphones: string[];
  onSave: (settings: AppSettings) => void;
  onClose: () => void;
}

const LANGUAGES = [
  { code: 'de', name: 'Deutsch' },
  { code: 'en', name: 'English' },
  { code: 'fr', name: 'Français' },
  { code: 'it', name: 'Italiano' },
];

const HOTKEY_MODES = [
  { value: 'double_tap_super', label: 'Double-tap Super/Meta' },
  { value: 'double_tap_ctrl', label: 'Double-tap Ctrl' },
  { value: 'double_tap_shift', label: 'Double-tap Shift' },
  { value: 'key_combination', label: 'Key combination' },
];

const HOTKEY_OPTIONS = [
  'Ctrl+Shift+Space',
  'Ctrl+Alt+R',
  'Ctrl+Shift+R',
  'Alt+Space',
];

function Settings({ settings, microphones, onSave, onClose }: SettingsProps) {
  const [formData, setFormData] = useState<AppSettings>({ ...settings });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onSave(formData);
  };

  const handleChange = (field: keyof AppSettings) => (
    e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>
  ) => {
    setFormData((prev) => ({ ...prev, [field]: e.target.value }));
  };

  const handleSelectChange = (field: keyof AppSettings) => (value: string) => {
    setFormData((prev) => ({ ...prev, [field]: value }));
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Settings</h2>
        <form onSubmit={handleSubmit}>
          <div className="form-group">
            <label htmlFor="api_key">OpenAI API Key</label>
            <input
              type="password"
              id="api_key"
              value={formData.api_key}
              onChange={handleChange('api_key')}
              placeholder="sk-..."
            />
          </div>

          <div className="form-group">
            <label htmlFor="hotkey_mode">Trigger Mode</label>
            <CustomSelect
              id="hotkey_mode"
              value={formData.hotkey_mode}
              options={HOTKEY_MODES}
              onChange={handleSelectChange('hotkey_mode')}
            />
          </div>

          {formData.hotkey_mode === 'key_combination' && (
            <div className="form-group">
              <label htmlFor="hotkey">Key Combination</label>
              <CustomSelect
                id="hotkey"
                value={formData.hotkey}
                options={HOTKEY_OPTIONS.map((hk) => ({ value: hk, label: hk }))}
                onChange={handleSelectChange('hotkey')}
              />
            </div>
          )}

          <div className="form-group">
            <label htmlFor="language">Language</label>
            <CustomSelect
              id="language"
              value={formData.language}
              options={LANGUAGES.map((lang) => ({ value: lang.code, label: lang.name }))}
              onChange={handleSelectChange('language')}
            />
          </div>

          <div className="form-group">
            <label htmlFor="microphone">Microphone</label>
            <CustomSelect
              id="microphone"
              value={formData.microphone}
              options={[
                { value: 'default', label: 'Default' },
                ...microphones.map((mic) => ({ value: mic, label: mic })),
              ]}
              onChange={handleSelectChange('microphone')}
            />
          </div>

          <div className="modal-actions">
            <button type="button" className="btn btn-secondary" onClick={onClose}>
              Cancel
            </button>
            <button type="submit" className="btn btn-primary">
              Save
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

export default Settings;
