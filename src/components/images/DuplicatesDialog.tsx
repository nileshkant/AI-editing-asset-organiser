import { useCallback, useEffect, useRef, useState } from 'react';
import { imageDuplicateGroup, imageDuplicateGroups, updateImageDuplicateGroup } from '../../api';
import type { DuplicateAction, DuplicateGroup, DuplicateMember, DuplicatePage } from '../../types';
import { ImagePreview } from '../image/ImagePreview';
import { ImageModal } from './ImageModal';
import { CompareDialog, MAX_COMPARE } from './CompareDialog';
import { errorCode } from './annotationText';
import { fileName } from './ImageGrid';
import type { PreviewBackground } from './ImageViewer';

/** Mirrors the core page bound (SS-062). */
export const DUPLICATE_PAGE = 25;
const messages: Record<string, string> = {
  CONFLICT: 'This stack changed elsewhere. The latest state is shown; review it and try again.',
  NOT_FOUND: 'This stack or copy is no longer in the library.',
  PERMISSION_DENIED: 'That copy is outside the current source view.',
  INVALID_ARGUMENT: 'That change does not apply to this copy.',
};
const duplicateError = (e: unknown) => messages[errorCode(e)] ?? `Duplicate change failed · ${errorCode(e).replaceAll('_', ' ').toLowerCase()}`;
const preferredNote: Record<DuplicateGroup['preferred_state'], string> = {
  none: '', available: '', separated: '',
  offline: 'Preferred copy is offline; its choice is kept.',
  inaccessible: 'Preferred copy cannot be read right now; its choice is kept.',
  missing: 'Preferred copy is missing from its folder. Choose another copy or relink the folder.',
  changed: 'Preferred copy changed and is no longer identical. Choose another copy.',
};
const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

type Props = { sourceId: string | null; sourceName: string; background: PreviewBackground; setBackground: (b: PreviewBackground) => void; onClose: () => void };

function Member({ m, group, busy, act }: { m: DuplicateMember; group: DuplicateGroup; busy: boolean; act: (a: DuplicateAction) => void }) {
  const h = m.image.header, name = fileName(h.relative_path), preferred = group.preferred_id === h.id;
  return <li className={m.separated ? 'separated' : ''}>
    <span className="image-duplicate-name">
      <strong className="path-text" title={h.relative_path}>{name}</strong>
      <span className="muted path-text">{m.source_name} · {h.relative_path}</span>
    </span>
    <span className="image-duplicate-badges">
      {preferred && <span className="badge">Preferred</span>}
      {m.new_since_dismissed && <span className="badge">New since dismissed</span>}
      {m.separated && <span className="badge">Kept separate</span>}
      {h.availability !== 'available' && <span className="badge warning">{h.availability === 'offline' ? 'Offline' : h.availability === 'inaccessible' ? 'Unreadable' : h.availability}</span>}
    </span>
    <span className="image-duplicate-actions">
      {!m.separated && !preferred && <button disabled={busy} aria-label={`Prefer ${name} in ${m.source_name}`} onClick={() => act({ action: 'prefer', asset_id: h.id })}>Prefer</button>}
      {!m.separated && <button disabled={busy} aria-label={`Keep ${name} in ${m.source_name} separate`} onClick={() => act({ action: 'separate', asset_id: h.id })}>Keep separate</button>}
      {m.separated && <button disabled={busy} aria-label={`Return ${name} in ${m.source_name} to the stack`} onClick={() => act({ action: 'rejoin', asset_id: h.id })}>Return to stack</button>}
    </span>
  </li>;
}

function Stack({ group, sourceId, onChange, onCompare }: { group: DuplicateGroup; sourceId: string | null; onChange: (g: DuplicateGroup | null) => void; onCompare: (g: DuplicateGroup) => void }) {
  const [busy, setBusy] = useState(false), [error, setError] = useState('');
  const act = async (action: DuplicateAction) => {
    setBusy(true); setError('');
    try { onChange(await updateImageDuplicateGroup(sourceId, group, action)); }
    catch (e) {
      setError(duplicateError(e));
      // Never retry blindly: show what is there now.
      try { onChange(await imageDuplicateGroup(sourceId, group.content_hash)); } catch { onChange(null); }
    } finally { setBusy(false); }
  };
  const rep = group.members.find(m => m.image.header.id === group.representative_id) ?? group.members[0];
  const title = `${plural(group.member_count, 'identical copy', 'identical copies')}`;
  const id = `duplicate-${group.content_hash.slice(0, 16)}`;
  const stacked = group.members.filter(m => !m.separated);
  return <section className="image-duplicate-group" aria-labelledby={id}>
    <div className="image-duplicate-thumb">
      {rep?.image.header.content_hash && rep.image.header.technical_state === 'ready'
        ? <ImagePreview assetId={rep.image.header.id} version={rep.image.header.content_hash} tier="small" label={fileName(rep.image.header.relative_path)} variant="fill" delayMs={120} />
        : <small>No preview</small>}
    </div>
    <div className="image-duplicate-body">
      <h3 id={id}><span className="badge">Exact copies</span> {title}{group.separated_count ? ` · ${plural(group.separated_count, 'kept separate', 'kept separate')}` : ''}{group.dismissed ? ' · dismissed' : ''}</h3>
      <p className="muted">Identical bytes. Each copy keeps its own location, tags and notes.</p>
      {preferredNote[group.preferred_state] && <p className="image-banner warning" role="status">{preferredNote[group.preferred_state]}</p>}
      <ul className="image-duplicate-members" aria-label={`Copies in this stack`}>
        {group.members.map(m => <Member key={m.image.header.id} m={m} group={group} busy={busy} act={a => void act(a)} />)}
      </ul>
      {group.members_truncated && <p className="muted">Showing the first {group.members.length} copies.</p>}
      <div className="modal-actions">
        <button disabled={busy || stacked.length < 2} onClick={() => onCompare(group)} title={`Compare up to ${MAX_COMPARE} copies`}>Compare</button>
        {group.preferred_id && <button disabled={busy} onClick={() => void act({ action: 'clear_preferred' })}>Clear preference</button>}
        {group.dismissed
          ? <button disabled={busy} onClick={() => void act({ action: 'restore' })}>Show again</button>
          : <button disabled={busy || group.member_count < 2} onClick={() => void act({ action: 'dismiss' })}>Dismiss</button>}
      </div>
      {error && <p className="error" role="alert">{error}</p>}
    </div>
  </section>;
}

/**
 * SS-062 review of exact-duplicate stacks in the gallery's current source view.
 * Decisions are recorded only; no file is deleted, moved or merged.
 */
export function DuplicatesDialog({ sourceId, sourceName, background, setBackground, onClose }: Props) {
  const [page, setPage] = useState<DuplicatePage | null>(null), [groups, setGroups] = useState<DuplicateGroup[]>([]);
  const [hidden, setHidden] = useState(false), [error, setError] = useState(''), [loading, setLoading] = useState(false);
  const [comparing, setComparing] = useState<DuplicateGroup | null>(null);
  const query = useRef(0);
  const load = useCallback(async (offset: number) => {
    const q = offset ? query.current : ++query.current;
    setLoading(true); setError('');
    try {
      const next = await imageDuplicateGroups(sourceId, hidden, offset, DUPLICATE_PAGE);
      if (q !== query.current) return;
      setPage(next);
      setGroups(old => offset ? [...old, ...next.groups.filter(g => !old.some(o => o.content_hash === g.content_hash))] : next.groups);
    } catch (e) { if (q === query.current) setError(duplicateError(e)); }
    finally { if (q === query.current) setLoading(false); }
  }, [sourceId, hidden]);
  useEffect(() => { void load(0); }, [load]);
  const replace = (hash: string, next: DuplicateGroup | null) => setGroups(old => old.map(g => g.content_hash === hash ? next : g).filter((g): g is DuplicateGroup => !!g && (hidden || !g.hidden || g.content_hash === hash)));
  const scope = sourceId ? `in ${sourceName}` : 'in all sources';
  // The comparison is a sibling dialog so its Escape closes only itself.
  return <><ImageModal title="Duplicate images" className="image-duplicates-dialog" onClose={onClose}>
    <p className="muted">Exact copies {scope}: files with identical bytes. Nothing is deleted or merged; choose a preferred copy, keep copies separate, or dismiss a stack.</p>
    <div className="image-duplicate-summary">
      <span role="status">{!page ? 'Looking for exact copies…' : page.open_groups
        ? `${plural(page.open_groups, 'stack')} · ${plural(page.extra_copies, 'extra copy', 'extra copies')}`
        : 'No exact copies to review.'}</span>
      <label className="checkbox"><input type="checkbox" checked={hidden} onChange={e => setHidden(e.target.checked)} />
        Show dismissed and resolved{page?.hidden_groups ? ` (${page.hidden_groups.toLocaleString()})` : ''}</label>
    </div>
    {groups.map(g => <Stack key={g.content_hash} group={g} sourceId={sourceId} onChange={next => replace(g.content_hash, next)} onCompare={setComparing} />)}
    {page && groups.length < page.total && <div className="modal-actions">
      {/* Stacks hidden by a decision here have left the server's open list; do not skip past others. */}
      <button disabled={loading} onClick={() => void load(groups.filter(g => hidden || !g.hidden).length)}>Show more stacks</button>
    </div>}
    {error && <p className="error" role="alert">{error}</p>}
  </ImageModal>
    {comparing && <CompareDialog candidates={comparing.members.filter(m => !m.separated).slice(0, MAX_COMPARE).map(m => m.image)}
      background={background} setBackground={setBackground} onClose={() => setComparing(null)} />}
  </>;
}
