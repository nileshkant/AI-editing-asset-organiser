//! Opt-in, process-owned MCP lifecycle. No catalog or filesystem tools.
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
mod tools;
use serde::{Deserialize, Serialize};
use soundshelf_core::agent::{Access, AgentLibrary};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::HashMap,
    net::TcpListener,
    sync::{Arc, Mutex, RwLock},
    thread::JoinHandle,
};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;
use tools::{Authorization, StatusService};
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
pub struct Client {
    pub id: String,
    pub name: String,
    pub access: Access,
}
#[derive(Serialize)]
pub struct Pairing {
    pub client: Client,
    pub token: String,
}
#[derive(Serialize)]
pub struct Status {
    pub endpoint: Option<String>,
    pub clients: Vec<Client>,
    pub startup_error: Option<String>,
}
struct Grant {
    client: Client,
    hash: [u8; 32],
    live: Arc<AtomicBool>,
}
#[derive(Clone)]
struct Gate {
    authority: String,
    grants: Arc<RwLock<HashMap<String, Grant>>>,
    slots: Arc<tokio::sync::Semaphore>,
}
async fn guard(State(gate): State<Gate>, request: Request, next: Next) -> Response {
    let h = request.headers();
    // Exact authority and same-origin only; DNS names and forwarded headers never widen access.
    if h.get_all("host").iter().count() != 1
        || h.get("host").and_then(|v| v.to_str().ok()) != Some(gate.authority.as_str())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if h.get_all("origin").iter().count() > 1
        || h.get("origin")
            .is_some_and(|v| v.to_str().ok() != Some(format!("http://{}", gate.authority).as_str()))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request.uri().query().is_some() || h.get_all("authorization").iter().count() != 1 {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let token = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let authorized = token.filter(|t| t.len() == 64).and_then(|t| {
        let hash = *blake3::hash(t.as_bytes()).as_bytes();
        gate.grants.read().ok().and_then(|g| {
            g.values()
                .find(|v| bool::from(v.hash.ct_eq(&hash)) && v.live.load(Ordering::Acquire))
                .map(|v| Authorization {
                    id: v.client.id.clone(),
                    access: v.client.access.clone(),
                    live: v.live.clone(),
                    permit: None,
                })
        })
    });
    let Some(mut authorized) = authorized else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(_permit) = gate.slots.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    // Limit body before it reaches the protocol parser, including chunked requests.
    let (mut parts, body) = request.into_parts();
    let Ok(Ok(bytes)) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        axum::body::to_bytes(body, 64 * 1024),
    )
    .await
    else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    authorized.permit = Some(Arc::new(_permit));
    parts.extensions.insert(authorized);
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}
struct Running {
    endpoint: String,
    cancel: CancellationToken,
    worker: JoinHandle<()>,
}
#[derive(Default)]
pub struct Agent {
    running: Mutex<Option<Running>>,
    grants: Arc<RwLock<HashMap<String, Grant>>>,
    discovery: Option<std::path::PathBuf>,
    library: Option<Arc<AgentLibrary>>,
    store: Option<Mutex<rusqlite::Connection>>,
    startup_error: Mutex<Option<String>>,
    storage_error: Option<String>,
}
impl Agent {
    pub fn with_discovery(path: std::path::PathBuf) -> Self {
        {
            let mut agent = Self::default();
            agent.discovery = Some(path);
            agent
        }
    }
    pub fn with_library(path: std::path::PathBuf, library: Arc<AgentLibrary>) -> Self {
        let mut agent = Self::with_discovery(path);
        agent.library = Some(library);
        agent
    }
    /// Pairing hashes are separate from the portable catalog and its backups.
    pub fn persistent(
        path: std::path::PathBuf,
        store_path: std::path::PathBuf,
        library: Option<Arc<AgentLibrary>>,
    ) -> Self {
        let mut agent = Self::with_discovery(path);
        agent.library = library;
        let load =
            (|| -> Result<(rusqlite::Connection, bool, u16, HashMap<String, Grant>), String> {
                std::fs::create_dir_all(
                    store_path
                        .parent()
                        .ok_or("MCP settings directory unavailable")?,
                )
                .map_err(|_| "MCP settings directory unavailable")?;
                let db = rusqlite::Connection::open(&store_path)
                    .map_err(|_| "MCP settings unavailable")?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&store_path, std::fs::Permissions::from_mode(0o600))
                        .map_err(|_| "MCP settings permissions unavailable")?;
                }
                db.execute_batch("CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL, port INTEGER NOT NULL); INSERT OR IGNORE INTO settings VALUES(1,0,0); CREATE TABLE IF NOT EXISTS clients (id TEXT PRIMARY KEY, client TEXT NOT NULL, hash BLOB NOT NULL);")
                .map_err(|_| "MCP settings could not be loaded")?;
                let (enabled, port): (bool, u16) = db
                    .query_row("SELECT enabled,port FROM settings WHERE id=1", [], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .map_err(|_| "MCP settings are invalid")?;
                let mut grants = HashMap::new();
                {
                    let mut query = db
                        .prepare("SELECT id,client,hash FROM clients")
                        .map_err(|_| "MCP clients unavailable")?;
                    let rows = query
                        .query_map([], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, Vec<u8>>(2)?,
                            ))
                        })
                        .map_err(|_| "MCP clients unavailable")?;
                    for row in rows {
                        let (id, json, bytes) = row.map_err(|_| "MCP client data invalid")?;
                        let client: Client =
                            serde_json::from_str(&json).map_err(|_| "MCP client data invalid")?;
                        let hash: [u8; 32] =
                            bytes.try_into().map_err(|_| "MCP client data invalid")?;
                        if client.id != id || grants.len() >= 32 {
                            return Err("MCP client data invalid".into());
                        }
                        grants.insert(
                            id,
                            Grant {
                                client,
                                hash,
                                live: Arc::new(AtomicBool::new(false)),
                            },
                        );
                    }
                }
                Ok((db, enabled, port, grants))
            })();
        match load {
            Ok((db, enabled, port, grants)) => {
                agent.store = Some(Mutex::new(db));
                agent.grants = Arc::new(RwLock::new(grants));
                if enabled {
                    if let Err(error) = agent.start(port) {
                        *agent.startup_error.lock().unwrap() = Some(error);
                    }
                }
            }
            Err(error) => {
                agent.storage_error = Some(error.clone());
                *agent.startup_error.lock().unwrap() = Some(error);
            }
        }
        agent
    }
    fn save_client(&self, client: &Client, hash: &[u8; 32]) -> Result<(), String> {
        if let Some(error) = &self.storage_error {
            return Err(error.clone());
        }
        if let Some(store) = &self.store {
            let db = store.lock().map_err(|_| "MCP settings unavailable")?;
            db.execute(
                "INSERT OR REPLACE INTO clients VALUES(?1,?2,?3)",
                rusqlite::params![
                    client.id,
                    serde_json::to_string(client).map_err(|_| "MCP client invalid")?,
                    hash.as_slice()
                ],
            )
            .map_err(|_| "MCP credential could not be saved")?;
        }
        Ok(())
    }
    fn save_enabled(&self, enabled: bool, port: u16) -> Result<(), String> {
        if let Some(error) = &self.storage_error {
            return Err(error.clone());
        }
        if let Some(store) = &self.store {
            store
                .lock()
                .map_err(|_| "MCP settings unavailable")?
                .execute(
                    "UPDATE settings SET enabled=?1,port=?2 WHERE id=1",
                    rusqlite::params![enabled, port],
                )
                .map_err(|_| "MCP startup setting could not be saved")?;
        }
        Ok(())
    }
    pub fn rotate(&self, id: &str) -> Result<Pairing, String> {
        let _running = self.running.lock().map_err(|_| "MCP state unavailable")?;
        let mut grants = self.grants.write().map_err(|_| "MCP clients unavailable")?;
        let grant = grants.get_mut(id).ok_or("Client pairing unavailable")?;
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let hash = *blake3::hash(token.as_bytes()).as_bytes();
        self.save_client(&grant.client, &hash)?;
        grant.live.store(false, Ordering::Release);
        grant.hash = hash;
        grant.live = Arc::new(AtomicBool::new(true));
        if let Some(library) = &self.library {
            library.revoke(id);
        }
        Ok(Pairing {
            client: grant.client.clone(),
            token,
        })
    }
    pub fn disable(&self) -> Result<(), String> {
        self.stop_internal(true)
    }
    fn remembered_port(&self) -> Result<u16, String> {
        if let Some(store) = &self.store {
            store
                .lock()
                .map_err(|_| "MCP settings unavailable")?
                .query_row("SELECT port FROM settings WHERE id=1", [], |r| r.get(0))
                .map_err(|_| "MCP settings unavailable".into())
        } else {
            Ok(0)
        }
    }
    pub fn client(&self, id: &str) -> Result<Client, String> {
        self.grants
            .read()
            .map_err(|_| "MCP clients unavailable")?
            .get(id)
            .map(|g| g.client.clone())
            .ok_or_else(|| "Client pairing unavailable".into())
    }
    pub fn status(&self) -> Result<Status, String> {
        let running = self.running.lock().map_err(|_| "MCP state unavailable")?;
        let mut clients: Vec<_> = self
            .grants
            .read()
            .map_err(|_| "MCP clients unavailable")?
            .values()
            .map(|g| g.client.clone())
            .collect();
        clients.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Status {
            endpoint: running.as_ref().map(|r| r.endpoint.clone()),
            clients,
            startup_error: self
                .startup_error
                .lock()
                .map_err(|_| "MCP state unavailable")?
                .clone(),
        })
    }
    pub fn start(&self, port: u16) -> Result<Status, String> {
        let mut state = self.running.lock().map_err(|_| "MCP state unavailable")?;
        if state.is_some() {
            return Err("MCP is already running".into());
        }
        if let Some(error) = &self.storage_error {
            return Err(error.clone());
        }
        let port = if port == 0 {
            self.remembered_port()?
        } else {
            port
        };
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).map_err(|_| {
            "MCP port unavailable. Close the conflicting service or choose another explicit port."
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "MCP listener unavailable")?;
        let listener_port = listener
            .local_addr()
            .map_err(|_| "MCP address unavailable")?
            .port();
        let authority = listener
            .local_addr()
            .map_err(|_| "MCP listener unavailable")?
            .to_string();
        let endpoint = format!("http://{authority}/mcp");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| "MCP runtime unavailable")?;
        let cancel = CancellationToken::new();
        let shutdown = cancel.clone();
        let gate = Gate {
            authority,
            grants: self.grants.clone(),
            slots: Arc::new(tokio::sync::Semaphore::new(16)),
        };
        if let Some(path) = &self.discovery {
            let parent = path.parent().ok_or("MCP runtime directory unavailable")?;
            std::fs::create_dir_all(parent).map_err(|_| "MCP runtime directory unavailable")?;
            std::fs::write(path, serde_json::json!({"endpoint":endpoint}).to_string())
                .map_err(|_| "MCP discovery could not be written")?;
        }
        let library = self.library.clone();
        let worker = std::thread::Builder::new()
            .name("CreativeShelf".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let mut config = StreamableHttpServerConfig::default();
                    config.legacy_session_mode = false;
                    config.json_response = true;
                    config.cancellation_token = shutdown.clone();
                    let service = StreamableHttpService::new(
                        move || {
                            Ok(StatusService {
                                library: library.clone(),
                            })
                        },
                        Arc::new(LocalSessionManager::default()),
                        config,
                    );
                    let router = Router::new()
                        .nest_service("/mcp", service)
                        .layer(middleware::from_fn_with_state(gate, guard));
                    if let Ok(listener) = tokio::net::TcpListener::from_std(listener) {
                        let server = axum::serve(listener, router)
                            .with_graceful_shutdown(shutdown.clone().cancelled_owned());
                        tokio::select! { _=server=>{}, _=shutdown.cancelled()=>{} }
                    }
                })
            })
            .map_err(|_| {
                if let Some(path) = &self.discovery {
                    let _ = std::fs::remove_file(path);
                }
                "MCP worker could not start"
            })?;
        if let Err(error) = self.save_enabled(true, listener_port) {
            cancel.cancel();
            let _ = worker.join();
            if let Some(path) = &self.discovery {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
        if let Ok(mut grants) = self.grants.write() {
            for grant in grants.values_mut() {
                grant.live = Arc::new(AtomicBool::new(true));
            }
        }
        *self
            .startup_error
            .lock()
            .map_err(|_| "MCP state unavailable")? = None;
        *state = Some(Running {
            endpoint,
            cancel,
            worker,
        });
        drop(state);
        self.status()
    }
    pub fn pair(&self, name: String) -> Result<Pairing, String> {
        self.pair_with_access(name, Access::default())
    }
    pub fn pair_with_access(&self, name: String, access: Access) -> Result<Pairing, String> {
        if let Some(library) = &self.library {
            library.validate_access(&access)?;
        } else if access.read
            || access.edit
            || access.export
            || access.paths
            || !access.source_ids.is_empty()
        {
            return Err("Catalog service unavailable".into());
        }
        let state = self.running.lock().map_err(|_| "MCP state unavailable")?;
        if state.is_none() {
            return Err("Start MCP before pairing a client".into());
        }
        let name = name.trim();
        if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            return Err("Use a client name of 1–80 bytes without control characters".into());
        }
        let mut grants = self.grants.write().map_err(|_| "MCP clients unavailable")?;
        if grants.len() >= 32 {
            return Err("Maximum 32 clients; revoke an existing client first".into());
        }
        let client = Client {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            access,
        };
        // 244 random bits; only the hash remains in application memory.
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let hash = *blake3::hash(token.as_bytes()).as_bytes();
        self.save_client(&client, &hash)?;
        grants.insert(
            client.id.clone(),
            Grant {
                client: client.clone(),
                hash,
                live: Arc::new(AtomicBool::new(true)),
            },
        );
        Ok(Pairing { client, token })
    }
    pub fn revoke(&self, id: &str) -> Result<(), String> {
        let _running = self.running.lock().map_err(|_| "MCP state unavailable")?;
        let mut grants = self.grants.write().map_err(|_| "MCP clients unavailable")?;
        if let Some(error) = &self.storage_error {
            return Err(error.clone());
        }
        if let Some(store) = &self.store {
            store
                .lock()
                .map_err(|_| "MCP settings unavailable")?
                .execute("DELETE FROM clients WHERE id=?1", [id])
                .map_err(|_| "MCP revocation could not be saved")?;
        }
        let grant = grants.remove(id);
        if let Some(grant) = grant {
            grant.live.store(false, Ordering::Release);
            if let Some(library) = &self.library {
                library.revoke(id);
            }
        }
        Ok(())
    }
    pub fn stop(&self) {
        let _ = self.stop_internal(false);
    }
    fn stop_internal(&self, disable: bool) -> Result<(), String> {
        {
            let mut state = self.running.lock().map_err(|_| "MCP state unavailable")?;
            if disable {
                self.save_enabled(false, self.remembered_port()?)?;
            }
            if let Some(running) = state.take() {
                if let Ok(grants) = self.grants.read() {
                    for grant in grants.values() {
                        grant.live.store(false, Ordering::Release);
                    }
                }
                running.cancel.cancel();
                let _ = running.worker.join();
                if let Some(path) = &self.discovery {
                    let _ = std::fs::remove_file(path);
                }
            }
            if let Some(library) = &self.library {
                library.shutdown();
            }
            if let Ok(mut grants) = self.grants.write() {
                if self.store.is_none() {
                    grants.clear();
                }
            }
        }
        Ok(())
    }
}
impl Drop for Agent {
    fn drop(&mut self) {
        self.stop();
    }
}
