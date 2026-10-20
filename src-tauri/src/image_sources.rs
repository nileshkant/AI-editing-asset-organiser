//! Thin IPC adapter for SS-047 source refresh, watching, relink and move
//! reconciliation. Rules live in soundshelf_core::image_sources/image_relink.
use crate::{image_watch::WatchStatus, service::AppState};
use soundshelf_core::{
    catalog::Catalog,
    image_contracts::AssetHeader,
    image_jobs::{ImageImportJob, ImportError as E, ScanKind},
    image_relink::RelinkPreview,
    image_sources::{self, ImageSourceStatus, MoveCandidate},
};
use std::sync::atomic::AtomicBool;
use tauri_plugin_dialog::DialogExt;

#[derive(serde::Serialize)]
pub struct SourceView {
    #[serde(flatten)]
    status: ImageSourceStatus,
    /// Runtime watcher state; null when watching is off.
    watch_state: Option<WatchStatus>,
}
fn view(state: &AppState, status: ImageSourceStatus) -> SourceView {
    let watch_state = status
        .watch
        .then(|| state.image_watchers.status(&status.source_id))
        .flatten();
    SourceView { status, watch_state }
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, E> + Send + 'static) -> Result<T, E> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|_| E::Internal)?
}
fn with_catalog<T>(c: &std::sync::Arc<std::sync::Mutex<Catalog>>, f: impl FnOnce(&mut Catalog) -> Result<T, E>) -> Result<T, E> {
    f(&mut *c.lock().map_err(|_| E::Internal)?)
}

#[tauri::command]
pub async fn image_source_statuses(state: tauri::State<'_, AppState>) -> Result<Vec<SourceView>, E> {
    let c = state.catalog.clone();
    let statuses = blocking(move || with_catalog(&c, |c| c.image_source_statuses())).await?;
    Ok(statuses.into_iter().map(|s| view(&state, s)).collect())
}
#[tauri::command]
pub async fn check_image_source(state: tauri::State<'_, AppState>, id: String) -> Result<SourceView, E> {
    let c = state.catalog.clone();
    let status = blocking(move || {
        image_sources::probe_image_source(&c, &id)?;
        with_catalog(&c, |c| c.image_source_status(&id))
    })
    .await?;
    state.image_watchers.nudge();
    Ok(view(&state, status))
}
#[tauri::command]
pub async fn refresh_image_source(
    state: tauri::State<'_, AppState>,
    id: String,
    scan: ScanKind,
    verify: bool,
) -> Result<ImageImportJob, E> {
    state.image_imports.require_enabled()?;
    let c = state.catalog.clone();
    let job = blocking(move || image_sources::refresh_image_source(&c, &id, scan, verify)).await?;
    state.image_imports.emit(job.clone());
    Ok(job)
}
#[tauri::command]
pub async fn set_image_source_watch(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<SourceView, E> {
    state.image_imports.require_enabled()?;
    let c = state.catalog.clone();
    let status = blocking(move || with_catalog(&c, |c| c.set_image_source_watch(&id, enabled))).await?;
    state.image_watchers.nudge();
    Ok(view(&state, status))
}
/// The replacement folder comes only from the native picker, never from IPC text.
#[tauri::command]
pub async fn choose_image_relink(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<RelinkPreview>, E> {
    state.image_imports.require_enabled()?;
    let (c, relinks) = (state.catalog.clone(), state.image_relinks.clone());
    blocking(move || {
        let Some(folder) = app.dialog().file().blocking_pick_folder() else {
            return Ok(None);
        };
        let folder = folder.into_path().map_err(|_| E::InvalidArgument)?;
        relinks.prepare(&c, &id, &folder, &AtomicBool::new(false)).map(Some)
    })
    .await
}
#[tauri::command]
pub async fn confirm_image_relink(
    state: tauri::State<'_, AppState>,
    preview_id: String,
    acknowledged: bool,
) -> Result<ImageImportJob, E> {
    state.image_imports.require_enabled()?;
    let (c, relinks) = (state.catalog.clone(), state.image_relinks.clone());
    let job = blocking(move || relinks.confirm(&c, &preview_id, acknowledged)).await?;
    state.image_watchers.nudge();
    state.image_imports.emit(job.clone());
    Ok(job)
}
#[tauri::command]
pub fn cancel_image_relink(state: tauri::State<AppState>, preview_id: String) -> Result<(), E> {
    state.image_relinks.discard(&preview_id)
}
#[tauri::command]
pub async fn image_move_candidates(
    state: tauri::State<'_, AppState>,
    source_id: String,
) -> Result<Vec<MoveCandidate>, E> {
    let c = state.catalog.clone();
    blocking(move || with_catalog(&c, |c| c.image_move_candidates(&source_id))).await
}
#[tauri::command]
pub async fn reconcile_image_move(
    state: tauri::State<'_, AppState>,
    missing_id: String,
    present_id: String,
    missing_revision: u32,
    present_revision: u32,
) -> Result<AssetHeader, E> {
    state.image_imports.require_enabled()?;
    let c = state.catalog.clone();
    blocking(move || {
        with_catalog(&c, |c| {
            c.reconcile_image_move(&missing_id, &present_id, missing_revision, present_revision)
                .map(|a| a.header)
        })
    })
    .await
}
