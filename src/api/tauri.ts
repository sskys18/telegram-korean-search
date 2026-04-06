import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  SearchResult,
  ChatRow,
  DbStats,
  SearchQuery,
  ApiCredentials,
  ConnectResult,
  SignInResponse,
  CollectionProgress,
  WikiTopic,
  WikiPage,
  WikiCategory,
  WikiTopicDetail,
  WikiSearchResult,
  WikiStatus,
  WikiProgress,
  WikiSourceMessage,
  WikiWorkerError,
  WikiWorkerStopped,
} from "../types";

export async function searchMessages(
  params: SearchQuery,
): Promise<SearchResult> {
  return invoke("search_messages", { params });
}

export async function getChats(): Promise<ChatRow[]> {
  return invoke("get_chats");
}

export async function getDbStats(): Promise<DbStats> {
  return invoke("get_db_stats");
}

// Auth API

export async function getApiCredentials(): Promise<ApiCredentials | null> {
  return invoke("get_api_credentials");
}

export async function saveApiCredentials(
  api_id: number,
  api_hash: string,
): Promise<void> {
  return invoke("save_api_credentials", { apiId: api_id, apiHash: api_hash });
}

export async function connectTelegram(): Promise<ConnectResult> {
  return invoke("connect_telegram");
}

export async function requestLoginCode(phone: string): Promise<void> {
  return invoke("request_login_code", { phone });
}

export async function submitLoginCode(code: string): Promise<SignInResponse> {
  return invoke("submit_login_code", { code });
}

export async function submitPassword(password: string): Promise<void> {
  return invoke("submit_password", { password });
}

export async function startCollection(): Promise<void> {
  return invoke("start_collection");
}

// Event listeners

export function onCollectionProgress(
  cb: (e: CollectionProgress) => void,
): Promise<() => void> {
  return listen<CollectionProgress>("collection-progress", (event) =>
    cb(event.payload),
  );
}

export function onCollectionComplete(
  cb: (e: { chats: number }) => void,
): Promise<() => void> {
  return listen<{ chats: number }>("collection-complete", (event) =>
    cb(event.payload),
  );
}

export function onCollectionError(
  cb: (e: string) => void,
): Promise<() => void> {
  return listen<string>("collection-error", (event) => cb(event.payload));
}

function normalizeDate(value: number | string | null | undefined): string | null {
  if (value == null) {
    return null;
  }
  if (typeof value === "number") {
    return new Date(value * 1000).toISOString();
  }
  return value;
}

function normalizeTopic(topic: {
  topic_id: number;
  title: string;
  title_ko: string | null;
  category_id: number | null;
  category_name: string | null;
  category_name_ko: string | null;
  trending_score: number;
  message_count: number;
  channel_count: number;
  first_seen_at: number | string | null;
  last_seen_at: number | string | null;
  last_summary_at: number | string | null;
  updated_at: string;
}): WikiTopic {
  return {
    ...topic,
    first_seen_at: normalizeDate(topic.first_seen_at) ?? "",
    last_seen_at: normalizeDate(topic.last_seen_at) ?? "",
    last_summary_at: normalizeDate(topic.last_summary_at),
  };
}

function normalizePage(page: {
  page_id: number;
  topic_id: number;
  content_ko: string;
  content_en: string | null;
  source_count: number | null;
  source_hash: string | null;
  version: number;
  created_at: string;
}): WikiPage {
  return {
    ...page,
    content_en: page.content_en,
    source_count: page.source_count ?? 0,
    source_hash: page.source_hash ?? "",
  };
}

// Wiki API

export async function saveOpenaiApiKey(key: string): Promise<void> {
  return invoke("save_openai_api_key", { key });
}

export async function getOpenaiApiKey(): Promise<string | null> {
  return invoke("get_openai_api_key");
}

export async function validateOpenaiApiKey(key: string): Promise<boolean> {
  return invoke("validate_openai_api_key", { key });
}

export async function startWikiWorker(): Promise<void> {
  return invoke("start_wiki_worker");
}

export async function stopWikiWorker(): Promise<void> {
  return invoke("stop_wiki_worker");
}

export async function getWikiStatus(): Promise<WikiStatus> {
  const status = await invoke<{
    queue_pending: number;
    queue_processing: number;
    queue_done: number;
    queue_failed: number;
    queue_skipped: number;
    topics_count: number;
    is_running: boolean;
  }>("get_wiki_status");

  return {
    pending: status.queue_pending,
    processing: status.queue_processing,
    done: status.queue_done,
    failed: status.queue_failed,
    skipped: status.queue_skipped,
    topics_count: status.topics_count,
    is_running: status.is_running,
  };
}

export async function reprocessWiki(): Promise<void> {
  return invoke("reprocess_wiki");
}

export async function clearWikiData(): Promise<void> {
  return invoke("clear_wiki_data");
}

export async function getTrendingTopics(
  categoryId?: number,
  limit = 50,
): Promise<WikiTopic[]> {
  const topics = await invoke<
    Array<Parameters<typeof normalizeTopic>[0]>
  >("get_trending_topics", {
    limit,
    offset: 0,
    categoryId,
  });
  return topics.map(normalizeTopic);
}

export async function getWikiCategories(): Promise<WikiCategory[]> {
  return invoke("get_wiki_categories");
}

export async function getTopicDetail(topicId: number): Promise<WikiTopicDetail> {
  const detail = await invoke<{
    topic: Parameters<typeof normalizeTopic>[0];
    latest_page: Parameters<typeof normalizePage>[0] | null;
    source_count: number;
  }>("get_topic_detail", { topicId });

  return {
    topic: normalizeTopic(detail.topic),
    page: detail.latest_page ? normalizePage(detail.latest_page) : null,
    source_count: detail.source_count,
  };
}

export async function getTopicSources(
  topicId: number,
  limit = 50,
  offset = 0,
): Promise<WikiSourceMessage[]> {
  return invoke("get_topic_sources", { topicId, limit, offset });
}

export async function searchWiki(
  query: string,
  categoryId?: number,
): Promise<WikiSearchResult> {
  const result = await invoke<{
    topics: Array<Parameters<typeof normalizeTopic>[0]>;
    pages: WikiSearchResult["pages"];
  }>("search_wiki", { query, limit: 8 });

  const topics = result.topics.map(normalizeTopic);
  if (categoryId == null) {
    return { topics, pages: result.pages };
  }

  const allowedTopicIds = new Set(
    topics
      .filter((topic) => topic.category_id === categoryId)
      .map((topic) => topic.topic_id),
  );

  return {
    topics: topics.filter((topic) => allowedTopicIds.has(topic.topic_id)),
    pages: result.pages.filter((page) => allowedTopicIds.has(page.topic_id)),
  };
}

export async function generateTopicSummary(topicId: number): Promise<WikiPage> {
  const page = await invoke<Parameters<typeof normalizePage>[0]>(
    "generate_topic_summary",
    { topicId },
  );
  return normalizePage(page);
}

// Wiki event listeners

export function onWikiWorkerProgress(
  cb: (e: WikiProgress) => void,
): Promise<() => void> {
  return listen<WikiProgress>("wiki-worker-progress", (event) =>
    cb(event.payload),
  );
}

export function onWikiWorkerError(
  cb: (e: WikiWorkerError) => void,
): Promise<() => void> {
  return listen<WikiWorkerError>("wiki-worker-error", (event) =>
    cb(event.payload),
  );
}

export function onWikiWorkerStopped(
  cb: (e: WikiWorkerStopped) => void,
): Promise<() => void> {
  return listen<WikiWorkerStopped>("wiki-worker-stopped", (event) =>
    cb(event.payload),
  );
}
