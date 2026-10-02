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
