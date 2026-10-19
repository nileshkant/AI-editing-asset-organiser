//! Thin IPC adapter for SS-045 gallery windows and inspector details (metadata only).
use crate::service::AppState;
use soundshelf_core::{
    image_gallery::{GalleryPage, GallerySort, ImageDetails},
    image_jobs::ImportError as E,
};
#[tauri::command]
pub async fn image_gallery_page(
    state: tauri::State<'_, AppState>,
    source_id: Option<String>,
    sort: GallerySort,
    offset: u32,
    limit: u32,
) -> Result<GalleryPage, E> {
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let scope = source_id.map(|s| vec![s]);
        c.lock()
            .map_err(|_| E::Internal)?
            .image_gallery_page(scope.as_deref(), sort, offset, limit)
            .map_err(|_| E::InvalidArgument)
    })
    .await
    .map_err(|_| E::Internal)?
}
#[tauri::command]
pub async fn image_details(state: tauri::State<'_, AppState>, id: String) -> Result<ImageDetails, E> {
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        c.lock()
            .map_err(|_| E::Internal)?
            .image_details(&id)
            .map_err(|_| E::NotFound)
    })
    .await
    .map_err(|_| E::Internal)?
}
