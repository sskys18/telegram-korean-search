import { markdownToHtml } from "../../utils/markdown";
import type { WikiSourceMessage, WikiTopicDetail } from "../../types";
import { SourceMessages } from "./SourceMessages";

function formatDate(value: string | null): string {
  if (!value) {
    return "Never";
  }
  const parsed = Date.parse(value.replace(" ", "T"));
  if (Number.isNaN(parsed)) {
    return value;
  }
  return new Date(parsed).toLocaleString();
}

interface WikiArticleProps {
  detail: WikiTopicDetail;
  sources: WikiSourceMessage[];
  loading: boolean;
  generating: boolean;
  onBack: () => void;
  onGenerateSummary: () => Promise<void>;
}

export function WikiArticle({
  detail,
  sources,
  loading,
  generating,
  onBack,
  onGenerateSummary,
}: WikiArticleProps) {
  const page = detail.page;

  return (
    <div className="wiki-article">
      <div className="wiki-article-header">
        <button type="button" className="wiki-inline-button" onClick={onBack}>
          Back
        </button>
        <div className="wiki-article-title-block">
          <h2>{detail.topic.title_ko || detail.topic.title}</h2>
          {detail.topic.title_ko && (
            <div className="wiki-article-subtitle">{detail.topic.title}</div>
          )}
        </div>
        <button
          type="button"
          className="wiki-primary-button"
          disabled={generating || loading}
          onClick={() => {
            void onGenerateSummary();
          }}
        >
          {generating ? "Generating..." : page ? "Refresh Summary" : "Generate Summary"}
        </button>
      </div>

      <div className="wiki-metadata-row">
        <span>
          {detail.topic.category_name_ko ||
            detail.topic.category_name ||
            "Uncategorized"}
        </span>
        <span>{detail.source_count} sources</span>
        <span>{detail.topic.message_count} messages</span>
        <span>
          Updated {formatDate(detail.topic.last_summary_at || detail.topic.updated_at)}
        </span>
      </div>

      {loading ? (
        <div className="wiki-empty">Loading topic details...</div>
      ) : page ? (
        <div className="wiki-article-content">
          <section>
            <h3>한국어</h3>
            <div
              className="wiki-markdown"
              dangerouslySetInnerHTML={{ __html: markdownToHtml(page.content_ko) }}
            />
          </section>
          {page.content_en && (
            <section>
              <h3>English</h3>
              <div
                className="wiki-markdown"
                dangerouslySetInnerHTML={{ __html: markdownToHtml(page.content_en) }}
              />
            </section>
          )}
        </div>
      ) : (
        <div className="wiki-empty">
          No generated article yet. Generate a summary from the linked source
          messages.
        </div>
      )}

      <SourceMessages sources={sources} />
    </div>
  );
}
