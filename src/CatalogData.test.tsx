import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CatalogData } from './components/views/CatalogData';
const invoke=vi.fn();
vi.mock('@tauri-apps/api/core',()=>({isTauri:()=>true,invoke:(...args:unknown[])=>invoke(...args)}));
const preview={token:'preview-token',preview:{schema:'soundshelf-catalog/v1',sources:[{id:'source-a',name:'Foley'}],sounds:12,legacy:false}};
beforeEach(()=>{vi.clearAllMocks();invoke.mockImplementation((cmd:string)=>{
  if(cmd==='catalog_preview') return Promise.resolve(preview);
  if(cmd==='catalog_map_root') return Promise.resolve('/relocated/audio');
  if(cmd==='catalog_import') return Promise.resolve({report:{sounds:12,sources:1,offline_sources:0},warnings:[]});
  if(cmd==='catalog_legacy_evidence') return Promise.resolve([]);
  return Promise.resolve(null);
});});
describe('Catalog data workflow',()=>{
  it('previews without importing and maps roots through the native picker before confirmation',async()=>{
    render(<CatalogData onError={vi.fn()}/>);
    fireEvent.click(screen.getByText('Import catalog'));
    await screen.findByLabelText('Catalog import preview');
    expect(invoke).toHaveBeenCalledWith('catalog_preview',{legacy:false});
    expect(invoke.mock.calls.some(c=>c[0]==='catalog_import')).toBe(false);
    fireEvent.click(screen.getByText('Choose folder for Foley'));
    await screen.findByText('/relocated/audio');
    expect(invoke).toHaveBeenCalledWith('catalog_map_root',{token:'preview-token',sourceId:'source-a'});
    fireEvent.click(screen.getByText('Confirm catalog import'));
    await screen.findByText(/Imported 12 sounds/);
    expect(invoke).toHaveBeenCalledWith('catalog_import',{token:'preview-token',confirmed:true});
  });
  it('requires every legacy root mapping and labels inference and measurement coverage',async()=>{
    invoke.mockImplementation((cmd:string)=>Promise.resolve(cmd==='catalog_preview'?{...preview,preview:{...preview.preview,legacy:true}}:cmd==='catalog_map_root'?'/legacy':null));
    render(<CatalogData onError={vi.fn()}/>);
    fireEvent.click(screen.getByText('Import legacy catalog'));
    await screen.findByLabelText('Catalog import preview');
    expect(invoke).toHaveBeenCalledWith('catalog_preview',{legacy:true});
    expect(screen.getByText('Confirm catalog import')).toBeDisabled();
    expect(screen.getByText(/Old measurements cover only the first 20 seconds/)).toBeInTheDocument();
    fireEvent.click(screen.getByText('Choose folder for Foley'));
    await waitFor(()=>expect(screen.getByText('Confirm catalog import')).toBeEnabled());
  });
  it('cancel revokes the preview without importing',async()=>{
    render(<CatalogData onError={vi.fn()}/>);fireEvent.click(screen.getByText('Import catalog'));
    await screen.findByLabelText('Catalog import preview');fireEvent.click(screen.getByText('Cancel import'));
    await waitFor(()=>expect(invoke).toHaveBeenCalledWith('catalog_cancel_preview',{token:'preview-token'}));
    expect(invoke.mock.calls.some(c=>c[0]==='catalog_import')).toBe(false);
  });
  it('reports import failure and consumes the submitted preview',async()=>{
    const error=vi.fn();const base=invoke.getMockImplementation()!;
    invoke.mockImplementation((cmd:string,...args:unknown[])=>cmd==='catalog_import'?Promise.reject(new Error('Root overlap; no changes made')):base(cmd,...args));
    render(<CatalogData onError={error}/>);fireEvent.click(screen.getByText('Import catalog'));
    await screen.findByLabelText('Catalog import preview');fireEvent.click(screen.getByText('Confirm catalog import'));
    await waitFor(()=>expect(error).toHaveBeenCalledWith('Error: Root overlap; no changes made'));
    expect(screen.queryByLabelText('Catalog import preview')).not.toBeInTheDocument();
  });
  it('export cancellation does not show success; write failures reach the error banner',async()=>{
    const error=vi.fn();render(<CatalogData onError={error}/>);
    fireEvent.click(screen.getByText('Export catalog JSON'));
    await waitFor(()=>expect(screen.getByText('Export catalog JSON')).toBeEnabled());
    expect(screen.queryByText(/Catalog exported/)).not.toBeInTheDocument();
    invoke.mockRejectedValueOnce(new Error('File exists'));
    fireEvent.click(screen.getByText('Export catalog Markdown'));
    await waitFor(()=>expect(error).toHaveBeenCalledWith('Error: File exists'));
  });
  it('disables repeated actions while a native operation is pending',async()=>{
    let finish!:(v:unknown)=>void;invoke.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
    render(<CatalogData onError={vi.fn()}/>);fireEvent.click(screen.getByText('Import catalog'));
    expect(screen.getByText('Import catalog')).toBeDisabled();fireEvent.click(screen.getByText('Import catalog'));
    expect(invoke.mock.calls.filter(c=>c[0]==='catalog_preview')).toHaveLength(1);
    await act(async()=>finish(null));
  });
});
