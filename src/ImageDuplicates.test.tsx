import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DuplicatesDialog } from './components/images/DuplicatesDialog';
import type { DuplicateGroup, DuplicateMember, DuplicatePage } from './types';
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);
const hash = 'a'.repeat(64);
const member = (n: number, over: Partial<DuplicateMember> = {}): DuplicateMember => ({
  image: {
    header: { id: `00000000-0000-4000-8000-00000000000${n}`, kind: 'image', source_id: 's1', relative_path: `set/copy-${n}.png`, content_hash: hash, technical_state: 'ready', availability: 'available', enrichment_state: 'disabled' },
    width: 8, height: 8, annotation_revision: 1,
  },
  source_name: 'Shoot', separated: false, new_since_dismissed: false, ...over,
});
const group = (over: Partial<DuplicateGroup> = {}): DuplicateGroup => ({
  kind: 'exact', content_hash: hash, revision: 0, member_count: 3, separated_count: 0, members: [member(1), member(2), member(3)], members_truncated: false,
  preferred_id: null, preferred_state: 'none', representative_id: member(1).image.header.id, dismissed: false, hidden: false, ...over,
});
const page = (groups: DuplicateGroup[], over: Partial<DuplicatePage> = {}): DuplicatePage => ({ groups, offset: 0, total: groups.length, open_groups: groups.length, extra_copies: 2, hidden_groups: 0, ...over });
const open = () => render(<DuplicatesDialog sourceId={null} sourceName="all sources" background="checkerboard" setBackground={vi.fn()} onClose={vi.fn()} />);
beforeEach(() => {
  invoke.mockReset();
  // Previews are not under test; keep them pending.
  invoke.mockImplementation((cmd: string) => cmd === 'image_preview' ? new Promise(() => {}) : Promise.reject('UNMOCKED'));
});
afterEach(cleanup);
function route(handlers: Record<string, (args: Record<string, unknown>) => unknown>) {
  invoke.mockImplementation((cmd: string, args: Record<string, unknown>) => {
    if (cmd === 'image_preview') return new Promise(() => {});
    if (cmd === 'cancel_image_preview') return Promise.resolve(true);
    return handlers[cmd] ? Promise.resolve().then(() => handlers[cmd](args)) : Promise.reject('UNMOCKED');
  });
}

describe('duplicate stacks dialog', () => {
  it('labels stacks as exact copies and states that nothing is deleted', async () => {
    route({ image_duplicate_groups: () => page([group()]) });
    open();
    expect(await screen.findByText('1 stack · 2 extra copies')).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 3 })).toHaveTextContent('Exact copies 3 identical copies');
    expect(screen.getByText(/Nothing is deleted or merged/)).toBeInTheDocument();
    expect(calls('image_duplicate_groups')[0]).toEqual({ sourceId: null, includeHidden: false, offset: 0, limit: 25 });
    // Only decisions are offered: no delete or merge control exists.
    expect(screen.queryByRole('button', { name: /delete|remove|merge/i })).toBeNull();
  });
  it('sends each decision from the revision it read and shows the result', async () => {
    const preferred = group({ revision: 1, preferred_id: member(2).image.header.id, preferred_state: 'available', representative_id: member(2).image.header.id });
    route({ image_duplicate_groups: () => page([group()]), update_image_duplicate_group: () => preferred });
    open();
    fireEvent.click(await screen.findByRole('button', { name: 'Prefer copy-2.png in Shoot' }));
    await waitFor(() => expect(calls('update_image_duplicate_group')).toEqual([{ sourceId: null, contentHash: hash, expectedRevision: 0, action: { action: 'prefer', asset_id: member(2).image.header.id } }]));
    const row = (await screen.findByText('Preferred')).closest('li')!;
    expect(within(row).getByText('copy-2.png')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Clear preference' })).toBeInTheDocument();
  });
  it('reloads the stack on a conflict instead of retrying', async () => {
    const fresh = group({ revision: 4, dismissed: true });
    route({ image_duplicate_groups: () => page([group()]), update_image_duplicate_group: () => { throw 'CONFLICT'; }, image_duplicate_group: () => fresh });
    open();
    fireEvent.click(await screen.findByRole('button', { name: 'Dismiss' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('changed elsewhere');
    expect(calls('update_image_duplicate_group')).toHaveLength(1);
    expect(calls('image_duplicate_group')).toEqual([{ sourceId: null, contentHash: hash }]);
    expect(await screen.findByRole('button', { name: 'Show again' })).toBeInTheDocument();
  });
  it('explains an unavailable preferred copy and marks separated and new copies', async () => {
    const g = group({
      preferred_id: member(1).image.header.id, preferred_state: 'missing', dismissed: true, separated_count: 1,
      members: [member(2, { new_since_dismissed: true }), member(3, { image: { ...member(3).image, header: { ...member(3).image.header, availability: 'offline' } } }), member(4, { separated: true })],
    });
    route({ image_duplicate_groups: () => page([g]) });
    open();
    expect(await screen.findByText(/Preferred copy is missing/)).toBeInTheDocument();
    expect(screen.getByText('New since dismissed')).toBeInTheDocument();
    expect(screen.getByText('Offline')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Return copy-4.png in Shoot to the stack' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Prefer copy-4.png in Shoot' })).toBeNull();
  });
  it('lists dismissed stacks on request and pages without duplicates', async () => {
    const second = group({ content_hash: 'b'.repeat(64), members: [member(5), member(6)], member_count: 2 });
    route({
      image_duplicate_groups: args => args.offset === 0
        ? page([group()], { total: 2, open_groups: 2 })
        : page([group(), second], { offset: 1, total: 2, open_groups: 2 }),
    });
    open();
    fireEvent.click(await screen.findByRole('button', { name: 'Show more stacks' }));
    await waitFor(() => expect(screen.getAllByRole('heading', { level: 3 })).toHaveLength(2));
    fireEvent.click(screen.getByRole('checkbox', { name: /Show dismissed and resolved/ }));
    await waitFor(() => expect(calls('image_duplicate_groups').at(-1)).toEqual({ sourceId: null, includeHidden: true, offset: 0, limit: 25 }));
  });
});
