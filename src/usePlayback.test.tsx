import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { usePlayback } from './hooks/usePlayback';
const invoke=vi.fn();
vi.mock('@tauri-apps/api/core',()=>({isTauri:()=>true,invoke:(...args:unknown[])=>invoke(...args)}));
const playing={sound_id:'sound-a',state:'playing',position_seconds:4,duration_seconds:12,volume:1,peak:0,error:null};
beforeEach(()=>{vi.useFakeTimers();invoke.mockReset();invoke.mockImplementation((cmd:string)=>Promise.resolve(cmd==='playback_status'?playing:null));});
afterEach(()=>vi.useRealTimers());
describe('playback command reconciliation',()=>{
  it('ignores a status response captured before pause, even when it arrives after pause completes',async()=>{
    const {result}=renderHook(()=>usePlayback(null,[]));
    await act(async()=>vi.advanceTimersByTimeAsync(120));
    let finish!:(status:unknown)=>void;
    invoke.mockImplementation((cmd:string)=>cmd==='playback_status'?new Promise(resolve=>{finish=resolve;}):Promise.resolve());
    await act(async()=>vi.advanceTimersByTimeAsync(120));
    await act(async()=>{await result.current.togglePlay();});
    expect(result.current.playback?.state).toBe('paused');
    await act(async()=>finish(playing));
    expect(result.current.playback?.state).toBe('paused');
    expect(result.current.playback?.position_seconds).toBe(4);
  });
  it('serializes rapid toggles until the pause command resolves',async()=>{
    const {result}=renderHook(()=>usePlayback(null,[]));
    await act(async()=>vi.advanceTimersByTimeAsync(120));
    let finish!:()=>void;
    invoke.mockImplementation((cmd:string)=>cmd==='playback_pause'?new Promise<void>(resolve=>{finish=resolve;}):Promise.resolve(null));
    act(()=>{void result.current.togglePlay();void result.current.togglePlay();});
    expect(invoke.mock.calls.filter(c=>c[0]==='playback_pause')).toHaveLength(1);
    expect(invoke.mock.calls.filter(c=>c[0]==='playback_resume')).toHaveLength(0);
    await act(async()=>finish());
    expect(result.current.isPlayPending).toBe(false);
  });
  it('restores current position and exposes failure when pause fails',async()=>{
    const {result}=renderHook(()=>usePlayback(null,[]));
    await act(async()=>vi.advanceTimersByTimeAsync(120));
    invoke.mockRejectedValueOnce(new Error('Pause failed'));
    await act(async()=>{await result.current.togglePlay();});
    expect(result.current.playback?.state).toBe('playing');
    expect(result.current.playback?.position_seconds).toBe(4);
    expect(result.current.error).toContain('Pause failed');
  });
});

it('restores prior playback when starting a different sound fails', async () => {
  const { result } = renderHook(() => usePlayback(null, []));
  await act(async () => vi.advanceTimersByTimeAsync(120));
  invoke.mockRejectedValueOnce(new Error('Output device is unavailable'));
  await act(async () => { await result.current.playSound({ id: 'missing', title: 'Missing sound' } as never); });
  expect(result.current.playback).toEqual(playing);
  expect(result.current.isPlayPending).toBe(false);
  expect(result.current.error).toContain('Output device is unavailable');
});

it('retries play after the backend reports an error', async () => {
  invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'playback_status' ? { ...playing, state: 'error' } : null));
  const { result } = renderHook(() => usePlayback({ id: 'sound-a', title: 'Sound A' } as never, []));
  await act(async () => vi.advanceTimersByTimeAsync(120));
  await act(async () => { await result.current.togglePlay(); });
  expect(invoke).toHaveBeenCalledWith('playback_play', { id: 'sound-a' });
});

it('serializes Stop with Play and rolls back a rejected stop', async () => {
  const { result } = renderHook(() => usePlayback(null, []));
  await act(async () => vi.advanceTimersByTimeAsync(120));
  let fail!: (error: Error) => void;
  invoke.mockImplementation((cmd: string) => cmd === 'playback_stop' ? new Promise((_, reject) => { fail = reject; }) : Promise.resolve(null));
  act(() => { void result.current.stopPlayback(); void result.current.togglePlay(); });
  expect(invoke.mock.calls.filter(call => call[0] === 'playback_play')).toHaveLength(0);
  expect(result.current.isPlayPending).toBe(true);
  await act(async () => fail(new Error('Stop failed')));
  expect(result.current.playback).toEqual(playing);
  expect(result.current.error).toContain('Stop failed');
});

it('keeps only one status request in flight when IPC stalls', async () => {
  invoke.mockImplementation(() => new Promise(() => {}));
  const { unmount } = renderHook(() => usePlayback(null, []));
  await act(async () => vi.advanceTimersByTimeAsync(1200));
  expect(invoke.mock.calls.filter(call => call[0] === 'playback_status')).toHaveLength(1);
  unmount();
});

it('coalesces rapid seeks and ignores status captured before seeking', async () => {
  const { result } = renderHook(() => usePlayback(null, []));
  await act(async () => vi.advanceTimersByTimeAsync(120));
  let oldStatus!: (value: unknown) => void;
  let finishSeek!: () => void;
  invoke.mockImplementation((cmd: string) => {
    if (cmd === 'playback_status') return new Promise(resolve => { oldStatus = resolve; });
    if (cmd === 'playback_seek') return new Promise<void>(resolve => { finishSeek = resolve; });
    return Promise.resolve(null);
  });
  await act(async () => vi.advanceTimersByTimeAsync(120));
  act(() => { void result.current.seekPlayback(1); void result.current.seekPlayback(2); void result.current.seekPlayback(3); });
  expect(invoke.mock.calls.filter(call => call[0] === 'playback_seek')).toHaveLength(1);
  await act(async () => finishSeek());
  expect(invoke.mock.calls.filter(call => call[0] === 'playback_seek').map(call => call[1])).toEqual([{positionSeconds: 1}, {positionSeconds: 3}]);
  await act(async () => finishSeek());
  await act(async () => oldStatus(playing));
  expect(result.current.playback?.position_seconds).toBe(3);
  expect(result.current.isPlayPending).toBe(false);
});
