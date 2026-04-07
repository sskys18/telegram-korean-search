import type { WikiSearchResult, WikiTopic } from "../../types";
import { TopicCard } from "./TopicCard";
import { WikiSearch } from "./WikiSearch";

interface TrendingDashboardProps {
  topics: WikiTopic[];
  searchResults: WikiSearchResult;
  loading: boolean;
  searching: boolean;
  onSearch: (query: string) => void;
  onSelectTopic: (topicId: number) => void;
}

export function TrendingDashboard({
  topics,
  searchResults,
  loading,
  searching,
  onSearch,
  onSelectTopic,
}: TrendingDashboardProps) {
  return (
    <div className="trending-dashboard">
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
