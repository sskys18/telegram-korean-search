import type { WikiCategory, WikiSearchResult, WikiTopic } from "../../types";
import { CategoryFilter } from "./CategoryFilter";
import { TopicCard } from "./TopicCard";
import { WikiSearch } from "./WikiSearch";

interface TrendingDashboardProps {
  categories: WikiCategory[];
  categoryId?: number;
  topics: WikiTopic[];
  searchResults: WikiSearchResult;
  loading: boolean;
  searching: boolean;
  onCategoryChange: (categoryId?: number) => void;
  onSearch: (query: string) => void;
  onSelectTopic: (topicId: number) => void;
}

export function TrendingDashboard({
  categories,
  categoryId,
  topics,
  searchResults,
  loading,
  searching,
  onCategoryChange,
  onSearch,
  onSelectTopic,
}: TrendingDashboardProps) {
  return (
    <div className="trending-dashboard">
      <CategoryFilter
        categories={categories}
        activeCategoryId={categoryId}
        onChange={onCategoryChange}
      />
      <WikiSearch
        results={searchResults}
        loading={searching}
        onSearch={onSearch}
        onSelectTopic={onSelectTopic}
      />
      <div className="wiki-section-header">
        <h2>Trending Topics</h2>
        <span>{topics.length}</span>
      </div>
      <div className="wiki-topic-list">
        {loading ? (
          <div className="wiki-empty">Loading trending topics...</div>
        ) : topics.length === 0 ? (
          <div className="wiki-empty">No wiki topics yet.</div>
        ) : (
          topics.map((topic) => (
            <TopicCard
              key={topic.topic_id}
              topic={topic}
              onSelect={onSelectTopic}
            />
          ))
        )}
      </div>
    </div>
  );
}
