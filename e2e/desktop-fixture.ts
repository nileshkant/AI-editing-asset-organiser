import type { Page } from '@playwright/test';

/** Renderer contract fixture only. Native media is verified in Rust/FFmpeg tests. */
export async function installDesktopFixture(page: Page) {
  await page.addInitScript(() => {
    const profile = {
      duration: 10, sample_rate: 48000, channels: 2, frames: 480000,
      peak: 0.7, rms: 0.2, description: 'Measured stereo recording', tags: ['stereo', 'transient'],
      waveform: Array.from({ length: 80 }, (_, index) => [-Math.abs(Math.sin(index) * 0.7), Math.abs(Math.cos(index) * 0.7)]),
    };
    const sounds = Array.from({ length: 80 }, (_, index) => ({
      id: `sound-${index}`, source_id: 'source', relative_path: `recordings/recording-${index}.wav`,
      title: index === 0 ? 'Cinematic whoosh — a very long recording title to exercise narrow window truncation' : `Recording ${index + 1}`,
      content_hash: `hash-${index}`, status: 'ready', user_tags: ['trailer'], comment: '', favorite: false, profile,
    }));
    const source = { id: 'source', name: 'Field recordings with a long source name',
      root: '/Volumes/Production/Audio/Field recordings with a long folder name/Session 2026/Originals',
      generation: 0, available: true, scope: 'folder', files: [] };
    let playback = { sound_id: null as string | null, state: 'stopped', position_seconds: 0, duration_seconds: 0, volume: 1, peak: 0, error: null };
    const calls: { command: string; args: any }[] = [];
    Object.assign(window, {
      isTauri: true,
      __fixtureCalls: calls,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
        transformCallback: () => 1,
        invoke: async (command: string, args: any = {}) => {
          calls.push({ command, args });
          switch (command) {
            case 'plugin:event|listen': return 1;
            case 'plugin:event|unlisten': return;
            case 'app_info': return { version: '0.1.0-test', data_directory: '/disposable-test-data', desktop: true, media_tools: true };
            case 'jobs': return [];
            case 'sources': return [source];
            case 'search_sounds': {
              let items = sounds.filter(sound => !args.query.favorites_only || sound.favorite);
              if (args.query.text) items = items.filter(sound => sound.title.toLowerCase().includes(args.query.text.toLowerCase()));
              return { items: items.slice(args.query.offset, args.query.offset + 75), total: items.length,
                interpretation: { terms: [], excluded: [], corrected: [], min_duration: null, max_duration: null } };
            }
            case 'get_sound': return sounds.find(sound => sound.id === args.id);
            case 'annotate': Object.assign(sounds.find(sound => sound.id === args.id)!, { user_tags: args.tags, comment: args.comment }); return;
            case 'set_favorite': Object.assign(sounds.find(sound => sound.id === args.id)!, { favorite: args.favorite }); return;
            case 'list_clips': return [];
            case 'get_waveform': return { channels: [profile.waveform.map(([min, max]) => ({ min, max, rms: 0.2 }))] };
            case 'playback_status': return playback;
            case 'playback_play': playback = { ...playback, sound_id: args.id, state: 'playing', duration_seconds: 10, position_seconds: 0 }; return;
            case 'playback_pause': playback = { ...playback, state: 'paused' }; return;
            case 'playback_resume': playback = { ...playback, state: 'playing' }; return;
            case 'playback_stop': playback = { ...playback, state: 'stopped', position_seconds: 0 }; return;
            case 'playback_seek': playback = { ...playback, position_seconds: args.positionSeconds }; return;
            case 'playback_set_volume': playback = { ...playback, volume: args.volume }; return;
            case 'folder_catalog_states': return [{ source_id: 'source', dirty: false, error: '' }];
            case 'resource_settings': return { preferences: { output_device: null, imports_paused: false }, output_devices: ['Built-in output'], import_workers: 1 };
            case 'mcp_status': return { endpoint: null, clients: [], startup_error: null };
            case 'relink_source': return source;
            case 'choose_folder': case 'choose_file': return null;
            default: throw new Error(`Unmodeled fixture command: ${command}`);
          }
        },
      },
    });
  });
}
