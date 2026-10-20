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
        agent.stop();
        let rollback = data.join(format!("restore-rollback-{}.sqlite", uuid::Uuid::new_v4()));
        exports
            .while_idle(|| {
                catalog
                    .lock()
                    .map_err(|_| soundshelf_core::invalid("Catalog lock unavailable"))?
                    .restore_previewed(&path, &rollback, &summary)
            })
            .map_err(|e| e.to_string())?;
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
