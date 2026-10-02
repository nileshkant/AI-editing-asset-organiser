import {beforeEach,expect,it,vi} from 'vitest';
import {fireEvent,render,screen,waitFor} from '@testing-library/react';
import {RecoverySettings} from './components/views/RecoverySettings';
import {call} from './api';
vi.mock('./api',()=>({call:vi.fn()}));
const error=vi.fn();
beforeEach(()=>{vi.clearAllMocks();vi.mocked(call).mockImplementation(async(command)=>{
  if(command==='resource_settings')return {preferences:{output_device:null,imports_paused:false},output_devices:['Speakers'],import_workers:1};
  if(command==='purge_waveform_cache')return 2;
  if(command==='database_restore')return '/private/rollback.sqlite';
  return null;
});});
it('confirms cache purge independently of analysis and can cancel restore',async()=>{
  render(<RecoverySettings onError={error}/>);await screen.findByRole('combobox',{name:'Output device'});
  fireEvent.click(screen.getByRole('button',{name:'Restore database'}));
  expect(call).not.toHaveBeenCalledWith('database_restore',expect.anything());
  fireEvent.click(screen.getByRole('button',{name:'Cancel recovery action'}));
  fireEvent.click(screen.getByRole('button',{name:'Clear waveform cache'}));
  fireEvent.click(screen.getByRole('button',{name:'Confirm cache clear'}));
  await screen.findByText(/Cleared 2 waveform/);expect(call).toHaveBeenCalledWith('purge_waveform_cache',{confirmed:true});
});
it('saves output and bounded import pause preference',async()=>{
  render(<RecoverySettings onError={error}/>);const output=await screen.findByRole('combobox',{name:'Output device'});
  fireEvent.change(output,{target:{value:'Speakers'}});
  await waitFor(()=>expect(call).toHaveBeenCalledWith('save_resource_settings',{preferences:{output_device:'Speakers',imports_paused:false}}));
  await screen.findByText(/Preferences saved/);
  fireEvent.click(screen.getByRole('checkbox',{name:'Pause queued imports'}));
  await waitFor(()=>expect(call).toHaveBeenCalledWith('save_resource_settings',{preferences:{output_device:'Speakers',imports_paused:true}}));
});
it('restore records rollback and restart instructions only after completion',async()=>{
  render(<RecoverySettings onError={error}/>);await screen.findByRole('combobox',{name:'Output device'});
  fireEvent.click(screen.getByRole('button',{name:'Restore database'}));fireEvent.click(screen.getByRole('button',{name:'Confirm and choose backup'}));
  await screen.findByText(/Rollback backup: \/private\/rollback.sqlite/);expect(call).toHaveBeenCalledWith('database_restore',{confirmed:true});
});
