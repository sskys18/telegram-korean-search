import type { WikiTopic } from "../../types";

function parseDate(value: string): number | null {
  if (!value) {
    return null;
  }
  if (/^\d+$/.test(value)) {
    return Number(value) * 1000;
  }
  const parsed = Date.parse(value.replace(" ", "T"));
  return Number.isNaN(parsed) ? null : parsed;
}

function formatRelativeTime(value: string): string {
  const timestamp = parseDate(value);
  if (timestamp == null) {
    return "Unknown";
  }
  const diffMs = Date.now() - timestamp;
  const diffMinutes = Math.max(1, Math.round(diffMs / 60000));
  if (diffMinutes < 60) {
    return `${diffMinutes}m ago`;
  }
  const diffHours = Math.round(diffMinutes / 60);
  if (diffHours < 24) {
    return `${diffHours}h ago`;
  }
  const diffDays = Math.round(diffHours / 24);
  return `${diffDays}d ago`;
}

interface TopicCardProps {
  topic: WikiTopic;
  onSelect: (topicId: number) => void;
}

export function TopicCard({ topic, onSelect }: TopicCardProps) {
  return (
    <button
      type="button"
      className="topic-card"
      onClick={() => onSelect(topic.topic_id)}
    >
      <div className="topic-card-header">
        <h3 className="topic-card-title">{topic.title_ko || topic.title}</h3>
        {topic.category_name && (
          <span className="topic-card-badge">
            {topic.category_name_ko || topic.category_name}
          </span>
        )}
      </div>
      <div className="topic-card-subtitle">{topic.title_ko ? topic.title : ""}</div>
      <div className="topic-card-stats">
        <span>{Math.round(topic.trending_score * 100)}% trending</span>
        <span>{topic.message_count} messages</span>
        <span>{topic.channel_count} channels</span>
        <span>{formatRelativeTime(topic.last_seen_at)}</span>
      </div>
    </button>
  );
}
