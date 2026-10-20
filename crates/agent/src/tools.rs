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
#[cfg(test)]
mod ss080_preview_content {
    use serde_json::json;
    #[test]
    fn previews_are_image_content_with_structured_meta_and_bounded() {
        let ok = super::preview_result(json!({"image":{"mime_type":"image/png","data":"iVBORw0KGgo="},"meta":{"width":4}})).unwrap();
        let blocks = serde_json::to_value(&ok.content).unwrap();
        assert_eq!(blocks[0]["type"], "image");
        assert_eq!((blocks[0]["mimeType"].as_str(), blocks[0]["data"].as_str()), (Some("image/png"), Some("iVBORw0KGgo=")));
        assert_eq!(ok.structured_content, Some(json!({"meta":{"width":4}})));
        assert_ne!(ok.is_error, Some(true));
        let big = "A".repeat(super::MAX_RESPONSE_BYTES + 1);
        let refused = super::preview_result(json!({"image":{"mime_type":"image/png","data":big},"meta":{}})).unwrap();
        assert_eq!(refused.is_error, Some(true));
        assert_eq!(refused.structured_content.unwrap()["code"], "RATE_LIMITED");
        assert!(super::preview_result(json!({"meta":{}})).is_err());
    }
}
#[cfg(test)]
mod ss040_contract_surface {
    #[test]
    fn audio_tool_surface_is_unchanged_and_only_the_image_slice_is_added() {
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/contracts/baseline-inventory.json"
        )).unwrap();
        let mut names: Vec<String> = super::definitions().iter().map(|t| t.name.to_string()).collect();
        names.sort();
        // SS-080 adds image tools; every baseline name stays exactly as it was.
        let baseline: Vec<String> = serde_json::from_value(expected["mcp_methods"].clone()).unwrap();
        let added: Vec<&String> = names.iter().filter(|n| !baseline.contains(n)).collect();
        assert!(baseline.iter().all(|b| names.contains(b)));
        assert_eq!(added, ["images.annotate", "images.get", "images.preview", "images.search"]);
        assert!(!names.iter().any(|n| n.starts_with("elements.")));
    }
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
            rmcp::model::Implementation::new("CreativeShelf", env!("CARGO_PKG_VERSION"));
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
        let preview = method == "images.preview";
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
            Ok(value) if preview => preview_result(value).map(Into::into),
            Ok(value) => {
                if value.to_string().len() > MAX_RESPONSE_BYTES {
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
/// Pixels travel as MCP image content; metadata stays structured. The encoded
/// image counts against the same 2 MiB budget as every other response.
fn preview_result(value: Value) -> Result<CallToolResult, ErrorData> {
    let (Some(data), Some(mime)) = (value["image"]["data"].as_str(), value["image"]["mime_type"].as_str()) else {
        return Err(ErrorData::internal_error("Agent operation failed", None));
    };
    if data.len() > MAX_RESPONSE_BYTES {
        let refusal = json!({"code":"RATE_LIMITED","message":"Preview exceeds 2 MiB; request a smaller size"});
        let mut result = CallToolResult::error(vec![ContentBlock::text(refusal.to_string())]);
        result.structured_content = Some(refusal);
        return Ok(result);
    }
    let meta = value["meta"].clone();
    let mut result = CallToolResult::success(vec![
        ContentBlock::image(data.to_owned(), mime.to_owned()),
        ContentBlock::text(meta.to_string()),
    ]);
    result.structured_content = Some(json!({"meta":meta}));
    Ok(result)
}
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
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
        ("jobs.list","List your export jobs and imports in authorized sources",object(page.clone(),&[])),
        ("jobs.cancel","Cancel your own export job; publication already completed cannot be undone",object(json!({"id":string}),&["id"])),
        ("catalog.export","Page an authorized portable catalog subset. Paths stay relative; root map is null without path permission",object(search,&[])),
        ("images.search","Find images in authorized sources by path, caption or tag text, required tags, untagged or favorite. Metadata only; version is the content hash. Creator notes are never searched or returned",object(json!({"text":{"type":"string","maxLength":512},"source_ids":{"type":"array","maxItems":128,"items":string},"tags":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":64}},"untagged_only":{"type":"boolean"},"favorites_only":{"type":"boolean"},"offset":{"type":"integer","minimum":0,"maximum":100000,"default":0},"limit":page["limit"]}),&[])),
        ("images.get","Read an image's metadata by ID and expected version",object(json!({"id":string,"version":version}),&["id","version"])),
        ("images.preview","View a bounded preview (long edge 128, 256 or 512 px) of an expected image version. Requires preview permission. Describe only what this preview shows",object(json!({"id":string,"version":version,"size":{"type":"string","enum":["small","medium","large"],"default":"large"}}),&["id","version"])),
        ("images.annotate","Add or remove tags on an expected image version and annotation revision; recorded in the owner's undo history. Requires edit",object(json!({"id":string,"version":version,"revision":{"type":"integer","minimum":1},"add_tags":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":64}},"remove_tags":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":64}}}),&["id","version","revision"]))
    ];
    specs
        .into_iter()
        .map(|(name, description, schema)| {
            Tool::new(name, description, schema.as_object().unwrap().clone())
        })
        .collect()
}
