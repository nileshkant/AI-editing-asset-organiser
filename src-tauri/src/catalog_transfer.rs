use crate::service::AppState;
use tauri_plugin_dialog::DialogExt;

// Catalog file access is bound to native pick/save dialogs, never a renderer path.
#[tauri::command]
pub async fn catalog_export(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    format: String,
) -> Result<Option<String>, String> {
    if !["json", "md"].contains(&format.as_str()) {
        return Err("Choose JSON or Markdown".into());
    }
    let catalog = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .add_filter("Catalog", &[&format])
            .set_file_name(format!("soundshelf-catalog.{}", format))
            .blocking_save_file();
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|e| e.to_string())?;
        let snapshot = catalog
            .lock()
            .map_err(|e| e.to_string())?
            .export_portable()
            .map_err(|e| e.to_string())?;
        let text = if format == "json" {
            snapshot.json().map_err(|e| e.to_string())?
        } else {
            snapshot.markdown()
        };
        soundshelf_core::portable::write_document(&path, &text).map_err(|e| e.to_string())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[derive(serde::Serialize)]
pub struct CatalogPreview {
    token: String,
    preview: soundshelf_core::portable::Preview,
}
#[tauri::command]
pub async fn catalog_preview(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    legacy: bool,
) -> Result<Option<CatalogPreview>, String> {
    let imports = state.catalog_imports.clone();
    let grants = state.catalog_roots.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .add_filter("JSON catalog", &["json"])
            .blocking_pick_file();
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|e| e.to_string())?;
        let text = soundshelf_core::portable::read_document(&path).map_err(|e| e.to_string())?;
        let preview =
            soundshelf_core::portable::preview(&text, legacy).map_err(|e| e.to_string())?;
        let token = uuid::Uuid::new_v4().to_string();
        let mut pending = imports.lock().map_err(|e| e.to_string())?;
        // Only one active preview, with immutable content so disk edits cannot change approval.
        pending.clear();
        pending.insert(token.clone(), (text, legacy));
        grants.lock().map_err(|e| e.to_string())?.clear();
        Ok(Some(CatalogPreview { token, preview }))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn catalog_map_root(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    token: String,
    source_id: String,
) -> Result<Option<String>, String> {
    {
        let pending = state.catalog_imports.lock().map_err(|e| e.to_string())?;
        let (text, legacy) = pending.get(&token).ok_or("Catalog preview expired")?;
        let preview =
            soundshelf_core::portable::preview(text, *legacy).map_err(|e| e.to_string())?;
        if !preview.sources.iter().any(|s| s.id == source_id) {
            return Err("Unknown source".into());
        }
    }
    let grants = state.catalog_roots.clone();
    let imports = state.catalog_imports.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app.dialog().file().blocking_pick_folder();
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked
            .into_path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let pending = imports.lock().map_err(|e| e.to_string())?;
        if !pending.contains_key(&token) {
            return Err("Catalog preview expired".into());
        }
        grants
            .lock()
            .map_err(|e| e.to_string())?
            .insert((token, source_id), path.clone());
        Ok(Some(path))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn catalog_cancel_preview(state: tauri::State<AppState>, token: String) -> Result<(), String> {
    state
        .catalog_imports
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&token);
    state
        .catalog_roots
        .lock()
        .map_err(|e| e.to_string())?
        .retain(|(t, _), _| t != &token);
    Ok(())
}
#[derive(serde::Serialize)]
pub struct CatalogImported {
    report: soundshelf_core::portable::ImportReport,
    warnings: Vec<String>,
}
#[tauri::command]
pub async fn catalog_import(
    state: tauri::State<'_, AppState>,
    token: String,
    confirmed: bool,
) -> Result<CatalogImported, String> {
    if !confirmed {
        return Err("Confirm metadata import after reviewing root mappings".into());
    }
    let (text, legacy) = state
        .catalog_imports
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&token)
        .ok_or("Catalog preview expired")?;
    let roots = {
        let mut grants = state.catalog_roots.lock().map_err(|e| e.to_string())?;
        let roots = grants
            .iter()
            .filter(|((t, _), _)| t == &token)
            .map(|((_, id), path)| (id.clone(), path.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        grants.retain(|(t, _), _| t != &token);
        roots
    };
    let catalog = state.catalog.clone();
    let offline = state.data_directory.join("unmapped-sources");
    let (report, source_ids) = tauri::async_runtime::spawn_blocking(move || {
        let data = soundshelf_core::portable::prepare_import(&text, &roots, legacy)
            .map_err(|e| e.to_string())?;
        let report = catalog
            .lock()
            .map_err(|e| e.to_string())?
            .import_portable(&data, &roots, &offline)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>((
            report,
            if legacy {
                data.sources.into_iter().map(|s| s.id).collect::<Vec<_>>()
            } else {
                vec![]
            },
        ))
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut warnings = vec![];
    for id in source_ids {
        let source = state
            .catalog
            .lock()
            .map_err(|e| e.to_string())?
            .source(&id)
            .map_err(|e| e.to_string())?;
        if let Err(e) = state.enqueue(source) {
            warnings.push(format!(
                "Metadata imported; reanalysis could not be queued: {e}. Use Rescan in Settings."
            ));
        }
    }
    Ok(CatalogImported { report, warnings })
}
#[tauri::command]
pub async fn catalog_legacy_evidence(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<soundshelf_core::portable::LegacyEvidence>, String> {
    let catalog = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        catalog
            .lock()
            .map_err(|e| e.to_string())?
            .legacy_evidence()
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
