import { useState, useEffect } from "react";
import { getChats } from "../api/tauri";
import type { ChatRow } from "../types";

interface ChannelSelectorProps {
  value: number | undefined;
  onChange: (chatId: number | undefined) => void;
}

export function ChannelSelector({ value, onChange }: ChannelSelectorProps) {
  const [chats, setChats] = useState<ChatRow[]>([]);

  useEffect(() => {
    getChats()
      .then(setChats)
      .catch((err) => console.error("Failed to load chats:", err));
  }, []);

  return (
    <select
      className="channel-selector"
      value={value ?? ""}
      onChange={(e) => {
        const val = e.target.value;
        onChange(val ? Number(val) : undefined);
      }}
    >
      <option value="">전체 채팅</option>
      {chats.map((chat) => (
        <option key={chat.chat_id} value={chat.chat_id}>
          {chat.title}
        </option>
      ))}
    </select>
  );
}
