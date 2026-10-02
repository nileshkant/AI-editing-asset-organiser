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
use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router, ServerHandler,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    net::TcpListener,
    sync::{Arc, Mutex, RwLock},
    thread::JoinHandle,
};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
struct StatusService {
    tool_router: ToolRouter<Self>,
}
#[tool_router]
impl StatusService {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
    #[tool(
        description = "SoundShelf MCP connection status. Contains no catalog data or local paths."
    )]
    fn service_status(&self) -> String {
        "SoundShelf is running. Catalog tools are not enabled.".into()
    }
}
#[tool_handler(router = self.tool_router)]
impl ServerHandler for StatusService {
    fn get_info(&self) -> ServerConfig {
        {
            let mut info = ServerConfig::default();
            info.capabilities = ServerCapabilities::builder().enable_tools().build();
            info.server_info = Implementation::new("SoundShelf", env!("CARGO_PKG_VERSION"));
            info
        }
    }
}
#[derive(Clone, Serialize)]
pub struct Client {
    pub id: String,
    pub name: String,
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
}
struct Grant {
    client: Client,
    hash: [u8; 32],
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
    let authorized = token.filter(|t| t.len() == 64).is_some_and(|t| {
        let hash = *blake3::hash(t.as_bytes()).as_bytes();
        gate.grants
            .read()
            .map(|g| g.values().any(|v| bool::from(v.hash.ct_eq(&hash))))
            .unwrap_or(false)
    });
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(_permit) = gate.slots.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    // Limit body before it reaches the protocol parser, including chunked requests.
    let (parts, body) = request.into_parts();
    let Ok(Ok(bytes)) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        axum::body::to_bytes(body, 64 * 1024),
    )
    .await
    else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
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
}
impl Agent {
    pub fn with_discovery(path: std::path::PathBuf) -> Self {
        {
            let mut agent = Self::default();
            agent.discovery = Some(path);
            agent
        }
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
        })
    }
    pub fn start(&self, port: u16) -> Result<Status, String> {
        let mut state = self.running.lock().map_err(|_| "MCP state unavailable")?;
        if state.is_some() {
            return Err("MCP is already running".into());
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).map_err(|_| {
            "MCP port unavailable. Choose another port or use 0 for an available port."
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "MCP listener unavailable")?;
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
        let worker = std::thread::Builder::new()
            .name("soundshelf-mcp".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let mut config = StreamableHttpServerConfig::default();
                    config.legacy_session_mode = false;
                    config.json_response = true;
                    config.cancellation_token = shutdown.clone();
                    let service = StreamableHttpService::new(
                        || Ok(StatusService::new()),
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
        *state = Some(Running {
            endpoint,
            cancel,
            worker,
        });
        drop(state);
        self.status()
    }
    pub fn pair(&self, name: String) -> Result<Pairing, String> {
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
        };
        // 244 random bits; only the hash remains in application memory.
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        grants.insert(
            client.id.clone(),
            Grant {
                client: client.clone(),
                hash: *blake3::hash(token.as_bytes()).as_bytes(),
            },
        );
        Ok(Pairing { client, token })
    }
    pub fn revoke(&self, id: &str) -> Result<(), String> {
        self.grants
            .write()
            .map_err(|_| "MCP clients unavailable")?
            .remove(id);
        Ok(())
    }
    pub fn stop(&self) {
        if let Ok(mut state) = self.running.lock() {
            if let Some(running) = state.take() {
                running.cancel.cancel();
                let _ = running.worker.join();
                if let Some(path) = &self.discovery {
                    let _ = std::fs::remove_file(path);
                }
            }
            if let Ok(mut grants) = self.grants.write() {
                grants.clear();
            }
        }
    }
}
impl Drop for Agent {
    fn drop(&mut self) {
        self.stop();
    }
}
