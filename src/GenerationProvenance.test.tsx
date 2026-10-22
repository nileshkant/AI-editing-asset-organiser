import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { GenerationProvenance } from './components/images/GenerationProvenance';
import { CatalogData } from './components/views/CatalogData';
import type { GenerationClaim, GenerationField, GenerationFieldView, GenerationView } from './types';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));

const source = { format: 'a1111_parameters', container: 'png_text', key: 'parameters' } as const;
const claim: GenerationClaim = {
  source,
  fields: { prompt: '<img src=x onerror=alert(1)>\nsecond line', negative_prompt: null, generator: null, model: 'sdxl', seed: '42', sampler: null,
    created: null, settings: { 'CFG scale': '7' }, references: [{ kind: 'image', value: 'C:/private/ref.png', private: true }], digital_source_type: null },
};
const field = (f: GenerationField, over: Partial<GenerationFieldView> = {}): GenerationFieldView =>
  ({ field: f, value: null, origin: 'unknown', source: null, conflicting: false, redacted: false, uncertain: false, ...over });
const base = (over: Partial<GenerationView> = {}): GenerationView => ({
  asset_id: 'img', revision: 0,
  embedded: { parser: 'creativeshelf-generation-v1', status: 'found', claims: [claim], issues: [], uninterpreted: [{ container: 'png_text', key: 'workflow', bytes: 52_000 }], redacted: [] },
  sidecar: null,
  fields: [
    field('prompt', { value: claim.fields.prompt, origin: 'imported', source }),
    field('negative_prompt'), field('generator'),
    field('model', { value: 'sdxl', origin: 'imported', source, conflicting: true }),
    field('seed', { value: '42', origin: 'imported', source }), field('sampler'),
    field('created', { value: '2024:01:02 03:04:05', origin: 'imported', source: { format: 'exif_datetime', container: 'png_exif', key: 'DateTimeOriginal' }, uncertain: true }),
    field('ai_generated'),
  ],
  claims: [claim], confirmed: false, confirmation_outdated: false, inherited_from: null, editable: true,
  settings: { search: 'metadata', snapshot: { prompts: false, private_references: false } }, ...over,
});

beforeEach(() => { vi.clearAllMocks(); });

describe('Generation provenance', () => {
  it('shows every value with its source, as plain text, without opening references', async () => {
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_generation' ? base() : null));
    const { container } = render(<GenerationProvenance id="img" version="hash" />);
    await screen.findByText('Generation metadata found in this file.');
    expect(screen.getByText(/onerror=alert/)).toBeInTheDocument();
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('a')).toBeNull();
    expect(screen.getAllByText(/Claimed by A1111-style parameters · PNG text “parameters”/).length).toBeGreaterThan(0);
    expect(screen.getByText(/another claim records a different value/)).toBeInTheDocument();
    expect(screen.getByText(/time zone unknown/)).toBeInTheDocument();
    expect(screen.getAllByText('Unknown').length).toBeGreaterThan(0);
    expect(screen.getByText(/workflow: 52,000 bytes, kept unread and never run/)).toBeInTheDocument();
    expect(screen.getByText(/Reference \(image, private\)/)).toBeInTheDocument();
    expect(screen.getByText(/not verified history/)).toBeInTheDocument();
  });

  it('distinguishes unread, redacted and empty values', async () => {
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_generation' ? base({
      embedded: null, claims: [],
      fields: [field('prompt', { redacted: true }), field('seed', { value: '', origin: 'user' })],
    }) : null));
    render(<GenerationProvenance id="img" version="hash" />);
    await screen.findByText(/has not been read for generation metadata yet/);
    expect(screen.getByText('Removed when this library was exported')).toBeInTheDocument();
    expect(screen.getByText('(recorded as empty)')).toBeInTheDocument();
    expect(screen.queryByText('Mark imported values reviewed')).toBeNull();
  });

  it('saves only changed fields against the shown revision and clears with an empty box', async () => {
    const view = base({ fields: base().fields.map(f => f.field === 'generator' ? { ...f, value: 'Mine', origin: 'user' } : f) });
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_generation' ? view : cmd === 'edit_image_generation' ? { ...view, revision: 1 } : null));
    const changed = vi.fn();
    render(<GenerationProvenance id="img" version="hash" onChanged={changed} />);
    fireEvent.click(await screen.findByText('Edit provenance'));
    expect(screen.getByLabelText('Generator')).toHaveValue('Mine');
    expect(screen.getByLabelText('Prompt')).toHaveValue('');
    fireEvent.change(screen.getByLabelText('Generator'), { target: { value: '' } });
    fireEvent.change(screen.getByLabelText('Seed'), { target: { value: '7' } });
    fireEvent.change(screen.getByLabelText('AI-generated'), { target: { value: 'yes' } });
    fireEvent.click(screen.getByText('Save provenance'));
    await screen.findByText('Saved provenance.');
    expect(invoke).toHaveBeenCalledWith('edit_image_generation', { id: 'img', expectedRevision: 0, patch: { set: { generator: null, seed: '7', ai_generated: 'yes' } } });
    expect(changed).toHaveBeenCalled();
  });

  it('reloads the latest provenance after a conflicting change', async () => {
    let loads = 0;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'image_generation') { loads += 1; return Promise.resolve(base({ revision: loads })); }
      if (cmd === 'edit_image_generation') return Promise.reject('CONFLICT');
      return Promise.resolve(null);
    });
    render(<GenerationProvenance id="img" version="hash" />);
    fireEvent.click(await screen.findByText('Mark imported values reviewed'));
    await screen.findByText(/Changed elsewhere/);
    await waitFor(() => expect(loads).toBe(2));
    expect(invoke).toHaveBeenCalledWith('edit_image_generation', { id: 'img', expectedRevision: 1, patch: { confirm: true } });
  });

  it('shows review state and stays read only while the original is unavailable', async () => {
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_generation' ? base({ confirmed: true, confirmation_outdated: true, editable: false }) : null));
    render(<GenerationProvenance id="img" version="hash" />);
    expect(await screen.findByText('Withdraw review')).toBeDisabled();
    expect(screen.getByText('Edit provenance')).toBeDisabled();
    expect(screen.getByText(/the file or sidecar has changed since/)).toBeInTheDocument();
    expect(screen.getByText(/Read only while the original is unavailable/)).toBeInTheDocument();
  });
});

describe('Generation metadata in catalog data', () => {
  const settings = { search: 'metadata', snapshot: { prompts: false, private_references: false } };
  it('keeps prompts out of exports unless chosen for this export', async () => {
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_import_capabilities' ? { enabled: true } : cmd === 'image_generation_settings' ? settings : cmd === 'catalog_export' ? '/out.json' : null));
    render(<CatalogData onError={vi.fn()} />);
    await screen.findByText('Image generation metadata');
    fireEvent.click(screen.getByText('Export catalog JSON'));
    await screen.findByText('Catalog exported to /out.json');
    expect(invoke).toHaveBeenCalledWith('catalog_export', { format: 'json', includePrompts: false, includePrivateReferences: false });
    fireEvent.click(screen.getByLabelText('Include prompts in this JSON export'));
    fireEvent.click(screen.getByText('Export catalog JSON'));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('catalog_export', { format: 'json', includePrompts: true, includePrivateReferences: false }));
    fireEvent.click(screen.getByText('Export catalog Markdown'));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('catalog_export', { format: 'md' }));
  });

  it('saves search and folder snapshot settings', async () => {
    invoke.mockImplementation((cmd: string, args?: { settings: unknown }) => Promise.resolve(cmd === 'image_import_capabilities' ? { enabled: true } : cmd === 'image_generation_settings' ? settings : cmd === 'set_image_generation_settings' ? args?.settings : null));
    render(<CatalogData onError={vi.fn()} />);
    fireEvent.change(await screen.findByLabelText('Search generation metadata'), { target: { value: 'prompts' } });
    await screen.findByText('Generation metadata settings saved.');
    expect(invoke).toHaveBeenCalledWith('set_image_generation_settings', { settings: { ...settings, search: 'prompts' } });
    fireEvent.click(screen.getByLabelText('Include prompts in folder catalog snapshots'));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('set_image_generation_settings', { settings: { search: 'prompts', snapshot: { prompts: true, private_references: false } } }));
  });

  it('is hidden when image support is off', async () => {
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_import_capabilities' ? { enabled: false } : null));
    render(<CatalogData onError={vi.fn()} />);
    await waitFor(() => expect(invoke.mock.calls.some(c => c[0] === 'image_import_capabilities')).toBe(true));
    expect(screen.queryByText('Image generation metadata')).toBeNull();
  });
});
