import { useCallback, useState } from 'react';
import type { ImageGalleryController } from '../../hooks/useImageGallery';
import type { ImageImportJob, ImageSummary, Source } from '../../types';
import { importTerminal } from '../../hooks/useImageImports';
import { ImageGrid } from './ImageGrid';
import { ImageInspector } from './ImageInspector';
import { CompareDialog, MAX_COMPARE } from './CompareDialog';
import { BulkAnnotationDialog } from './BulkAnnotationDialog';
import { TagManagerDialog } from './TagManagerDialog';
import { AnnotationHistoryDialog } from './AnnotationHistoryDialog';
import { MAX_BULK } from './annotationText';
import type { PreviewBackground } from './ImageViewer';

const SIZES = [{ label: 'Small', px: 96 }, { label: 'Medium', px: 144 }, { label: 'Large', px: 208 }];
type Props = { gallery: ImageGalleryController; sources: Source[]; jobs: ImageImportJob[]; onOpenImports: () => void };

export function ImageGallery({ gallery: g, sources, jobs, onOpenImports }: Props) {
  const [cell, setCell] = useState(144), [focusIndex, setFocusIndex] = useState(0);
  const [active, setActive] = useState<{ index: number; summary: ImageSummary } | null>(null);
  const [background, setBackground] = useState<PreviewBackground>('checkerboard'), [comparing, setComparing] = useState(false);
  const [dialog, setDialog] = useState<'bulk' | 'tags' | 'history' | null>(null), [annotationsVersion, setAnnotationsVersion] = useState(0);
  /** After any annotation change, reload visible windows and the open inspector. */
  const annotated = useCallback(() => { g.refresh(); setAnnotationsVersion(n => n + 1); }, [g]);
  const total = g.total ?? 0;
  const open = useCallback((index: number, summary: ImageSummary) => {
    setActive({ index, summary });
    requestAnimationFrame(() => document.querySelector<HTMLElement>('.image-inspector')?.focus());
  }, []);
  const close = useCallback(() => {
    const index = active?.index ?? focusIndex;
    setActive(null);
    requestAnimationFrame(() => document.querySelector<HTMLElement>(`.image-grid [data-index="${index}"]`)?.focus());
  }, [active, focusIndex]);
  const running = jobs.filter(j => !importTerminal(j));
  const failed = jobs.filter(j => importTerminal(j) && j.failed > 0).reduce((n, j) => n + j.failed, 0);
  const selected = [...g.selected.values()];
  // A view change keeps the selection; the inspector follows its own identity.
  const switchView = (fn: () => void) => { fn(); setActive(null); setFocusIndex(0); };
  return <section className="image-workspace" aria-label="Image library">
    <div className="image-toolbar">
      <label>Source <select value={g.sourceId} onChange={e => switchView(() => g.setSourceId(e.target.value))}>
        <option value="">All sources</option>{sources.map(s => <option key={s.id} value={s.id}>{s.name}{s.available ? '' : ' (offline)'}</option>)}
      </select></label>
      <label>Sort <select value={g.sort} onChange={e => switchView(() => g.setSort(e.target.value as 'name_asc' | 'name_desc'))}>
        <option value="name_asc">Name A–Z</option><option value="name_desc">Name Z–A</option>
      </select></label>
      <label>Thumbnail size <select value={cell} onChange={e => setCell(Number(e.target.value))}>{SIZES.map(s => <option key={s.px} value={s.px}>{s.label}</option>)}</select></label>
      <span className="image-selection" aria-live="polite">{selected.length ? `${selected.length} selected` : 'No selection'}</span>
      {selected.length > 0 && <button onClick={g.clearSelection}>Clear selection</button>}
      <button disabled={selected.length < 2 || selected.length > MAX_COMPARE} onClick={() => setComparing(true)}
        title={`Select 2–${MAX_COMPARE} images to compare`}>Compare selected</button>
      <button disabled={!selected.length || selected.length > MAX_BULK} onClick={() => setDialog('bulk')}
        title={`Tag, favorite or rate up to ${MAX_BULK.toLocaleString()} selected images`}>Edit selected…</button>
      <button onClick={() => setDialog('tags')} title="Rename image tags and manage aliases">Tags…</button>
      <button onClick={() => setDialog('history')} title="Recent annotation changes with undo">History…</button>
    </div>
    <p className="keyboard-help" id="image-grid-help">Arrow keys move · Space selects · Enter opens details · Escape closes · Ctrl or ⌘ click adds to selection</p>
    {running.length > 0 && <p className="image-banner" role="status">Importing images: {running.reduce((n, j) => n + j.completed, 0)} of {running.reduce((n, j) => n + j.total, 0)} files checked. The gallery updates as images become ready.</p>}
    {failed > 0 && <p className="image-banner warning" role="status">{failed} {failed === 1 ? 'file' : 'files'} could not be imported. <button onClick={onOpenImports}>Review in Imports</button></p>}
    {g.error && <p className="error" role="alert">Images could not be listed · {g.error}</p>}
    <div className="results-bar"><span>{g.total === null ? 'Loading images…' : `${total.toLocaleString()} ${total === 1 ? 'image' : 'images'}`}</span><span>Local previews · originals unchanged</span></div>
    <div className={`library-body image-body${active ? ' with-inspector' : ''}`}>
      {g.total === null ? <div className="catalog-loading" role="status">Loading images…</div>
        : !sources.length ? <div className="image-empty"><h2>No sources yet</h2><p>Import a folder or files that contain JPEG, PNG or WebP images.</p><button onClick={onOpenImports}>Go to Imports</button></div>
        : total === 0 ? <div className="image-empty"><h2>{running.length ? 'Import in progress' : 'No images here'}</h2><p>{running.length ? 'Images appear as soon as their technical analysis finishes.' : 'No supported images (JPEG, PNG or WebP) in this view. Choose another source or import images.'}</p>{!running.length && <button onClick={onOpenImports}>Go to Imports</button>}</div>
        : <ImageGrid version={g.version} total={total} cell={cell} item={g.item} ensure={g.ensure} focusIndex={focusIndex} setFocusIndex={setFocusIndex}
            selected={g.selected} activeId={active?.summary.header.id ?? null} onOpen={open} onToggle={g.toggle} onStale={g.refresh} busy={g.total === null} />}
      {active && <ImageInspector key={active.summary.header.id} summary={g.item(active.index)?.header.id === active.summary.header.id ? g.item(active.index)! : active.summary}
        background={background} setBackground={setBackground} onClose={close} onStale={g.refresh} annotationsVersion={annotationsVersion} onAnnotated={g.refresh} />}
    </div>
    {dialog === 'bulk' && <BulkAnnotationDialog ids={selected.map(s => s.header.id)} onClose={() => setDialog(null)} onChanged={annotated} />}
    {dialog === 'tags' && <TagManagerDialog sourceId={g.sourceId || null} sourceName={sources.find(s => s.id === g.sourceId)?.name ?? 'all sources'} onClose={() => setDialog(null)} onChanged={annotated} />}
    {dialog === 'history' && <AnnotationHistoryDialog onClose={() => setDialog(null)} onChanged={annotated} />}
    {comparing && <CompareDialog candidates={selected.slice(0, MAX_COMPARE)} background={background} setBackground={setBackground} onClose={() => setComparing(false)} />}
  </section>;
}
