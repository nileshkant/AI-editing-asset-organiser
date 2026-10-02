use crate::service::AppState;
use soundshelf_agent::{Pairing, Status};
use soundshelf_core::{
    agent::Access,
    export::{DestinationGrant, ExportOptions},
};
use tauri_plugin_dialog::DialogExt;
#[tauri::command]
pub fn mcp_status(state: tauri::State<AppState>) -> Result<Status, String> {
    state.agent.status()
}
#[tauri::command]
pub async fn mcp_start(state: tauri::State<'_, AppState>, port: u16) -> Result<Status, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.start(port))
        .await
        .map_err(|_| "MCP start failed")?
}
#[tauri::command]
pub async fn mcp_stop(state: tauri::State<'_, AppState>) -> Result<Status, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || {
        agent.stop();
        agent.status()
    })
    .await
    .map_err(|_| "MCP stop failed")?
}
#[tauri::command]
pub fn mcp_pair(
    state: tauri::State<AppState>,
    name: String,
    access: Option<Access>,
) -> Result<Pairing, String> {
    state
        .agent
        .pair_with_access(name, access.unwrap_or_default())
}
#[tauri::command]
pub fn mcp_revoke(state: tauri::State<AppState>, id: String) -> Result<Status, String> {
    state.agent.revoke(&id)?;
    state.agent.status()
}

#[tauri::command]
pub async fn mcp_approve_destination(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    client_id: String,
    format: soundshelf_core::export::ExportFormat,
) -> Result<Option<DestinationGrant>, String> {
    let agent = state.agent.clone();
    let library = state.agent_library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let client = agent.client(&client_id)?;
        if !client.access.export {
            return Err("Client export permission is disabled".into());
        }
        let selected = app
            .dialog()
            .file()
            .add_filter("Lossless audio", &[format.extension()])
            .set_file_name(format!("clip.{}", format.extension()))
            .blocking_save_file();
        selected
            .map(|p| {
                let path = p.into_path().map_err(|_| "Choose an export file")?;
                // Pairing may have been revoked while the dialog was open.
                agent.client(&client_id)?;
                library.approve_destination(
                    &client_id,
                    &path,
                    &ExportOptions {
                        format,
                        ..Default::default()
                    },
                )
            })
            .transpose()
    })
    .await
    .map_err(|_| "Destination approval failed")?
}
