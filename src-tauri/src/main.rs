#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod service;
use service::AppState;
use soundshelf_core::{catalog::{Source,Sound},library::Progress,search::{SearchQuery,SearchResults,search}};
use std::path::PathBuf;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

#[derive(serde::Serialize)]
struct AppInfo { version: &'static str, data_directory: String, desktop: bool, media_tools: bool }

#[tauri::command]
fn app_info(state:tauri::State<AppState>) -> AppInfo {AppInfo{version:env!("CARGO_PKG_VERSION"),data_directory:state.data_directory.to_string_lossy().into_owned(),desktop:true,media_tools:state.tools.is_some()}}
#[tauri::command]
async fn choose_folder(app:tauri::AppHandle)->Result<Option<String>,String>{tauri::async_runtime::spawn_blocking(move ||app.dialog().file().blocking_pick_folder().map(|p|p.into_path().map(|p|p.to_string_lossy().into_owned()).map_err(|e|e.to_string())).transpose()).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn sources(state:tauri::State<'_,AppState>)->Result<Vec<Source>,String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.sources().map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn import_root(state:tauri::State<'_,AppState>,path:String)->Result<Source,String>{let c=state.catalog.clone();let source=tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.add_source(&PathBuf::from(path)).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())??;state.enqueue(source.clone())?;Ok(source)}
#[tauri::command]
fn scan_source(state:tauri::State<AppState>,id:String)->Result<(),String>{let source=state.catalog.lock().map_err(|e|e.to_string())?.source(&id).map_err(|e|e.to_string())?;state.enqueue(source)}
#[tauri::command]
async fn relink_source(state:tauri::State<'_,AppState>,id:String,path:String)->Result<Source,String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.relink(&id,&PathBuf::from(path)).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn search_sounds(state:tauri::State<'_,AppState>,query:SearchQuery)->Result<SearchResults,String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||{let c=c.lock().map_err(|e|e.to_string())?;let online=c.sources().map_err(|e|e.to_string())?.into_iter().filter(|s|s.available).map(|s|s.id).collect::<Vec<_>>();search(c.all_sounds().map_err(|e|e.to_string())?,&query,&online).map_err(|e|e.to_string())}).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn get_sound(state:tauri::State<'_,AppState>,id:String)->Result<Sound,String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.sound(&id).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn annotate(state:tauri::State<'_,AppState>,id:String,tags:Vec<String>,comment:String,favorite:bool)->Result<(),String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.annotate(&id,&tags,&comment,favorite).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
fn jobs(state:tauri::State<AppState>)->Result<Vec<Progress>,String>{state.progress.lock().map(|s|s.clone()).map_err(|e|e.to_string())}
#[tauri::command]
fn cancel_import(state:tauri::State<AppState>){state.cancel.store(true,std::sync::atomic::Ordering::Relaxed);}

fn main() {
    tauri::Builder::default().plugin(tauri_plugin_dialog::init())
        .setup(|app|{let path=if cfg!(debug_assertions){std::env::var_os("SOUNDSHELF_DATA_DIR").map(PathBuf::from).unwrap_or(app.path().app_data_dir()?)}else{app.path().app_data_dir()?};let resources=app.path().resource_dir()?;app.manage(AppState::new(path,resources)?);Ok(())})
        .invoke_handler(tauri::generate_handler![app_info,choose_folder,sources,import_root,scan_source,relink_source,search_sounds,get_sound,annotate,jobs,cancel_import])
        .build(tauri::generate_context!()).expect("SoundShelf could not start")
        .run(|app,event|if matches!(event,tauri::RunEvent::Exit){app.state::<AppState>().shutdown();});
}
