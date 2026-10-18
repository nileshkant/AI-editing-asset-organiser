//! Thin IPC adapter for SS-044 previews. Authorization, versions and the cache
//! live in core; this exposes asset-ID requests, never a file path or URL scheme.
use crate::service::AppState;
use soundshelf_core::{
    catalog::Catalog,
    image_preview::{
        PreviewCache, PreviewCacheStatus, PreviewError as E, PreviewRequest, PreviewService,
        PreviewTier, DEFAULT_QUOTA,
    },
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub struct ImagePreviews {
    cache: Arc<PreviewCache>,
    /// Present only when the trusted image worker is enabled for this build.
    service: Option<Arc<PreviewService>>,
}
impl ImagePreviews {
    pub fn new(data: &Path, catalog: &Arc<Mutex<Catalog>>, worker: Option<PathBuf>) -> Self {
        let quota = catalog
            .lock()
            .ok()
            .and_then(|c| c.image_preview_quota().ok())
            .unwrap_or(DEFAULT_QUOTA);
        let cache = Arc::new(PreviewCache::new(
            data.join("cache").join("image-previews"),
            quota,
        ));
        let service = worker.map(|w| Arc::new(PreviewService::new(catalog.clone(), cache.clone(), w)));
        let previews = Self { cache, service };
        previews.refresh(catalog);
        previews
    }
    /// Reload the quota and drop previews of content no record references, in the
    /// background. Used at startup and after a database restore. The quota is
    /// applied under the catalog lock, serializing it with a user quota change.
    pub fn refresh(&self, catalog: &Arc<Mutex<Catalog>>) {
        let cache = self.cache.clone();
        let catalog = catalog.clone();
        std::thread::spawn(move || {
            cache.status(); // index the cache directory before taking the catalog lock
            let digests = catalog.lock().ok().and_then(|c| {
                let _ = cache.set_quota(c.image_preview_quota().ok()?);
                c.image_content_digests().ok()
            });
            if let Some(digests) = digests {
                cache.prune(&digests);
            }
        });
    }
    fn status(&self) -> PreviewCacheStatus {
        match &self.service {
            Some(s) => s.status(),
            None => self.cache.status(),
        }
    }
}
#[tauri::command]
pub async fn image_preview(
    state: tauri::State<'_, AppState>,
    request_id: String,
    asset_id: String,
    version: String,
    tier: PreviewTier,
    tile: Option<[u32; 2]>,
) -> Result<tauri::ipc::Response, E> {
    let service = state
        .image_previews
        .service
        .clone()
        .ok_or(E::WorkerUnavailable)?;
    let request = PreviewRequest {
        request_id,
        asset_id,
        version,
        tier,
        tile,
    };
    tauri::async_runtime::spawn_blocking(move || {
        service
            .preview(&request)
            .map(|p| tauri::ipc::Response::new(p.envelope()))
    })
    .await
    .map_err(|_| E::Internal)?
}
#[tauri::command]
pub fn cancel_image_preview(state: tauri::State<AppState>, request_id: String) -> bool {
    state
        .image_previews
        .service
        .as_ref()
        .is_some_and(|s| s.cancel(&request_id))
}
#[tauri::command]
pub async fn image_preview_cache_status(
    state: tauri::State<'_, AppState>,
) -> Result<PreviewCacheStatus, E> {
    let previews = state.image_previews.clone();
    tauri::async_runtime::spawn_blocking(move || previews.status())
        .await
        .map_err(|_| E::Internal)
}
#[tauri::command]
pub async fn set_image_preview_cache_quota(
    state: tauri::State<'_, AppState>,
    quota_bytes: u64,
) -> Result<PreviewCacheStatus, E> {
    let previews = state.image_previews.clone();
    let catalog = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || {
        previews.cache.status(); // index outside the catalog lock
        {
            let c = catalog.lock().map_err(|_| E::Internal)?;
            c.set_image_preview_quota(quota_bytes)
                .map_err(|_| E::InvalidArgument)?;
            previews.cache.set_quota(quota_bytes)?;
        }
        Ok(previews.status())
    })
    .await
    .map_err(|_| E::Internal)?
}
/// Removes generated previews only; analysis, annotations and originals stay.
#[tauri::command]
pub async fn clear_image_preview_cache(
    state: tauri::State<'_, AppState>,
    confirmed: bool,
) -> Result<u64, E> {
    if !confirmed {
        return Err(E::InvalidArgument);
    }
    let previews = state.image_previews.clone();
    tauri::async_runtime::spawn_blocking(move || previews.cache.clear())
        .await
        .map_err(|_| E::Internal)
}

#[cfg(all(test, feature = "image-development"))]
mod tests {
    use super::*;
    use soundshelf_core::image_analysis::{ImageAnalysisRequest, ImageSelection};
    use std::sync::atomic::AtomicBool;
    #[test]
    fn app_state_serves_versioned_previews_from_the_owned_data_cache() {
        let dir = std::env::temp_dir().join(format!("ss044-native-{}", uuid::Uuid::new_v4()));
        let images = dir.join("resources").join("images");
        std::fs::create_dir_all(&images).unwrap();
        let name = if cfg!(windows) {
            "creativeshelf-image-worker.exe"
        } else {
            "creativeshelf-image-worker"
        };
        let exe = std::env::current_exe().unwrap();
        let binary = exe.parent().unwrap().parent().unwrap().join(name);
        assert!(binary.is_file(), "build the workspace image helper first");
        std::fs::copy(binary, images.join(name)).unwrap();
        let state = AppState::new(dir.join("app-data"), dir.join("resources")).unwrap();
        let service = state.image_previews.service.clone().expect("trusted worker enabled");
        let root = dir.join("media");
        std::fs::create_dir(&root).unwrap();
        let bytes = include_bytes!("../../fixtures/images/solid-lossy.webp");
        std::fs::write(root.join("native.webp"), bytes).unwrap();
        let (id, hash) = {
            let mut c = state.catalog.lock().unwrap();
            let source = c.add_source(&root).unwrap();
            let hash = ImageSelection::capture(&c, &source.id, "native.webp")
                .unwrap()
                .hash(&AtomicBool::new(false))
                .unwrap();
            let id = c.register_image(&source, "native.webp", Some(&hash)).unwrap().header.id;
            let worker = state.image_imports.helper.clone().unwrap();
            ImageAnalysisRequest::capture(&c, &id)
                .unwrap()
                .analyze(&worker, Arc::new(AtomicBool::new(false)))
                .unwrap()
                .reverify(&AtomicBool::new(false))
                .unwrap()
                .publish(&mut c)
                .unwrap();
            (id, hash)
        };
        let preview = service
            .preview(&PreviewRequest {
                request_id: "native-1".into(),
                asset_id: id.clone(),
                version: hash,
                tier: PreviewTier::Small,
                tile: None,
            })
            .unwrap();
        let envelope = preview.envelope();
        assert_eq!(&envelope[..4], b"CSPV");
        let n = u32::from_le_bytes(envelope[4..8].try_into().unwrap()) as usize;
        let meta = std::str::from_utf8(&envelope[8..8 + n]).unwrap();
        assert!(meta.contains(&format!("\"asset_id\":\"{id}\"")), "{meta}");
        assert_eq!(&envelope[8 + n..], preview.bytes.as_slice());
        assert!(dir.join("app-data/cache/image-previews/tmp").is_dir());
        let status = format!("{:?}", state.image_previews.status());
        assert!(!status.contains(root.to_str().unwrap()) && !status.contains(&id));
        assert_eq!(std::fs::read(root.join("native.webp")).unwrap(), bytes);
        state.shutdown();
        drop(state);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
