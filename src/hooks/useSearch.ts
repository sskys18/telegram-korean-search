import { useState, useRef, useCallback } from "react";
import { searchMessages } from "../api/tauri";
import type { SearchItem, Cursor } from "../types";

const DEBOUNCE_MS = 200;

export function useSearch() {
  const [query, setQuery] = useState("");
  const [chatId, setChatId] = useState<number | undefined>(undefined);
  const [items, setItems] = useState<SearchItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [nextCursor, setNextCursor] = useState<Cursor | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const doSearch = useCallback(
    async (q: string, scopeChatId?: number, cursor?: Cursor) => {
      if (!q.trim()) {
        setItems([]);
        setNextCursor(null);
        return;
      }
      setLoading(true);
      try {
        const result = await searchMessages({
          query: q,
          chat_id: scopeChatId,
          cursor,
        });
        if (cursor) {
          setItems((prev) => [...prev, ...result.items]);
        } else {
          setItems(result.items);
        }
        setNextCursor(result.next_cursor);
      } catch (err) {
        console.error("Search failed:", err);
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  const handleQueryChange = useCallback(
    (value: string) => {
      setQuery(value);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        doSearch(value, chatId);
      }, DEBOUNCE_MS);
    },
    [chatId, doSearch],
  );

  const handleChatChange = useCallback(
    (id: number | undefined) => {
      setChatId(id);
      if (query.trim()) {
        doSearch(query, id);
      }
    },
    [query, doSearch],
  );

  const loadMore = useCallback(() => {
    if (nextCursor && !loading) {
      doSearch(query, chatId, nextCursor);
    }
  }, [nextCursor, loading, query, chatId, doSearch]);

  return {
    query,
    chatId,
    items,
    loading,
    hasMore: nextCursor !== null,
    setQuery: handleQueryChange,
    setChatId: handleChatChange,
    loadMore,
  };
}
