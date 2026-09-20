#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use tauri::Manager;

#[derive(serde::Serialize)]
struct AppInfo { version: &'static str, data_directory: String, desktop: bool }

#[tauri::command]
fn app_info(app: tauri::AppHandle) -> Result<AppInfo, String> {
    let path = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(AppInfo { version: env!("CARGO_PKG_VERSION"), data_directory: path.to_string_lossy().into_owned(), desktop: true })
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("SoundShelf could not start");
}
