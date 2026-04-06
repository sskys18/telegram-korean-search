import { useState, useCallback } from "react";
import type { SearchItem } from "../types";
import { applyHighlights } from "../utils/highlight";
import { formatTimestamp } from "../utils/format";
import {
  MessageModal,
  shouldSkipModal,
  openInTelegram,
} from "./MessageModal";

interface ResultItemProps {
  item: SearchItem;
}

export function ResultItem({ item }: ResultItemProps) {
  const [showModal, setShowModal] = useState(false);
  const segments = applyHighlights(item.text, item.highlights);

  const handleClick = useCallback(() => {
    if (shouldSkipModal()) {
      openInTelegram(item.link);
    } else {
      setShowModal(true);
    }
  }, [item.link]);

  return (
    <>
      <div
        className="result-item"
        onClick={handleClick}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => e.key === "Enter" && handleClick()}
      >
        <div className="result-header">
          <span className="result-chat">{item.chat_title}</span>
          <span className="result-time">{formatTimestamp(item.timestamp)}</span>
        </div>
        <div className="result-text">
          {segments.map((seg, i) =>
            seg.highlighted ? (
              <mark key={i}>{seg.text}</mark>
            ) : (
              <span key={i}>{seg.text}</span>
            ),
          )}
        </div>
      </div>
      {showModal && (
        <MessageModal item={item} onClose={() => setShowModal(false)} />
      )}
    </>
  );
}
