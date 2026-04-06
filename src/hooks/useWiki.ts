import { useCallback, useEffect, useState } from "react";
import type {
  WikiCategory,
  WikiPage,
  WikiSearchResult,
  WikiSourceMessage,
  WikiTopic,
  WikiTopicDetail,
} from "../types";
import {
  generateTopicSummary,
  getTopicDetail,
  getTopicSources,
  getTrendingTopics,
  getWikiCategories,
  searchWiki as searchWikiApi,
} from "../api/tauri";

export function useWiki() {
  const [categories, setCategories] = useState<WikiCategory[]>([]);
  const [topics, setTopics] = useState<WikiTopic[]>([]);
  const [selectedTopic, setSelectedTopic] = useState<WikiTopicDetail | null>(null);
  const [selectedSources, setSelectedSources] = useState<WikiSourceMessage[]>([]);
  const [categoryId, setCategoryId] = useState<number | undefined>(undefined);
  const [searchResults, setSearchResults] = useState<WikiSearchResult>({
    topics: [],
    pages: [],
  });
  const [searching, setSearching] = useState(false);
  const [loading, setLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadTrending = useCallback(async (nextCategoryId?: number) => {
    const nextTopics = await getTrendingTopics(nextCategoryId);
    setTopics(nextTopics);
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const [loadedCategories, loadedTopics] = await Promise.all([
          getWikiCategories(),
          getTrendingTopics(),
        ]);
        setCategories(loadedCategories);
        setTopics(loadedTopics);
      } catch (err) {
        setError(String(err).replace(/^Error:\s*/i, ""));
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  const selectTopic = useCallback(async (topicId: number) => {
    setDetailLoading(true);
    setError(null);
    try {
      const [detail, sources] = await Promise.all([
        getTopicDetail(topicId),
        getTopicSources(topicId),
      ]);
      setSelectedTopic(detail);
      setSelectedSources(sources);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setDetailLoading(false);
    }
  }, []);

  const goBack = useCallback(() => {
    setSelectedTopic(null);
    setSelectedSources([]);
  }, []);

  const setCategory = useCallback(
    async (nextCategoryId?: number) => {
      setCategoryId(nextCategoryId);
      setLoading(true);
      setError(null);
      try {
        await loadTrending(nextCategoryId);
        setSearchResults({ topics: [], pages: [] });
      } catch (err) {
        setError(String(err).replace(/^Error:\s*/i, ""));
      } finally {
        setLoading(false);
      }
    },
    [loadTrending],
  );

  const searchWiki = useCallback(
    async (query: string) => {
      const trimmed = query.trim();
      if (!trimmed) {
        setSearchResults({ topics: [], pages: [] });
        return;
      }

      setSearching(true);
      setError(null);
      try {
        const results = await searchWikiApi(trimmed, categoryId);
        setSearchResults(results);
      } catch (err) {
        setError(String(err).replace(/^Error:\s*/i, ""));
      } finally {
        setSearching(false);
      }
    },
    [categoryId],
  );

  const refreshTrending = useCallback(async () => {
    setLoading(true);
    try {
      await loadTrending(categoryId);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setLoading(false);
    }
  }, [categoryId, loadTrending]);

  const refreshSelectedTopic = useCallback(async () => {
    if (!selectedTopic) {
      return;
    }
    await selectTopic(selectedTopic.topic.topic_id);
  }, [selectTopic, selectedTopic]);

  const generateSummary = useCallback(async (): Promise<WikiPage | null> => {
    if (!selectedTopic) {
      return null;
    }

    setGenerating(true);
    try {
      const page = await generateTopicSummary(selectedTopic.topic.topic_id);
      setSelectedTopic((prev) => (prev ? { ...prev, page } : prev));
      await refreshTrending();
      return page;
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
      return null;
    } finally {
      setGenerating(false);
    }
  }, [refreshTrending, selectedTopic]);

  return {
    categories,
    topics,
    selectedTopic,
    selectedSources,
    categoryId,
    searchResults,
    searching,
    loading,
    detailLoading,
    generating,
    error,
    selectTopic,
    goBack,
    setCategory,
    searchWiki,
    refreshTrending,
    refreshSelectedTopic,
    generateSummary,
  };
}
