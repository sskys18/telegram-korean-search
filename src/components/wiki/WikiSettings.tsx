import { useState } from "react";
import type { WikiProgress, WikiStatus } from "../../types";

interface WikiSettingsProps {
  apiKey: string;
  setApiKey: (value: string) => void;
  savedKeyMask: string | null;
  status: WikiStatus;
  progress: WikiProgress | null;
  loading: boolean;
  busy: boolean;
  validating: boolean;
  error: string | null;
  validationMessage: string | null;
  onSaveApiKey: () => Promise<boolean>;
  onValidateApiKey: () => Promise<boolean>;
  onStartWorker: () => Promise<void>;
  onStopWorker: () => Promise<void>;
  onReprocessWiki: () => Promise<void>;
  onClearWikiData: () => Promise<void>;
}

export function WikiSettings({
  apiKey,
  setApiKey,
  savedKeyMask,
  status,
  progress,
  loading,
  busy,
  validating,
  error,
  validationMessage,
  onSaveApiKey,
  onValidateApiKey,
  onStartWorker,
  onStopWorker,
  onReprocessWiki,
  onClearWikiData,
}: WikiSettingsProps) {
  const [openPanel, setOpenPanel] = useState(false);

  const handleReprocess = async () => {
    if (window.confirm("Rebuild the wiki queue and summaries from collected messages?")) {
      await onReprocessWiki();
    }
  };

  const handleClear = async () => {
    if (window.confirm("Clear all wiki topics, pages, and queue data?")) {
      await onClearWikiData();
    }
  };

  return (
    <div className="wiki-settings">
      <button
        type="button"
        className="wiki-settings-toggle"
        onClick={() => setOpenPanel((prev) => !prev)}
      >
        <span>Wiki Settings</span>
        <span>{openPanel ? "Hide" : "Show"}</span>
      </button>
      {openPanel && (
        <div className="wiki-settings-panel">
          <label className="wiki-field">
            <span className="wiki-field-label">OpenAI API Key</span>
            <input
              type="password"
              className="search-input"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={savedKeyMask || "sk-..."}
              spellCheck={false}
            />
          </label>
          <div className="wiki-actions-row">
            <button
              type="button"
              className="wiki-primary-button"
              disabled={busy || !apiKey.trim()}
              onClick={() => {
                void onSaveApiKey();
              }}
            >
              Save Key
            </button>
            <button
              type="button"
              className="wiki-inline-button"
              disabled={busy || validating || !apiKey.trim()}
              onClick={() => {
                void onValidateApiKey();
              }}
            >
              {validating ? "Validating..." : "Validate"}
            </button>
          </div>

          <div className="wiki-queue-grid">
            <div>
              <span className="wiki-stat-label">Pending</span>
              <strong>{loading ? "-" : status.pending}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Processing</span>
              <strong>{loading ? "-" : status.processing}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Done</span>
              <strong>{loading ? "-" : status.done}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Failed</span>
              <strong>{loading ? "-" : status.failed}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Skipped</span>
              <strong>{loading ? "-" : status.skipped}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Topics</span>
              <strong>{loading ? "-" : status.topics_count}</strong>
            </div>
          </div>

          {progress && (
            <div className="wiki-progress-text">
              Processed {progress.processed}/{progress.total} with{" "}
              {progress.queue_remaining} queued.
            </div>
          )}

          <div className="wiki-actions-row">
            <button
              type="button"
              className="wiki-primary-button"
              disabled={busy || loading}
              onClick={() => {
                if (status.is_running) {
                  void onStopWorker();
                } else {
                  void onStartWorker();
                }
              }}
            >
              {status.is_running ? "Stop Worker" : "Start Worker"}
            </button>
            <button
              type="button"
              className="wiki-inline-button"
              disabled={busy}
              onClick={() => {
                void handleReprocess();
              }}
            >
              Reprocess
            </button>
            <button
              type="button"
              className="wiki-danger-button"
              disabled={busy}
              onClick={() => {
                void handleClear();
              }}
            >
              Clear
            </button>
          </div>

          {validationMessage && <div className="wiki-note">{validationMessage}</div>}
          {error && <div className="wiki-error">{error}</div>}
        </div>
      )}
    </div>
  );
}
