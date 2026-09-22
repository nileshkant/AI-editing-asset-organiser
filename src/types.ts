/**
 * Represents a sound library source folder.
 */
export type Source = {
  id: string;
  name: string;
  root: string;
  generation: number;
  available: boolean;
};

/**
 * Technical profile of an analyzed audio file.
 */
export type Profile = {
  duration: number;
  sample_rate: number;
  channels: number;
  channel_layout?: string;
  channel_peaks?: number[];
  channel_rms?: number[];
  frames: number;
  peak: number;
  rms: number;
  description: string;
  tags: string[];
  waveform: [number, number][];
};

/**
 * Represents an individual sound in the library.
 */
export type Sound = {
  id: string;
  source_id: string;
  relative_path: string;
  title: string;
  content_hash: string;
  status: string;
  profile: Profile | null;
  user_tags: string[];
  comment: string;
  favorite: boolean;
};

/**
 * Progress report for a background job like import.
 */
export type Progress = {
  job_id: string;
  source_id: string;
  status: string;
  completed: number;
  total: number;
  reused: number;
  failed: number;
  current: string;
  errors: string[];
};

/**
 * An item in a search facet, representing a filter option and its count.
 */
export type FacetItem = {
  value: string;
  count: number;
};

/**
 * Search facets providing aggregated stats for filters.
 */
export type SearchFacets = {
  tags: FacetItem[];
  layouts: FacetItem[];
  durations: FacetItem[];
};

/**
 * Query parameters for searching sounds.
 */
export type SearchQuery = {
  text: string;
  source_ids?: string[];
  tags?: string[];
  favorites_only?: boolean;
  min_duration?: number | null;
  max_duration?: number | null;
  offset?: number;
  limit?: number | null;
};

/**
 * How the search query text was interpreted.
 */
export type Interpretation = {
  terms: string[];
  phrases?: string[];
  excluded: string[];
  min_duration: number | null;
  max_duration: number | null;
  corrected: string[];
};

/**
 * Result of a sound search.
 */
export type SearchResults = {
  items: Sound[];
  total: number;
  interpretation: Interpretation;
  facets?: SearchFacets;
};

/**
 * Application environment information.
 */
export type AppInfo = {
  version: string;
  data_directory: string;
  desktop: boolean;
  media_tools: boolean;
};

/**
 * The state of the audio player.
 */
export type PlaybackState = 'stopped' | 'playing' | 'paused' | 'finished' | 'error';

/**
 * Current status of playback.
 */
export type PlaybackStatus = {
  sound_id: string | null;
  state: PlaybackState;
  position_seconds: number;
  duration_seconds: number;
  volume: number;
  peak: number;
  error: string | null;
};

/**
 * Represents a bucket of audio samples for waveform rendering.
 */
export type ChannelBucket = {
  min: number;
  max: number;
  rms: number;
};

/**
 * Waveform data for detailed rendering.
 */
export type WaveformResponse = {
  channels: ChannelBucket[][];
  start_frame: number;
  end_frame: number;
  frames_per_point: number;
  total_frames: number;
  sample_rate: number;
  channels_count: number;
};

/**
 * Application page routing.
 */
export type Page = 'library' | 'favorites' | 'imports' | 'settings';

/**
 * Non-destructive clip recipe using source sample frame boundaries.
 */
export type ClipRecipe = {
  asset_id: string;
  asset_version_id: string;
  source_sample_rate_hz: number;
  start_frame: string;
  end_frame: string;
  channel_policy?: string;
  gain_db?: number;
  fade_in_ms?: number;
  fade_out_ms?: number;
};

/**
 * Saved virtual clip variant.
 */
export type Clip = {
  id: string;
  sound_id: string;
  name: string;
  recipe: ClipRecipe;
  revision: number;
  is_stale: boolean;
  stale_reason?: string | null;
  created_at: number;
  updated_at: number;
};
