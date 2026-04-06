import { useEffect, useRef, useState } from "react";
import type { WikiSearchResult } from "../../types";

function renderSnippet(snippet: string) {
  const parts = snippet.split(/(<b>.*?<\/b>)/g).filter(Boolean);
  return parts.map((part, index) => {
    const match = part.match(/^<b>(.*)<\/b>$/);
    if (match) {
      return <mark key={index}>{match[1]}</mark>;
    }
    return <span key={index}>{part}</span>;
  });
}

interface WikiSearchProps {
  results: WikiSearchResult;
  loading: boolean;
  onSearch: (query: string) => void;
  onSelectTopic: (topicId: number) => void;
}

export function WikiSearch({
  results,
  loading,
  onSearch,
  onSelectTopic,
}: WikiSearchProps) {
  const [query, setQuery] = useState("");
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    if (timerRef.current) {
      window.clearTimeout(timerRef.current);
    }
    timerRef.current = window.setTimeout(() => {
      onSearch(query);
    }, 300);

    return () => {
      if (timerRef.current) {
        window.clearTimeout(timerRef.current);
      }
    };
  }, [onSearch, query]);

  const hasResults = results.topics.length > 0 || results.pages.length > 0;

  return (
    <div className="wiki-search">
      <input
        type="text"
        className="search-input wiki-search-input"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search wiki topics and summaries..."
        spellCheck={false}
      />
      {(loading || query.trim() || hasResults) && (
        <div className="wiki-search-results">
          {loading && <div className="wiki-search-state">Searching...</div>}
          {!loading && !hasResults && query.trim() && (
            <div className="wiki-search-state">No matching wiki results.</div>
          )}
          {!loading &&
            results.topics.map((topic) => (
              <button
                key={`topic-${topic.topic_id}`}
                type="button"
                className="wiki-search-result"
                onClick={() => onSelectTopic(topic.topic_id)}
              >
                <div className="wiki-search-result-title">
                  {topic.title_ko || topic.title}
                </div>
                <div className="wiki-search-result-subtitle">Topic</div>
              </button>
            ))}
          {!loading &&
            results.pages.map((page) => (
              <button
                key={`page-${page.topic_id}`}
                type="button"
                className="wiki-search-result"
                onClick={() => onSelectTopic(page.topic_id)}
              >
                <div className="wiki-search-result-title">{page.topic_title}</div>
                <div className="wiki-search-result-snippet">
                  {renderSnippet(page.snippet)}
                </div>
              </button>
            ))}
        </div>
      )}
    </div>
  );
}
