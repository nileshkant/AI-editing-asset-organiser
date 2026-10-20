//! Thin IPC adapter for SS-062 exact-duplicate stacks; rules live in the core.
use crate::service::AppState;
use soundshelf_core::{
    catalog::Catalog,
    image_duplicates::{DuplicateAction, DuplicateError as E, DuplicateGroup, DuplicatePage},
};

async fn with<T: Send + 'static>(
    state: tauri::State<'_, AppState>,
    f: impl FnOnce(&mut Catalog) -> Result<T, E> + Send + 'static,
) -> Result<T, E> {
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || f(&mut *c.lock().map_err(|_| E::Internal)?))
        .await
        .map_err(|_| E::Internal)?
}

/// The desktop owner sees every source; `source_id` narrows the view like the gallery filter.
#[tauri::command]
pub async fn image_duplicate_groups(
    state: tauri::State<'_, AppState>,
    source_id: Option<String>,
    include_hidden: bool,
    offset: u32,
    limit: u32,
) -> Result<DuplicatePage, E> {
    with(state, move |c| {
        let scope = source_id.map(|s| vec![s]);
        c.image_duplicate_groups(scope.as_deref(), include_hidden, offset, limit)
    })
    .await
}
#[tauri::command]
pub async fn image_duplicate_group(
    state: tauri::State<'_, AppState>,
    source_id: Option<String>,
    content_hash: String,
) -> Result<DuplicateGroup, E> {
    with(state, move |c| {
        let scope = source_id.map(|s| vec![s]);
        c.image_duplicate_group(scope.as_deref(), &content_hash)
    })
    .await
}
#[tauri::command]
pub async fn update_image_duplicate_group(
    state: tauri::State<'_, AppState>,
    source_id: Option<String>,
    content_hash: String,
    expected_revision: u32,
    action: DuplicateAction,
) -> Result<DuplicateGroup, E> {
    with(state, move |c| {
        let scope = source_id.map(|s| vec![s]);
        c.update_image_duplicate_group(scope.as_deref(), &content_hash, expected_revision, &action)
    })
    .await
}
