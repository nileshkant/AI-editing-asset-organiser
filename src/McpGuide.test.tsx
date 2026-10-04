import { render, screen, fireEvent } from '@testing-library/react';
import { expect, it } from 'vitest';
import { McpGuide, ReferenceDocument } from './components/views/McpGuide';
import { PageHeader } from './components/layout/PageHeader';
it('explains actual capabilities and bundles the complete offline tool/access contract', () => {
  render(<McpGuide />);
  expect(screen.getByRole('article', { name: 'MCP guide' })).toBeInTheDocument();
  expect(screen.getByText(/MCP cannot import files/)).toBeInTheDocument();
  expect(screen.getByText(/an audio-capable model that can actually listen/)).toBeInTheDocument();
  expect(screen.getByText(/Upgrading from alpha.5/)).toBeInTheDocument();
  for (const tool of ['service_status', 'library.status', 'sources.list', 'sounds.facets', 'sounds.resolve', 'sounds.annotate', 'clips.create', 'clips.update', 'clips.get', 'clips.list', 'clips.export', 'jobs.get', 'jobs.list', 'jobs.cancel', 'catalog.export']) {
    expect(screen.getAllByText(tool).length).toBeGreaterThan(0);
  }
  fireEvent.change(screen.getByLabelText('Reference document'), { target: { value: 'access' } });
  expect(screen.getByText(/only IPv4 loopback/)).toBeInTheDocument();
  expect(screen.getByText(/Invalid or unreadable pairing storage fails closed/)).toBeInTheDocument();
});
it('does not label a packaged app as a development build', () => {
  render(<PageHeader page="library" onAddFolder={() => {}} />);
  expect(screen.queryByText('Development build')).toBeNull();
  expect(screen.getByRole('heading', { name: 'Library' })).toBeInTheDocument();
});

it('renders the complete contract with Windows checkout line endings', () => {
  render(<ReferenceDocument text={'# Tools\r\n\r\n## Contract\r\n\r\n- `service_status`: status.\r\n- `sounds.search`: search.\r\n\r\nDetails.'} />);
  expect(screen.getByRole('heading', { name: 'Tools' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'Contract' })).toBeInTheDocument();
  expect(screen.getByText('service_status')).toBeInTheDocument();
  expect(screen.getByText('sounds.search')).toBeInTheDocument();
  expect(screen.getAllByRole('listitem')).toHaveLength(2);
});
