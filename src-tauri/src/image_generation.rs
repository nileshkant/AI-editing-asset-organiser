//! Thin IPC adapter for SS-055 generation metadata and prompt provenance; rules live in the core.
use crate::service::AppState;
use soundshelf_core::{
    catalog::Catalog,
    image_provenance::{GenerationSettings, ProvenanceError as E, ProvenancePatch, ProvenanceView},
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
pub async fn image_generation(state: tauri::State<'_, AppState>, id: String) -> Result<ProvenanceView, E> {
    with(state, move |c| c.image_generation(&id)).await
}
#[tauri::command]
pub async fn edit_image_generation(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: u32,
    patch: ProvenancePatch,
) -> Result<ProvenanceView, E> {
    with(state, move |c| c.edit_image_generation(&id, expected_revision, &patch)).await
}
#[tauri::command]
pub async fn image_generation_settings(state: tauri::State<'_, AppState>) -> Result<GenerationSettings, E> {
    with(state, |c| c.generation_settings()).await
}
#[tauri::command]
pub async fn set_image_generation_settings(
    state: tauri::State<'_, AppState>,
    settings: GenerationSettings,
) -> Result<GenerationSettings, E> {
    with(state, move |c| c.set_generation_settings(settings)).await
}
