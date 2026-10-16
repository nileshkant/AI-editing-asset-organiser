/** SS-040 contracts only. Not registered in IPC, UI navigation, or MCP. */
export const IMAGE_CONTRACT_SCHEMA = 'creativeshelf-image-contract/v1';
export const IMAGE_LIMITS = {
  maxSafeInteger: Number.MAX_SAFE_INTEGER, pathBytes: 32768, textBytes: 40000,
  tagBytes: 256, tags: 64, records: 1000, evidence: 128, recipes: 128,
  dimension: 32768, pixels: 100000000, encodedBytes: 512 * 1024 * 1024,
  bundleBytes: 2 * 1024 * 1024, previewDimension: 2048,
  previewBytes: 4 * 1024 * 1024, pageSize: 100,
} as const;
export type AssetKind = 'image' | 'element';
export type TechnicalState = 'pending' | 'ready' | 'failed';
export type Availability = 'available' | 'offline' | 'missing' | 'inaccessible';
export type EnrichmentState = 'disabled' | 'queued' | 'running' | 'complete' | 'failed' | 'cancelled' | 'stale';
export type ImageHeader = {
  id: string; kind: AssetKind; source_id: string; relative_path: string;
  content_hash: string | null; technical_state: TechnicalState;
  availability: Availability; enrichment_state: EnrichmentState;
};
export type ImageProfile = {
  raw_width: number; raw_height: number; orientation: number; width: number; height: number;
  format: 'jpeg' | 'png' | 'webp'; encoded_bytes: number; bit_depth: number;
  source_color_profile: string | null; alpha_channel: boolean;
  transparency: 'none' | 'binary' | 'partial'; analyzer_version: string;
};
export type ImageAnnotations = {
  revision: number; tags: string[]; comment: string; favorite: boolean;
  rating: number | null; caption: string | null;
};
export type ModelRef = {
  provider: string; model: string; revision: string; preprocessing: string; output_schema: string;
};
export type ImageEvidence = {
  id: string; content_hash: string; origin: 'model' | 'filename' | 'imported' | 'user';
  coverage: 'full_image' | 'preview' | 'element'; tags: string[]; caption: string | null;
  model: ModelRef | null; confidence: number | null;
  confidence_kind: 'unavailable' | 'reported' | 'calibrated';
  review_state: 'suggested' | 'accepted' | 'edited' | 'rejected';
  stale: boolean; created_at_ms: number;
};
export type PixelRect = { left: number; top: number; right: number; bottom: number };
export type ImageRecipe = {
  id: string; asset_id: string; asset_version: string; revision: number;
  source_width: number; source_height: number; crop: PixelRect;
  output_width: number; output_height: number; allow_upscale: boolean;
  color_policy: 'srgb'; stale: boolean;
};
export type DerivedLineage = {
  parent_id: string; parent_version: string; recipe_id: string;
  recipe_revision: number; mask_hash: string; mask_revision: number;
};
export type ImageAsset = {
  header: ImageHeader; profile: ImageProfile | null; annotations: ImageAnnotations;
  evidence: ImageEvidence[]; recipes: ImageRecipe[]; lineage: DerivedLineage | null;
};
/** Conformance fixture envelope, NOT a production portable catalog. */
export type ImageContractBundle = {
  schema: typeof IMAGE_CONTRACT_SCHEMA;
  sources: { id: string; scope: 'folder' | 'files'; files: string[] }[];
  audio_ids: string[]; images: ImageAsset[];
};
export type ImageEditArgs = {
  assetId: string; expectedContentHash: string; expectedRevision: number;
};
export type ContractErrorCode =
  | 'INVALID_ARGUMENT' | 'UNSUPPORTED_SCHEMA' | 'LIMIT_EXCEEDED' | 'INVALID_PATH'
  | 'WRONG_ASSET_KIND' | 'IDENTITY_CONFLICT' | 'INVALID_REFERENCE' | 'STALE_VERSION' | 'INVALID_RANGE';
export class ImageContractError extends Error {
  constructor(public readonly code: ContractErrorCode) { super(code); this.name = 'ImageContractError'; }
}
function check(ok: unknown, code: ContractErrorCode = 'INVALID_ARGUMENT'): asserts ok {
  if (!ok) throw new ImageContractError(code);
}
function shape(value: unknown, keys: string[]): Record<string, unknown> {
  check(value !== null && typeof value === 'object' && !Array.isArray(value));
  const o = value as Record<string, unknown>;
  check(Object.keys(o).length === keys.length && keys.every(k => Object.hasOwn(o, k)));
  return o;
}
function list(value: unknown, max: number): unknown[] {
  check(Array.isArray(value)); check(value.length <= max, 'LIMIT_EXCEEDED');
  for (let i=0; i<value.length; i++) check(Object.hasOwn(value,i));
  return value;
}
function string(value: unknown): string {
  check(typeof value === 'string' && !/[\uD800-\uDFFF]/u.test(value));
  return value;
}
function bytes(s: string): number { return new TextEncoder().encode(s).length; }
function text(value: unknown, max = 256, empty = false): string {
  const s = string(value);
  check((empty || s.length > 0) && bytes(s) <= max && !/[\u0000-\u001f\u007f]/u.test(s));
  return s;
}
function uuid(value: unknown): string {
  const s = string(value);
  check(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(s) && s !== '00000000-0000-0000-0000-000000000000');
  return s;
}
function digest(value: unknown): string {
  const s = string(value); check(/^[0-9a-f]{64}$/.test(s)); return s;
}
function bool(value: unknown): boolean { check(typeof value === 'boolean'); return value; }
function integer(value: unknown, min = 0, max = 4294967295): number {
  check(typeof value === 'number' && Number.isSafeInteger(value) && value >= min && value <= max);
  return value;
}
function choice(value: unknown, values: readonly string[]): string {
  const s = string(value); check(values.includes(s)); return s;
}
export function validateImageRelativePath(value: unknown): string {
  const s = string(value);
  check(s.length > 0 && bytes(s) <= IMAGE_LIMITS.pathBytes && !/[\\:\u0000-\u001f\u007f]/u.test(s) &&
    s.split('/').every(p => p.length > 0 && p !== '.' && p !== '..'), 'INVALID_PATH');
  return s;
}
function dimensions(w: unknown, h: unknown): [number, number] {
  const width = integer(w), height = integer(h);
  check(width > 0 && height > 0 && width <= IMAGE_LIMITS.dimension && height <= IMAGE_LIMITS.dimension &&
    width * height <= IMAGE_LIMITS.pixels, 'LIMIT_EXCEEDED');
  return [width, height];
}
function tags(value: unknown): void {
  const a = list(value, IMAGE_LIMITS.tags).map(v => text(v, IMAGE_LIMITS.tagBytes));
  check(a.every(s => !/^\p{White_Space}|\p{White_Space}$/u.test(s)) && new Set(a).size === a.length);
}
function caption(value: unknown): void {
  if (value !== null) {
    const s = string(value); check(bytes(s) <= IMAGE_LIMITS.textBytes && !s.includes('\0'), 'LIMIT_EXCEEDED');
  }
}
function validateAsset(value: unknown): void {
  const a = shape(value, ['header','profile','annotations','evidence','recipes','lineage']);
  const h = shape(a.header, ['id','kind','source_id','relative_path','content_hash','technical_state','availability','enrichment_state']);
  const assetId = uuid(h.id);
  choice(h.kind, ['image','element']); text(h.source_id); validateImageRelativePath(h.relative_path);
  if (h.content_hash !== null) digest(h.content_hash);
  choice(h.technical_state, ['pending','ready','failed']);
  choice(h.availability, ['available','offline','missing','inaccessible']);
  choice(h.enrichment_state, ['disabled','queued','running','complete','failed','cancelled','stale']);
  check(h.technical_state !== 'ready' || (h.content_hash !== null && a.profile !== null));
  let profile: Record<string, unknown> | null = null;
  if (a.profile !== null) {
    const p = profile = shape(a.profile, ['raw_width','raw_height','orientation','width','height','format','encoded_bytes','bit_depth','source_color_profile','alpha_channel','transparency','analyzer_version']);
    check(h.content_hash !== null);
    const raw = dimensions(p.raw_width,p.raw_height), oriented = dimensions(p.width,p.height);
    const orientation = integer(p.orientation,1,8);
    check(oriented[0] === raw[orientation >= 5 ? 1 : 0] && oriented[1] === raw[orientation >= 5 ? 0 : 1], 'INVALID_RANGE');
    choice(p.format,['jpeg','png','webp']);
    const encoded = integer(p.encoded_bytes,0,Number.MAX_SAFE_INTEGER);
    check(encoded > 0 && encoded <= IMAGE_LIMITS.encodedBytes,'LIMIT_EXCEEDED');
    check([1,2,4,8,10,12,16].includes(integer(p.bit_depth,0,255)));
    if (p.source_color_profile !== null) text(p.source_color_profile);
    text(p.analyzer_version);
    const alpha = bool(p.alpha_channel);
    choice(p.transparency,['none','binary','partial']);
    check(alpha || p.transparency === 'none');
    check(p.format !== 'jpeg' || (!alpha && p.transparency === 'none'));
  }
  const annotations = shape(a.annotations,['revision','tags','comment','favorite','rating','caption']);
  integer(annotations.revision,1); tags(annotations.tags); string(annotations.comment); caption(annotations.comment); caption(annotations.caption);
  bool(annotations.favorite);
  if (annotations.rating !== null) integer(annotations.rating,1,5);
  const evidenceIds = new Set<string>();
  for (const item of list(a.evidence, IMAGE_LIMITS.evidence)) {
    const e = shape(item,['id','content_hash','origin','coverage','tags','caption','model','confidence','confidence_kind','review_state','stale','created_at_ms']);
    const eid=uuid(e.id); digest(e.content_hash); tags(e.tags); caption(e.caption);
    check(!evidenceIds.has(eid),'IDENTITY_CONFLICT'); evidenceIds.add(eid);
    choice(e.origin,['model','filename','imported','user']); choice(e.coverage,['full_image','preview','element']);
    choice(e.review_state,['suggested','accepted','edited','rejected']);
    check(bool(e.stale) || h.content_hash === e.content_hash,'STALE_VERSION');
    integer(e.created_at_ms,0,Number.MAX_SAFE_INTEGER);
    check((e.origin === 'model') === (e.model !== null));
    if (e.model !== null) {
      const m=shape(e.model,['provider','model','revision','preprocessing','output_schema']);
      Object.values(m).forEach(v=>text(v));
    }
    choice(e.confidence_kind,['unavailable','reported','calibrated']);
    check((e.confidence_kind === 'unavailable') === (e.confidence === null));
    if (e.confidence !== null) check(typeof e.confidence === 'number' && Number.isFinite(e.confidence) && e.confidence >= 0 && e.confidence <= 1 && e.origin === 'model');
  }
  const recipeIds = new Set<string>();
  if (h.enrichment_state === 'complete') {
    check(h.technical_state === 'ready' && (a.evidence as ImageEvidence[]).some(e => e.origin === 'model' && !e.stale && e.content_hash === h.content_hash));
  }
  for (const item of list(a.recipes, IMAGE_LIMITS.recipes)) {
    const r=shape(item,['id','asset_id','asset_version','revision','source_width','source_height','crop','output_width','output_height','allow_upscale','color_policy','stale']);
    const rid=uuid(r.id); uuid(r.asset_id); digest(r.asset_version);
    check(!recipeIds.has(rid),'IDENTITY_CONFLICT'); recipeIds.add(rid);
    check(r.asset_id === assetId,'INVALID_REFERENCE'); integer(r.revision,1);
    check(bool(r.stale) || h.content_hash === r.asset_version,'STALE_VERSION');
    const source=dimensions(r.source_width,r.source_height), output=dimensions(r.output_width,r.output_height);
    if (!r.stale) {
      check(profile !== null,'INVALID_REFERENCE');
      check(source[0] === profile.width && source[1] === profile.height,'INVALID_RANGE');
    }
    const c=shape(r.crop,['left','top','right','bottom']);
    const left=integer(c.left), top=integer(c.top), right=integer(c.right), bottom=integer(c.bottom);
    check(left < right && top < bottom && right <= source[0] && bottom <= source[1],'INVALID_RANGE');
    check(bool(r.allow_upscale) || (output[0] <= right-left && output[1] <= bottom-top),'INVALID_RANGE');
    choice(r.color_policy,['srgb']);
  }
  check((h.kind === 'element') === (a.lineage !== null),'WRONG_ASSET_KIND');
  if (a.lineage !== null) {
    const l=shape(a.lineage,['parent_id','parent_version','recipe_id','recipe_revision','mask_hash','mask_revision']);
    check(uuid(l.parent_id) !== assetId,'INVALID_REFERENCE'); digest(l.parent_version); uuid(l.recipe_id); digest(l.mask_hash);
    integer(l.recipe_revision,1); integer(l.mask_revision,1);
  }
}
export function validateImageEditArgs(value: unknown): ImageEditArgs {
  const a=shape(value,['assetId','expectedContentHash','expectedRevision']);
  uuid(a.assetId); digest(a.expectedContentHash); integer(a.expectedRevision,1);
  return a as ImageEditArgs;
}
export function validateImageContract(value: unknown): ImageContractBundle {
  check(value !== null && typeof value === 'object' && (value as Record<string, unknown>).schema === IMAGE_CONTRACT_SCHEMA,'UNSUPPORTED_SCHEMA');
  const b=shape(value,['schema','sources','audio_ids','images']);
  const sources=new Map<string,{scope:string;files:string[]}>();
  for (const item of list(b.sources,IMAGE_LIMITS.records)) {
    const s=shape(item,['id','scope','files']); const sid=text(s.id);
    const scope=choice(s.scope,['folder','files']);
    check(!sources.has(sid),'IDENTITY_CONFLICT');
    const files=list(s.files,IMAGE_LIMITS.records).map(validateImageRelativePath);
    check(scope !== 'folder' || files.length === 0);
    check(new Set(files).size === files.length,'IDENTITY_CONFLICT'); sources.set(sid,{scope,files});
  }
  const ids=new Set<string>();
  const audio=list(b.audio_ids,IMAGE_LIMITS.records).map(v=>text(v));
  for (const aid of audio) { check(!ids.has(aid),'IDENTITY_CONFLICT');ids.add(aid); }
  const items=list(b.images,IMAGE_LIMITS.records);
  items.forEach(validateAsset);
  const images=items as ImageAsset[];
  const byId=new Map(images.map(a=>[a.header.id,a]));
  const paths=new Set<string>();
  for (const a of images) {
    const h=a.header;
    check(!ids.has(h.id),'IDENTITY_CONFLICT');ids.add(h.id);
    const key=JSON.stringify([h.source_id,h.relative_path]);
    check(!paths.has(key),'IDENTITY_CONFLICT');paths.add(key);
    const source=sources.get(h.source_id);check(source,'INVALID_REFERENCE');
    check(source.scope !== 'files' || source.files.includes(h.relative_path),'INVALID_REFERENCE');
    const ancestors=new Set([h.id]);let current: ImageAsset | undefined=a;
    while(current?.lineage) {
      const parent=current.lineage.parent_id;
      check(!audio.includes(parent),'WRONG_ASSET_KIND');
      check(!ancestors.has(parent),'INVALID_REFERENCE');ancestors.add(parent);current=byId.get(parent);
    }
  }
  return value as ImageContractBundle;
}
export function parseImageContract(json: string): ImageContractBundle {
  check(json.length <= IMAGE_LIMITS.bundleBytes,'LIMIT_EXCEEDED');
  check(bytes(json) <= IMAGE_LIMITS.bundleBytes,'LIMIT_EXCEEDED');
  let value: unknown;
  try { value=JSON.parse(json); } catch { throw new ImageContractError('INVALID_ARGUMENT'); }
  return validateImageContract(value);
}
