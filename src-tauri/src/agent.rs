use crate::service::AppState;
use soundshelf_agent::{Pairing, Status};
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
pub fn mcp_pair(state: tauri::State<AppState>, name: String) -> Result<Pairing, String> {
    state.agent.pair(name)
}
#[tauri::command]
pub fn mcp_revoke(state: tauri::State<AppState>, id: String) -> Result<Status, String> {
    state.agent.revoke(&id)?;
    state.agent.status()
}
