import { invoke } from "@tauri-apps/api/core";
import type {
  Genre,
  Entry,
  EntrySummary,
  CreateEntryRequest,
  UpdateEntryRequest,
  SearchQuery,
  ImportResult,
} from "./types";

export type { Genre, Entry, EntrySummary, CreateEntryRequest, UpdateEntryRequest, SearchQuery };

export async function getGenres(): Promise<Genre[]> {
  return invoke("get_genres");
}

export async function createGenre(name: string): Promise<Genre> {
  return invoke("create_genre", { name });
}

export async function deleteGenre(id: string): Promise<void> {
  return invoke("delete_genre", { id });
}

export async function getEntries(query: SearchQuery): Promise<EntrySummary[]> {
  return invoke("get_entries", { query });
}

export async function getEntry(id: string): Promise<Entry> {
  return invoke("get_entry", { id });
}

export async function createEntry(req: CreateEntryRequest): Promise<Entry> {
  return invoke("create_entry", { req });
}

export async function updateEntry(req: UpdateEntryRequest): Promise<Entry> {
  return invoke("update_entry", { req });
}

export async function deleteEntries(ids: string[]): Promise<void> {
  return invoke("delete_entries", { ids });
}

export async function getEntriesCount(): Promise<number> {
  return invoke("get_entries_count");
}

export async function getTags(): Promise<string[]> {
  return invoke("get_all_tags");
}

export async function getTastingYears(): Promise<number[]> {
  return invoke("get_tasting_years");
}

export async function addEntryImage(
  entryId: string,
  path: string,
  isPrimary: boolean
): Promise<void> {
  return invoke("add_entry_image", {
    entryId,
    path,
    isPrimary,
  });
}

export async function deleteEntryImage(id: string): Promise<void> {
  return invoke("delete_entry_image", { id });
}

export async function setPrimaryImage(id: string): Promise<void> {
  return invoke("set_primary_image", { id });
}

export async function exportEntries(
  ids: string[] | null,
  format: string,
  includeImages: boolean
): Promise<string> {
  return invoke("export_entries", {
    ids,
    format,
    includeImages,
  });
}

export async function backupDatabase(): Promise<string> {
  return invoke("backup_database");
}

export async function importDatabase(sourcePath: string): Promise<void> {
  return invoke("import_database", { sourcePath });
}

export async function importEntries(path: string, format: string): Promise<ImportResult> {
  return invoke("import_entries", { path, format });
}

// ============================================================================
// 封面爬取
// ============================================================================

export interface CoverSource {
  id: string;
  name: string;
  source_type: string;
  usage: string;
}

export interface CoverCandidate {
  url: string;
  thumbnail_url: string | null;
  title: string | null;
  source: string;
  width: number | null;
  height: number | null;
}

export async function getCoverSources(): Promise<CoverSource[]> {
  return invoke("get_cover_sources");
}

export async function fetchCoverCandidates(
  title: string,
  creator: string | null,
  sourceId: string
): Promise<CoverCandidate[]> {
  return invoke("fetch_cover_candidates", { title, creator, sourceId });
}

export async function downloadCover(
  url: string,
  title: string,
  creator: string | null
): Promise<string> {
  return invoke("download_cover", { url, title, creator });
}

export async function getImageBase64(path: string): Promise<string> {
  return invoke("get_image_base64", { path });
}

export async function importLocalImage(
  sourcePath: string,
  title: string,
  creator: string | null
): Promise<string> {
  return invoke("import_local_image", { sourcePath, title, creator });
}
