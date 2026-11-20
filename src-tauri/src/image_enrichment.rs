//! Thin IPC adapter for SS-052 consented image analysis. Consent, ceilings,
//! billing semantics and fences live in soundshelf_core::image_enrichment; this
//! module schedules one background worker and exposes asset-ID commands. The
//! worker exists only when the trusted image worker is enabled for the build.
use crate::service::AppState;
use soundshelf_core::{
    catalog::Catalog,
    image_enrichment::{
        disclosure, Consent, ConsentRequest, Disclosure, EnrichError as E, Estimate, EstimateRequest, ItemView, JobView,
        PreviewPixels, ProviderDispatcher, Runner, StartRequest, Step,
    },
    image_preview::PreviewService,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct Enrichment {
    pub enabled: bool,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Enrichment {
    pub fn new(
        catalog: Arc<Mutex<Catalog>>,
        previews: Option<Arc<PreviewService>>,
        providers: Arc<soundshelf_core::ai_provider::Providers>,
        imports: Arc<crate::image_import::ImageImports>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let Some(previews) = previews else {
            return Self { enabled: false, stop, worker: Mutex::new(None) };
        };
        let busy_catalog = catalog.clone();
        let runner = Runner {
            catalog: catalog.clone(),
            pixels: Arc::new(PreviewPixels { previews }),
            dispatcher: Arc::new(ProviderDispatcher { providers }),
            // Imports and interactive native work keep the machine to themselves.
            busy: Arc::new(move || {
                imports.native_tasks.load(Ordering::Relaxed) > 0
                    || busy_catalog.lock().ok().and_then(|c| c.image_imports_active().ok()).unwrap_or(false)
            }),
            owner: uuid::Uuid::new_v4().to_string(),
        };
        let flag = stop.clone();
        let handle = thread::spawn(move || {
            // A request in flight when the app last stopped has an unknown outcome.
            if let Ok(mut c) = catalog.lock() {
                let _ = c.recover_enrichment(true);
            }
            let mut recovered = std::time::Instant::now();
            while !flag.load(Ordering::Relaxed) {
                if recovered.elapsed() >= Duration::from_secs(60) {
                    if let Ok(mut c) = catalog.lock() {
                        let _ = c.recover_enrichment(false);
                    }
                    recovered = std::time::Instant::now();
                }
                let pause = match runner.step() {
                    Ok(Step::Worked) => Duration::from_millis(20),
                    Ok(Step::Yielded) => Duration::from_millis(1000),
                    Ok(Step::Idle) | Err(_) => Duration::from_millis(1500),
                };
                let mut waited = Duration::ZERO;
                while waited < pause && !flag.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(20));
                    waited += Duration::from_millis(20);
                }
            }
        });
        Self { enabled: true, stop, worker: Mutex::new(Some(handle)) }
    }
    /// Queues a finished import's images when its source opted in to that.
    pub fn after_import(catalog: &Arc<Mutex<Catalog>>, import_job_id: &str) {
        if let Ok(mut c) = catalog.lock() {
            let _ = c.enqueue_new_from_import(import_job_id);
        }
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut handle) = self.worker.lock() {
            if let Some(h) = handle.take() {
                let _ = h.join();
            }
        }
    }
}
impl Drop for Enrichment {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn with<T: Send + 'static>(state: tauri::State<'_, AppState>, f: impl FnOnce(&mut Catalog) -> Result<T, E> + Send + 'static) -> Result<T, E> {
    if !state.image_enrichment.enabled {
        return Err(E::Unavailable);
    }
    let c = state.catalog.clone();
    tauri::async_runtime::spawn_blocking(move || f(&mut *c.lock().map_err(|_| E::Internal)?)).await.map_err(|_| E::Internal)?
}

#[tauri::command]
pub fn image_enrichment_disclosure() -> Disclosure {
    disclosure()
}
#[tauri::command]
pub async fn image_enrichment_consents(state: tauri::State<'_, AppState>) -> Result<Vec<Consent>, E> {
    with(state, |c| c.enrichment_consents()).await
}
#[tauri::command]
pub async fn image_enrichment_grant(state: tauri::State<'_, AppState>, request: ConsentRequest) -> Result<Consent, E> {
    with(state, move |c| c.grant_enrichment_consent(&request)).await
}
#[tauri::command]
pub async fn image_enrichment_revoke(state: tauri::State<'_, AppState>, source_id: String) -> Result<(), E> {
    with(state, move |c| c.revoke_enrichment_consent(&source_id)).await
}
#[tauri::command]
pub async fn image_enrichment_estimate(state: tauri::State<'_, AppState>, request: EstimateRequest) -> Result<Estimate, E> {
    with(state, move |c| c.estimate_enrichment(&request)).await
}
#[tauri::command]
pub async fn image_enrichment_start(state: tauri::State<'_, AppState>, request: StartRequest) -> Result<JobView, E> {
    with(state, move |c| c.start_enrichment(&request)).await
}
#[tauri::command]
pub async fn image_enrichment_jobs(state: tauri::State<'_, AppState>) -> Result<Vec<JobView>, E> {
    with(state, |c| c.enrichment_jobs()).await
}
#[tauri::command]
pub async fn image_enrichment_items(state: tauri::State<'_, AppState>, job_id: String, offset: i64, limit: i64) -> Result<Vec<ItemView>, E> {
    with(state, move |c| c.enrichment_items(&job_id, offset, limit)).await
}
#[tauri::command]
pub async fn image_enrichment_cancel(state: tauri::State<'_, AppState>, job_id: String) -> Result<(), E> {
    with(state, move |c| c.cancel_enrichment_job(&job_id)).await
}
#[tauri::command]
pub async fn image_enrichment_resume(state: tauri::State<'_, AppState>, job_id: String) -> Result<(), E> {
    with(state, move |c| c.resume_enrichment_job(&job_id)).await
}
#[tauri::command]
pub async fn image_enrichment_retry(state: tauri::State<'_, AppState>, job_id: String, asset_id: String, accept_possible_charge: bool) -> Result<(), E> {
    with(state, move |c| c.retry_enrichment_item(&job_id, &asset_id, accept_possible_charge)).await
}
