import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAudioSearch } from './hooks/useAudioSearch';
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));
const source = (name: string) => ({ id: 's1', name, root: '/audio', generation: 0, available: true, scope: 'folder', files: [] });
const empty = { items: [], total: 0, interpretation: { terms: [], excluded: [], min_duration: null, max_duration: null, corrected: [] } };
beforeEach(() => { vi.useFakeTimers(); invoke.mockReset(); });
afterEach(() => vi.useRealTimers());

describe('audio workspace polling', () => {
  it('keeps the same sources and jobs while the polled content is unchanged', async () => {
    let name = 'Field recordings';
    // A new array and new objects on every call, as IPC delivers them.
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'sources' ? [source(name)] : cmd === 'jobs' ? [] : cmd === 'search_sounds' ? empty : null));
    let renders = 0;
    const { result } = renderHook(() => { ++renders; return useAudioSearch('library'); });
    // Includes the one extra pass React makes before it bails out of an unchanged update.
    for (let i = 0; i < 3; ++i) await act(async () => vi.advanceTimersByTimeAsync(1500));
    const roots = result.current.roots, jobs = result.current.jobs, settled = renders;
    expect(roots).toHaveLength(1);
    await act(async () => vi.advanceTimersByTimeAsync(6000));
    expect(invoke.mock.calls.filter(c => c[0] === 'sources').length).toBeGreaterThanOrEqual(4);
    expect(result.current.roots).toBe(roots);
    expect(result.current.jobs).toBe(jobs);
    expect(renders).toBe(settled);
    name = 'Renamed';
    await act(async () => vi.advanceTimersByTimeAsync(1500));
    expect(result.current.roots[0].name).toBe('Renamed');
  });
});
