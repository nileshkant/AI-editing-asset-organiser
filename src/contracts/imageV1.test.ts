// @vitest-environment node
import { describe, expect, it } from 'vitest';
import base from '../../fixtures/contracts/image-v1.json';
import cases from '../../fixtures/contracts/image-v1-cases.json';
import ipc from '../../fixtures/contracts/image-edit-args.json';
import legacy from '../../fixtures/contracts/legacy-audio-v1.json';
import {
  IMAGE_LIMITS, ImageContractError, parseImageContract, validateImageContract, validateImageEditArgs,
} from './imageV1';
import type { Clip, Sound } from '../types';

type Edit = { path: string; value?: unknown; remove?: boolean };
function edited(value: unknown, edits: Edit[]): unknown {
  const copy = JSON.parse(JSON.stringify(value)) as Record<string, unknown>;
  for (const edit of edits) {
    const path = edit.path.slice(1).split('/');
    let parent: Record<string, unknown> = copy;
    for (const key of path.slice(0,-1)) parent = parent[key] as Record<string, unknown>;
    const key = path.at(-1)!;
    if (edit.remove) delete parent[key]; else parent[key] = edit.value;
  }
  return copy;
}
describe('SS-040 shared wire conformance', () => {
  it('round trips the complete typed bundle with explicit nulls', () => {
    const parsed = parseImageContract(JSON.stringify(base));
    expect(parsed).toEqual(base);
    expect(parseImageContract(JSON.stringify(parsed))).toEqual(parsed);
  });
  for (const test of cases) {
    it(test.name, () => {
      const value = edited(base, test.edits);
      if (test.valid) {
        expect(validateImageContract(value)).toEqual(value);
        expect(parseImageContract(JSON.stringify(value))).toEqual(value);
      } else {
        expect(() => validateImageContract(value)).toThrow(ImageContractError);
        expect(() => parseImageContract(JSON.stringify(value))).toThrow(ImageContractError);
        if ('error' in test && test.error) {
          try { validateImageContract(value); } catch (e) {
            expect((e as ImageContractError).code).toBe(test.error);
          }
        }
      }
    });
  }
  it('bounds UTF-8 bytes, counts, payloads, and dimensions', () => {
    expect(() => validateImageContract(edited(base,[{path:'/images/0/annotations/comment',value:'é'.repeat(IMAGE_LIMITS.textBytes/2+1)}]))).toThrow();
    expect(() => validateImageContract(edited(base,[{path:'/images/0/annotations/tags',value:Array(IMAGE_LIMITS.tags+1).fill('fox')}]))).toThrow();
    expect(() => parseImageContract(' '.repeat(IMAGE_LIMITS.bundleBytes+1))).toThrow('LIMIT_EXCEEDED');
    expect(() => parseImageContract('{broken')).toThrow('INVALID_ARGUMENT');
    expect(() => parseImageContract(JSON.stringify(base).replace('fixture-image-v1','\\ud800'))).toThrow();
    expect(() => validateImageContract(edited(base,[{path:'/images/0/profile/width',value:4294967296}]))).toThrow();
  });
  it('rejects nonfinite JS values before JSON can silently turn them into null', () => {
    for (const value of [NaN, Infinity, -Infinity]) {
      expect(() => validateImageContract(edited(base,[
        {path:'/images/0/evidence/0/confidence',value},
        {path:'/images/0/evidence/0/confidence_kind',value:'reported'},
      ]))).toThrow();
    }
    expect(() => validateImageContract(edited(base,[{path:'/images/0/annotations/tags',value:Array(1)}]))).toThrow();
  });
  it('accepts integral JSON number spellings and rejects fractions', () => {
    const raw=JSON.stringify(base);
    for (const spelling of ['6.0','6e0']) expect(parseImageContract(raw.replace('"orientation":6','"orientation":'+spelling))).toEqual(base);
    expect(() => parseImageContract(raw.replace('"orientation":6','"orientation":6.5'))).toThrow();
  });
  it('validates exact camelCase invocation names and positive expected revision', () => {
    expect(validateImageEditArgs(ipc)).toEqual(ipc);
    for (const mutation of [
      {path:'/expectedRevision',remove:true},
      {path:'/expectedRevision',value:0},
      {path:'/asset_id',value:ipc.assetId},
    ]) expect(() => validateImageEditArgs(edited(ipc,[mutation]))).toThrow();
  });
  it('keeps the existing audio TypeScript shape and string frame counters', () => {
    // JSON import inference broadens tuples, so check the established Sound
    // boundary with an explicitly typed waveform tuple, without changing types.ts.
    const original=legacy.portable.sounds[0];
    const sound: Sound = {...original, profile:{...original.profile,waveform:original.profile.waveform.map(([min,max]):[number,number]=>[min,max])}};
    expect(sound).toEqual(original);
    const clip: Clip = legacy.clips_get;
    expect(clip).toEqual(legacy.portable.clips[0]);
    expect(legacy.clips_get.recipe.start_frame).toBe('0');
    expect(legacy.clips_get.recipe.end_frame).toBe('24000');
    expect(legacy.sounds_get.sound.profile.waveform).toEqual([]);
    expect(legacy.portable.schema).toBe('soundshelf-catalog/v1');
  });
});
