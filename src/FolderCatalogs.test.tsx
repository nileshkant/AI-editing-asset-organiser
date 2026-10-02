import {render,screen,fireEvent,waitFor,cleanup} from '@testing-library/react';
import {describe,it,expect,vi,beforeEach,afterEach} from 'vitest';
import {FolderCatalogs} from './components/views/FolderCatalogs';
const invoke=vi.fn();
vi.mock('@tauri-apps/api/core',()=>({isTauri:()=>true,invoke:(...args:unknown[])=>invoke(...args)}));
const roots=[{id:'root',name:'Foley',root:'/foley',generation:0,available:true,scope:'folder' as const}];
beforeEach(()=>invoke.mockImplementation((command:string)=>Promise.resolve(command==='folder_catalog_states'?[{source_id:'root',dirty:true,error:'Read-only source'}]:null)));
afterEach(()=>{cleanup();invoke.mockReset();});
describe('folder metadata recovery',()=>{
 it('shows persisted errors and retries the selected source',async()=>{render(<FolderCatalogs roots={roots} onError={vi.fn()}/>);await screen.findByText('Read-only source');fireEvent.click(screen.getByText('Save metadata'));await waitFor(()=>expect(invoke).toHaveBeenCalledWith('retry_folder_catalog',{id:'root'}));});
 it('backs up catalog only after explicit rebuild confirmation',async()=>{render(<FolderCatalogs roots={roots} onError={vi.fn()}/>);fireEvent.click(screen.getByText('Rebuild catalog'));expect(invoke.mock.calls.some(c=>c[0]==='rebuild_folder_catalog')).toBe(false);fireEvent.click(screen.getByText('Cancel rebuild'));expect(screen.queryByText('Confirm rebuild')).toBeNull();fireEvent.click(screen.getByText('Rebuild catalog'));fireEvent.click(screen.getByText('Confirm rebuild'));await waitFor(()=>expect(invoke).toHaveBeenCalledWith('rebuild_folder_catalog',{id:'root',confirmed:true}));});
});
