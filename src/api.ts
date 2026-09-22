import { invoke, isTauri } from '@tauri-apps/api/core';
export type Source = { id:string;name:string;root:string;generation:number;available:boolean };
export type Profile = {duration:number;sample_rate:number;channels:number;channel_layout?:string;channel_peaks?:number[];channel_rms?:number[];frames:number;peak:number;rms:number;description:string;tags:string[];waveform:[number,number][]};
export type Sound = {id:string;source_id:string;relative_path:string;title:string;content_hash:string;status:string;profile:Profile|null;user_tags:string[];comment:string;favorite:boolean};
export type Progress = {job_id:string;source_id:string;status:string;completed:number;total:number;reused:number;failed:number;current:string;errors:string[]};
export type SearchResults = {items:Sound[];total:number;interpretation:{terms:string[];excluded:string[];min_duration:number|null;max_duration:number|null;corrected:string[]}};
export type AppInfo = {version:string;data_directory:string;desktop:boolean;media_tools:boolean};
export function call<T>(command:string,args?:Record<string,unknown>):Promise<T> {
  if (!isTauri()) return Promise.reject(new Error('Open SoundShelf as a desktop application.'));
  return invoke<T>(command,args);
}
export function duration(seconds:number):string {const minutes=Math.floor(seconds/60);return `${minutes}:${(seconds%60).toFixed(2).padStart(5,'0')}`;}
