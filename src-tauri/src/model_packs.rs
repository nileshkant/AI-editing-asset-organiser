//! Thin IPC adapter for SS-051 optional local model packs. Rules, verification
//! and files live in soundshelf_core::model_pack*; this module only schedules
//! blocking work off the UI thread. Nothing here is needed for the base app.
use crate::service::AppState;
use soundshelf_core::model_pack::Capability;
use soundshelf_core::model_pack_fetch::HttpFetcher;
use soundshelf_core::model_pack_store::{ModelPacks, Overview};
use std::{sync::Arc, time::Duration};

/// How long replacing or removing a pack waits for its running tasks.
const DRAIN: Duration = Duration::from_secs(10);

fn store(state: &tauri::State<'_, AppState>) -> Result<Arc<ModelPacks>, String> {
    state.model_packs.clone().map_err(|reason| format!("Local model packs are unavailable: {reason}"))
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, soundshelf_core::model_pack::PackError> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|_| "The model pack operation failed".to_owned())?.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn model_packs(state: tauri::State<'_, AppState>) -> Result<Overview, String> {
    let packs = store(&state)?;
    tauri::async_runtime::spawn_blocking(move || packs.overview()).await.map_err(|_| "The model pack list failed".to_owned())
}
/// Starts or resumes a download. The call returns when it finishes, fails or
/// is cancelled; the UI polls `model_packs` for progress meanwhile.
#[tauri::command]
pub async fn model_pack_install(state: tauri::State<'_, AppState>, id: String, version: String) -> Result<(), String> {
    let packs = store(&state)?;
    blocking(move || packs.install(&id, &version, &HttpFetcher::new(false))).await
}
#[tauri::command]
pub async fn model_pack_cancel_install(state: tauri::State<'_, AppState>) -> Result<(), String> {
    store(&state)?.cancel_install();
    Ok(())
}
#[tauri::command]
pub async fn model_pack_discard_partial(state: tauri::State<'_, AppState>, id: String, version: String) -> Result<(), String> {
    let packs = store(&state)?;
    blocking(move || packs.discard_partial(&id, &version)).await
}
#[tauri::command]
pub async fn model_pack_activate(state: tauri::State<'_, AppState>, id: String, version: String) -> Result<(), String> {
    let packs = store(&state)?;
    blocking(move || packs.activate(&id, &version, DRAIN).map(|_| ())).await
}
#[tauri::command]
pub async fn model_pack_deactivate(state: tauri::State<'_, AppState>, capability: Capability) -> Result<(), String> {
    let packs = store(&state)?;
    blocking(move || packs.deactivate(capability, DRAIN)).await
}
#[tauri::command]
pub async fn model_pack_uninstall(state: tauri::State<'_, AppState>, id: String, version: String, confirmed: bool) -> Result<(), String> {
    if !confirmed {
        return Err("Confirm removing this pack".into());
    }
    let packs = store(&state)?;
    blocking(move || packs.uninstall(&id, &version, DRAIN)).await
}
