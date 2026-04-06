export interface HighlightRange {
  start: number;
  end: number;
}

export interface SearchItem {
  message_id: number;
  chat_id: number;
  timestamp: number;
  text: string;
  link: string | null;
  chat_title: string;
  highlights: HighlightRange[];
}

export interface Cursor {
  timestamp: number;
  chat_id: number;
  message_id: number;
}

export interface SearchResult {
  items: SearchItem[];
  next_cursor: Cursor | null;
}

export interface ChatRow {
  chat_id: number;
  title: string;
  chat_type: string;
  username: string | null;
  access_hash: number | null;
  is_excluded: boolean;
}

export interface DbStats {
  chats: number;
  messages: number;
}

export interface SearchQuery {
  query: string;
  chat_id?: number;
  cursor?: Cursor;
  limit?: number;
}

// Auth types

export interface ApiCredentials {
  api_id: number;
  api_hash: string;
}

export interface ConnectResult {
  authorized: boolean;
}

export interface SignInResponse {
  success: boolean;
  requires_2fa: boolean;
  hint: string | null;
}

export interface CollectionProgress {
  phase: "chats" | "messages";
  chat_title?: string;
  chats_done?: number;
  chats_total?: number;
  detail?: string;
  active_chats?: string[];
}

// Wiki types

export interface WikiTopic {
  topic_id: number;
  title: string;
  title_ko: string | null;
  category_id: number | null;
  category_name: string | null;
  category_name_ko: string | null;
  trending_score: number;
  message_count: number;
  channel_count: number;
  first_seen_at: string;
  last_seen_at: string;
  last_summary_at: string | null;
  updated_at: string;
}

export interface WikiPage {
  page_id: number;
  topic_id: number;
  content_ko: string;
  content_en: string | null;
  source_count: number;
  source_hash: string;
  version: number;
  created_at: string;
}

export interface WikiCategory {
  category_id: number;
  name: string;
  name_ko: string | null;
  sort_order: number;
}

export interface WikiTopicDetail {
  topic: WikiTopic;
  page: WikiPage | null;
  source_count: number;
}

export interface WikiSearchResult {
  topics: WikiTopic[];
  pages: { topic_id: number; topic_title: string; snippet: string }[];
}

export interface WikiStatus {
  pending: number;
  processing: number;
  done: number;
  failed: number;
  skipped: number;
  topics_count: number;
  is_running: boolean;
}

export interface WikiProgress {
  processed: number;
  total: number;
  queue_remaining: number;
}

export interface WikiSourceMessage {
  message_id: number;
  chat_id: number;
  timestamp: number;
  text_plain: string;
  link: string | null;
  chat_title: string;
}

export interface WikiWorkerError {
  message: string;
  recoverable: boolean;
}

export interface WikiWorkerStopped {
  reason: string;
}
