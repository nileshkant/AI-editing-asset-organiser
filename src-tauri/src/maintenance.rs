use crate::service::AppState;
use tauri::Emitter;
use soundshelf_core::maintenance::Preferences;
use tauri_plugin_dialog::DialogExt;
#[derive(serde::Serialize)]
pub struct ResourceSettings {
    preferences: Preferences,
    output_devices: Vec<String>,
    import_workers: u8,
}
#[tauri::command]
pub async fn resource_settings(
    state: tauri::State<'_, AppState>,
) -> Result<ResourceSettings, String> {
    let catalog = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let preferences = catalog
            .lock()
            .map_err(|e| e.to_string())?
            .preferences()
            .map_err(|e| e.to_string())?;
        let output_devices =
            soundshelf_core::playback::Player::output_devices().map_err(|e| e.to_string())?;
        Ok(ResourceSettings {
            preferences,
            output_devices,
            import_workers: 1,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn save_resource_settings(
    state: tauri::State<'_, AppState>,
    preferences: Preferences,
) -> Result<(), String> {
    let catalog = state.catalog.clone();
    let player = state.player.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Validate/store first; output change stops playback deliberately.
        let previous = catalog
            .lock()
            .map_err(|e| e.to_string())?
            .preferences()
            .map_err(|e| e.to_string())?;
        catalog
            .lock()
            .map_err(|e| e.to_string())?
            .set_preferences(&preferences)
            .map_err(|e| e.to_string())?;
        if previous.output_device != preferences.output_device {
            player
                .set_output_device(preferences.output_device)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn database_backup(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let catalog = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .add_filter("SQLite backup", &["sqlite"])
            .set_file_name("creativeshelf-backup.sqlite")
            .blocking_save_file();
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|e| e.to_string())?;
        catalog
            .lock()
            .map_err(|e| e.to_string())?
            .backup_database(&path)
            .map_err(|e| e.to_string())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[derive(serde::Serialize)]
pub struct RestorePreview {
    token: String,
    summary: soundshelf_core::maintenance::BackupSummary,
}
/// Pick and fully validate a backup without changing anything. Only the
/// natively picked path is retained, bound to the previewed file digest.
#[tauri::command]
pub async fn database_restore_preview(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<RestorePreview>, String> {
    let catalog = state.catalog.clone();
    let pending = state.restore_preview.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .add_filter("SQLite backup", &["sqlite"])
            .blocking_pick_file();
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|e| e.to_string())?;
        let summary = catalog
            .lock()
            .map_err(|e| e.to_string())?
            .inspect_backup(&path)
            .map_err(|e| e.to_string())?;
        let token = uuid::Uuid::new_v4().to_string();
        *pending.lock().map_err(|e| e.to_string())? = Some((token.clone(), path, summary.clone()));
        Ok(Some(RestorePreview { token, summary }))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn database_restore_cancel(state: tauri::State<AppState>) -> Result<(), String> {
    *state.restore_preview.lock().map_err(|e| e.to_string())? = None;
    Ok(())
}
#[tauri::command]
pub async fn database_restore(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    token: String,
    confirmed: bool,
) -> Result<Option<String>, String> {
    if !confirmed {
        return Err("Confirm restoring the database".into());
    }
    let (path, summary) = {
        let mut pending = state.restore_preview.lock().map_err(|e| e.to_string())?;
        match pending.take() {
            Some((t, path, summary)) if t == token => (path, summary),
            _ => return Err("Restore preview expired; choose the backup again".into()),
        }
    };
    let catalog = state.catalog.clone();
    let player = state.player.clone();
    let agent = state.agent.clone();
    let exports = state.exports.clone();
    let data = state.data_directory.clone();
    let progress = state.progress.clone();
    let imports = state.catalog_imports.clone();
    let roots = state.catalog_roots.clone();
    let image_previews = state.image_imports.previews.clone();
    let rendered_previews = state.image_previews.clone();
    tauri::async_runtime::spawn_blocking(move || {
        player.stop();
        let rollback = data.join(format!("restore-rollback-{}.sqlite", uuid::Uuid::new_v4()));
        with_mcp_stopped(&agent, || {
            exports
                .while_idle(|| {
                    catalog
                        .lock()
                        .map_err(|_| soundshelf_core::invalid("Catalog lock unavailable"))?
                        .restore_previewed(&path, &rollback, &summary)
                })
                .map_err(|e| e.to_string())
        })?;
        image_previews.invalidate().map_err(|e|e.to_string())?;
        rendered_previews.refresh(&catalog);
        let _=app.emit("image-import-reset",());
        progress.lock().map_err(|e| e.to_string())?.clear();
        imports.lock().map_err(|e| e.to_string())?.clear();
        roots.lock().map_err(|e| e.to_string())?.clear();
        let preferences = catalog
            .lock()
            .map_err(|e| e.to_string())?
            .preferences()
            .map_err(|e| e.to_string())?;
        player
            .set_output_device(preferences.output_device)
            .map_err(|e| e.to_string())?;
        Ok(Some(rollback.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}
/// MCP clients must not read the catalog while it is replaced. A restore that
/// succeeds leaves MCP stopped until the app restarts; one that is refused
/// changed nothing, so MCP comes back on its remembered port.
fn with_mcp_stopped<T>(
    agent: &soundshelf_agent::Agent,
    restore: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let was_running = agent.status().is_ok_and(|s| s.endpoint.is_some());
    agent.stop();
    let result = restore();
    if result.is_err() && was_running {
        // Best effort: the restore error is the one the user needs to see.
        let _ = agent.start(0);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::with_mcp_stopped;
    use soundshelf_agent::Agent;

    fn running() -> (Agent, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("soundshelf-restore-mcp-{}", uuid::Uuid::new_v4()));
        let agent = Agent::with_discovery(root.join("mcp.json"));
        assert!(agent.start(0).unwrap().endpoint.is_some());
        (agent, root)
    }
    #[test]
    fn a_refused_restore_brings_mcp_back() {
        let (agent, root) = running();
        let refused: Result<(), String> = with_mcp_stopped(&agent, || {
            assert!(agent.status().unwrap().endpoint.is_none(), "MCP is stopped during the restore");
            Err("Finish or cancel imports before restoring".into())
        });
        assert_eq!(refused.unwrap_err(), "Finish or cancel imports before restoring");
        assert!(agent.status().unwrap().endpoint.is_some(), "a refused restore must not leave MCP stopped");
        agent.stop();
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn a_successful_restore_leaves_mcp_stopped_and_a_stopped_mcp_is_never_started() {
        let (agent, root) = running();
        with_mcp_stopped(&agent, || Ok(())).unwrap();
        assert!(agent.status().unwrap().endpoint.is_none());
        let refused: Result<(), String> = with_mcp_stopped(&agent, || Err("refused".into()));
        assert!(refused.is_err());
        assert!(agent.status().unwrap().endpoint.is_none(), "MCP was off before the restore and stays off");
        let _ = std::fs::remove_dir_all(root);
    }
}
#[tauri::command]
pub async fn purge_waveform_cache(
    state: tauri::State<'_, AppState>,
    confirmed: bool,
) -> Result<u64, String> {
    if !confirmed {
        return Err("Confirm clearing waveform cache".into());
    }
    let waveforms = state.waveforms.clone();
    tauri::async_runtime::spawn_blocking(move || waveforms.purge_cache().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn support_report(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let catalog = state.catalog.clone();
    let media = state.tools.is_some();
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .add_filter("Redacted diagnostics", &["json"])
            .set_file_name("creativeshelf-support.json")
            .blocking_save_file();
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|e| e.to_string())?;
        let text = catalog
            .lock()
            .map_err(|e| e.to_string())?
            .support_report(media)
            .map_err(|e| e.to_string())?;
        soundshelf_core::portable::write_document(&path, &text).map_err(|e| e.to_string())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}
