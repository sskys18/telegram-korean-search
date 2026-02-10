import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { SearchBar } from "../components/SearchBar";
import { ChannelSelector } from "../components/ChannelSelector";
import { ResultList } from "../components/ResultList";
import { useSearch } from "../hooks/useSearch";

export function SearchPage() {
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
        <ChannelSelector value={chatId} onChange={setChatId} />
      </div>
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
