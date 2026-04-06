import { useState } from "react";

interface WorkerApi {
  apiKey: string;
  setApiKey: (key: string) => void;
  savedKeyMask: string | null;
  status: {
    pending: number;
    done: number;
    failed: number;
    skipped: number;
    topics_count: number;
    is_running: boolean;
  };
  progress: {
    processed: number;
    total: number;
    queue_remaining: number;
  } | null;
  error: string | null;
  validationMessage: string | null;
  validating: boolean;
  busy: boolean;
  saveApiKey: () => Promise<boolean>;
  validateApiKey: () => Promise<boolean>;
  startWorker: () => Promise<void>;
  stopWorker: () => Promise<void>;
  reprocessWiki: () => Promise<void>;
  clearWikiData: () => Promise<void>;
}

interface WikiSettingsProps {
  worker: WorkerApi;
}

export function WikiSettings({ worker }: WikiSettingsProps) {
  const [expanded, setExpanded] = useState(!worker.savedKeyMask);
  const hasKey = !!worker.savedKeyMask;

  return (
    <div className="wiki-settings">
      <button
        className="settings-toggle"
        onClick={() => setExpanded(!expanded)}
      >
        {expanded ? "\u25B2" : "\u2699"} Wiki Settings
      </button>

      {expanded && (
        <div className="settings-panel">
          <div className="settings-row">
            <label>OpenAI API Key:</label>
            {hasKey ? (
              <span className="api-key-display">{worker.savedKeyMask}</span>
            ) : (
              <div className="key-input-group">
                <input
                  type="password"
                  value={worker.apiKey}
                  onChange={(e) => worker.setApiKey(e.target.value)}
                  placeholder="sk-..."
                  className="key-input"
                />
                <button
                  onClick={() => worker.saveApiKey()}
                  disabled={worker.busy || !worker.apiKey.trim()}
                  className="key-save-btn"
                >
                  Save
                </button>
                <button
                  onClick={() => worker.validateApiKey()}
                  disabled={worker.validating || !worker.apiKey.trim()}
                  className="action-btn"
                >
                  {worker.validating ? "..." : "Validate"}
                </button>
              </div>
            )}
          </div>

          {worker.validationMessage && (
            <div className="settings-stats">{worker.validationMessage}</div>
          )}

          <div className="settings-stats">
            <div>
              Queue: {worker.status.pending} pending /{" "}
              {worker.status.done + worker.status.skipped} done /{" "}
              {worker.status.failed} failed
            </div>
            <div>Topics: {worker.status.topics_count}</div>
            <div>
              Worker: {worker.status.is_running ? "Running" : "Stopped"}
            </div>
          </div>

          {worker.progress && (
            <div className="settings-progress">
              <div className="progress-bar-container">
                <div
                  className="progress-bar-fill"
                  style={{
                    width: `${worker.progress.total > 0 ? (worker.progress.processed / worker.progress.total) * 100 : 0}%`,
                  }}
                />
              </div>
              <div className="progress-text">
                {worker.progress.processed} / {worker.progress.total} (
                {worker.progress.queue_remaining} remaining)
              </div>
            </div>
          )}

          {worker.error && <div className="settings-error">{worker.error}</div>}

          <div className="settings-actions">
            {hasKey && (
              <>
                {worker.status.is_running ? (
                  <button
                    onClick={worker.stopWorker}
                    disabled={worker.busy}
                    className="action-btn"
                  >
                    Stop Worker
                  </button>
                ) : (
                  <button
                    onClick={worker.startWorker}
                    disabled={worker.busy}
                    className="action-btn action-primary"
                  >
                    Start Worker
                  </button>
                )}
                <button
                  onClick={worker.reprocessWiki}
                  disabled={worker.busy}
                  className="action-btn"
                >
                  Reprocess All
                </button>
                <button
                  onClick={worker.clearWikiData}
                  disabled={worker.busy}
                  className="action-btn action-danger"
                >
                  Clear Wiki
                </button>
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
