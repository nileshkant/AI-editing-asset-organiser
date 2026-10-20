import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { AiProviderSettings } from './components/views/AiProviderSettings';
import { call } from './api';
import type { ProviderView } from './types';
vi.mock('./api', () => ({ call: vi.fn() }));
const error = vi.fn();
const KEY = 'sk-renderer-test-key-0001';
const base: ProviderView = {
  id: 'p1', kind: 'openai', label: 'Studio OpenAI', base_url: 'https://api.openai.com/v1', model: 'vision-model', allow_local: false,
  key_storage: 'none', key_origin: null, revision: 1, locality: 'remote', has_key: false,
  capabilities: { image_input: 'untested', structured_output: 'untested', embeddings: 'untested', tested_revision: null, tested_at: null, model_version: null },
};
let providers: ProviderView[];
beforeEach(() => {
  vi.clearAllMocks();
  providers = [base];
  vi.mocked(call).mockImplementation(async (command, args) => {
    if (command === 'ai_providers') return providers;
    if (command === 'ai_provider_set_key') { providers = [{ ...base, key_storage: 'keychain', has_key: true, revision: 2 }]; return providers[0]; }
    if (command === 'ai_provider_test') {
      providers = [{ ...providers[0], capabilities: { ...base.capabilities, image_input: 'unsupported', structured_output: 'supported', tested_revision: 2, model_version: 'vision-model-2026' } }];
      return { provider: providers[0], usage: { input_tokens: 12, output_tokens: 3 }, steps: [
        { check: 'model_access', passed: true, error: null },
        { check: 'image_input', passed: false, error: { code: 'IMAGE_UNSUPPORTED', message: 'This model does not accept image input' } },
        { check: 'structured_output', passed: true, error: null }] };
    }
    if (command === 'ai_provider_create') return { ...base, id: 'p2', label: (args as { draft: { label: string } }).draft.label };
    if (command === 'ai_provider_delete') { providers = []; return null; }
    return null;
  });
});
it('stores a key write-only, then tests explicitly and shows typed capability results', async () => {
  render(<AiProviderSettings onError={error} />);
  const card = await screen.findByRole('article', { name: 'Provider Studio OpenAI' });
  expect(within(card).getByText(/No key/)).toBeVisible();
  expect(within(card).getByRole('button', { name: 'Test connection to Studio OpenAI' })).toBeDisabled();
  expect(call).toHaveBeenCalledTimes(1);
  const form = within(card).getByRole('form', { name: 'API key for Studio OpenAI' });
  const input = within(form).getByLabelText('API key');
  expect(input).toHaveAttribute('type', 'password');
  fireEvent.change(input, { target: { value: KEY } });
  fireEvent.click(within(form).getByRole('button', { name: 'Save key for Studio OpenAI' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('ai_provider_set_key', { id: 'p1', revision: 1, key: KEY, store: 'keychain' }));
  await screen.findByText(/Key in system keychain/);
  expect(screen.getByLabelText('API key')).toHaveValue('');
  expect(document.body.textContent).not.toContain(KEY);
  fireEvent.click(screen.getByRole('button', { name: 'Test connection to Studio OpenAI' }));
  const results = await screen.findByRole('list', { name: 'Connection test results for Studio OpenAI' });
  expect(results.textContent).toContain('Image input: failed · This model does not accept image input');
  expect(results.textContent).toContain('12 input and 3 output tokens');
  const caps = screen.getByLabelText('Tested capabilities of Studio OpenAI');
  expect(caps.textContent).toContain('Image inputNot supported');
  expect(caps.textContent).toContain('Structured outputSupported');
  expect(call).toHaveBeenCalledWith('ai_provider_test', { id: 'p1', revision: 2, embeddings: false });
});
it('adds a provider with defaults and confirms removal', async () => {
  render(<AiProviderSettings onError={error} />);
  await screen.findByRole('article', { name: 'Provider Studio OpenAI' });
  fireEvent.click(screen.getByRole('button', { name: 'Add AI provider' }));
  const form = screen.getByRole('form', { name: 'Add provider' });
  fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'Second' } });
  fireEvent.change(within(form).getByLabelText('Model'), { target: { value: 'claude-test' } });
  fireEvent.change(within(form).getByLabelText('Provider'), { target: { value: 'anthropic' } });
  fireEvent.click(within(form).getByRole('button', { name: 'Add provider' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('ai_provider_create', { draft: { kind: 'anthropic', label: 'Second', base_url: null, model: 'claude-test', allow_local: false } }));
  fireEvent.click(screen.getByRole('button', { name: 'Remove Studio OpenAI' }));
  expect(call).not.toHaveBeenCalledWith('ai_provider_delete', expect.anything());
  fireEvent.click(screen.getByRole('button', { name: 'Confirm removing Studio OpenAI' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('ai_provider_delete', { id: 'p1', revision: 1, confirmed: true }));
  await screen.findByText('No AI providers configured.');
});
it('requires an endpoint for compatible servers and reports refusals', async () => {
  vi.mocked(call).mockImplementation(async command => {
    if (command === 'ai_providers') return [];
    if (command === 'ai_provider_create') throw 'This is a local or private network endpoint; turn on local endpoints for this provider to use it';
    return null;
  });
  render(<AiProviderSettings onError={error} />);
  fireEvent.click(await screen.findByRole('button', { name: 'Add AI provider' }));
  const form = screen.getByRole('form', { name: 'Add provider' });
  fireEvent.change(within(form).getByLabelText('Provider'), { target: { value: 'openai_compatible' } });
  fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'Local' } });
  fireEvent.change(within(form).getByLabelText('Model'), { target: { value: 'llava' } });
  expect(within(form).getByRole('button', { name: 'Add provider' })).toBeDisabled();
  fireEvent.change(within(form).getByLabelText('Endpoint URL'), { target: { value: 'http://127.0.0.1:11434/v1' } });
  fireEvent.click(within(form).getByRole('button', { name: 'Add provider' }));
  await waitFor(() => expect(error).toHaveBeenCalledWith(expect.stringContaining('turn on local endpoints')));
  expect(screen.getByRole('form', { name: 'Add provider' })).toBeVisible();
});
