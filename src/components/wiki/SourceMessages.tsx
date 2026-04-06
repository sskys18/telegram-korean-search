import { useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import type { WikiSourceMessage } from "../../types";
import { formatTimestamp, truncate } from "../../utils/format";

interface SourceMessagesProps {
  sources: WikiSourceMessage[];
}

export function SourceMessages({ sources }: SourceMessagesProps) {
  const [openPanel, setOpenPanel] = useState(false);

  return (
    <div className="source-messages">
      <button
        type="button"
        className="source-toggle"
        onClick={() => setOpenPanel((prev) => !prev)}
      >
        <span>Source Messages</span>
        <span>{openPanel ? "Hide" : `Show ${sources.length}`}</span>
      </button>
      {openPanel && (
        <div className="source-list">
          {sources.length === 0 ? (
            <div className="wiki-empty">No source messages linked yet.</div>
          ) : (
            <ol>
              {sources.map((source, index) => (
                <li key={`${source.chat_id}-${source.message_id}`} className="source-item">
                  <div className="source-meta">
                    <span>
                      {index + 1}. {source.chat_title}
                    </span>
                    <span>{formatTimestamp(source.timestamp)}</span>
                  </div>
                  <div className="source-text">{truncate(source.text_plain, 280)}</div>
                  {source.link && (
                    <button
                      type="button"
                      className="wiki-inline-button"
                      onClick={() =>
                        open(source.link!).catch((err) =>
                          console.error("Failed to open source link:", err),
                        )
                      }
                    >
                      Open Message
                    </button>
                  )}
                </li>
              ))}
            </ol>
          )}
        </div>
      )}
    </div>
  );
}
