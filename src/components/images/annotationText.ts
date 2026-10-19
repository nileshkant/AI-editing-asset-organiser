import type { BulkResult } from '../../types';

/** Limits mirrored from the core (SS-046) for early, explanatory feedback only. */
export const MAX_BULK = 1000;
export const MAX_NOTE_BYTES = 40_000;
export const noteBytes = (s: string) => new TextEncoder().encode(s).length;

/** Comma-separated tag input; the core normalizes spelling, case and aliases. */
export const splitTags = (input: string) => input.split(',').map(t => t.trim()).filter(Boolean);
/** Mirrors the core tag identity (NFC, underscores as spaces, collapsed whitespace, case-insensitive). */
export const tagIdentity = (tag: string) => tag.normalize('NFC').replace(/_/g, ' ').split(/\s+/).filter(Boolean).join(' ').toLowerCase();

const messages: Record<string, string> = {
  CONFLICT: 'Changed elsewhere. The latest annotations are shown; review them and try again.',
  SOURCE_UNAVAILABLE: 'The original or its source is offline, so annotations cannot be edited now.',
  PERMISSION_DENIED: 'This image is outside its approved source scope.',
  NOT_FOUND: 'This image is no longer in the library.',
  LIMIT_EXCEEDED: 'Too long or too many: tags are up to 64 characters (64 per image), notes up to 40,000 bytes.',
  INVALID_ARGUMENT: 'That change is not valid. Check the tag names and rating.',
};
export const errorCode = (e: unknown) => String(e instanceof Error ? e.message : e).replace(/^Error:\s*/, '');
export const annotationError = (e: unknown) => messages[errorCode(e)] ?? `Annotation change failed · ${errorCode(e).replaceAll('_', ' ').toLowerCase()}`;

const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
export function resultSummary(r: BulkResult, verb = 'Updated') {
  const parts = [`${verb} ${plural(r.applied, 'image')}`];
  if (r.unchanged) parts.push(`${plural(r.unchanged, 'image')} already matched`);
  if (r.conflicts) parts.push(`${plural(r.conflicts, 'image')} changed elsewhere and kept their newer annotations`);
  if (r.skipped) parts.push(`${plural(r.skipped, 'image')} skipped (offline, missing or out of scope)`);
  return parts.join(' · ');
}
