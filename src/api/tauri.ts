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
