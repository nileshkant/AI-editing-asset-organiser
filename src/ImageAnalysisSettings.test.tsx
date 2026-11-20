import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { ImageAnalysisSettings } from './components/views/ImageAnalysisSettings';
import { call } from './api';
import type { EnrichConsent, EnrichJob } from './types';
vi.mock('./api', () => ({ call: vi.fn() }));
const error = vi.fn();
const provider = { id: 'p', kind: 'openai_compatible', label: 'Vision', base_url: 'http://x', model: 'm1', allow_local: false, key_storage: 'keychain', key_origin: null, revision: 3, locality: 'remote', has_key: true,
  capabilities: { image_input: 'supported', structured_output: 'supported', embeddings: 'untested', tested_revision: 3, tested_at: 1, model_version: null } };
const source = { source_id: 's1', name: 'Shoot', root: '/shoot', scope: 'folder' };
const consentRow: EnrichConsent = { source_id: 's1', provider_id: 'p', provider_revision: 3, model: 'm1', analyze_new: false, revision: 1, granted_at: 1 };
const counts = { pending: 0, running: 0, done: 0, reused: 0, failed: 0, unknown_outcome: 0, stale: 0, revoked: 0 };
const usage = { requests: 0, used_units: 0, reserved_units: 0, unknown_units: 0, estimated_entries: 0 };
const job = (over: Partial<EnrichJob> = {}): EnrichJob => ({ id: 'j1', source_id: 's1', kind: 'backfill', provider_id: 'p', model: 'm1', state: 'running', error_code: null, max_requests: 10, max_units: 30000, counts, usage, created_at: 1, ...over });
let enabled = true, consents: EnrichConsent[], jobs: EnrichJob[], providers: unknown[];
beforeEach(() => {
  vi.clearAllMocks();
  enabled = true; consents = []; jobs = []; providers = [provider];
  vi.mocked(call).mockImplementation(async (command: string) => {
    switch (command) {
      case 'image_import_capabilities': return { enabled };
      case 'image_enrichment_disclosure': return { sends: 'A re-encoded copy of each image.', never_sends: ['file names or folder paths'], purpose: 'Describe images' };
      case 'image_source_statuses': return [source];
      case 'ai_providers': return providers;
      case 'image_enrichment_consents': return consents;
      case 'image_enrichment_jobs': return jobs;
      default: return null;
    }
  });
});

it('renders nothing and calls nothing else when images are not enabled in this build', async () => {
  enabled = false;
  const { container } = render(<ImageAnalysisSettings onError={error} />);
  await waitFor(() => expect(call).toHaveBeenCalledWith('image_import_capabilities'));
  expect(container).toBeEmptyDOMElement();
  expect(call).toHaveBeenCalledTimes(1);
});

it('states what is sent and sends nothing until the user agrees explicitly', async () => {
  render(<ImageAnalysisSettings onError={error} />);
  expect(await screen.findByText(/A re-encoded copy of each image/)).toBeVisible();
  expect(screen.getByText('file names or folder paths')).toBeVisible();
  const allow = screen.getByRole('button', { name: 'Allow analysis' });
  expect(allow).toBeDisabled();
  fireEvent.click(screen.getByLabelText(/I understand copies of these images will be sent to Vision over the internet/));
  expect(allow).toBeEnabled();
  expect(call).not.toHaveBeenCalledWith('image_enrichment_grant', expect.anything());
  fireEvent.click(allow);
  await waitFor(() => expect(call).toHaveBeenCalledWith('image_enrichment_grant', { request: { source_id: 's1', provider_id: 'p', provider_revision: 3, analyze_new: false } }));
});

it('only offers providers that passed their test at the current revision', async () => {
  providers = [{ ...provider, capabilities: { ...provider.capabilities, tested_revision: 2 } }];
  render(<ImageAnalysisSettings onError={error} />);
  expect(await screen.findByText(/No provider has passed/)).toBeVisible();
  expect(screen.getByRole('option', { name: /Vision · m1 \(test it first\)/ })).toBeDisabled();
});

it('estimates first, then starts with the shown ceilings, and never starts without a valid ceiling', async () => {
  consents = [consentRow];
  vi.mocked(call).mockImplementation(async (command: string) => {
    if (command === 'image_enrichment_estimate') return { to_send: 4, reusable: 1, already_done: 2, already_queued: 0, not_ready: 0, estimated_units: 12096, units_per_request: 3024, unit: 'tokens', cost: 'Cost depends on your provider’s prices.', suggested_max_requests: 4, suggested_max_units: 12096 };
    if (command === 'image_import_capabilities') return { enabled };
    if (command === 'image_enrichment_disclosure') return { sends: 's', never_sends: [], purpose: 'p' };
    if (command === 'image_source_statuses') return [source];
    if (command === 'ai_providers') return providers;
    if (command === 'image_enrichment_consents') return consents;
    if (command === 'image_enrichment_jobs') return jobs;
    return null;
  });
  render(<ImageAnalysisSettings onError={error} />);
  fireEvent.click(await screen.findByRole('button', { name: 'Estimate analysis of Shoot' }));
  expect(await screen.findByText(/4 images would be sent/)).toBeVisible();
  expect(call).not.toHaveBeenCalledWith('image_enrichment_start', expect.anything());
  fireEvent.change(screen.getByLabelText('Most requests to send'), { target: { value: '0' } });
  expect(screen.getByRole('button', { name: 'Start analysis' })).toBeDisabled();
  fireEvent.change(screen.getByLabelText('Most requests to send'), { target: { value: '3' } });
  fireEvent.click(screen.getByRole('button', { name: 'Start analysis' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('image_enrichment_start', { request: { source_id: 's1', provider_id: 'p', provider_revision: 3, kind: 'backfill', asset_ids: null, max_requests: 3, max_units: 12096 } }));
});

it('asks for a new agreement when the provider changed after consent', async () => {
  consents = [consentRow];
  providers = [{ ...provider, revision: 4 }];
  render(<ImageAnalysisSettings onError={error} />);
  expect(await screen.findByText(/Nothing is sent until you allow it again/)).toBeVisible();
  expect(screen.queryByRole('button', { name: /Estimate analysis/ })).toBeNull();
});

it('revokes, cancels and resumes batches with plain reasons', async () => {
  consents = [consentRow];
  jobs = [job({ state: 'paused', error_code: 'REQUEST_CEILING', counts: { ...counts, done: 2, pending: 3 }, usage: { ...usage, requests: 2, used_units: 6048 } })];
  render(<ImageAnalysisSettings onError={error} />);
  const card = await screen.findByRole('article', { name: 'Analysis batch for Shoot' });
  expect(within(card).getByText(/The request limit for this batch was reached/)).toBeVisible();
  expect(within(card).getByText(/Requests sent: 2 of at most 10/)).toBeVisible();
  fireEvent.click(within(card).getByRole('button', { name: 'Resume batch for Shoot' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('image_enrichment_resume', { jobId: 'j1' }));
  fireEvent.click(screen.getByRole('button', { name: 'Stop allowing analysis for Shoot' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('image_enrichment_revoke', { sourceId: 's1' }));
});

it('makes the user accept a possible second charge before retrying an unknown outcome', async () => {
  consents = [consentRow];
  jobs = [job({ state: 'complete', counts: { ...counts, unknown_outcome: 1 }, usage: { ...usage, requests: 1, unknown_units: 3024 } })];
  vi.mocked(call).mockImplementation(async (command: string) => {
    if (command === 'image_enrichment_items') return [{ asset_id: 'a1', state: 'unknown_outcome', error_code: null, attempts: 1, operation_id: 'o' }];
    if (command === 'image_import_capabilities') return { enabled };
    if (command === 'image_enrichment_disclosure') return { sends: 's', never_sends: [], purpose: 'p' };
    if (command === 'image_source_statuses') return [source];
    if (command === 'ai_providers') return providers;
    if (command === 'image_enrichment_consents') return consents;
    if (command === 'image_enrichment_jobs') return jobs;
    return null;
  });
  render(<ImageAnalysisSettings onError={error} />);
  expect(await screen.findByRole('alert')).toHaveTextContent(/may or may not have reached the provider/);
  fireEvent.click(screen.getByRole('button', { name: /Review 1 image/ }));
  const retry = await screen.findByRole('button', { name: 'Retry a1' });
  expect(retry).toBeDisabled();
  fireEvent.click(screen.getByLabelText('I accept a possible second charge'));
  fireEvent.click(retry);
  await waitFor(() => expect(call).toHaveBeenCalledWith('image_enrichment_retry', { jobId: 'j1', assetId: 'a1', acceptPossibleCharge: true }));
});
