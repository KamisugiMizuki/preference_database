export interface Genre {
  id: string;
  name: string;
  is_default: boolean;
  created_at: string;
}

export interface ExternalLink {
  id: string;
  entry_id: string;
  url: string;
  label: string;
}

export interface EntryImage {
  id: string;
  entry_id: string;
  path: string;
  is_primary: boolean;
}

export interface Entry {
  id: string;
  name: string;
  genre_id: string;
  creator: string | null;
  rating: string;
  review: string;
  tasting_date: string | null;
  links: ExternalLink[];
  tags: string[];
  images: EntryImage[];
  created_at: string;
  updated_at: string;
}

export interface EntrySummary {
  id: string;
  name: string;
  genre_name: string;
  rating: string;
  review_preview: string;
  primary_image: string | null;
  tags: string[];
}

export interface CreateEntryRequest {
  name: string;
  genre_id: string;
  creator: string | null;
  rating: string;
  review: string;
  tasting_date: string | null;
  links: ExternalLink[];
  tags: string[];
}

export interface UpdateEntryRequest extends CreateEntryRequest {
  id: string;
}

export interface SearchQuery {
  keyword: string | null;
  search_field: string | null;
  genre_ids: string[];
  ratings: string[];
  tag_filter: string[];
  year: number | null;
  sort_by: string;
  sort_order: string;
  offset: number;
  limit: number;
}
