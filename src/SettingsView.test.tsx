import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { SettingsView } from './components/views/SettingsView';
import { call } from './api';
vi.mock('./api', () => ({ call: vi.fn() }));
const error = vi.fn();
const root = { id: 'rain', name: 'Rain recordings', root: '/fixtures/rain', generation: 1, available: true };
const props = { roots: [root], onRescan: vi.fn(), onRelink: vi.fn(), onError: error };
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(call).mockImplementation(async command => {
    if (command === 'folder_catalog_states') return [];
    if (command === 'mcp_status') return { endpoint: 'http://127.0.0.1:4321/mcp', clients: [] };
    if (command === 'resource_settings') return { preferences: { output_device: null, imports_paused: false }, output_devices: [], import_workers: 1 };
    if (command === 'catalog_preview') return { token: 'preview-token', preview: { sounds: 4, sources: [], legacy: false } };
    return null;
  });
});
it('exposes one labelled pane and supports arrow, Home and End keyboard navigation', () => {
  render(<SettingsView {...props} />);
  const sourceTab = screen.getByRole('tab', { name: 'Sources' });
  sourceTab.focus();
  expect(screen.getAllByRole('tabpanel')).toHaveLength(1);
  expect(screen.getByRole('tabpanel', { name: 'Sources' })).toBeVisible();
  fireEvent.keyDown(sourceTab, { key: 'ArrowRight' });
  expect(screen.getByRole('tab', { name: 'Catalog data' })).toHaveFocus();
  expect(screen.getByRole('tabpanel', { name: 'Catalog data' })).toBeVisible();
  expect(sourceTab).toHaveAttribute('tabindex', '-1');
  fireEvent.keyDown(document.activeElement!, { key: 'End' });
  expect(screen.getByRole('tab', { name: 'MCP guide' })).toHaveFocus();
  fireEvent.keyDown(document.activeElement!, { key: 'ArrowRight' });
  expect(sourceTab).toHaveFocus();
  fireEvent.keyDown(sourceTab, { key: 'ArrowLeft' });
  expect(screen.getByRole('tab', { name: 'MCP guide' })).toHaveFocus();
  fireEvent.keyDown(document.activeElement!, { key: 'Home' });
  expect(sourceTab).toHaveFocus();
});
it('preserves client form and import preview while switching categories', async () => {
  const view = render(<SettingsView {...props} />);
  fireEvent.click(screen.getByRole('tab', { name: 'Agent access' }));
  const name = await screen.findByRole('textbox', { name: 'Client name' });
  fireEvent.change(name, { target: { value: 'My editor' } });
  fireEvent.click(screen.getByRole('tab', { name: 'Catalog data' }));
  fireEvent.click(screen.getByRole('button', { name: 'Import catalog' }));
  await screen.findByRole('region', { name: 'Catalog import preview' });
  fireEvent.click(screen.getByRole('tab', { name: 'Agent access' }));
  expect(screen.getByRole('textbox', { name: 'Client name' })).toHaveValue('My editor');
  expect(call).not.toHaveBeenCalledWith('catalog_cancel_preview', expect.anything());
  fireEvent.click(screen.getByRole('tab', { name: 'Catalog data' }));
  expect(screen.getByRole('region', { name: 'Catalog import preview' })).toBeVisible();
  view.unmount();
  await waitFor(() => expect(call).toHaveBeenCalledWith('catalog_cancel_preview', { token: 'preview-token' }));
});
it('keeps destructive source confirmation and returns focus on Escape', () => {
  render(<SettingsView {...props} />);
  const remove = screen.getByRole('button', { name: 'Remove source' });
  remove.focus(); fireEvent.click(remove);
  expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  expect(remove).toHaveFocus();
  expect(call).not.toHaveBeenCalledWith('remove_source', expect.anything());
});
it('shows the reported version without falsely labelling release builds Development', () => {
  render(<SettingsView {...props} info={{ version: '0.1.0-alpha.5', desktop: true, media_tools: true, data_directory: '/fixtures/data' }} />);
  fireEvent.click(screen.getByRole('tab', { name: 'Application' }));
  expect(screen.getByText('0.1.0-alpha.5')).toBeVisible();
  expect(screen.queryByText(/Development/)).not.toBeInTheDocument();
});
