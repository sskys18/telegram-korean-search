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
      <div className="result-empty">메시지를 검색하세요</div>
    );
  }

  if (items.length === 0 && !loading) {
    return <div className="result-empty">검색 결과가 없습니다</div>;
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
      {loading && <div className="result-loading">로딩 중...</div>}
    </div>
  );
}
