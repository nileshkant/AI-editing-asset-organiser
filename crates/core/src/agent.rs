//! Scoped agent operations over the same catalog/export services used by the UI.
use crate::{
    catalog::{Catalog, ClipRecipe, Sound},
    export::{DestinationGrant, ExportOptions, ExportService},
    media::MediaTools,
    search::SearchQuery,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Access {
    pub source_ids: Vec<String>,
    pub read: bool,
    pub edit: bool,
    pub export: bool,
    pub paths: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Failure {
    pub code: &'static str,
    pub message: &'static str,
}
type Result<T> = std::result::Result<T, Failure>;
fn failure(code: &'static str) -> Failure {
    Failure {
        code,
        message: match code {
            "PERMISSION_DENIED" => "Client permission does not allow this operation",
            "ASSET_CHANGED" => "Sound version or clip revision changed",
            "ASSET_MISSING" => "Sound is missing",
            "SOURCE_OFFLINE" => "Source is offline",
            "NOT_READY" => "Sound is not ready",
            "RATE_LIMITED" => "Agent operation limit reached",
            "MODEL_UNAVAILABLE" => "Media tools are unavailable",
            "JOB_CANCELLED" => "Export was cancelled",
            "EXPORT_COLLISION" => "Export destination already exists",
            "INVALID_ARGUMENT" => "Arguments are invalid",
            "INVALID_RANGE" => "Clip range or rendering options are invalid",
            "DISK_FULL" => "Export destination has no free space",
            _ => "Operation failed; check the desktop application",
        },
    }
}
fn mapped(error: crate::Error) -> Failure {
    if matches!(&error,crate::Error::Io(e) if e.kind()==std::io::ErrorKind::NotFound) {
        return failure("ASSET_MISSING");
    }
    if matches!(&error,crate::Error::Io(e) if e.kind()==std::io::ErrorKind::AlreadyExists) {
        return failure("EXPORT_COLLISION");
    }
    if matches!(&error,crate::Error::Io(e) if e.kind()==std::io::ErrorKind::StorageFull) {
        return failure("DISK_FULL");
    }
    let text = error.to_string();
    failure(if text.contains("outside") || text.contains("escape") {
        "PERMISSION_DENIED"
    } else if text.contains("version")
        || text.contains("stale")
        || text.contains("revision")
        || text.contains("changed")
    {
        "ASSET_CHANGED"
    } else if text.contains("offline") {
        "SOURCE_OFFLINE"
    } else if text.contains("missing") || text.contains("not found") {
        "ASSET_MISSING"
    } else if text.contains("not ready") {
        "NOT_READY"
    } else if text.contains("cancelled") {
        "JOB_CANCELLED"
    } else if text.contains("already exists") {
        "EXPORT_COLLISION"
    } else if text.contains("frame")
        || text.contains("range")
        || text.contains("fade")
        || text.contains("sample rate")
        || text.contains("gain")
        || text.contains("channel policy")
    {
        "INVALID_RANGE"
    } else if text.contains("already running") {
        "RATE_LIMITED"
    } else {
        "INVALID_ARGUMENT"
    })
}
fn args<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| failure("INVALID_ARGUMENT"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoundArgs {
    id: String,
    version: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Annotation {
    id: String,
    version: String,
    tags: Vec<String>,
    comment: String,
    favorite: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    sound_id: String,
    name: String,
    recipe: ClipRecipe,
    idempotency_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    id: String,
    name: String,
    recipe: ClipRecipe,
    revision: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    #[serde(default)]
    offset: usize,
    #[serde(default = "page_limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipList {
    id: String,
    version: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "page_limit")]
    limit: usize,
}
fn page_limit() -> usize {
    50
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Export {
    clip_id: String,
    revision: u32,
    destination_id: String,
    options: ExportOptions,
    idempotency_key: String,
}
#[derive(Clone, Serialize)]
pub struct AgentJob {
    pub id: String,
    pub state: String,
    pub clip_id: String,
    pub result: Option<Value>,
    pub error: Option<Failure>,
}
struct OwnedJob {
    owner: String,
    destination: String,
    cancel: Arc<AtomicBool>,
    snapshot: AgentJob,
}
struct Replay {
    hash: String,
    result: Value,
}
#[derive(Default)]
struct Operations {
    destinations: HashMap<String, String>,
    jobs: HashMap<String, OwnedJob>,
    replays: HashMap<(String, String, String), Replay>,
    workers: Vec<JoinHandle<()>>,
}
pub struct AgentLibrary {
    pub catalog: Arc<Mutex<Catalog>>,
    pub exports: Arc<ExportService>,
    tools: Option<MediaTools>,
    operations: Mutex<Operations>,
}
impl AgentLibrary {
    pub fn new(
        catalog: Arc<Mutex<Catalog>>,
        exports: Arc<ExportService>,
        tools: Option<MediaTools>,
    ) -> Self {
        Self {
            catalog,
            exports,
            tools,
            operations: Mutex::new(Operations::default()),
        }
    }
    pub fn validate_access(&self, access: &Access) -> std::result::Result<(), String> {
        if access.source_ids.len() > 128
            || (!access.read
                && (access.edit || access.export || access.paths || !access.source_ids.is_empty()))
        {
            return Err("Choose read access and at most 128 explicit sources".into());
        }
        let catalog = self.catalog.lock().map_err(|_| "Catalog unavailable")?;
        for id in &access.source_ids {
            catalog
                .source(id)
                .map_err(|_| "Choose an existing source")?;
        }
        Ok(())
    }
    fn catalog(&self) -> Result<std::sync::MutexGuard<'_, Catalog>> {
        self.catalog.lock().map_err(|_| failure("INTERNAL_ERROR"))
    }
    fn sound(c: &Catalog, access: &Access, id: &str, version: Option<&str>) -> Result<Sound> {
        // A denied/nonexistent ID has the same public failure, before returning its state.
        let sound = c.sound(id).map_err(|_| failure("PERMISSION_DENIED"))?;
        if !access.read || !access.source_ids.contains(&sound.source_id) {
            return Err(failure("PERMISSION_DENIED"));
        }
        let source = c.source(&sound.source_id).map_err(mapped)?;
        c.validate_source_entry(&source.id, &sound.relative_path)
            .map_err(|_| failure("PERMISSION_DENIED"))?;
        if !source.available {
            return Err(failure("SOURCE_OFFLINE"));
        }
        if sound.status == "missing" {
            return Err(failure("ASSET_MISSING"));
        }
        if sound.status != "ready" || sound.profile.is_none() {
            return Err(failure("NOT_READY"));
        }
        if version.is_some_and(|v| v != sound.content_hash) {
            return Err(failure("ASSET_CHANGED"));
        }
        Ok(sound)
    }
    pub fn allows(access: &Access, method: &str) -> bool {
        match method {
            "sounds.annotate" | "clips.create" | "clips.update" => access.read && access.edit,
            "clips.export" | "jobs.cancel" => access.read && access.export,
            "sounds.resolve" => access.read && access.paths,
            _ => access.read,
        }
    }
    fn query(value: Value, access: &Access) -> Result<SearchQuery> {
        let object = value
            .as_object()
            .ok_or_else(|| failure("INVALID_ARGUMENT"))?;
        const FIELDS: &[&str] = &[
            "text",
            "source_ids",
            "tags",
            "favorites_only",
            "min_duration",
            "max_duration",
            "offset",
            "limit",
        ];
        if object.keys().any(|k| !FIELDS.contains(&k.as_str())) {
            return Err(failure("INVALID_ARGUMENT"));
        }
        let mut query: SearchQuery = args(value)?;
        if query.offset > 1_000_000
            || query.limit.unwrap_or(50) == 0
            || query.limit.unwrap_or(50) > 100
        {
            return Err(failure("INVALID_ARGUMENT"));
        }
        if query
            .source_ids
            .iter()
            .any(|id| !access.source_ids.contains(id))
        {
            return Err(failure("PERMISSION_DENIED"));
        }
        query.limit = Some(query.limit.unwrap_or(50));
        Ok(query)
    }
    fn search(
        c: &Catalog,
        access: &Access,
        query: &SearchQuery,
    ) -> Result<crate::search::SearchResults> {
        let sounds = c
            .scoped_sounds(&access.source_ids)
            .map_err(mapped)?
            .into_iter()
            .filter(|s| {
                access.source_ids.contains(&s.source_id)
                    && c.validate_source_entry(&s.source_id, &s.relative_path)
                        .is_ok()
            })
            .collect();
        let online = c
            .source_headers()
            .map_err(mapped)?
            .into_iter()
            .filter(|s| s.available && access.source_ids.contains(&s.id))
            .map(|s| s.id)
            .collect::<Vec<_>>();
        crate::search::search(sounds, query, &online).map_err(mapped)
    }
    pub fn execute(
        self: &Arc<Self>,
        owner: &str,
        access: &Access,
        method: &str,
        value: Value,
    ) -> Result<Value> {
        if !Self::allows(access, method) {
            return Err(failure("PERMISSION_DENIED"));
        }
        if method == "clips.export" {
            return self.begin_export(owner, access, value);
        }
        if method.starts_with("jobs.") {
            return self.jobs(owner, access, method, value);
        }
        let mut c = self.catalog()?;
        match method {
            "library.status" => {
                if !value.as_object().is_some_and(|o| o.is_empty()) {
                    return Err(failure("INVALID_ARGUMENT"));
                }
                let query = SearchQuery {
                    limit: Some(1),
                    ..Default::default()
                };
                let result = Self::search(&c, access, &query)?;
                Ok(json!({"ready_available":result.total,"source_count":access.source_ids.len()}))
            }
            "sources.list" => {
                let page: List = args(value)?;
                validate_page(&page)?;
                let sources=c.source_headers().map_err(mapped)?.into_iter().filter(|s|access.source_ids.contains(&s.id)).skip(page.offset).take(page.limit).map(|s|json!({"id":s.id,"name":s.name,"scope":s.scope,"available":s.available})).collect::<Vec<_>>();
                Ok(json!({"items":sources}))
            }
            "sounds.search" | "sounds.facets" => {
                let query = Self::query(value, access)?;
                let mut result = Self::search(&c, access, &query)?;
                for sound in &mut result.items {
                    if let Some(p) = &mut sound.profile {
                        p.waveform.clear();
                    }
                }
                let next = (query.offset + result.items.len() < result.total)
                    .then_some(query.offset + result.items.len());
                if method == "sounds.facets" {
                    Ok(json!({"facets":result.facets,"total":result.total}))
                } else {
                    let available = result
                        .items
                        .into_iter()
                        .map(|s| json!({"sound":s,"availability":"available"}))
                        .collect::<Vec<_>>();
                    Ok(
                        json!({"items":available,"total":result.total,"interpretation":result.interpretation,"facets":result.facets,"next_offset":next}),
                    )
                }
            }
            "sounds.get" | "sounds.resolve" => {
                let a: SoundArgs = args(value)?;
                let mut sound = Self::sound(&c, access, &a.id, Some(&a.version))?;
                if method == "sounds.resolve" {
                    let path = c.resolve(&a.id).map_err(mapped)?;
                    return Ok(json!({"id":a.id,"version":a.version,"path":path}));
                }
                if let Some(p) = &mut sound.profile {
                    p.waveform.clear();
                }
                Ok(json!({"sound":sound,"availability":"available"}))
            }
            "sounds.annotate" => {
                let a: Annotation = args(value)?;
                Self::sound(&c, access, &a.id, Some(&a.version))?;
                c.annotate(&a.id, &a.tags, &a.comment, a.favorite)
                    .map_err(mapped)?;
                Ok(json!({"id":a.id,"updated":true}))
            }
            "clips.create" => {
                let a: Create = args(value.clone())?;
                Self::sound(&c, access, &a.sound_id, Some(&a.recipe.asset_version_id))?;
                if a.recipe.asset_id != a.sound_id {
                    return Err(failure("INVALID_ARGUMENT"));
                }
                let mut operations = self
                    .operations
                    .lock()
                    .map_err(|_| failure("INTERNAL_ERROR"))?;
                if let Some(result) =
                    replayed(&operations, owner, method, &a.idempotency_key, &value)?
                {
                    return Ok(result);
                }
                let clip = c
                    .create_clip(&a.sound_id, &a.name, &a.recipe)
                    .map_err(mapped)?;
                let result = json!(clip);
                remember(
                    &mut operations,
                    owner,
                    method,
                    &a.idempotency_key,
                    &value,
                    result.clone(),
                );
                Ok(result)
            }
            "clips.update" => {
                let a: Update = args(value)?;
                let clip = c
                    .get_clip(&a.id)
                    .map_err(|_| failure("PERMISSION_DENIED"))?;
                let sound =
                    Self::sound(&c, access, &clip.sound_id, Some(&a.recipe.asset_version_id))?;
                if sound.profile.as_ref().map(|p| p.sample_rate)
                    != Some(a.recipe.source_sample_rate_hz)
                {
                    return Err(failure("INVALID_RANGE"));
                }
                if a.recipe.asset_id != clip.sound_id {
                    return Err(failure("INVALID_ARGUMENT"));
                }
                Ok(json!(c
                    .update_clip(&a.id, &a.name, &a.recipe, a.revision)
                    .map_err(mapped)?))
            }
            "clips.get" => {
                let a: Id = args(value)?;
                let clip = c
                    .get_clip(&a.id)
                    .map_err(|_| failure("PERMISSION_DENIED"))?;
                Self::sound(&c, access, &clip.sound_id, None)?;
                Ok(json!(clip))
            }
            "clips.list" => {
                let a: ClipList = args(value)?;
                validate_page(&List {
                    offset: a.offset,
                    limit: a.limit,
                })?;
                Self::sound(&c, access, &a.id, Some(&a.version))?;
                let clips = c.list_clips_for_sound(&a.id).map_err(mapped)?;
                let total = clips.len();
                Ok(
                    json!({"items":clips.into_iter().skip(a.offset).take(a.limit).collect::<Vec<_>>(),"total":total,"next_offset":(a.offset+a.limit<total).then_some(a.offset+a.limit)}),
                )
            }
            "catalog.export" => {
                let query = Self::query(value, access)?;
                let selected = Self::search(&c, access, &query)?;
                let ids = selected
                    .items
                    .iter()
                    .map(|s| s.id.clone())
                    .collect::<Vec<_>>();
                let source_ids = selected
                    .items
                    .iter()
                    .map(|s| s.source_id.clone())
                    .collect::<std::collections::HashSet<_>>();
                let mut sources = vec![];
                for id in source_ids {
                    let source = c.source(&id).map_err(mapped)?;
                    sources.push(crate::portable::PortableSource {
                        id: source.id,
                        name: source.name,
                        scope: source.scope,
                        files: source
                            .files
                            .into_iter()
                            .filter(|p| {
                                selected.items.iter().any(|s| {
                                    s.source_id == id && s.relative_path == p.relative_path
                                })
                            })
                            .map(|p| p.relative_path)
                            .collect(),
                    });
                }
                sources.sort_by(|a, b| a.id.cmp(&b.id));
                let mut clips = vec![];
                for sound in &selected.items {
                    clips.extend(c.list_clips_for_sound(&sound.id).map_err(mapped)?);
                    if clips.len() > 1000 {
                        return Err(failure("RATE_LIMITED"));
                    }
                }
                let document = crate::portable::PortableCatalog {
                    schema: crate::portable::PORTABLE_SCHEMA.into(),
                    sources,
                    sounds: ids
                        .iter()
                        .map(|id| c.sound(id).map_err(mapped))
                        .collect::<Result<Vec<_>>>()?,
                    clips,
                    saved_searches: vec![],
                    legacy_evidence: c
                        .legacy_evidence()
                        .map_err(mapped)?
                        .into_iter()
                        .filter(|e| ids.contains(&e.sound_id))
                        .collect(),
                };
                document.validate().map_err(mapped)?;
                let root_map=document.sources.iter().map(|s|json!({"source_id":s.id,"root":if access.paths {c.source(&s.id).ok().map(|s|s.root)}else{None}})).collect::<Vec<_>>();
                Ok(
                    json!({"catalog":document,"root_map":root_map,"total":selected.total,"next_offset":(query.offset+ids.len()<selected.total).then_some(query.offset+ids.len())}),
                )
            }
            _ => Err(failure("INVALID_ARGUMENT")),
        }
    }
    pub fn approve_destination(
        &self,
        owner: &str,
        path: &Path,
        options: &ExportOptions,
    ) -> std::result::Result<DestinationGrant, String> {
        let grant = self
            .exports
            .grant(path, options.format)
            .map_err(|_| "Destination could not be approved")?;
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "Agent operations unavailable")?;
        operations.destinations.retain(|_, o| o != owner);
        operations
            .destinations
            .insert(grant.id.clone(), owner.into());
        Ok(grant)
    }
    fn begin_export(self: &Arc<Self>, owner: &str, access: &Access, value: Value) -> Result<Value> {
        let a: Export = args(value.clone())?;
        {
            let c = self.catalog()?;
            let clip = c
                .get_clip(&a.clip_id)
                .map_err(|_| failure("PERMISSION_DENIED"))?;
            Self::sound(
                &c,
                access,
                &clip.sound_id,
                Some(&clip.recipe.asset_version_id),
            )?;
            if clip.is_stale || clip.revision != a.revision {
                return Err(failure("ASSET_CHANGED"));
            }
        }
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| failure("INTERNAL_ERROR"))?;
        if let Some(result) = replayed(
            &operations,
            owner,
            "clips.export",
            &a.idempotency_key,
            &value,
        )? {
            return Ok(result);
        }
        if operations
            .destinations
            .get(&a.destination_id)
            .map(String::as_str)
            != Some(owner)
        {
            return Err(failure("PERMISSION_DENIED"));
        }
        if operations.jobs.len() >= 200
            || operations
                .jobs
                .values()
                .filter(|j| j.snapshot.state == "running")
                .count()
                >= 2
        {
            return Err(failure("RATE_LIMITED"));
        }
        let tools = self
            .tools
            .clone()
            .ok_or_else(|| failure("MODEL_UNAVAILABLE"))?;
        let id = uuid::Uuid::new_v4().to_string();
        let result = json!({"job_id":id});
        let snapshot = AgentJob {
            id: id.clone(),
            state: "running".into(),
            clip_id: a.clip_id.clone(),
            result: None,
            error: None,
        };
        let library = self.clone();
        let job_id = id.clone();
        let destination = a.destination_id.clone();
        let key = a.idempotency_key.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        operations.workers.retain(|w| !w.is_finished());
        let worker = std::thread::Builder::new()
            .name("soundshelf-agent-export".into())
            .spawn(move || {
                let exported = if worker_cancel.load(Ordering::Acquire) {
                    Err(failure("JOB_CANCELLED"))
                } else {
                    library
                        .exports
                        .export(
                            &library.catalog,
                            &tools,
                            &a.destination_id,
                            &a.clip_id,
                            a.revision,
                            a.options,
                        )
                        .map_err(mapped)
                };
                let exported = exported.map_err(|e| {
                    if worker_cancel.load(Ordering::Acquire) {
                        failure("JOB_CANCELLED")
                    } else {
                        e
                    }
                });
                if let Ok(mut operations) = library.operations.lock() {
                    if let Some(job) = operations.jobs.get_mut(&job_id) {
                        match exported {
                            Ok(mut result) => {
                                if result.warning.is_some(){result.warning=Some("Media exported; catalog indexing needs attention in the desktop app".into());}
                                job.snapshot.state = "complete".into();
                                job.snapshot.result = Some(json!(result));
                            }
                            Err(error) => {
                                job.snapshot.state = if error.code == "JOB_CANCELLED" {
                                    "cancelled"
                                } else {
                                    "failed"
                                }
                                .into();
                                job.snapshot.error = Some(error);
                            }
                        }
                    }
                }
            })
            .map_err(|_| failure("INTERNAL_ERROR"))?;
        operations.destinations.remove(&destination);
        operations.jobs.insert(
            id,
            OwnedJob {
                owner: owner.into(),
                destination,
                cancel,
                snapshot,
            },
        );
        operations.workers.push(worker);
        remember(
            &mut operations,
            owner,
            "clips.export",
            &key,
            &value,
            result.clone(),
        );
        Ok(result)
    }
    fn jobs(&self, owner: &str, access: &Access, method: &str, value: Value) -> Result<Value> {
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| failure("INTERNAL_ERROR"))?;
        if method == "jobs.list" {
            let page: List = args(value)?;
            validate_page(&page)?;
            let mut jobs = operations
                .jobs
                .values()
                .filter(|j| j.owner == owner)
                .map(|j| json!(j.snapshot))
                .collect::<Vec<_>>();
            drop(operations);
            let c = self.catalog()?;
            for job in c
                .active_jobs()
                .map_err(mapped)?
                .into_iter()
                .filter(|j| access.source_ids.contains(&j.source_id))
            {
                jobs.push(json!({"id":job.id,"source_id":job.source_id,"kind":"import","state":job.state,"status":job.status,"completed":job.completed,"total":job.total,"failed":job.failed}));
            }
            jobs.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
            let total = jobs.len();
            return Ok(
                json!({"items":jobs.into_iter().skip(page.offset).take(page.limit).collect::<Vec<_>>(),"total":total,"next_offset":(page.offset+page.limit<total).then_some(page.offset+page.limit)}),
            );
        }
        let a: Id = args(value)?;
        if let Some(job) = operations.jobs.get_mut(&a.id) {
            if job.owner != owner {
                return Err(failure("PERMISSION_DENIED"));
            }
            if method == "jobs.cancel" && job.snapshot.state == "running" {
                job.cancel.store(true, Ordering::Release);
                let _ = self.exports.cancel(&job.destination);
            }
            return Ok(json!(job.snapshot));
        }
        drop(operations);
        if method == "jobs.cancel" {
            return Err(failure("PERMISSION_DENIED"));
        }
        let c = self.catalog()?;
        let job = c.job(&a.id).map_err(|_| failure("PERMISSION_DENIED"))?;
        if !access.source_ids.contains(&job.source_id) {
            return Err(failure("PERMISSION_DENIED"));
        }
        Ok(
            json!({"id":job.id,"source_id":job.source_id,"kind":"import","state":job.state,"status":job.status,"completed":job.completed,"total":job.total,"failed":job.failed}),
        )
    }
    pub fn revoke(&self, owner: &str) {
        if let Ok(mut operations) = self.operations.lock() {
            for job in operations
                .jobs
                .values()
                .filter(|j| j.owner == owner && j.snapshot.state == "running")
            {
                job.cancel.store(true, Ordering::Release);
                let _ = self.exports.cancel(&job.destination);
            }
            for (id, client) in &operations.destinations {
                if client == owner {
                    let _ = self.exports.cancel(id);
                }
            }
            operations.destinations.retain(|_, o| o != owner);
            operations.replays.retain(|(o, _, _), _| o != owner);
            operations.jobs.retain(|_, j| j.owner != owner);
        }
    }
    pub fn shutdown(&self) {
        let workers = if let Ok(mut operations) = self.operations.lock() {
            for job in operations
                .jobs
                .values()
                .filter(|j| j.snapshot.state == "running")
            {
                job.cancel.store(true, Ordering::Release);
                let _ = self.exports.cancel(&job.destination);
            }
            for id in operations.destinations.keys() {
                let _ = self.exports.cancel(id);
            }
            std::mem::take(&mut operations.workers)
        } else {
            vec![]
        };
        for worker in workers {
            let _ = worker.join();
        }
        if let Ok(mut operations) = self.operations.lock() {
            *operations = Operations::default();
        }
    }
}
fn validate_page(page: &List) -> Result<()> {
    if page.limit == 0 || page.limit > 100 || page.offset > 1_000_000 {
        Err(failure("INVALID_ARGUMENT"))
    } else {
        Ok(())
    }
}
fn replayed(
    operations: &Operations,
    owner: &str,
    method: &str,
    key: &str,
    value: &Value,
) -> Result<Option<Value>> {
    if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) {
        return Err(failure("INVALID_ARGUMENT"));
    }
    let hash = blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string();
    if let Some(replay) = operations
        .replays
        .get(&(owner.into(), method.into(), key.into()))
    {
        if replay.hash != hash {
            return Err(failure("ASSET_CHANGED"));
        }
        return Ok(Some(replay.result.clone()));
    }
    if operations.replays.len() >= 1000 {
        return Err(failure("RATE_LIMITED"));
    }
    Ok(None)
}
fn remember(
    operations: &mut Operations,
    owner: &str,
    method: &str,
    key: &str,
    value: &Value,
    result: Value,
) {
    operations.replays.insert(
        (owner.into(), method.into(), key.into()),
        Replay {
            hash: blake3::hash(value.to_string().as_bytes())
                .to_hex()
                .to_string(),
            result,
        },
    );
}
