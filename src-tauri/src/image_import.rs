//! Native-only image development adapters. Production capability remains gated.
use crate::service::AppState;
use soundshelf_core::{
    catalog::Catalog,
    image_catalog::ImagePage,
    image_import::ImageImportEngine,
    image_import_preview::{MediaImportPreview, MediaImportPreviews, NativeSelectionKind},
    image_jobs::{ImageImportItemPage, ImageImportJob, ImportError as E, ImportMode},
    media::MediaTools,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
type Sink = Arc<dyn Fn(ImageImportJob) + Send + Sync>;
pub struct ImageImports {
    pub previews: Arc<MediaImportPreviews>,
    pub enabled: bool,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    sink: Arc<Mutex<Option<Sink>>>,
    pub native_tasks: AtomicUsize,
}
impl ImageImports {
    pub fn new(catalog: Arc<Mutex<Catalog>>, resources: &Path, tools: Option<MediaTools>) -> Self {
        let name = if cfg!(windows) {
            "creativeshelf-image-worker.exe"
        } else {
            "creativeshelf-image-worker"
        };
        let mut worker = resources.join("images").join(name);
        if cfg!(debug_assertions) && !worker.is_file() {
            if let Ok(exe) = std::env::current_exe() {
                let parent = exe.parent().unwrap_or(Path::new(""));
                worker = parent.join(name);
                if !worker.is_file() && parent.file_name().is_some_and(|s| s == "deps") {
                    worker = parent.parent().unwrap_or(parent).join(name);
                }
            }
        }
        let enabled =
            cfg!(feature = "image-development") && worker.is_absolute() && worker.is_file();
        let stop = Arc::new(AtomicBool::new(false));
        let sink = Arc::new(Mutex::new(None::<Sink>));
        let handle = if enabled {
            let stop = stop.clone();
            let sink = sink.clone();
            Some(thread::spawn(move || {
                let owner = uuid::Uuid::new_v4().to_string();
                let engine = ImageImportEngine {
                    catalog: catalog.clone(),
                    worker,
                    tools,
                };
                if let Ok(c) = catalog.lock() {
                    let _ = c.recover_image_imports(soundshelf_core::jobs::now_secs());
                }
                let mut last = Instant::now() - Duration::from_secs(1);
                while !stop.load(Ordering::Relaxed) {
                    match engine.tick(&owner, &stop) {
                        Ok(Some(job)) => {
                            if job.terminal() || last.elapsed() >= Duration::from_millis(100) {
                                if let Ok(s) = sink.lock() {
                                    if let Some(send) = s.as_ref() {
                                        send(job);
                                    }
                                }
                                last = Instant::now();
                            }
                        }
                        Ok(None) | Err(_) => thread::sleep(Duration::from_millis(100)),
                    }
                }
                if let Ok(c) = catalog.lock() {
                    let _ = c.checkpoint_image_import_owner(&owner);
                }
            }))
        } else {
            None
        };
        Self {
            previews: Arc::new(MediaImportPreviews::default()),
            enabled,
            stop,
            worker: Mutex::new(handle),
            sink,
            native_tasks: AtomicUsize::new(0),
        }
    }
    pub fn require_enabled(&self) -> Result<(), E> {
        if self.enabled {
            Ok(())
        } else {
            Err(E::WorkerUnavailable)
        }
    }
    pub fn set_events(&self, app: tauri::AppHandle) {
        if let Ok(mut s) = self.sink.lock() {
            *s = Some(Arc::new(move |j| {
                let _ = app.emit("image-import-progress", j);
            }));
        }
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut handle) = self.worker.lock() {
            if let Some(h) = handle.take() {
                let _ = h.join();
            }
        }
    }
}
impl Drop for ImageImports {
    fn drop(&mut self) {
        self.shutdown();
    }
}
#[derive(serde::Serialize)]
pub struct ImportCapabilities {
    enabled: bool,
    reference_in_place: bool,
    production_images: bool,
}
#[tauri::command]
pub fn image_import_capabilities(state: tauri::State<AppState>) -> ImportCapabilities {
    ImportCapabilities {
        enabled: state.image_imports.enabled,
        reference_in_place: true,
        production_images: soundshelf_core::image_catalog::IMAGE_CAPABILITY_ENABLED,
    }
}
#[tauri::command]
pub async fn choose_media_import(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    folders: bool,
) -> Result<Vec<String>, E> {
    state.image_imports.require_enabled()?;
    let previews = state.image_imports.previews.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let picker = app.dialog().file();
        let paths = if folders {
            picker.blocking_pick_folders()
        } else {
            picker
                .add_filter(
                    "Images and audio",
                    &[
                        "png", "jpg", "jpeg", "webp", "wav", "mp3", "flac", "ogg", "opus", "m4a",
                        "aac", "aif", "aiff", "wma", "caf",
                    ],
                )
                .blocking_pick_files()
        }
        .unwrap_or_default();
        if paths.is_empty() {
            return Ok(vec![]);
        }
        let paths = paths
            .into_iter()
            .map(|p| p.into_path().map_err(|_| E::InvalidArgument))
            .collect::<Result<Vec<_>, _>>()?;
        previews.approve_native(
            &paths,
            if folders {
                NativeSelectionKind::Folders
            } else {
                NativeSelectionKind::Files
            },
        )
    })
    .await
    .map_err(|_| E::Internal)?
}
#[tauri::command]
pub async fn preview_media_import(
    state: tauri::State<'_, AppState>,
    request_id: String,
    paths: Vec<String>,
    mode: ImportMode,
) -> Result<MediaImportPreview, E> {
    state.image_imports.require_enabled()?;
    let previews = state.image_imports.previews.clone();
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || previews.prepare(&c, &request_id, paths, mode))
        .await
        .map_err(|_| E::Internal)?
}
#[tauri::command]
pub fn cancel_media_preview(
    state: tauri::State<AppState>,
    request_id: String,
    preview_id: Option<String>,
) -> Result<(), E> {
    state.image_imports.previews.cancel_prepare(&request_id)?;
    if let Some(id) = preview_id {
        state.image_imports.previews.discard(&id)?;
    }
    Ok(())
}
#[tauri::command]
pub async fn confirm_media_import(
    state: tauri::State<'_, AppState>,
    preview_id: String,
) -> Result<Vec<ImageImportJob>, E> {
    state.image_imports.require_enabled()?;
    let p = state.image_imports.previews.clone();
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || p.confirm(&c, &preview_id))
        .await
        .map_err(|_| E::Internal)?
}
#[tauri::command]
pub async fn image_import_jobs(
    state: tauri::State<'_, AppState>,
    after: Option<String>,
) -> Result<Vec<ImageImportJob>, E> {
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        c.lock()
            .map_err(|_| E::Internal)?
            .image_import_jobs(after.as_deref(), 50)
    })
    .await
    .map_err(|_| E::Internal)?
}
#[tauri::command]
pub async fn image_import_items(
    state: tauri::State<'_, AppState>,
    id: String,
    after: Option<String>,
) -> Result<ImageImportItemPage, E> {
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        c.lock()
            .map_err(|_| E::Internal)?
            .image_import_items(&id, after.as_deref(), 50)
    })
    .await
    .map_err(|_| E::Internal)?
}
#[tauri::command]
pub async fn cancel_image_import(
    state: tauri::State<'_, AppState>,
    id: String,
    source_generation: String,
) -> Result<ImageImportJob, E> {
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        c.lock().map_err(|_| E::Internal)?.cancel_image_import(
            &id,
            source_generation.parse().map_err(|_| E::InvalidArgument)?,
        )
    })
    .await
    .map_err(|_| E::Internal)?
}
#[tauri::command]
pub async fn retry_image_import(
    state: tauri::State<'_, AppState>,
    id: String,
    source_generation: String,
) -> Result<ImageImportJob, E> {
    state.image_imports.require_enabled()?;
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        c.lock().map_err(|_| E::Internal)?.retry_image_import(
            &id,
            source_generation.parse().map_err(|_| E::InvalidArgument)?,
        )
    })
    .await
    .map_err(|_| E::Internal)?
}
#[tauri::command]
pub async fn imported_images(
    state: tauri::State<'_, AppState>,
    source_id: String,
    after: Option<String>,
) -> Result<ImagePage, E> {
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        c.lock()
            .map_err(|_| E::Internal)?
            .list_images(&[source_id], after.as_deref(), 50)
            .map_err(|_| E::Internal)
    })
    .await
    .map_err(|_| E::Internal)?
}
/// OS-owned drop paths enter here; renderer-provided paths cannot create grants.
pub fn native_drop(window: &tauri::Window, paths: Vec<PathBuf>) {
    let state = window.state::<AppState>();
    let imports = state.image_imports.clone();
    if !imports.enabled {
        return;
    }
    if imports
        .native_tasks
        .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |v| {
            (v < 4).then_some(v + 1)
        })
        .is_err()
    {
        let _ = window.emit("image-import-drop-error", E::Busy);
        return;
    }
    let window = window.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = imports
            .previews
            .approve_native(&paths, NativeSelectionKind::Drop);
        imports.native_tasks.fetch_sub(1, Ordering::Release);
        match result {
            Ok(paths) => {
                let _ = window.emit("image-import-drop", paths);
            }
            Err(error) => {
                let _ = window.emit("image-import-drop-error", error);
            }
        }
    });
}

#[cfg(all(test, feature = "image-development"))]
mod tests {
    use super::*;
    #[test]
    fn image_import_native_state_uses_trusted_worker_and_persists_cancel_before_shutdown() {
        let dir = std::env::temp_dir().join(format!("ss043-native-{}", uuid::Uuid::new_v4()));
        let resources = dir.join("resources");
        let images = resources.join("images");
        std::fs::create_dir_all(&images).unwrap();
        let name = if cfg!(windows) {
            "creativeshelf-image-worker.exe"
        } else {
            "creativeshelf-image-worker"
        };
        let exe = std::env::current_exe().unwrap();
        let binary = exe.parent().unwrap().parent().unwrap().join(name);
        assert!(
            binary.is_file(),
            "build the workspace image helper before this native fixture"
        );
        std::fs::copy(binary, images.join(name)).unwrap();
        let state = AppState::new(dir.join("app-data"), resources).unwrap();
        assert!(state.image_imports.enabled);
        let root = dir.join("media");
        std::fs::create_dir(&root).unwrap();
        // A checked-in synthetic still fixture exercises the actual worker, not
        // renderer mocks. Original bytes remain unchanged.
        let file = root.join("native.webp");
        let bytes = include_bytes!("../../fixtures/images/solid-lossy.webp");
        std::fs::write(&file, bytes).unwrap();
        let raw = state
            .image_imports
            .previews
            .approve_native(&[file.clone()], NativeSelectionKind::Files)
            .unwrap();
        let preview = state
            .image_imports
            .previews
            .prepare(
                &state.catalog,
                &uuid::Uuid::new_v4().to_string(),
                raw,
                ImportMode::Images,
            )
            .unwrap();
        let jobs = state
            .image_imports
            .previews
            .confirm(&state.catalog, &preview.id)
            .unwrap();
        let id = &jobs[0].id;
        let start = Instant::now();
        loop {
            let job = state.catalog.lock().unwrap().image_import_job(id).unwrap();
            if job.terminal() {
                assert_eq!(job.ready, 1, "{job:?}");
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(15));
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert!(state
            .catalog
            .lock()
            .unwrap()
            .active_jobs()
            .unwrap()
            .is_empty());
        let retry = state
            .catalog
            .lock()
            .unwrap()
            .retry_image_import(id, 0)
            .unwrap();
        state
            .catalog
            .lock()
            .unwrap()
            .cancel_image_import(&retry.id, 0)
            .unwrap();
        state.shutdown();
        let reopened = Catalog::open(&dir.join("app-data/library.sqlite")).unwrap();
        assert_eq!(
            reopened.image_import_job(&retry.id).unwrap().state,
            soundshelf_core::image_jobs::ImportState::Cancelled
        );
        drop(reopened);
        drop(state);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
