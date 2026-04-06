import { useEffect, useRef } from "react";
import { open } from "@tauri-apps/plugin-shell";
import type { SearchItem } from "../types";
import { formatTimestamp } from "../utils/format";

const SKIP_MODAL_KEY = "skipMessageModal";

interface MessageModalProps {
  item: SearchItem;
  onClose: () => void;
}

export function shouldSkipModal(): boolean {
  return localStorage.getItem(SKIP_MODAL_KEY) === "1";
}

export function openInTelegram(link: string | null) {
  if (link) {
    open(link).catch((err) => console.error("Failed to open link:", err));
  }
}

export function MessageModal({ item, onClose }: MessageModalProps) {
  const checkboxRef = useRef<HTMLInputElement>(null);
  const backdropRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === backdropRef.current) onClose();
  };

  const handleOpenTelegram = () => {
    if (checkboxRef.current?.checked) {
      localStorage.setItem(SKIP_MODAL_KEY, "1");
    }
    openInTelegram(item.link);
    onClose();
  };

  return (
    <div className="modal-backdrop" ref={backdropRef} onClick={handleBackdropClick}>
      <div className="modal-content">
        <div className="modal-header">
          <span className="modal-chat">{item.chat_title}</span>
          <span className="modal-time">{formatTimestamp(item.timestamp)}</span>
          <button className="modal-close" onClick={onClose}>
            &times;
          </button>
        </div>
        <div className="modal-body">{item.text}</div>
        <div className="modal-footer">
          {item.link && (
            <button className="modal-open-btn" onClick={handleOpenTelegram}>
              Open in Telegram
            </button>
          )}
          <label className="modal-checkbox">
            <input type="checkbox" ref={checkboxRef} />
            <span>Don't show next time</span>
          </label>
        </div>
      </div>
    </div>
  );
}
