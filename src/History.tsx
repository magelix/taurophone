interface HistoryEntry {
  id: number;
  text: string;
  timestamp: string;
}

interface HistoryProps {
  entries: HistoryEntry[];
  onCopy: (text: string) => void;
  onClear: () => void;
  onClose: () => void;
}

function formatTimestamp(timestamp: string): string {
  const date = new Date(timestamp);
  const now = new Date();
  const diff = now.getTime() - date.getTime();

  // If less than 24 hours, show relative time
  if (diff < 24 * 60 * 60 * 1000) {
    const hours = Math.floor(diff / (60 * 60 * 1000));
    const minutes = Math.floor((diff % (60 * 60 * 1000)) / (60 * 1000));

    if (hours > 0) {
      return `${hours}h ${minutes}m ago`;
    } else if (minutes > 0) {
      return `${minutes}m ago`;
    } else {
      return 'Just now';
    }
  }

  // Otherwise show date
  return date.toLocaleDateString() + ' ' + date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function History({ entries, onCopy, onClear, onClose }: HistoryProps) {
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal history-modal" onClick={(e) => e.stopPropagation()}>
        <div className="history-header">
          <h2>Transcription History</h2>
          {entries.length > 0 && (
            <button className="btn btn-small btn-danger" onClick={onClear}>
              Clear All
            </button>
          )}
        </div>

        <div className="history-list">
          {entries.length === 0 ? (
            <p className="history-empty">No transcriptions yet</p>
          ) : (
            entries.map((entry) => (
              <div
                key={entry.id}
                className="history-entry"
                onClick={() => onCopy(entry.text)}
                title="Click to copy"
              >
                <div className="history-entry-text">{entry.text}</div>
                <div className="history-entry-meta">
                  <span className="history-entry-time">{formatTimestamp(entry.timestamp)}</span>
                  <button
                    className="copy-btn"
                    onClick={(e) => {
                      e.stopPropagation();
                      onCopy(entry.text);
                    }}
                  >
                    Copy
                  </button>
                </div>
              </div>
            ))
          )}
        </div>

        <div className="modal-actions">
          <button className="btn btn-secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

export default History;
