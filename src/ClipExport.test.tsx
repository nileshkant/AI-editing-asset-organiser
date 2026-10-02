import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { ClipExport } from './components/sound/ClipExport';
import type { Clip } from './types';
const api = vi.hoisted(() => ({ chooseExportDestination: vi.fn(), exportClip: vi.fn(), cancelExport: vi.fn() }));
vi.mock('./api', () => api);
const clip: Clip = {
  id: 'clip-1', sound_id: 'sound-1', name: 'Transient', revision: 3, is_stale: false, stale_reason: null, created_at: 1, updated_at: 1,
  recipe: { asset_id: 'sound-1', asset_version_id: 'hash', source_sample_rate_hz: 48000, start_frame: '5926', end_frame: '29926', channel_policy: 'preserve', gain_db: 0, fade_in_ms: 0, fade_out_ms: 0 },
};
const result = { path: '/export with spaces/audio.wav', manifest_path: '/export with spaces/audio.wav.soundshelf.json', frames: '24000', sound_id: 'export-1', warning: null };
function open() { render(<ClipExport clip={clip} />); fireEvent.click(screen.getByRole('button', { name: 'Export clip Transient' })); }
function start() { fireEvent.click(screen.getByRole('button', { name: 'Choose destination and export' })); }
beforeEach(() => {
  vi.resetAllMocks(); api.chooseExportDestination.mockResolvedValue({ id: 'grant', path: result.path }); api.exportClip.mockResolvedValue(result); api.cancelExport.mockResolvedValue(undefined);
});
describe('clip export', () => {
  it('defaults to WAV/source rate and sends revision with native destination grant', async () => {
    open(); expect(screen.getByLabelText('Export format')).toHaveValue('wav'); start();
    await screen.findByText('Exported 24000 frames.');
    expect(api.exportClip).toHaveBeenCalledWith('grant', clip.id, 3, { format: 'wav', sample_rate: null, fade_in_ms: 0, fade_out_ms: 0 });
    expect(screen.getByText(/Import this audio file in your editor/)).toBeInTheDocument();
    expect(screen.getByText(result.path)).toBeInTheDocument();
  });
  it('supports FLAC, resampling and explicit fades', async () => {
    open(); fireEvent.change(screen.getByLabelText('Export format'), { target: { value: 'flac' } });
    fireEvent.change(screen.getByLabelText('Export sample rate'), { target: { value: '44100' } });
    fireEvent.change(screen.getByLabelText('Export fade in milliseconds'), { target: { value: '10' } });
    fireEvent.change(screen.getByLabelText('Export fade out milliseconds'), { target: { value: '20' } }); start();
    await screen.findByText('Exported 24000 frames.');
    expect(api.chooseExportDestination).toHaveBeenCalledWith('flac');
    expect(api.exportClip).toHaveBeenCalledWith('grant', clip.id, 3, { format: 'flac', sample_rate: 44100, fade_in_ms: 10, fade_out_ms: 20 });
  });
  it('native dialog cancellation starts no render and restores controls', async () => {
    api.chooseExportDestination.mockResolvedValue(null); open(); start();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Choose destination and export' })).toBeEnabled());
    expect(api.exportClip).not.toHaveBeenCalled(); expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
  it('prevents duplicate submissions and cancels the active grant', async () => {
    let reject!: (e: Error) => void;
    api.exportClip.mockImplementation(() => new Promise((_resolve, rej) => { reject = rej; }));
    open(); start(); start();
    await screen.findByText('Rendering and verifying export…');
    expect(api.exportClip).toHaveBeenCalledTimes(1); expect(screen.getByLabelText('Export format')).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel export' }));
    await waitFor(() => expect(api.cancelExport).toHaveBeenCalledWith('grant'));
    reject(new Error('Export cancelled'));
    expect(await screen.findByRole('alert')).toHaveTextContent('Export cancelled');
    expect(screen.getByRole('button', { name: 'Choose destination and export' })).toBeEnabled();
  });
  it('shows collision/disk errors and permits retry', async () => {
    api.exportClip.mockRejectedValueOnce(new Error('Destination already exists'));
    open(); start(); expect(await screen.findByRole('alert')).toHaveTextContent('Destination already exists');
    start(); await screen.findByText('Exported 24000 frames.'); expect(api.exportClip).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
  it('refuses stale clips and invalid fades', async () => {
    const view = render(<ClipExport clip={{ ...clip, is_stale: true }} />);
    expect(screen.getByRole('button', { name: 'Export clip Transient' })).toBeDisabled(); view.unmount();
    open(); fireEvent.change(screen.getByLabelText('Export fade in milliseconds'), { target: { value: '0.5' } }); start();
    expect(await screen.findByRole('alert')).toHaveTextContent('whole milliseconds'); expect(api.chooseExportDestination).not.toHaveBeenCalled();
  });
  it('retains successful handoff when catalog indexing fails', async () => {
    api.exportClip.mockResolvedValue({ ...result, sound_id: null, warning: 'Index failed; import folder to retry.' });
    open(); start(); await screen.findByText('Exported 24000 frames.'); expect(screen.getByRole('alert')).toHaveTextContent('Index failed');
    expect(screen.getByRole('button', { name: 'Copy exported path' })).toBeEnabled();
  });
  it('copies a usable local path and displays clipboard failures', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    open(); start(); await screen.findByText('Exported 24000 frames.');
    fireEvent.click(screen.getByRole('button', { name: 'Copy exported path' })); await screen.findByRole('button', { name: 'Path copied' });
    expect(writeText).toHaveBeenCalledWith(result.path);
    writeText.mockRejectedValue(new Error('Denied')); fireEvent.click(screen.getByRole('button', { name: 'Path copied' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not copy path');
  });
});

it('reports cancellation IPC errors while keeping the render controls visible', async () => {
  api.exportClip.mockImplementation(() => new Promise(() => {})); api.cancelExport.mockRejectedValue(new Error('Cancel unavailable'));
  open(); start(); await screen.findByText('Rendering and verifying export…');
  fireEvent.click(screen.getByRole('button', { name: 'Cancel export' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Cancel unavailable');
  expect(screen.getByRole('button', { name: 'Cancel export' })).toBeEnabled();
});

it('reports an unavailable clipboard instead of throwing from the click handler', async () => {
  Object.defineProperty(navigator, 'clipboard', { value: undefined, configurable: true });
  open(); start(); await screen.findByText('Exported 24000 frames.');
  fireEvent.click(screen.getByRole('button', { name: 'Copy exported path' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Could not copy path');
});

it('refreshes library search after an export is indexed', async () => {
  const onExported = vi.fn();
  render(<ClipExport clip={clip} onExported={onExported} />);
  fireEvent.click(screen.getByRole('button', { name: 'Export clip Transient' })); start();
  await screen.findByText('Exported 24000 frames.'); expect(onExported).toHaveBeenCalledTimes(1);
});
