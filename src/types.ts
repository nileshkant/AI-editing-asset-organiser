/**
 * Represents a sound library source folder.
 */
export type Source = {
  id: string;
  name: string;
  root: string;
  generation: number;
  available: boolean;
  scope?: 'folder' | 'files';
  files?: { relative_path: string; sound_id: string | null; status: string }[];
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
  source_start_seconds?: number | null;
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
export type Page = 'library' | 'favorites' | 'images' | 'imports' | 'settings';

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

export type ExportOptions = {
  format: 'wav' | 'flac';
  sample_rate: number | null;
  fade_in_ms: number | null;
  fade_out_ms: number | null;
};
export type DestinationGrant = { id: string; path: string };
export type ExportResult = {
  path: string;
  manifest_path: string;
  frames: string;
  sound_id: string | null;
  warning: string | null;
};

export type ImageImportMode = 'images' | 'mixed';
export type ImageImportError = 'INVALID_ARGUMENT' | 'PERMISSION_DENIED' | 'SOURCE_CHANGED' | 'SOURCE_UNAVAILABLE' | 'FILE_MISSING' | 'SOURCE_CONFLICT' | 'EXISTING_CATALOG' | 'SOURCE_OVERLAP' | 'QUEUE_FULL' | 'LIMIT_EXCEEDED' | 'NOT_FOUND' | 'LEASE_LOST' | 'CANCELLED' | 'DISCOVERY_INCOMPLETE' | 'DECODE_FAILED' | 'WRONG_ASSET_KIND' | 'INACCESSIBLE' | 'UNSUPPORTED_ANIMATION' | 'UNSUPPORTED_COLOR_PROFILE' | 'AUDIO_DECODE_FAILED' | 'WORKER_UNAVAILABLE' | 'BUSY' | 'TIMED_OUT' | 'UNSUPPORTED_FORMAT' | 'FILTERED_MEDIA' | 'NO_ELIGIBLE_FILES' | 'NO_MATCHING_FILES' | 'CONFLICT' | 'INTERNAL';
export type ImageImportJob = {
  id: string; source_id: string; source_generation: string; sequence: string;
  mode: ImageImportMode; state: 'queued' | 'running' | 'complete' | 'failed' | 'cancelled';
  phase: 'discovery' | 'analysis' | 'complete'; discovery_complete: boolean; history_truncated: boolean;
  total: number; completed: number; ready: number; reused: number; failed: number; unsupported: number;
  current: string; error_code: ImageImportError | null;
  scan: 'discover' | 'listed'; verify: boolean; origin: 'import' | 'refresh' | 'watch' | 'relink';
};
export type MediaImportPreview = {
  id: string; request_id: string; mode: ImageImportMode;
  scopes: { root: string; recursive: boolean; files: string[] }[];
  images: number; audio: number; unsupported: number; inaccessible: number; complete: boolean;
  issues: { path: string; code: ImageImportError }[]; reference_in_place: boolean; expires_at: number;
};
export type ImageImportItem = {
  job_id: string; relative_path: string; media_kind: 'audio' | 'image' | 'unsupported';
  state: 'pending' | 'running' | 'ready' | 'failed' | 'unsupported'; asset_id: string | null;
  content_hash: string | null; error_code: ImageImportError | null; attempts: number;
  warnings: ('extension_mismatch' | 'assumed_srgb' | 'high_bit_depth_display8')[];
};
export type ImageImportItemPage = { items: ImageImportItem[]; next_after: string | null };
export type ImportedImagePage = { items: { header: import('./contracts/imageV1').ImageHeader; width: number | null; height: number | null; annotation_revision: number }[]; next_after: string | null };
/** SS-047 source lifecycle. Root state is the folder; asset availability is per image. */
export type ImageRootState = 'unknown' | 'online' | 'offline' | 'inaccessible' | 'changed';
export type ImageWatchState = { state: 'watching' | 'paused' | 'failed'; error: string | null };
export type ImageSourceStatus = {
  source_id: string; name: string; root: string; scope: 'folder' | 'files'; source_generation: string;
  root_state: ImageRootState; watch: boolean; mode: ImageImportMode; checked_at: number;
  images: number; missing: number; inaccessible: number; failed: number; pending: number;
  move_candidates: number; active_job: string | null; watch_state: ImageWatchState | null;
};
export type ImageRelinkPreview = {
  id: string; source_id: string; source_generation: string; root: string;
  members: number; present: number; absent: number; sampled: number; matched: number; changed: number; unreadable: number;
  complete: boolean; acknowledgement_required: boolean; expires_at: number;
};
export type ImageMoveCandidate = {
  missing_id: string; missing_path: string; missing_revision: number;
  present_id: string; present_path: string; present_revision: number; same_file: boolean;
};
export type ImageImportCapabilities = { enabled: boolean; reference_in_place: boolean; production_images: boolean; metadata_present: boolean };
export type ImagePreviewTier = 'small' | 'medium' | 'large' | 'inspect' | 'tile';
export type ImagePreviewError = 'INVALID_ARGUMENT' | 'NOT_FOUND' | 'STALE_VERSION' | 'NOT_READY' | 'ORIGINAL_UNAVAILABLE' | 'SOURCE_CHANGED' | 'CANCELLED' | 'BUSY' | 'TIMED_OUT' | 'LIMIT_EXCEEDED' | 'INVALID_DATA' | 'UNSUPPORTED' | 'PERMISSION_DENIED' | 'WORKER_UNAVAILABLE' | 'WORKER_FAILED' | 'INTERNAL';
/** SS-044 preview metadata. A cached preview never proves the original is available. */
export type ImagePreviewMeta = {
  asset_id: string; version: string; tier: ImagePreviewTier; tile: [number, number] | null;
  width: number; height: number; source_width: number; source_height: number;
  mime: 'image/png' | 'image/jpeg'; transparency: boolean; cache: 'hit' | 'generated' | 'uncached';
  original_available: boolean; offline_reference: boolean; preview_version: string;
};
export type ImagePreviewCacheStatus = {
  enabled: boolean; entries: number; bytes: number; quota_bytes: number; active: number; waiting: number;
  counters: { hits: number; misses: number; generated: number; coalesced: number; evicted: number; repaired: number; write_failures: number; cancelled: number; stale_rejected: number; offline_served: number };
};
export type ImageSummary = ImportedImagePage['items'][number];
export type GallerySort = 'name_asc' | 'name_desc';
/** SS-058 canonical image query (the core's `ImageQuery`, snake_case on the wire). */
export type ImageSort = 'relevance' | 'name_asc' | 'name_desc' | 'rating_desc' | 'added_desc' | 'added_asc' | 'pixels_desc';
export type TextField = 'name' | 'tags' | 'suggested_tags' | 'caption' | 'notes' | 'ocr' | 'generation' | 'usage_notes';
export type TagState = 'confirmed' | 'suggested' | 'any';
export type Orientation = 'landscape' | 'portrait' | 'square';
export type ImageQuery = {
  version?: number; text?: string; fields?: TextField[]; kinds?: ('image' | 'element')[]; source_ids?: string[];
  tags?: string[]; exclude_tags?: string[]; tag_state?: TagState; favorite?: boolean | null;
  min_rating?: number | null; max_rating?: number | null; rated?: boolean | null;
  min_width?: number | null; max_width?: number | null; min_height?: number | null; max_height?: number | null;
  orientations?: Orientation[]; alpha_channel?: boolean | null; transparency?: ('none' | 'binary' | 'partial')[];
  formats?: ('jpeg' | 'png' | 'webp')[]; technical?: import('./contracts/imageV1').TechnicalState[];
  availability?: import('./contracts/imageV1').Availability[]; ready_only?: boolean;
  enrichment?: import('./contracts/imageV1').EnrichmentState[]; review?: 'needs_review' | 'no_pending_suggestions' | null;
  provenance?: ('model' | 'filename' | 'imported' | 'user')[]; added_after?: number | null; added_before?: number | null; sort?: ImageSort;
};
export type SearchErrorCode = 'INVALID_ARGUMENT' | 'UNSUPPORTED_FIELD' | 'UNSUPPORTED_VERSION' | 'LIMIT_EXCEEDED' | 'NOT_FOUND' | 'CONFLICT' | 'INTERNAL';
export type SearchClause = { text: string; kind: 'term' | 'phrase' | 'prefix'; excluded: boolean; matches: number | null };
export type ResolvedTag = { requested: string; tag: string; via_alias: boolean };
export type SearchInterpretation = {
  clauses: SearchClause[]; fields: TextField[]; tags: ResolvedTag[]; exclude_tags: ResolvedTag[]; ignored: string[];
  suggestions: { term: string; candidates: string[] }[]; text_only_matches: number | null;
};
/** Why a result matched: tier 0 exact tag, 1 tags, 2 caption, 3 name, 4 notes, 5 across fields; null without text. */
export type HitExplanation = { id: string; tier: number | null; fields: TextField[] };
export type FacetCount = { value: string; count: number };
export type ImageFacets = {
  tags: FacetCount[]; suggested_tags: FacetCount[]; orientations: FacetCount[]; transparency: FacetCount[]; ratings: FacetCount[];
  formats: FacetCount[]; availability: FacetCount[]; technical: FacetCount[]; sources: FacetCount[]; favorites: number; needs_review: number;
};
export type ImageSearchPage = {
  items: ImageSummary[]; explanations: HitExplanation[]; offset: number; total: number;
  interpretation: SearchInterpretation; facets: ImageFacets | null;
};
export type FieldCapability = { field: TextField; available: boolean; owner: string | null };
/** `query` is null when it was saved by a newer version (kept, never rewritten). */
export type SavedImageSearch = { id: string; name: string; query: ImageQuery | null; created_at: number; updated_at: number };
/** SS-045 window: a live observation in a deterministic order (sort, then asset ID). */
export type GalleryPage = { items: ImageSummary[]; offset: number; total: number };
export type ImageDetails = {
  header: ImageSummary['header']; profile: import('./contracts/imageV1').ImageProfile | null; source_name: string; source_available: boolean;
  tags: string[]; caption: string | null; comment: string; favorite: boolean;
  /** 1–5 stars; null is unrated (never zero). */
  rating: number | null; annotation_revision: number;
};
/** SS-046 explicit changes. Absent fields are kept; `rating: null` clears it. */
export type AnnotationPatch = { add_tags?: string[]; remove_tags?: string[]; comment?: string; favorite?: boolean; rating?: number | null };
export type AnnotationError = 'INVALID_ARGUMENT' | 'NOT_FOUND' | 'PERMISSION_DENIED' | 'CONFLICT' | 'SOURCE_UNAVAILABLE' | 'LIMIT_EXCEEDED' | 'INTERNAL';
export type ImageAnnotations = import('./contracts/imageV1').ImageAnnotations;
export type AnnotationEditResult = { annotations: ImageAnnotations; action_id: string | null };
export type BulkTarget = { id: string; revision: number };
export type ItemOutcome = 'applied' | 'unchanged' | 'conflict' | 'not_found' | 'permission_denied' | 'source_unavailable' | 'limit_exceeded';
export type ItemResult = { id: string; outcome: ItemOutcome; revision: number | null };
export type TagEffect = { tag: string; count: number };
export type BulkPreview = { targets: BulkTarget[]; changes: number; unchanged: number; skipped: ItemResult[]; additions: TagEffect[]; removals: TagEffect[] };
export type BulkResult = { action_id: string | null; results: ItemResult[]; applied: number; unchanged: number; conflicts: number; skipped: number };
export type AnnotationAction = { id: string; kind: 'edit' | 'bulk' | 'rename'; label: string; state: 'applied' | 'undone'; created_at: number; items: number };
export type TagAlias = { alias: string; canonical: string };
export type TagList = { tags: { tag: string; count: number }[]; truncated: boolean; aliases: TagAlias[] };
export type RenamePreview = { from: string; to: string; images: number; merged: number; unavailable: number; audio_unchanged: number };
/** SS-049 provider settings. Keys are write-only: no command ever returns one. */
export type ProviderKind = 'openai' | 'anthropic' | 'gemini' | 'openai_compatible';
export type CapabilityState = 'untested' | 'supported' | 'unsupported' | 'inconclusive';
export type ProviderView = {
  id: string; kind: ProviderKind; label: string; base_url: string; model: string; allow_local: boolean;
  key_storage: 'none' | 'keychain' | 'session'; key_origin: string | null; revision: number;
  capabilities: { image_input: CapabilityState; structured_output: CapabilityState; embeddings: CapabilityState; tested_revision: number | null; tested_at: number | null; model_version: string | null };
  locality: 'remote' | 'private' | 'loopback' | null; has_key: boolean;
};
export type ProviderDraft = { kind: ProviderKind; label: string; base_url: string | null; model: string; allow_local: boolean };
export type ProviderTestReport = {
  provider: ProviderView;
  steps: { check: 'model_access' | 'image_input' | 'structured_output' | 'embeddings'; passed: boolean; error: { code: string; message: string } | null }[];
  usage: { input_tokens: number | null; output_tokens: number | null };
};
export type AiFeatureState =
  | { state: 'ready'; provider_id: string; provider_label: string }
  | { state: 'needs_key' | 'needs_test'; provider_id: string; provider_label: string }
  | { state: 'needs_provider' | 'unsupported' | 'not_offered' };
export type AiFeatureStatus = AiFeatureState & {
  feature: 'image_descriptions' | 'semantic_search' | 'offline_tagging';
  quality: { state: 'pending_evaluation' };
};
