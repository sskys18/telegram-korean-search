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
