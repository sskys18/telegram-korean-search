import { useState } from "react";
import type { WikiSourceMessage } from "../../types";
import { getTopicSources } from "../../api/tauri";

interface SourceMessagesProps {
  topicId: number;
  sourceCount: number;
}

function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  return (
    d.toLocaleDateString() +
    " " +
    d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
  );
}

export function SourceMessages({ topicId, sourceCount }: SourceMessagesProps) {
  const [expanded, setExpanded] = useState(false);
  const [messages, setMessages] = useState<WikiSourceMessage[]>([]);
  const [loaded, setLoaded] = useState(false);

  const toggle = async () => {
    if (!expanded && !loaded) {
      try {
        const msgs = await getTopicSources(topicId, 50, 0);
        setMessages(msgs);
        setLoaded(true);
      } catch {
        // ignore
      }
    }
    setExpanded(!expanded);
  };

  return (
    <div className="source-messages">
      <button className="source-toggle" onClick={toggle}>
        {expanded ? "\u25B2" : "\u25BC"} View {sourceCount} source messages
      </button>
      {expanded && (
        <div className="source-list">
          {messages.map((msg, i) => (
            <div
              key={`${msg.chat_id}-${msg.message_id}`}
              className="source-item"
            >
              <div className="source-item-header">
                <span className="source-index">[{i + 1}]</span>
                <span className="source-chat">{msg.chat_title}</span>
                <span className="source-time">{formatTime(msg.timestamp)}</span>
              </div>
              <div className="source-item-text">{msg.text_plain}</div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
