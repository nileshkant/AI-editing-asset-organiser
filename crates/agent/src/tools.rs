use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
        ListToolsResult, PaginatedRequestParams, Tool,
    },
    service::{RequestContext, RoleServer},
    ServerHandler,
};
use serde_json::{json, Value};
use soundshelf_core::agent::{Access, AgentLibrary};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[derive(Clone)]
pub(crate) struct Authorization {
    pub id: String,
    pub access: Access,
    pub live: Arc<AtomicBool>,
    pub permit: Option<Arc<tokio::sync::OwnedSemaphorePermit>>,
}
#[derive(Clone)]
pub(crate) struct StatusService {
    pub library: Option<Arc<AgentLibrary>>,
}
fn authorization(context: &RequestContext<RoleServer>) -> Option<Authorization> {
    context
        .extensions
        .get::<axum::http::request::Parts>()?
        .extensions
        .get::<Authorization>()
        .filter(|a| a.live.load(Ordering::Acquire))
        .cloned()
}
fn response(value: Value, error: bool) -> CallToolResponse {
    let mut result = if error {
        CallToolResult::error(vec![ContentBlock::text(value.to_string())])
    } else {
        CallToolResult::success(vec![ContentBlock::text(value.to_string())])
    };
    result.structured_content = Some(value);
    result.into()
}
impl ServerHandler for StatusService {
    fn get_tool(&self, name: &str) -> Option<Tool> {
        definitions().into_iter().find(|tool| tool.name == name)
    }
    fn get_info(&self) -> rmcp::model::ServerConfig {
        let mut info = rmcp::model::ServerConfig::default();
        info.capabilities = rmcp::model::ServerCapabilities::builder()
            .enable_tools()
            .build();
        info.server_info =
            rmcp::model::Implementation::new("SoundShelf", env!("CARGO_PKG_VERSION"));
        info
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let access = authorization(&context)
            .map(|a| a.access)
            .unwrap_or_default();
        let mut result = ListToolsResult::default();
        result.tools = definitions()
            .into_iter()
            .filter(|tool| {
                tool.name == "service_status"
                    || (self.library.is_some() && AgentLibrary::allows(&access, &tool.name))
            })
            .collect();
        Ok(result)
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let value = Value::Object(request.arguments.unwrap_or_default());
        if request.name == "service_status" {
            if value.as_object().is_some_and(|v| v.is_empty()) {
                return Ok(response(
                    json!({"running":true,"catalog_tools":self.library.is_some()}),
                    false,
                ));
            }
            return Err(ErrorData::invalid_params(
                "Unexpected status arguments",
                None,
            ));
        }
        if !definitions().iter().any(|tool| tool.name == request.name) {
            return Err(ErrorData::invalid_params("Unknown tool", None));
        }
        let Some(auth) = authorization(&context) else {
            return Ok(response(
                json!({"code":"PERMISSION_DENIED","message":"Client access was revoked"}),
                true,
            ));
        };
        let Some(library) = self.library.clone() else {
            return Ok(response(
                json!({"code":"NOT_READY","message":"Catalog service unavailable"}),
                true,
            ));
        };
        let method = request.name.to_string();
        let result = tokio::task::spawn_blocking(move || {
            // Retain the request slot even if HTTP/SDK cancellation drops its caller.
            let _permit = auth.permit;
            if !auth.live.load(Ordering::Acquire) {
                return Err(soundshelf_core::agent::Failure {
                    code: "PERMISSION_DENIED",
                    message: "Client access was revoked",
                });
            }
            library.execute(&auth.id, &auth.access, &method, value)
        })
        .await
        .map_err(|_| ErrorData::internal_error("Agent operation failed", None))?;
        match result {
            Ok(value) => {
                if value.to_string().len() > 2 * 1024 * 1024 {
                    Ok(response(
                        json!({"code":"RATE_LIMITED","message":"Response exceeds 2 MiB; request a smaller page"}),
                        true,
                    ))
                } else {
                    Ok(response(value, false))
                }
            }
            Err(error) if error.code == "INVALID_ARGUMENT" => {
                Err(ErrorData::invalid_params(error.message, None))
            }
            Err(error) => Ok(response(json!(error), true)),
        }
    }
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn definitions() -> Vec<Tool> {
    let string = json!({"type":"string","minLength":1,"maxLength":256});
    let version = string.clone();
    let key = json!({"type":"string","minLength":1,"maxLength":128});
    let page = json!({"offset":{"type":"integer","minimum":0,"maximum":1000000,"default":0},"limit":{"type":"integer","minimum":1,"maximum":100,"default":50}});
    let search = json!({"text":{"type":"string","maxLength":512},"source_ids":{"type":"array","maxItems":128,"items":string},"tags":{"type":"array","maxItems":64,"items":{"type":"string"}},"favorites_only":{"type":"boolean"},"min_duration":{"type":"number","minimum":0},"max_duration":{"type":"number","minimum":0},"offset":page["offset"],"limit":page["limit"]});
    let sound = object(json!({"id":string,"version":version}), &["id", "version"]);
    let recipe = object(
        json!({"asset_id":string,"asset_version_id":version,"source_sample_rate_hz":{"type":"integer","minimum":1},"start_frame":{"type":"string","pattern":"^[0-9]+$"},"end_frame":{"type":"string","pattern":"^[0-9]+$"},"channel_policy":{"type":"string","enum":["preserve"]},"gain_db":{"type":"number"},"fade_in_ms":{"type":"integer","minimum":0},"fade_out_ms":{"type":"integer","minimum":0}}),
        &[
            "asset_id",
            "asset_version_id",
            "source_sample_rate_hz",
            "start_frame",
            "end_frame",
        ],
    );
    let options = object(
        json!({"format":{"type":"string","enum":["wav","flac"]},"sample_rate":{"type":["integer","null"]},"fade_in_ms":{"type":["integer","null"]},"fade_out_ms":{"type":["integer","null"]}}),
        &[],
    );
    let specs=vec![
        ("service_status","Check the running app connection",object(json!({}),&[])),
        ("library.status","Count ready and available sounds in authorized sources without rescanning",object(json!({}),&[])),
        ("sources.list","List authorized source IDs, names and availability; no machine paths",object(page.clone(),&[])),
        ("sounds.search","Search the indexed catalog using UI filters; content_hash is the content version. Results exclude unavailable/pending sounds",object(search.clone(),&[])),
        ("sounds.facets","Facets/counts for the same scoped search filters",object(search.clone(),&[])),
        ("sounds.get","Read a current sound by ID and expected content version",sound.clone()),
        ("sounds.resolve","Resolve a scoped sound to a local path; requires explicit path permission",sound.clone()),
        ("sounds.annotate","Update tags, comment and favorite on an expected current sound; requires edit",object(json!({"id":string,"version":version,"tags":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":64}},"comment":{"type":"string","maxLength":10000},"favorite":{"type":"boolean"}}),&["id","version","tags","comment","favorite"])),
        ("clips.create","Create a version-bound clip. Retry only with the same idempotency key and arguments",object(json!({"sound_id":string,"name":{"type":"string","minLength":1,"maxLength":100},"recipe":recipe,"idempotency_key":key}),&["sound_id","name","recipe","idempotency_key"])),
        ("clips.update","Revision-checked clip edit",object(json!({"id":string,"name":string,"recipe":recipe,"revision":{"type":"integer","minimum":1}}),&["id","name","recipe","revision"])),
        ("clips.get","Get a clip in the authorized sources",object(json!({"id":string}),&["id"])),
        ("clips.list","Page clips for an authorized current sound",object(json!({"id":string,"version":version,"offset":page["offset"],"limit":page["limit"]}),&["id","version"])),
        ("clips.export","Start a bounded export job to a client-specific desktop-approved destination ID. Original media is immutable",object(json!({"clip_id":string,"revision":{"type":"integer","minimum":1},"destination_id":string,"options":options,"idempotency_key":key}),&["clip_id","revision","destination_id","options","idempotency_key"])),
        ("jobs.get","Get your export job or a permitted import job, with sanitized errors",object(json!({"id":string}),&["id"])),
        ("jobs.list","List your export jobs and imports in authorized sources",object(page,&[])),
        ("jobs.cancel","Cancel your own export job; publication already completed cannot be undone",object(json!({"id":string}),&["id"])),
        ("catalog.export","Page an authorized portable catalog subset. Paths stay relative; root map is null without path permission",object(search,&[]))
    ];
    specs
        .into_iter()
        .map(|(name, description, schema)| {
            Tool::new(name, description, schema.as_object().unwrap().clone())
        })
        .collect()
}
