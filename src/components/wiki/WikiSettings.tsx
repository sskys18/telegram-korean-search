import { useState } from "react";
import type { WikiProgress, WikiStatus } from "../../types";

interface WikiSettingsProps {
  worker: {
    codexAvailable: boolean | null;
    codexValid: boolean | null;
    status: WikiStatus;
    progress: WikiProgress | null;
    loading: boolean;
    busy: boolean;
    validating: boolean;
    error: string | null;
    validateCodex: () => Promise<void>;
    startWorker: () => Promise<void>;
    stopWorker: () => Promise<void>;
    reprocessWiki: () => Promise<void>;
    clearWikiData: () => Promise<void>;
  };
  onDataChanged?: () => Promise<void>;
}

export function WikiSettings({ worker, onDataChanged }: WikiSettingsProps) {
  const [openPanel, setOpenPanel] = useState(false);

  const handleReprocess = async () => {
    if (window.confirm("Rebuild the wiki queue and summaries from collected messages?")) {
      await worker.reprocessWiki();
      await onDataChanged?.();
    }
  };

  const handleClear = async () => {
    if (window.confirm("Clear all wiki topics, pages, and queue data?")) {
      await worker.clearWikiData();
      await onDataChanged?.();
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
          {/* Codex OAuth Status */}
          <div className="wiki-field">
            <span className="wiki-field-label">Codex CLI</span>
            {worker.codexAvailable === null || worker.loading ? (
              <span>Checking...</span>
            ) : worker.codexAvailable ? (
              <div className="wiki-actions-row">
                <span style={{ color: "#4caf50" }}>
                  codex CLI found
                </span>
                <button
                  type="button"
                  className="wiki-inline-button"
                  disabled={worker.validating}
                  onClick={() => { void worker.validateCodex(); }}
                >
                  {worker.validating ? "Validating..." : "Validate Token"}
                </button>
                {worker.codexValid === true && (
                  <span style={{ color: "#4caf50" }}>Valid</span>
                )}
                {worker.codexValid === false && (
                  <span style={{ color: "#ff6b6b" }}>Invalid</span>
                )}
              </div>
            ) : (
              <div>
                <span style={{ color: "#ff6b6b" }}>
                  Not found. Install with{" "}
                  <code style={{ background: "#2d2d2d", padding: "2px 6px", borderRadius: 4 }}>
                    npm i -g @openai/codex
                  </code>{" "}
                  in terminal first.
                </span>
              </div>
            )}
          </div>

          {/* Queue Stats */}
          <div className="wiki-queue-grid">
            <div>
              <span className="wiki-stat-label">Pending</span>
              <strong>{worker.loading ? "-" : worker.status.pending}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Processing</span>
              <strong>{worker.loading ? "-" : worker.status.processing}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Done</span>
              <strong>{worker.loading ? "-" : worker.status.done}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Failed</span>
              <strong>{worker.loading ? "-" : worker.status.failed}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Skipped</span>
              <strong>{worker.loading ? "-" : worker.status.skipped}</strong>
            </div>
            <div>
              <span className="wiki-stat-label">Topics</span>
              <strong>{worker.loading ? "-" : worker.status.topics_count}</strong>
            </div>
          </div>

          {/* Progress */}
          {worker.progress && (
            <div className="wiki-progress-text">
              Processed {worker.progress.processed}/{worker.progress.total} with{" "}
              {worker.progress.queue_remaining} queued.
            </div>
          )}

          {/* Error */}
          {worker.error && <div className="wiki-error">{worker.error}</div>}

          {/* Actions */}
          <div className="wiki-actions-row">
            {worker.codexAvailable && (
              <>
                <button
                  type="button"
                  className="wiki-primary-button"
                  disabled={worker.busy || worker.loading}
                  onClick={() => {
                    if (worker.status.is_running) {
                      void worker.stopWorker();
                    } else {
                      void worker.startWorker();
                    }
                  }}
                >
                  {worker.status.is_running ? "Stop Worker" : "Start Worker"}
                </button>
                <button
                  type="button"
                  className="wiki-inline-button"
                  disabled={worker.busy}
                  onClick={() => { void handleReprocess(); }}
                >
                  Reprocess
                </button>
                <button
                  type="button"
                  className="wiki-danger-button"
                  disabled={worker.busy}
                  onClick={() => { void handleClear(); }}
                >
                  Clear
                </button>
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
