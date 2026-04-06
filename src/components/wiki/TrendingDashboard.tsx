import type { WikiTopic, WikiCategory, WikiSearchResult } from "../../types";
import { CategoryFilter } from "./CategoryFilter";
import { TopicCard } from "./TopicCard";
import { WikiSearch } from "./WikiSearch";

interface TrendingDashboardProps {
  topics: WikiTopic[];
  categories: WikiCategory[];
  selectedCategory: number | undefined;
  searchQuery: string;
  searchResults: WikiSearchResult | null;
  loading: boolean;
  onSelectCategory: (id: number | undefined) => void;
  onSelectTopic: (id: number) => void;
  onSearch: (query: string) => void;
  onRefresh: () => void;
}

export function TrendingDashboard({
  topics,
  categories,
  selectedCategory,
  searchQuery,
  searchResults,
  loading,
  onSelectCategory,
  onSelectTopic,
  onSearch,
  onRefresh,
}: TrendingDashboardProps) {
  const showSearch = searchResults && searchQuery.trim().length >= 2;

  return (
    <div className="trending-dashboard">
      <WikiSearch query={searchQuery} onSearch={onSearch} />
      <CategoryFilter
        categories={categories}
        selected={selectedCategory}
        onSelect={onSelectCategory}
      />

      {showSearch ? (
        <div className="search-results-section">
          <h3 className="section-title">Search Results</h3>
          {searchResults.topics.length === 0 && searchResults.pages.length === 0 ? (
            <div className="empty-state">No results found</div>
          ) : (
            <>
              {searchResults.topics.map((t, i) => (
                <TopicCard
                  key={t.topic_id}
                  topic={t}
                  rank={i + 1}
                  onClick={() => onSelectTopic(t.topic_id)}
                />
              ))}
              {searchResults.pages.map((p) => (
                <div
                  key={`page-${p.topic_id}`}
                  className="topic-card"
                  onClick={() => onSelectTopic(p.topic_id)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => e.key === "Enter" && onSelectTopic(p.topic_id)}
                >
                  <div className="topic-card-header">
                    <span className="topic-title">{p.topic_title}</span>
                  </div>
                  <div className="topic-card-meta">
                    {/* Snippet contains pre-sanitized highlight HTML from backend */}
                    <span dangerouslySetInnerHTML={{ __html: p.snippet }} />
                  </div>
                </div>
              ))}
            </>
          )}
        </div>
      ) : (
        <div className="trending-section">
          <div className="section-header">
            <h3 className="section-title">Trending</h3>
            <button className="refresh-btn" onClick={onRefresh}>Refresh</button>
          </div>
          {loading ? (
            <div className="empty-state">Loading...</div>
          ) : topics.length === 0 ? (
            <div className="empty-state">No topics yet. Collect messages and start the wiki worker.</div>
          ) : (
            <div className="topic-list">
              {topics.map((t, i) => (
                <TopicCard
                  key={t.topic_id}
                  topic={t}
                  rank={i + 1}
                  onClick={() => onSelectTopic(t.topic_id)}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
