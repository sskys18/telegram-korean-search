import { invoke } from "@tauri-apps/api/core";
import type { SearchResult, ChatRow, DbStats, SearchQuery } from "../types";

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
