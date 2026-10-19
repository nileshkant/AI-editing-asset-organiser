import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ImageInspector } from './components/images/ImageInspector';
import { AnnotationEditor } from './components/images/AnnotationEditor';
import { BulkAnnotationDialog } from './components/images/BulkAnnotationDialog';
import { TagManagerDialog } from './components/images/TagManagerDialog';
import { AnnotationHistoryDialog } from './components/images/AnnotationHistoryDialog';
import { splitTags, tagIdentity } from './components/images/annotationText';
import type { ImageDetails, ImageSummary } from './types';
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));
const id = '00000000-0000-4000-8000-000000000001';
const header: ImageSummary['header'] = { id, kind: 'image', source_id: 's1', relative_path: 'hero.png', content_hash: null, technical_state: 'pending', availability: 'available', enrichment_state: 'disabled' };
const details = (over: Partial<ImageDetails> = {}): ImageDetails => ({ header, profile: null, source_name: 'Shoot', source_available: true, tags: ['Brand A', 'old'], caption: null, comment: '', favorite: false, rating: null, annotation_revision: 3, ...over });
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);
beforeEach(() => { invoke.mockReset(); });
afterEach(cleanup);

describe('annotation editor', () => {
  it('sends explicit operations from the revision it read', async () => {
    invoke.mockResolvedValue({ annotations: {}, action_id: 'act-1' });
    const onChanged = vi.fn();
    render(<AnnotationEditor details={details()} name="hero.png" onChanged={onChanged} />);
    const input = screen.getByLabelText('Add tags to hero.png');
    fireEvent.change(input, { target: { value: 'Summer campaign,  brand_b ,' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(calls('edit_image_annotations')).toHaveLength(1));
    expect(calls('edit_image_annotations')[0]).toEqual({ id, expectedRevision: 3, patch: { add_tags: ['Summer campaign', 'brand_b'] } });
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    fireEvent.click(screen.getByRole('button', { name: 'Remove tag old' }));
    await waitFor(() => expect(calls('edit_image_annotations')[1].patch).toEqual({ remove_tags: ['old'] }));
    // "None" clears to null; it is never a zero rating.
    fireEvent.click(screen.getByRole('radio', { name: /^4 of 5 stars/ }));
    await waitFor(() => expect(calls('edit_image_annotations')[2].patch).toEqual({ rating: 4 }));
    fireEvent.click(screen.getByRole('button', { name: 'Favorite' }));
    await waitFor(() => expect(calls('edit_image_annotations')[3].patch).toEqual({ favorite: true }));
    // The last change can be undone from here.
    fireEvent.click(await screen.findByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(calls('undo_image_annotation_action')).toEqual([{ actionId: 'act-1' }]));
  });
  it('clears a rating with null and shows the unrated state', async () => {
    invoke.mockResolvedValue({ annotations: {}, action_id: 'a' });
    render(<AnnotationEditor details={details({ rating: 2 })} name="hero.png" onChanged={vi.fn()} />);
    expect(screen.getByRole('radio', { name: /^2 of 5 stars/ })).toBeChecked();
    fireEvent.click(screen.getByRole('radio', { name: 'No rating (unrated)' }));
    await waitFor(() => expect(calls('edit_image_annotations')[0].patch).toEqual({ rating: null }));
  });
  it('reports a concurrent change as a conflict and reloads instead of overwriting', async () => {
    invoke.mockImplementation(() => Promise.reject('CONFLICT'));
    const onChanged = vi.fn();
    render(<AnnotationEditor details={details()} name="hero.png" onChanged={onChanged} />);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Changed elsewhere');
    expect(onChanged).toHaveBeenCalledTimes(1);
    expect(calls('edit_image_annotations')).toHaveLength(1);
  });
  it('is read only while the original is offline', () => {
    render(<AnnotationEditor details={details({ source_available: false })} name="hero.png" onChanged={vi.fn()} />);
    expect(screen.getByText('Read only while the original is unavailable.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Remove tag old' })).toBeDisabled();
    expect(screen.getByLabelText('Notes')).toBeDisabled();
  });
});

describe('inspector notes', () => {
  it('renders untrusted notes and tags as plain text, and Escape keeps an unsaved draft', async () => {
    const xss = '<img src=x onerror="window.pwned=1"><script>window.pwned=2</script>';
    let current = details({ comment: xss, tags: ['<b>bold</b>'] });
    invoke.mockImplementation((cmd: string, args: { patch?: { comment?: string } }) => {
      if (cmd === 'image_details') return Promise.resolve(current);
      if (cmd === 'edit_image_annotations') { current = { ...current, comment: args.patch!.comment!, annotation_revision: current.annotation_revision + 1 }; return Promise.resolve({ annotations: {}, action_id: 'x' }); }
      return Promise.resolve(null);
    });
    const onClose = vi.fn();
    const summary: ImageSummary = { header, width: null, height: null, annotation_revision: 3 };
    const { container } = render(<ImageInspector summary={summary} background="checkerboard" setBackground={vi.fn()} onClose={onClose} onStale={vi.fn()} annotationsVersion={0} onAnnotated={vi.fn()} />);
    const notes = await screen.findByLabelText('Notes') as HTMLTextAreaElement;
    expect(notes.value).toBe(xss);
    expect(screen.getByText('<b>bold</b>')).toBeInTheDocument();
    expect(container.querySelector('aside img, aside script, aside b')).toBeNull();
    expect((window as { pwned?: number }).pwned).toBeUndefined();
    fireEvent.change(notes, { target: { value: 'draft' } });
    fireEvent.keyDown(notes, { key: 'Escape' });
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByText('Notes not saved yet.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Save notes' }));
    await waitFor(() => expect(calls('edit_image_annotations')[0]).toEqual({ id, expectedRevision: 3, patch: { comment: 'draft' } }));
    // The saved value reloads from the core; Escape now closes.
    await waitFor(() => expect(calls('image_details').length).toBeGreaterThan(1));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save notes' })).toBeDisabled());
    fireEvent.keyDown(screen.getByLabelText('Notes'), { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });
});

describe('bulk edits', () => {
  it('previews counts, applies with revision tokens and reports per-item outcomes', async () => {
    const targets = [{ id: 'a', revision: 2 }, { id: 'b', revision: 5 }];
    invoke.mockImplementation((cmd: string) => Promise.resolve(
      cmd === 'preview_image_bulk_annotations' ? { targets, changes: 2, unchanged: 0, skipped: [{ id: 'c', outcome: 'source_unavailable', revision: null }], additions: [{ tag: 'Brand A', count: 1 }], removals: [] }
        : cmd === 'apply_image_bulk_annotations' ? { action_id: 'bulk-1', results: [], applied: 1, unchanged: 0, conflicts: 1, skipped: 0 }
          : cmd === 'undo_image_annotation_action' ? { action_id: 'bulk-1', results: [], applied: 1, unchanged: 0, conflicts: 0, skipped: 0 } : null));
    const onChanged = vi.fn();
    render(<BulkAnnotationDialog ids={['a', 'b', 'c']} onClose={vi.fn()} onChanged={onChanged} />);
    expect(screen.getByRole('dialog', { name: 'Edit 3 selected images' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Preview changes' })).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Add tags'), { target: { value: 'Brand A, Summer' } });
    fireEvent.change(screen.getByLabelText('Rating'), { target: { value: 'clear' } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview changes' }));
    expect(await screen.findByText(/will change/)).toHaveTextContent('2 images will change');
    expect(screen.getByText('Add “Brand A”: 1 already have it')).toBeInTheDocument();
    expect(screen.getByText(/1 skipped: 1 offline or missing/)).toBeInTheDocument();
    expect(calls('preview_image_bulk_annotations')[0]).toEqual({ ids: ['a', 'b', 'c'], patch: { add_tags: ['Brand A', 'Summer'], remove_tags: [], rating: null } });
    fireEvent.click(screen.getByRole('button', { name: 'Apply to 2 images' }));
    expect(await screen.findByText(/Updated 1 image/)).toHaveTextContent('1 image changed elsewhere and kept their newer annotations');
    expect(calls('apply_image_bulk_annotations')[0].targets).toEqual(targets);
    expect(onChanged).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    expect(await screen.findByText(/· undone/)).toBeInTheDocument();
    expect(onChanged).toHaveBeenCalledTimes(2);
  });
  it('refuses selections above the bulk bound and traps focus in the dialog', () => {
    const onClose = vi.fn();
    render(<BulkAnnotationDialog ids={Array.from({ length: 1001 }, (_, i) => `${i}`)} onClose={onClose} onChanged={vi.fn()} />);
    expect(screen.getByRole('alert')).toHaveTextContent('limited to 1,000 images');
    fireEvent.change(screen.getByLabelText('Add tags'), { target: { value: 'x' } });
    expect(screen.getByRole('button', { name: 'Preview changes' })).toBeDisabled();
    expect(screen.getByLabelText('Add tags')).toHaveFocus();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });
});

describe('tag rename and aliases', () => {
  const list = { tags: [{ tag: 'bg', count: 12 }, { tag: 'Brand A', count: 3 }], truncated: false, aliases: [{ alias: 'backdrop', canonical: 'bg' }] };
  it('previews the image scope, reports unchanged audio and keeps an alias', async () => {
    invoke.mockImplementation((cmd: string) => Promise.resolve(
      cmd === 'image_tags' ? list
        : cmd === 'preview_image_tag_rename' ? { from: 'bg', to: 'background', images: 12, merged: 2, unavailable: 1, audio_unchanged: 4 }
          : cmd === 'rename_image_tag' ? { action_id: 'r1', results: [], applied: 12, unchanged: 0, conflicts: 0, skipped: 1 } : null));
    render(<TagManagerDialog sourceId={null} sourceName="all sources" onClose={vi.fn()} onChanged={vi.fn()} />);
    fireEvent.click(await screen.findByRole('button', { name: /^bg 12/ }));
    const to = screen.getByLabelText('New name');
    fireEvent.change(to, { target: { value: 'background' } });
    expect(screen.getByRole('checkbox', { name: /Keep “bg” as an alias/ })).toBeChecked();
    fireEvent.click(screen.getByRole('button', { name: 'Preview rename' }));
    const preview = await screen.findByText(/will change/);
    expect(preview).toHaveTextContent('12 images will change (2 already have “background” and will keep one). 1 offline or missing image keeps “bg”. 4 audio sounds use this tag and are not changed.');
    fireEvent.click(screen.getByRole('button', { name: 'Rename' }));
    await waitFor(() => expect(calls('rename_image_tag')[0]).toEqual({ from: 'bg', to: 'background', sourceId: null, keepAlias: true }));
    expect(await screen.findByRole('button', { name: 'Undo rename' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Remove alias backdrop' }));
    await waitFor(() => expect(calls('remove_image_tag_alias')).toEqual([{ alias: 'backdrop' }]));
  });
  it('disables aliases for a one-source rename and for a respelling', async () => {
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_tags' ? list : { from: 'bg', to: 'BG', images: 1, merged: 0, unavailable: 0, audio_unchanged: 0 }));
    render(<TagManagerDialog sourceId="s1" sourceName="Shoot" onClose={vi.fn()} onChanged={vi.fn()} />);
    expect(await screen.findByText('Showing tags in Shoot. Renaming changes image tags only; audio tags are unchanged.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /^bg 12/ }));
    expect(screen.getByRole('checkbox', { name: /alias/ })).toBeDisabled();
    cleanup();
    render(<TagManagerDialog sourceId={null} sourceName="all sources" onClose={vi.fn()} onChanged={vi.fn()} />);
    fireEvent.click(await screen.findByRole('button', { name: /^bg 12/ }));
    fireEvent.change(screen.getByLabelText('New name'), { target: { value: ' BG ' } });
    expect(screen.getByRole('checkbox', { name: /alias/ })).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: /alias/ })).toBeDisabled();
  });
});

describe('history', () => {
  it('lists recent actions and undoes only applied ones', async () => {
    let actions = [
      { id: 'n', kind: 'bulk', label: 'Add “Brand A” · 80 images', state: 'applied', created_at: 1_760_000_000, items: 80 },
      { id: 'o', kind: 'edit', label: 'Rate 3/5 · 1 image', state: 'undone', created_at: 1_759_000_000, items: 1 },
    ];
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'image_annotation_history') return Promise.resolve(actions);
      actions = actions.map(a => a.id === 'n' ? { ...a, state: 'undone' } : a);
      return Promise.resolve({ action_id: 'n', results: [], applied: 78, unchanged: 0, conflicts: 2, skipped: 0 });
    });
    const onChanged = vi.fn();
    render(<AnnotationHistoryDialog onClose={vi.fn()} onChanged={onChanged} />);
    const list = await screen.findByRole('list');
    expect(within(list).getAllByRole('listitem')).toHaveLength(2);
    expect(within(list).getAllByRole('button')).toHaveLength(1);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Undo Add “Brand A” · 80 images' })); });
    expect(await screen.findByText(/Restored 78 images · 2 images changed elsewhere/)).toBeInTheDocument();
    expect(onChanged).toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole('button', { name: /^Undo/ })).not.toBeInTheDocument());
  });
});

describe('tag text helpers', () => {
  it('split comma lists and mirror the core identity', () => {
    expect(splitTags(' a, ,b c ,')).toEqual(['a', 'b c']);
    expect(tagIdentity('Brand_A  Summer')).toBe(tagIdentity('brand a summer'));
    expect(tagIdentity('Café')).toBe(tagIdentity('CAFÉ'));
  });
});
