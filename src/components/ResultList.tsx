import type { SearchItem } from "../types";
import { ResultItem } from "./ResultItem";
import { useInfiniteScroll } from "../hooks/useInfiniteScroll";

interface ResultListProps {
  items: SearchItem[];
  loading: boolean;
  hasMore: boolean;
  loadMore: () => void;
  query: string;
}

export function ResultList({
  items,
  loading,
  hasMore,
  loadMore,
  query,
}: ResultListProps) {
  const sentinelRef = useInfiniteScroll(loadMore, hasMore, loading);

  if (!query.trim()) {
    return (
      <div className="result-empty">Type to search messages</div>
    );
  }

  if (items.length === 0 && !loading) {
    return <div className="result-empty">No results found</div>;
  }

  return (
    <div className="result-list">
      {items.map((item) => (
        <ResultItem
          key={`${item.chat_id}-${item.message_id}`}
          item={item}
        />
      ))}
      <div ref={sentinelRef} className="scroll-sentinel" />
      {loading && <div className="result-loading">Loading...</div>}
    </div>
  );
}
