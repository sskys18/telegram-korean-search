import { useWiki } from "../hooks/useWiki";
import { useWikiWorker } from "../hooks/useWikiWorker";
import { TrendingDashboard } from "../components/wiki/TrendingDashboard";
import { WikiArticle } from "../components/wiki/WikiArticle";
import { WikiSettings } from "../components/wiki/WikiSettings";

export function WikiPage() {
  const wiki = useWiki();
  const worker = useWikiWorker();

  return (
    <div className="wiki-page">
      <WikiSettings worker={worker} />
      {wiki.selectedTopic ? (
        <WikiArticle
          detail={wiki.selectedTopic}
          onBack={wiki.goBack}
          onGenerate={() => wiki.generateSummary()}
          generating={wiki.generating}
        />
      ) : (
        <TrendingDashboard
          topics={wiki.topics}
          categories={wiki.categories}
          selectedCategory={wiki.categoryId}
          searchQuery=""
          searchResults={wiki.searchResults}
          loading={wiki.loading}
          onSelectCategory={wiki.setCategory}
          onSelectTopic={wiki.selectTopic}
          onSearch={wiki.searchWiki}
          onRefresh={wiki.refreshTrending}
        />
      )}
    </div>
  );
}
