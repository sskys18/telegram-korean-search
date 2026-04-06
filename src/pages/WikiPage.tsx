import { TrendingDashboard } from "../components/wiki/TrendingDashboard";
import { WikiArticle } from "../components/wiki/WikiArticle";
import { WikiSettings } from "../components/wiki/WikiSettings";
import { useWiki } from "../hooks/useWiki";
import { useWikiWorker } from "../hooks/useWikiWorker";

export function WikiPage() {
  const wiki = useWiki();
  const worker = useWikiWorker();

  return (
    <div className="wiki-page">
      <WikiSettings
        apiKey={worker.apiKey}
        setApiKey={worker.setApiKey}
        savedKeyMask={worker.savedKeyMask}
        status={worker.status}
        progress={worker.progress}
        loading={worker.loading}
        busy={worker.busy}
        validating={worker.validating}
        error={worker.error}
        validationMessage={worker.validationMessage}
        onSaveApiKey={worker.saveApiKey}
        onValidateApiKey={worker.validateApiKey}
        onStartWorker={worker.startWorker}
        onStopWorker={worker.stopWorker}
        onReprocessWiki={worker.reprocessWiki}
        onClearWikiData={worker.clearWikiData}
      />

      {wiki.error && <div className="wiki-banner-error">{wiki.error}</div>}

      {wiki.selectedTopic ? (
        <WikiArticle
          detail={wiki.selectedTopic}
          sources={wiki.selectedSources}
          loading={wiki.detailLoading}
          generating={wiki.generating}
          onBack={wiki.goBack}
          onGenerateSummary={async () => {
            await wiki.generateSummary();
            await wiki.refreshSelectedTopic();
          }}
        />
      ) : (
        <TrendingDashboard
          categories={wiki.categories}
          categoryId={wiki.categoryId}
          topics={wiki.topics}
          searchResults={wiki.searchResults}
          loading={wiki.loading}
          searching={wiki.searching}
          onCategoryChange={(categoryId) => {
            void wiki.setCategory(categoryId);
          }}
          onSearch={(query) => {
            void wiki.searchWiki(query);
          }}
          onSelectTopic={(topicId) => {
            void wiki.selectTopic(topicId);
          }}
        />
      )}
    </div>
  );
}
