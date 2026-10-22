//! Thin IPC adapter for SS-058 exact image search and saved image searches.
//! The desktop owner searches every registered source; rules live in the core.
use crate::service::AppState;
use soundshelf_core::{
    catalog::Catalog,
    image_search::{FieldCapability, ImageQuery, ImageSearchPage, SavedImageSearch, SearchError as E},
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

#[tauri::command]
pub async fn search_images(
    state: tauri::State<'_, AppState>,
    query: ImageQuery,
    offset: u32,
    limit: u32,
    facets: bool,
) -> Result<ImageSearchPage, E> {
    with(state, move |c| c.search_images(None, &query, offset, limit, facets)).await
}
#[tauri::command]
pub fn image_search_capabilities() -> Vec<FieldCapability> {
    soundshelf_core::image_search::capabilities()
}
#[tauri::command]
pub async fn saved_image_searches(state: tauri::State<'_, AppState>) -> Result<Vec<SavedImageSearch>, E> {
    with(state, |c| c.saved_image_searches()).await
}
#[tauri::command]
pub async fn save_image_search(
    state: tauri::State<'_, AppState>,
    name: String,
    query: ImageQuery,
    replace: bool,
) -> Result<SavedImageSearch, E> {
    with(state, move |c| c.save_image_search(&name, &query, replace)).await
}
#[tauri::command]
pub async fn delete_saved_image_search(state: tauri::State<'_, AppState>, id: String) -> Result<(), E> {
    with(state, move |c| c.delete_saved_image_search(&id)).await
}
