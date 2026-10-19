//! Thin IPC adapter for SS-046 image annotations; rules live in the core.
use crate::service::AppState;
use soundshelf_core::{
    catalog::Catalog,
    image_annotations::{
        ActionSummary, AnnotationError as E, AnnotationPatch, BulkPreview, BulkResult, BulkTarget, EditResult,
        RenamePreview, TagList,
    },
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
pub async fn edit_image_annotations(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: u32,
    patch: AnnotationPatch,
) -> Result<EditResult, E> {
    with(state, move |c| c.edit_image_annotations(&id, expected_revision, &patch)).await
}
#[tauri::command]
pub async fn preview_image_bulk_annotations(
    state: tauri::State<'_, AppState>,
    ids: Vec<String>,
    patch: AnnotationPatch,
) -> Result<BulkPreview, E> {
    with(state, move |c| c.preview_bulk_annotations(&ids, &patch)).await
}
#[tauri::command]
pub async fn apply_image_bulk_annotations(
    state: tauri::State<'_, AppState>,
    targets: Vec<BulkTarget>,
    patch: AnnotationPatch,
) -> Result<BulkResult, E> {
    with(state, move |c| c.apply_bulk_annotations(&targets, &patch)).await
}
#[tauri::command]
pub async fn image_annotation_history(state: tauri::State<'_, AppState>, limit: u32) -> Result<Vec<ActionSummary>, E> {
    with(state, move |c| c.image_annotation_history(limit)).await
}
#[tauri::command]
pub async fn undo_image_annotation_action(state: tauri::State<'_, AppState>, action_id: String) -> Result<BulkResult, E> {
    with(state, move |c| c.undo_image_annotation_action(&action_id)).await
}
#[tauri::command]
pub async fn image_tags(state: tauri::State<'_, AppState>, source_id: Option<String>) -> Result<TagList, E> {
    with(state, move |c| c.image_tags(source_id.as_deref())).await
}
#[tauri::command]
pub async fn preview_image_tag_rename(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
    source_id: Option<String>,
) -> Result<RenamePreview, E> {
    with(state, move |c| c.preview_image_tag_rename(&from, &to, source_id.as_deref())).await
}
#[tauri::command]
pub async fn rename_image_tag(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
    source_id: Option<String>,
    keep_alias: bool,
) -> Result<BulkResult, E> {
    with(state, move |c| c.rename_image_tag(&from, &to, source_id.as_deref(), keep_alias)).await
}
#[tauri::command]
pub async fn remove_image_tag_alias(state: tauri::State<'_, AppState>, alias: String) -> Result<(), E> {
    with(state, move |c| c.remove_image_tag_alias(&alias)).await
}
