#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod service;
use service::AppState;
use soundshelf_core::{
    catalog::{SavedSearch, Source, Sound},
    library::Progress,
    playback::PlaybackStatus,
    search::{SearchQuery, SearchResults},
};
use std::path::PathBuf;
use std::{fs::{create_dir_all,OpenOptions},io::Write};
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
async fn import_root(state:tauri::State<'_,AppState>,path:String)->Result<Source,String>{let c=state.catalog.clone();let source=tauri::async_runtime::spawn_blocking(move||{let path=PathBuf::from(path);if !path.is_dir(){return Err("SoundShelf imports folders in this increment; choose a folder containing the audio files.".to_string());}c.lock().map_err(|e|e.to_string())?.add_source(&path).map_err(|e|e.to_string())}).await.map_err(|e|e.to_string())??;state.enqueue(source.clone())?;Ok(source)}
#[tauri::command]
fn scan_source(state:tauri::State<AppState>,id:String)->Result<(),String>{let source=state.catalog.lock().map_err(|e|e.to_string())?.source(&id).map_err(|e|e.to_string())?;state.enqueue(source)}
#[tauri::command]
async fn relink_source(state:tauri::State<'_,AppState>,id:String,path:String)->Result<Source,String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.relink(&id,&PathBuf::from(path)).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn search_sounds(state:tauri::State<'_,AppState>,query:SearchQuery)->Result<SearchResults,String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.search(&query).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn save_search(state:tauri::State<'_,AppState>,name:String,query:SearchQuery)->Result<SavedSearch,String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.save_search(&name,&query).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn saved_searches(state:tauri::State<'_,AppState>)->Result<Vec<SavedSearch>,String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.saved_searches().map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn delete_saved_search(state:tauri::State<'_,AppState>,id:String)->Result<(),String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.delete_saved_search(&id).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn get_sound(state:tauri::State<'_,AppState>,id:String)->Result<Sound,String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.ready_sound(&id).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
async fn annotate(state:tauri::State<'_,AppState>,id:String,tags:Vec<String>,comment:String,favorite:bool)->Result<(),String>{let c=state.catalog.clone();tauri::async_runtime::spawn_blocking(move||c.lock().map_err(|e|e.to_string())?.annotate(&id,&tags,&comment,favorite).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?}
#[tauri::command]
fn jobs(state:tauri::State<AppState>)->Result<Vec<Progress>,String>{state.progress.lock().map(|s|s.clone()).map_err(|e|e.to_string())}
#[tauri::command]
fn cancel_import(state:tauri::State<AppState>){state.cancel.store(true,std::sync::atomic::Ordering::Relaxed);}

#[tauri::command]
async fn playback_play(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let (path, duration) = {
        let c = state.catalog.lock().map_err(|e| e.to_string())?;
        let sound = c.ready_sound(&id).map_err(|e| e.to_string())?;
        let path = c.resolve(&id).map_err(|e| e.to_string())?;
        let duration = sound.profile.as_ref().map(|p| p.duration).unwrap_or(0.0);
        (path, duration)
    };
    state.player.play(&id, &path, duration).map_err(|e| e.to_string())
}

#[tauri::command]
fn playback_pause(state: tauri::State<'_, AppState>) {
    state.player.pause();
}

#[tauri::command]
fn playback_resume(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.player.resume().map_err(|e| e.to_string())
}

#[tauri::command]
fn playback_stop(state: tauri::State<'_, AppState>) {
    state.player.stop();
}

#[tauri::command]
fn playback_seek(state: tauri::State<'_, AppState>, position_seconds: f64) -> Result<(), String> {
    state.player.seek(position_seconds).map_err(|e| e.to_string())
}

#[tauri::command]
fn playback_set_volume(state: tauri::State<'_, AppState>, volume: f32) {
    state.player.set_volume(volume);
}

#[tauri::command]
fn playback_status(state: tauri::State<'_, AppState>) -> PlaybackStatus {
    state.player.status()
}

#[tauri::command]
async fn get_waveform(
    state: tauri::State<'_, AppState>,
    id: String,
    start_frame: Option<u64>,
    end_frame: Option<u64>,
    max_points: Option<usize>,
) -> Result<soundshelf_core::waveform::WaveformResponse, String> {
    let (sound, path) = {
        let c = state.catalog.lock().map_err(|e| e.to_string())?;
        let sound = c.ready_sound(&id).map_err(|e| e.to_string())?;
        let path = c.resolve(&id).map_err(|e| e.to_string())?;
        (sound, path)
    };
    let profile = sound.profile.ok_or_else(|| "Sound has no profile".to_string())?;
    let tools = state.tools.as_ref().ok_or_else(|| "Media tools unavailable".to_string())?.clone();
    let waveforms = state.waveforms.clone();
    let cancel = state.cancel.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let pyramid = waveforms.load_or_build(
            &sound.content_hash,
            &path,
            profile.sample_rate,
            profile.channels,
            &tools,
            cancel,
        ).map_err(|e| e.to_string())?;

        Ok(pyramid.query_window(start_frame, end_frame, max_points))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn install_startup_diagnostics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let home = std::env::var_os("HOME").or_else(||std::env::var_os("USERPROFILE")).map(PathBuf::from);
        let path = match home {
            Some(home) if cfg!(target_os = "macos") => home.join("Library/Logs/SoundShelf/startup.log"),
            Some(home) if cfg!(target_os = "windows") => home.join("AppData/Local/SoundShelf/Logs/startup.log"),
            Some(home) => home.join(".local/state/soundshelf/startup.log"),
            None => std::env::temp_dir().join("soundshelf-startup.log"),
        };
        if let Some(parent) = path.parent() {
            let _ = create_dir_all(parent);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{info}\n{}\n", std::backtrace::Backtrace::force_capture());
        }
        previous(info);
    }));
}

fn fallback_data_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| {
            if cfg!(target_os = "macos") {
                home.join("Library/Application Support/SoundShelf")
            } else if cfg!(target_os = "windows") {
                home.join("AppData/Local/SoundShelf")
            } else {
                home.join(".local/share/SoundShelf")
            }
        })
        .unwrap_or_else(|| PathBuf::from(".soundshelf"))
}

fn fallback_resource_directory() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("../Resources")))
        .unwrap_or_else(|| PathBuf::from("Resources"))
}

fn main() {
    install_startup_diagnostics();
    tauri::Builder::default().plugin(tauri_plugin_dialog::init())
        .setup(|app|{let path=if cfg!(debug_assertions){std::env::var_os("SOUNDSHELF_DATA_DIR").map(PathBuf::from).or_else(||app.path().app_data_dir().ok()).unwrap_or_else(fallback_data_directory)}else{app.path().app_data_dir().unwrap_or_else(|_|fallback_data_directory())};let resources=app.path().resource_dir().unwrap_or_else(|_|fallback_resource_directory());app.manage(AppState::new(path,resources)?);Ok(())})
        .invoke_handler(tauri::generate_handler![
            app_info,
            choose_folder,
            sources,
            import_root,
            scan_source,
            relink_source,
            search_sounds,
            save_search,
            saved_searches,
            delete_saved_search,
            get_sound,
            annotate,
            jobs,
            cancel_import,
            playback_play,
            playback_pause,
            playback_resume,
            playback_stop,
            playback_seek,
            playback_set_volume,
            playback_status,
            get_waveform
        ])
        .build(tauri::generate_context!()).expect("SoundShelf could not start")
        .run(|app,event|if matches!(event,tauri::RunEvent::Exit){app.state::<AppState>().shutdown();});
}
