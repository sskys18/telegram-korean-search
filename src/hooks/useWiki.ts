import { useCallback, useEffect, useState } from "react";
import type {
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
  searchWiki as searchWikiApi,
} from "../api/tauri";

export function useWiki() {
  const [topics, setTopics] = useState<WikiTopic[]>([]);
  const [selectedTopic, setSelectedTopic] = useState<WikiTopicDetail | null>(null);
  const [selectedSources, setSelectedSources] = useState<WikiSourceMessage[]>([]);
  const [searchResults, setSearchResults] = useState<WikiSearchResult>({
    topics: [],
    pages: [],
  });
  const [searching, setSearching] = useState(false);
  const [loading, setLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadTrending = useCallback(async () => {
    const nextTopics = await getTrendingTopics();
    setTopics(nextTopics);
  }, []);

  useEffect(() => {
    (async () => {
      try {
        await loadTrending();
      } catch (err) {
        setError(String(err).replace(/^Error:\s*/i, ""));
      } finally {
        setLoading(false);
      }
    })();
  }, [loadTrending]);

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

  const searchWiki = useCallback(async (query: string) => {
    const trimmed = query.trim();
    if (!trimmed) {
      setSearchResults({ topics: [], pages: [] });
      return;
    }

    setSearching(true);
    setError(null);
    try {
      const results = await searchWikiApi(trimmed);
      setSearchResults(results);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setSearching(false);
    }
  }, []);

  const refreshTrending = useCallback(async () => {
    setLoading(true);
    try {
      await loadTrending();
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setLoading(false);
    }
  }, [loadTrending]);

  const refreshAll = useCallback(async () => {
    setLoading(true);
    try {
      await loadTrending();
      setSearchResults({ topics: [], pages: [] });
      setSelectedTopic(null);
      setSelectedSources([]);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/i, ""));
    } finally {
      setLoading(false);
    }
  }, [loadTrending]);

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
    topics,
    selectedTopic,
    selectedSources,
    searchResults,
    searching,
    loading,
    detailLoading,
    generating,
    error,
    selectTopic,
    goBack,
    searchWiki,
    refreshAll,
    refreshTrending,
    refreshSelectedTopic,
    generateSummary,
  };
}
