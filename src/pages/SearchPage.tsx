import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { SearchBar } from "../components/SearchBar";
import { ChannelSelector } from "../components/ChannelSelector";
import { ResultList } from "../components/ResultList";
import { useSearch } from "../hooks/useSearch";
import type { CollectionProgress } from "../types";

interface SearchPageProps {
  syncing: boolean;
  progress: CollectionProgress | null;
}

export function SearchPage({ syncing, progress }: SearchPageProps) {
  const { query, chatId, items, loading, hasMore, setQuery, setChatId, loadMore } =
    useSearch();

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        getCurrentWindow().hide();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  return (
    <div className="search-page">
      <div className="search-header">
        <SearchBar value={query} onChange={setQuery} loading={loading} />
        <ChannelSelector
          value={chatId}
          onChange={setChatId}
          key={syncing ? "s" : "d"}
        />
      </div>
      {syncing && (
        <div className="sync-bar">
          <span className="sync-dot" />
          <span className="sync-text">
            {progress?.phase === "chats"
              ? "Syncing chats..."
              : progress?.chat_title
                ? `Syncing: ${progress.chat_title} (${(progress.chats_done ?? 0) + 1}/${progress.chats_total})`
                : "Syncing messages..."}
          </span>
        </div>
      )}
      <ResultList
        items={items}
        loading={loading}
        hasMore={hasMore}
        loadMore={loadMore}
        query={query}
      />
    </div>
  );
}
