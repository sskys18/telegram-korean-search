import type { WikiTopic } from "../../types";

interface TopicCardProps {
  topic: WikiTopic;
  rank: number;
  onClick: () => void;
}

function formatTimeAgo(timestamp: string | null): string {
  if (!timestamp) return "";
  const seconds = Math.floor((Date.now() - new Date(timestamp).getTime()) / 1000);
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

export function TopicCard({ topic, rank, onClick }: TopicCardProps) {
  const trendPct = Math.min(Math.round(topic.trending_score * 100), 999);

  return (
    <div className="topic-card" onClick={onClick} role="button" tabIndex={0} onKeyDown={(e) => e.key === "Enter" && onClick()}>
      <div className="topic-card-header">
        <span className="topic-rank">{rank}.</span>
        <span className="topic-title">{topic.title}</span>
        {trendPct > 0 && <span className="topic-trend">+{trendPct}%</span>}
      </div>
      <div className="topic-card-meta">
        {topic.category_name && <span className="topic-category-badge">{topic.category_name}</span>}
        <span>{topic.message_count} msgs</span>
        <span>{topic.channel_count} channels</span>
        <span>{formatTimeAgo(topic.last_seen_at)}</span>
      </div>
      {topic.title_ko && <div className="topic-title-ko">{topic.title_ko}</div>}
    </div>
  );
}
