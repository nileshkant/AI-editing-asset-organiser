import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { ModelPackSettings, formatBytes } from './components/views/ModelPackSettings';
import { call } from './api';
import type { ModelPacksOverview, PackView } from './types';
vi.mock('./api', () => ({ call: vi.fn() }));
const error = vi.fn();
const pack: PackView = {
  id: 'describe', version: '1.0', display_name: 'Describe', capability: 'image_description', phase: 'not_installed', detail: null,
  incompatible: null, memory_unchecked: false, license_spdx: 'Apache-2.0', notice: 'Bundled notice text', notice_url: null,
  download_bytes: 3 * 1024 * 1024, installed_bytes: null, resumable_bytes: 0, min_ram_bytes: 2 * 1024 ** 3, runtime: 'onnx',
  progress: null, in_use: 0, update_available: null, retired: false,
};
let packs: PackView[];
let runtime = true;
const overview = (): ModelPacksOverview => ({
  packs, storage: { installed_bytes: 0, partial_bytes: 0, free_bytes: 50 * 1024 ** 3 }, active: [], generation: 0, pending_reindex: [], runtime_available: runtime,
});
beforeEach(() => {
  vi.clearAllMocks();
  packs = [pack];
  runtime = true;
  vi.mocked(call).mockImplementation(async command => {
    if (command === 'model_packs') return overview();
    return null;
  });
});

it('formats sizes', () => {
  expect([formatBytes(512), formatBytes(1536), formatBytes(5 * 1024 ** 3)]).toEqual(['512 B', '1.5 KiB', '5.0 GiB']);
});

it('says plainly that nothing is available when the trusted list is empty, and the base app is unaffected', async () => {
  packs = [];
  render(<ModelPackSettings onError={error} />);
  expect(await screen.findByText(/No local model packs are available yet/)).toBeVisible();
  expect(screen.getByText(/works fully without any model/)).toBeVisible();
  expect(screen.queryByRole('button', { name: /Download/ })).toBeNull();
  expect(call).toHaveBeenCalledTimes(1);
  expect(call).toHaveBeenCalledWith('model_packs');
});

it('downloads only on an explicit press, then offers activation and confirms before removal', async () => {
  render(<ModelPackSettings onError={error} />);
  const card = await screen.findByRole('article', { name: 'Pack Describe 1.0' });
  expect(within(card).getByText('Not installed')).toBeVisible();
  expect(call).not.toHaveBeenCalledWith('model_pack_install', expect.anything());
  expect(within(card).getByText(/Download 3.0 MiB/)).toBeVisible();
  expect(within(card).getByText('License: Apache-2.0')).toBeVisible();
  vi.mocked(call).mockImplementation(async command => {
    if (command === 'model_pack_install') { packs = [{ ...pack, phase: 'verified', installed_bytes: 3 * 1024 * 1024 }]; return null; }
    if (command === 'model_pack_activate') { packs = [{ ...packs[0], phase: 'active' }]; return null; }
    if (command === 'model_pack_uninstall') { packs = [pack]; return null; }
    return overview();
  });
  fireEvent.click(within(card).getByRole('button', { name: 'Download Describe 1.0' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('model_pack_install', { id: 'describe', version: '1.0' }));
  await screen.findByText('Downloaded and verified, not in use');
  fireEvent.click(screen.getByRole('button', { name: 'Activate Describe 1.0' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('model_pack_activate', { id: 'describe', version: '1.0' }));
  await screen.findByText('Active');
  fireEvent.click(screen.getByRole('button', { name: 'Remove Describe 1.0' }));
  const confirm = await screen.findByRole('alertdialog', { name: 'Confirm removing Describe 1.0' });
  expect(call).not.toHaveBeenCalledWith('model_pack_uninstall', expect.anything());
  expect(confirm.textContent).toContain('Your tags and annotations are kept');
  fireEvent.click(within(confirm).getByRole('button', { name: 'Remove Describe 1.0' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('model_pack_uninstall', { id: 'describe', version: '1.0', confirmed: true }));
  expect(error).not.toHaveBeenCalled();
});

it('shows progress and cancel while downloading, and resume or discard for an interrupted download', async () => {
  packs = [{ ...pack, phase: 'downloading', progress: [1024 * 1024, 3 * 1024 * 1024] }];
  const { unmount } = render(<ModelPackSettings onError={error} />);
  const bar = await screen.findByLabelText('Download progress for Describe 1.0');
  expect(bar).toHaveAttribute('value', String(1024 * 1024));
  fireEvent.click(screen.getByRole('button', { name: 'Cancel download of Describe 1.0' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('model_pack_cancel_install'));
  unmount();
  packs = [{ ...pack, resumable_bytes: 1024 * 1024 }];
  render(<ModelPackSettings onError={error} />);
  expect(await screen.findByRole('button', { name: 'Resume download of Describe 1.0' })).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'Discard partial download of Describe 1.0' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('model_pack_discard_partial', { id: 'describe', version: '1.0' }));
});

it('explains incompatibility and failure, and never offers a download for an incompatible pack', async () => {
  packs = [
    { ...pack, phase: 'incompatible', incompatible: { code: 'low_memory', detail: 'It needs about 2048 MiB of memory and this computer has 1024 MiB' } },
    { ...pack, id: 'broken', display_name: 'Broken', phase: 'failed', detail: 'The downloaded files do not match the trusted list: model.onnx does not match its trusted digest' },
  ];
  render(<ModelPackSettings onError={error} />);
  const low = await screen.findByRole('article', { name: 'Pack Describe 1.0' });
  expect(low.textContent).toContain('Not compatible with this computer');
  expect(low.textContent).toContain('1024 MiB');
  expect(within(low).queryByRole('button', { name: /Download/ })).toBeNull();
  const broken = screen.getByRole('article', { name: 'Pack Broken 1.0' });
  expect(within(broken).getByRole('alert').textContent).toContain('do not match the trusted list');
  expect(within(broken).getByRole('button', { name: 'Retry Broken 1.0' })).toBeVisible();
});

it('states the runtime limit and a pending reindex without claiming analysis is available', async () => {
  runtime = false;
  packs = [{ ...pack, phase: 'active' }];
  vi.mocked(call).mockImplementation(async command => command === 'model_packs'
    ? { ...overview(), pending_reindex: [{ capability: 'image_embedding', from_space: 'a', to_space: 'b', generation: 3 }] } : null);
  render(<ModelPackSettings onError={error} />);
  expect(await screen.findByText(/cannot run local models yet/)).toBeVisible();
  expect(screen.getByText(/not mixed with the new one/)).toBeVisible();
});

it('reports a failed action once and refreshes the list', async () => {
  vi.mocked(call).mockImplementation(async command => {
    if (command === 'model_pack_install') throw 'Not enough free disk space: 100 bytes are needed and 5 are free';
    return overview();
  });
  render(<ModelPackSettings onError={error} />);
  fireEvent.click(await screen.findByRole('button', { name: 'Download Describe 1.0' }));
  await waitFor(() => expect(error).toHaveBeenCalledWith(expect.stringContaining('Not enough free disk space')));
  expect(error).toHaveBeenCalledTimes(1);
});
