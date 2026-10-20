import type { Page } from '@playwright/test';

/** Renderer contract fixture only. Native media is verified in Rust/FFmpeg tests. */
export async function installDesktopFixture(page: Page, images = false, galleryTotal = 240) {
  await page.addInitScript(([images, galleryTotal]: [boolean, number]) => {
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
    // SS-047 renderer contract: one image source with a missing file and a possible move.
    let imageSource: any = { source_id: 'source', name: source.name, root: source.root, scope: 'folder', source_generation: '0', root_state: 'offline',
      watch: false, mode: 'mixed', checked_at: 0, images: 3, missing: 1, inaccessible: 0, failed: 1, pending: 0, move_candidates: 1, active_job: null, watch_state: null };
    let moves = [{ missing_id: 'image-old', missing_path: 'shoot/before-move.png', missing_revision: 3, present_id: 'image-new', present_path: 'shoot/after/' + '長い名前-'.repeat(8) + '.png', present_revision: 1, same_file: false }];
    // Synthetic 16x12 RGBA PNG (opaque red | transparent and half-alpha blue) in the SS-044 envelope.
    const previewPng = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAABAAAAAMCAYAAABr5z2BAAAAIklEQVR4nGO4Y6PxHx/WiLrDgA8zjBpAHwMa8OFRA6hgAABstnLwKEaeUAAAAABJRU5ErkJggg=='), c => c.charCodeAt(0));
    const previewEnvelope = (meta: object) => {
      const json = new TextEncoder().encode(JSON.stringify(meta)), out = new Uint8Array(8 + json.length + previewPng.length);
      out.set([0x43, 0x53, 0x50, 0x56]); new DataView(out.buffer).setUint32(4, json.length, true); out.set(json, 8); out.set(previewPng, 8 + json.length);
      return out.buffer;
    };
    const calls: { command: string; args: any }[] = [];
    // Deterministic synthetic image library for SS-045 (metadata only).
    const pad = (n: number, w: number) => String(n).padStart(w, '0');
    const galleryItem = (i: number) => ({ header: { id: `00000000-0000-4000-8000-${pad(i, 12)}`, kind: 'image', source_id: 'source',
      relative_path: `shoot/${i === 3 ? '長い名前-'.repeat(10) : ''}image-${pad(i, 6)}.png`, content_hash: (i.toString(16) + 'a'.repeat(64)).slice(0, 64),
      technical_state: i % 37 === 5 ? 'pending' : 'ready', availability: i % 50 === 7 ? 'offline' : 'available', enrichment_state: 'disabled' },
      width: 4000, height: 3000, annotation_revision: notesFor(i).revision });
    // SS-046 renderer contract: revisioned annotations, history with undo, tag rename/aliases.
    type Notes = { revision: number; tags: string[]; comment: string; favorite: boolean; rating: number | null };
    const notes = new Map<number, Notes>(), history: any[] = [], aliases: { alias: string; canonical: string }[] = [];
    const key = (t: string) => t.normalize('NFC').replace(/_/g, ' ').split(/\s+/).filter(Boolean).join(' ').toLowerCase();
    function notesFor(i: number): Notes {
      if (!notes.has(i)) notes.set(i, { revision: 1, tags: i === 3 ? ['very-long-tag-'.repeat(6), 'thumbnail'] : ['thumbnail'], comment: '', favorite: false, rating: null });
      return notes.get(i)!;
    }
    const indexOf = (id: string) => parseInt(id.slice(-12), 10);
    const change = (n: Notes, p: any): Notes => {
      const remove = new Set((p.remove_tags ?? []).map(key));
      const tags = n.tags.filter(t => !remove.has(key(t)));
      for (const raw of p.add_tags ?? []) { const t = raw.trim().replace(/_/g, ' '); const canonical = aliases.find(a => key(a.alias) === key(t))?.canonical ?? t; if (t && !tags.some(x => key(x) === key(canonical))) tags.push(canonical); }
      return { revision: n.revision, tags, comment: p.comment ?? n.comment, favorite: p.favorite ?? n.favorite, rating: 'rating' in p ? p.rating : n.rating };
    };
    const same = (a: Notes, b: Notes) => JSON.stringify({ ...a, revision: 0 }) === JSON.stringify({ ...b, revision: 0 });
    const record = (kind: string, label: string, entries: { i: number; before: Notes; after: number }[]) => {
      if (!entries.length) return null;
      const id = `00000000-0000-4000-9000-${pad(history.length + 1, 12)}`;
      history.unshift({ id, kind, label, state: 'applied', created_at: 1_760_000_000 + history.length, items: entries.length, entries });
      return id;
    };
    const editable = (i: number) => galleryItem(i).header.availability === 'available';
    const bulk = (targets: { id: string; revision: number }[], fn: (n: Notes) => Notes, kind: string, label: string) => {
      const results: any[] = [], entries: any[] = [];
      for (const t of targets) {
        const i = indexOf(t.id), n = notesFor(i);
        if (!editable(i)) { results.push({ id: t.id, outcome: 'source_unavailable', revision: null }); continue; }
        if (n.revision !== t.revision) { results.push({ id: t.id, outcome: 'conflict', revision: n.revision }); continue; }
        const next = fn(n);
        if (same(next, n)) { results.push({ id: t.id, outcome: 'unchanged', revision: n.revision }); continue; }
        notes.set(i, { ...next, revision: n.revision + 1 });
        entries.push({ i, before: n, after: n.revision + 1 });
        results.push({ id: t.id, outcome: 'applied', revision: n.revision + 1 });
      }
      const count = (o: string[]) => results.filter(r => o.includes(r.outcome)).length;
      return { action_id: record(kind, `${label} · ${entries.length} ${entries.length === 1 ? 'image' : 'images'}`, entries), results, applied: count(['applied']), unchanged: count(['unchanged']), conflicts: count(['conflict']), skipped: count(['source_unavailable']) };
    };
    const describe = (p: any) => [p.add_tags?.length && `Add ${p.add_tags.map((t: string) => `“${t}”`).join(', ')}`, p.remove_tags?.length && `remove ${p.remove_tags.map((t: string) => `“${t}”`).join(', ')}`,
      p.favorite !== undefined && (p.favorite ? 'mark favorite' : 'unmark favorite'), 'rating' in p && (p.rating ? `rate ${p.rating}/5` : 'clear rating'), p.comment !== undefined && 'edit notes'].filter(Boolean).join(', ');
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
            case 'confirm_media_import': mediaJob = { id: 'image-job', source_id: 'source', source_generation: '0', sequence: '10', mode: 'mixed', state: 'failed', phase: 'complete', discovery_complete: true, history_truncated: false, total: 2, completed: 2, ready: 1, reused: 0, failed: 1, unsupported: 0, current: '', error_code: 'DECODE_FAILED', scan: 'discover', verify: true, origin: 'import' }; return [mediaJob];
            case 'image_import_items': return { items: [{ job_id: 'image-job', relative_path: 'chosen.png', media_kind: 'image', state: 'ready', asset_id: 'image-one', content_hash: 'a'.repeat(64), error_code: null, attempts: 1, warnings: ['assumed_srgb'] }, { job_id: 'image-job', relative_path: 'corrupt.png', media_kind: 'image', state: 'failed', asset_id: null, content_hash: null, error_code: 'DECODE_FAILED', attempts: 1, warnings: [] }], next_after: null };
            case 'imported_images': return { items: [{ header: { id: 'image-one', kind: 'image', source_id: 'source', relative_path: 'chosen.png', content_hash: 'a'.repeat(64), technical_state: 'ready', availability: 'available', enrichment_state: 'disabled' }, width: 640, height: 480, annotation_revision: 1 },
              { header: { id: 'image-two', kind: 'image', source_id: 'source', relative_path: 'offline-' + '長い名前-'.repeat(12) + '.png', content_hash: 'b'.repeat(64), technical_state: 'ready', availability: 'offline', enrichment_state: 'disabled' }, width: 640, height: 480, annotation_revision: 1 }], next_after: null };
            case 'image_preview': { const offline = args.assetId === 'image-two';
              return previewEnvelope({ asset_id: args.assetId, version: args.version, tier: args.tier, tile: args.tile, width: 16, height: 12, source_width: 640, source_height: 480, mime: 'image/png', transparency: true, cache: 'hit', original_available: !offline, offline_reference: offline, preview_version: 'creativeshelf-preview-v1' }); }
            case 'cancel_image_preview': return false;
            case 'image_gallery_page': {
              const total = images ? galleryTotal : 0, end = Math.min(total, args.offset + args.limit), items = [];
              for (let i = args.offset; i < end; ++i) items.push(galleryItem(args.sort === 'name_desc' ? total - 1 - i : i));
              return { items, offset: args.offset, total };
            }
            case 'image_details': { const i = parseInt(args.id.slice(-12), 10), item = galleryItem(i);
              return { header: item.header, profile: item.header.technical_state === 'ready' ? { raw_width: 3000, raw_height: 4000, orientation: 6, width: 4000, height: 3000, format: 'png', encoded_bytes: 9_437_184, bit_depth: 8, source_color_profile: null, alpha_channel: true, transparency: 'partial', analyzer_version: 'creativeshelf-still-v1-image02510-mox091' } : null,
                source_name: source.name, source_available: true, caption: null, tags: notesFor(i).tags, comment: notesFor(i).comment, favorite: notesFor(i).favorite, rating: notesFor(i).rating, annotation_revision: notesFor(i).revision }; }
            case 'edit_image_annotations': { const i = indexOf(args.id), n = notesFor(i);
              if (!editable(i)) throw 'SOURCE_UNAVAILABLE';
              if (n.revision !== args.expectedRevision) throw 'CONFLICT';
              const r = bulk([{ id: args.id, revision: n.revision }], x => change(x, args.patch), 'edit', describe(args.patch));
              return { annotations: notesFor(i), action_id: r.action_id }; }
            case 'preview_image_bulk_annotations': {
              const ids = args.ids as string[], ok = ids.filter(id => editable(indexOf(id)));
              const has = (t: string) => ok.filter(id => notesFor(indexOf(id)).tags.some(x => key(x) === key(t))).length;
              return { targets: ok.map(id => ({ id, revision: notesFor(indexOf(id)).revision })), changes: ok.filter(id => !same(change(notesFor(indexOf(id)), args.patch), notesFor(indexOf(id)))).length,
                unchanged: ok.filter(id => same(change(notesFor(indexOf(id)), args.patch), notesFor(indexOf(id)))).length,
                skipped: ids.filter(id => !editable(indexOf(id))).map(id => ({ id, outcome: 'source_unavailable', revision: null })),
                additions: (args.patch.add_tags ?? []).map((tag: string) => ({ tag, count: has(tag) })), removals: (args.patch.remove_tags ?? []).map((tag: string) => ({ tag, count: has(tag) })) }; }
            case 'apply_image_bulk_annotations': return bulk(args.targets, x => change(x, args.patch), 'bulk', describe(args.patch));
            case 'image_annotation_history': return history.slice(0, args.limit).map(({ entries, ...a }) => a);
            case 'undo_image_annotation_action': { const a = history.find(h => h.id === args.actionId);
              if (!a) throw 'NOT_FOUND'; if (a.state !== 'applied') throw 'CONFLICT';
              const results = a.entries.map((e: any) => { const n = notesFor(e.i);
                if (n.revision !== e.after) return { id: galleryItem(e.i).header.id, outcome: 'conflict', revision: n.revision };
                notes.set(e.i, { ...e.before, revision: n.revision + 1 }); return { id: galleryItem(e.i).header.id, outcome: 'applied', revision: n.revision + 1 }; });
              if (a.alias) aliases.splice(aliases.indexOf(a.alias), 1);
              a.state = 'undone';
              return { action_id: a.id, results, applied: results.filter((r: any) => r.outcome === 'applied').length, unchanged: 0, conflicts: results.filter((r: any) => r.outcome === 'conflict').length, skipped: 0 }; }
            case 'image_tags': { const counts = new Map<string, { tag: string; count: number }>();
              for (let i = 0; i < (images ? galleryTotal : 0); ++i) for (const t of notesFor(i).tags) { const c = counts.get(key(t)) ?? { tag: t, count: 0 }; c.count++; counts.set(key(t), c); }
              return { tags: [...counts.values()].sort((a, b) => b.count - a.count), truncated: false, aliases }; }
            case 'preview_image_tag_rename': { let n = 0, merged = 0, unavailable = 0;
              for (let i = 0; i < galleryTotal; ++i) { const t = notesFor(i).tags; if (!t.some(x => key(x) === key(args.from))) continue;
                if (!editable(i)) unavailable++; else { n++; if (key(args.from) !== key(args.to) && t.some(x => key(x) === key(args.to))) merged++; } }
              return { from: args.from, to: args.to, images: n, merged, unavailable, audio_unchanged: sounds.filter(s => s.user_tags.some(t => key(t) === key(args.from))).length }; }
            case 'rename_image_tag': {
              const targets = []; for (let i = 0; i < galleryTotal; ++i) if (editable(i) && notesFor(i).tags.some(x => key(x) === key(args.from))) targets.push({ id: galleryItem(i).header.id, revision: notesFor(i).revision });
              const r = bulk(targets, n => { const has = key(args.from) !== key(args.to) && n.tags.some(x => key(x) === key(args.to));
                return { ...n, tags: n.tags.flatMap(t => key(t) === key(args.from) ? (has ? [] : [args.to]) : [t]) }; }, 'rename', `Rename “${args.from}” to “${args.to}”`);
              if (args.keepAlias) { const alias = { alias: args.from, canonical: args.to }; aliases.push(alias); history[0].alias = alias; }
              return r; }
            case 'remove_image_tag_alias': { const at = aliases.findIndex(a => key(a.alias) === key(args.alias)); if (at < 0) throw 'NOT_FOUND'; aliases.splice(at, 1); return null; }
            case 'image_preview_cache_status': return images ? { enabled: true, entries: 6, bytes: 48_000, quota_bytes: 512 * 1024 * 1024, active: 0, waiting: 0, counters: { hits: 9, misses: 3, generated: 2, coalesced: 0, evicted: 0, repaired: 0, write_failures: 0, cancelled: 1, stale_rejected: 0, offline_served: 1 } }
              : { enabled: false, entries: 0, bytes: 0, quota_bytes: 512 * 1024 * 1024, active: 0, waiting: 0, counters: { hits: 0, misses: 0, generated: 0, coalesced: 0, evicted: 0, repaired: 0, write_failures: 0, cancelled: 0, stale_rejected: 0, offline_served: 0 } };
            case 'image_source_statuses': return images ? [imageSource] : [];
            case 'check_image_source': imageSource = { ...imageSource, root_state: 'online', checked_at: 1_760_000_000 }; return imageSource;
            case 'set_image_source_watch': imageSource = { ...imageSource, watch: args.enabled, watch_state: args.enabled ? { state: 'watching', error: null } : null }; return imageSource;
            case 'refresh_image_source': mediaJob = { id: 'refresh-job', source_id: 'source', source_generation: imageSource.source_generation, sequence: '20', mode: 'mixed', state: 'complete', phase: 'complete', discovery_complete: true, history_truncated: false, total: 3, completed: 3, ready: 3, reused: 3, failed: 0, unsupported: 0, current: '', error_code: null, scan: args.scan, verify: args.verify, origin: 'refresh' }; return mediaJob;
            case 'choose_image_relink': return { id: 'relink-plan', source_id: 'source', source_generation: imageSource.source_generation, root: '/Volumes/New drive/' + 'long-folder-'.repeat(12), members: 3, present: 2, absent: 1, sampled: 2, matched: 1, changed: 1, unreadable: 0, complete: true, acknowledgement_required: true, expires_at: Date.now() / 1000 + 300 };
            case 'cancel_image_relink': return;
            case 'confirm_image_relink': imageSource = { ...imageSource, root_state: 'online', source_generation: '1' };
              mediaJob = { id: 'relink-job', source_id: 'source', source_generation: '1', sequence: '30', mode: 'mixed', state: 'queued', phase: 'discovery', discovery_complete: false, history_truncated: false, total: 0, completed: 0, ready: 0, reused: 0, failed: 0, unsupported: 0, current: '', error_code: null, scan: 'listed', verify: true, origin: 'relink' }; return mediaJob;
            case 'image_move_candidates': return moves;
            case 'reconcile_image_move': moves = []; imageSource = { ...imageSource, move_candidates: 0, missing: 0 }; return { id: args.missingId };
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
  }, [images, galleryTotal] as [boolean, number]);
}
