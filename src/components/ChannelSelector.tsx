import { useState, useEffect, useRef, useCallback } from "react";
import { getChats, setChatExcluded } from "../api/tauri";
import type { ChatRow } from "../types";

interface ChannelSelectorProps {
  value: number | undefined;
  onChange: (chatId: number | undefined) => void;
  onExclusionChange?: () => void;
}

export function ChannelSelector({ value, onChange, onExclusionChange }: ChannelSelectorProps) {
  const [chats, setChats] = useState<ChatRow[]>([]);
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const [highlightIdx, setHighlightIdx] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const loadChats = useCallback(() => {
    getChats()
      .then(setChats)
      .catch((err) => console.error("Failed to load chats:", err));
  }, []);

  useEffect(() => {
    loadChats();
  }, [loadChats]);

  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, []);

  const toggleExcluded = async (e: React.MouseEvent, chatId: number, currentlyExcluded: boolean) => {
    e.stopPropagation();
    const newExcluded = !currentlyExcluded;
    try {
      await setChatExcluded(chatId, newExcluded);
      setChats((prev) =>
        prev.map((c) =>
          c.chat_id === chatId ? { ...c, is_excluded: newExcluded } : c,
        ),
      );
      onExclusionChange?.();
    } catch (err) {
      console.error("Failed to toggle channel:", err);
    }
  };

  const selectedTitle = chats.find((c) => c.chat_id === value)?.title ?? "전체 채팅";

  const lowerFilter = filter.toLowerCase();
  const filtered = filter
    ? chats.filter((c) => c.title.toLowerCase().includes(lowerFilter))
    : chats;

  const allOption = { chat_id: undefined as number | undefined, title: "전체 채팅", is_excluded: false, isAll: true };
  const sortedFiltered = [...filtered].sort((a, b) => {
    if (a.is_excluded !== b.is_excluded) return a.is_excluded ? -1 : 1;
    return 0;
  });
  const options = [allOption, ...sortedFiltered.map((c) => ({ chat_id: c.chat_id as number | undefined, title: c.title, is_excluded: c.is_excluded, isAll: false }))];

  const select = (chatId: number | undefined) => {
    onChange(chatId);
    setOpen(false);
    setFilter("");
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlightIdx((i) => Math.min(i + 1, options.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlightIdx((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (options[highlightIdx]) {
        select(options[highlightIdx].chat_id);
      }
    } else if (e.key === "Escape") {
      e.stopPropagation();
      setOpen(false);
    }
  };

  useEffect(() => {
    if (!open || !listRef.current) return;
    const items = listRef.current.children;
    if (items[highlightIdx]) {
      (items[highlightIdx] as HTMLElement).scrollIntoView({ block: "nearest" });
    }
  }, [highlightIdx, open]);

  useEffect(() => {
    setHighlightIdx(0);
  }, [filter]);

  return (
    <div className="channel-selector-wrap" ref={containerRef}>
      <button
        className="channel-selector"
        onClick={() => {
          setOpen(!open);
          setFilter("");
          setTimeout(() => inputRef.current?.focus(), 0);
        }}
      >
        {selectedTitle}
        <span className="channel-selector-arrow">▾</span>
      </button>
      {open && (
        <div className="channel-dropdown">
          <input
            ref={inputRef}
            className="channel-filter"
            type="text"
            placeholder="채널 검색..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={handleKeyDown}
          />
          <div className="channel-list" ref={listRef}>
            {options.map((opt, i) => (
              <div
                key={opt.chat_id ?? "all"}
                className={
                  "channel-option" +
                  (opt.chat_id === value ? " selected" : "") +
                  (i === highlightIdx ? " highlighted" : "") +
                  (opt.is_excluded ? " excluded" : "")
                }
                onMouseEnter={() => setHighlightIdx(i)}
                onClick={() => select(opt.chat_id)}
              >
                {!opt.isAll && (
                  <input
                    type="checkbox"
                    className="channel-check"
                    checked={!opt.is_excluded}
                    onClick={(e) => toggleExcluded(e, opt.chat_id!, opt.is_excluded)}
                    onChange={() => {}}
                  />
                )}
                <span className="channel-option-title">{opt.title}</span>
              </div>
            ))}
            {options.length === 0 && (
              <div className="channel-option empty">결과 없음</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
