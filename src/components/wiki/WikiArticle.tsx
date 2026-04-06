import type { WikiTopicDetail } from "../../types";
import { SourceMessages } from "./SourceMessages";

interface WikiArticleProps {
  detail: WikiTopicDetail;
  onBack: () => void;
  onGenerate: () => void;
  generating: boolean;
}

function formatTimeAgo(timestamp: string | null): string {
  if (!timestamp) return "never";
  const seconds = Math.floor((Date.now() - new Date(timestamp).getTime()) / 1000);
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

/**
 * Converts markdown content to HTML for rendering wiki articles.
 * Content is generated locally by the wiki worker from trusted sources,
 * and HTML entities are escaped before any other transformations.
 */
function markdownToHtml(md: string): string {
  return md
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/^### (.+)$/gm, "<h3>$1</h3>")
    .replace(/^## (.+)$/gm, "<h2>$1</h2>")
    .replace(/^# (.+)$/gm, "<h1>$1</h1>")
    .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
    .replace(/\*(.+?)\*/g, "<em>$1</em>")
    .replace(/^- (.+)$/gm, "<li>$1</li>")
    .replace(/(<li>.*<\/li>\n?)+/g, "<ul>$&</ul>")
    .replace(/\[(\d+)\]/g, '<sup class="citation">[$1]</sup>')
    .replace(/\n\n/g, "</p><p>")
    .replace(/\n/g, "<br>")
    .replace(/^/, "<p>")
    .replace(/$/, "</p>");
}

export function WikiArticle({ detail, onBack, onGenerate, generating }: WikiArticleProps) {
  const { topic, page, source_count } = detail;

  return (
    <div className="wiki-article">
      <button className="back-btn" onClick={onBack}>
        ← Back to Trending
      </button>

      <div className="article-header">
        <h1 className="article-title">{topic.title}</h1>
        {topic.title_ko && <div className="article-title-ko">{topic.title_ko}</div>}
        <div className="article-meta">
          {topic.category_name && <span className="topic-category-badge">{topic.category_name}</span>}
          <span>{source_count} sources</span>
          <span>Updated {formatTimeAgo(topic.last_summary_at)}</span>
        </div>
      </div>

      {page ? (
        <div className="article-content">
          {/* Content is sanitized via HTML entity escaping in markdownToHtml before rendering */}
          <div className="article-section" dangerouslySetInnerHTML={{
            __html: markdownToHtml(page.content_ko)
          }} />
          <hr className="article-divider" />
          <div className="article-section" dangerouslySetInnerHTML={{
            __html: markdownToHtml(page.content_en ?? "")
          }} />
        </div>
      ) : (
        <div className="article-empty">
          <p>No wiki article generated yet.</p>
          <button className="generate-btn" onClick={onGenerate} disabled={generating}>
            {generating ? "Generating..." : "Generate Summary"}
          </button>
        </div>
      )}

      <SourceMessages topicId={topic.topic_id} sourceCount={source_count} />
    </div>
  );
}
