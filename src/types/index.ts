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
  terms: number;
  postings: number;
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
}
