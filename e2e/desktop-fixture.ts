import type { Page } from '@playwright/test';

/** Renderer contract fixture only. Native media is verified in Rust/FFmpeg tests. */
export async function installDesktopFixture(page: Page, images = false) {
  await page.addInitScript((images: boolean) => {
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
    let mediaJob: any = null;
    // Synthetic 16x12 RGBA PNG (opaque red | transparent and half-alpha blue) in the SS-044 envelope.
    const previewPng = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAABAAAAAMCAYAAABr5z2BAAAAIklEQVR4nGO4Y6PxHx/WiLrDgA8zjBpAHwMa8OFRA6hgAABstnLwKEaeUAAAAABJRU5ErkJggg=='), c => c.charCodeAt(0));
    const previewEnvelope = (meta: object) => {
      const json = new TextEncoder().encode(JSON.stringify(meta)), out = new Uint8Array(8 + json.length + previewPng.length);
      out.set([0x43, 0x53, 0x50, 0x56]); new DataView(out.buffer).setUint32(4, json.length, true); out.set(json, 8); out.set(previewPng, 8 + json.length);
      return out.buffer;
    };
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
            case 'image_import_capabilities': return { enabled: images, reference_in_place: true, production_images: false };
            case 'image_import_jobs': return mediaJob ? [mediaJob] : [];
            case 'choose_media_import': return ['/素材/' + 'long-folder-'.repeat(30)];
            case 'preview_media_import': return { id: 'native-preview', request_id: args.requestId, mode: args.mode, scopes: [{ root: '/素材/' + 'long-folder-'.repeat(30), recursive: args.mode === 'mixed', files: args.mode === 'images' ? ['chosen.png'] : [] }], images: 2, audio: 0, unsupported: 1, inaccessible: 0, complete: true, issues: [], reference_in_place: true, expires_at: Date.now() / 1000 + 300 };
            case 'cancel_media_preview': return;
            case 'confirm_media_import': mediaJob = { id: 'image-job', source_id: 'source', source_generation: '0', sequence: '10', mode: 'mixed', state: 'failed', phase: 'complete', discovery_complete: true, history_truncated: false, total: 2, completed: 2, ready: 1, reused: 0, failed: 1, unsupported: 0, current: '', error_code: 'DECODE_FAILED' }; return [mediaJob];
            case 'image_import_items': return { items: [{ job_id: 'image-job', relative_path: 'chosen.png', media_kind: 'image', state: 'ready', asset_id: 'image-one', content_hash: 'a'.repeat(64), error_code: null, attempts: 1, warnings: ['assumed_srgb'] }, { job_id: 'image-job', relative_path: 'corrupt.png', media_kind: 'image', state: 'failed', asset_id: null, content_hash: null, error_code: 'DECODE_FAILED', attempts: 1, warnings: [] }], next_after: null };
            case 'imported_images': return { items: [{ header: { id: 'image-one', kind: 'image', source_id: 'source', relative_path: 'chosen.png', content_hash: 'a'.repeat(64), technical_state: 'ready', availability: 'available', enrichment_state: 'disabled' }, width: 640, height: 480, annotation_revision: 1 },
              { header: { id: 'image-two', kind: 'image', source_id: 'source', relative_path: 'offline-' + '長い名前-'.repeat(12) + '.png', content_hash: 'b'.repeat(64), technical_state: 'ready', availability: 'offline', enrichment_state: 'disabled' }, width: 640, height: 480, annotation_revision: 1 }], next_after: null };
            case 'image_preview': { const offline = args.assetId === 'image-two';
              return previewEnvelope({ asset_id: args.assetId, version: args.version, tier: args.tier, tile: args.tile, width: 16, height: 12, source_width: 640, source_height: 480, mime: 'image/png', transparency: true, cache: 'hit', original_available: !offline, offline_reference: offline, preview_version: 'creativeshelf-preview-v1' }); }
            case 'cancel_image_preview': return false;
            case 'image_preview_cache_status': return images ? { enabled: true, entries: 6, bytes: 48_000, quota_bytes: 512 * 1024 * 1024, active: 0, waiting: 0, counters: { hits: 9, misses: 3, generated: 2, coalesced: 0, evicted: 0, repaired: 0, write_failures: 0, cancelled: 1, stale_rejected: 0, offline_served: 1 } }
              : { enabled: false, entries: 0, bytes: 0, quota_bytes: 512 * 1024 * 1024, active: 0, waiting: 0, counters: { hits: 0, misses: 0, generated: 0, coalesced: 0, evicted: 0, repaired: 0, write_failures: 0, cancelled: 0, stale_rejected: 0, offline_served: 0 } };
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
  }, images);
}
