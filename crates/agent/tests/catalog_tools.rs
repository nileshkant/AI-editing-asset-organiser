use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use soundshelf_agent::Agent;
use soundshelf_core::{
    agent::{Access, AgentLibrary},
    catalog::{hash_file, Catalog, ClipRecipe, Profile},
    export::{ExportOptions, ExportService},
    media::{analyze, MediaTools},
    search::SearchQuery,
};
use std::{
    fs,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};
use tempfile::{tempdir, TempDir};
type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;
struct Fixture {
    library: Arc<AgentLibrary>,
    agent: Agent,
    endpoint: String,
    source: String,
    hidden_source: String,
    id: String,
    hidden: String,
    version: String,
    hidden_version: String,
    _dir: TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.agent.stop();
    }
}
fn fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("public")).unwrap();
    fs::create_dir(root.join("secret")).unwrap();
    fs::write(root.join("public/metal_scrape.wav"), b"public fixture").unwrap();
    fs::write(root.join("secret/zzprivateterma.wav"), b"hidden fixture").unwrap();
    let mut c = Catalog::open(&root.join("catalog.sqlite")).unwrap();
    let source = c.add_source(&root.join("public")).unwrap();
    let hidden_source = c.add_source(&root.join("secret")).unwrap();
    let version = hash_file(&root.join("public/metal_scrape.wav")).unwrap();
    let hidden_version = hash_file(&root.join("secret/zzprivateterma.wav")).unwrap();
    let profile = Profile {
        duration: 1.0,
        sample_rate: 48000,
        channels: 1,
        frames: 48000,
        peak: 0.5,
        rms: 0.2,
        channel_peaks: vec![0.5],
        channel_rms: vec![0.2],
        channel_layout: "mono".into(),
        description: "Measured fixture".into(),
        tags: vec!["metal".into()],
        waveform: vec![[-0.5, 0.5]],
    };
    let id = c.register(&source, "metal_scrape.wav", &version).unwrap();
    c.publish(&source, &id, &version, &profile).unwrap();
    let hidden = c
        .register(&hidden_source, "zzprivateterma.wav", &hidden_version)
        .unwrap();
    let mut secret = profile.clone();
    secret.tags = vec!["private tag".into()];
    c.publish(&hidden_source, &hidden, &hidden_version, &secret)
        .unwrap();
    let catalog = Arc::new(Mutex::new(c));
    let exports = Arc::new(ExportService::new(root.join("journal")).unwrap());
    let library = Arc::new(AgentLibrary::new(catalog, exports, None));
    let agent = Agent::with_library(root.join("runtime/mcp.json"), library.clone());
    let endpoint = agent.start(0).unwrap().endpoint.unwrap();
    Fixture {
        _dir: dir,
        library,
        agent,
        endpoint,
        source: source.id,
        hidden_source: hidden_source.id,
        id,
        hidden,
        version,
        hidden_version,
    }
}
fn access(f: &Fixture) -> Access {
    Access {
        source_ids: vec![f.source.clone()],
        read: true,
        ..Default::default()
    }
}
async fn connect(f: &Fixture, access: Access) -> Client {
    let paired = f.agent.pair_with_access("test".into(), access).unwrap();
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    ().serve(StreamableHttpClientTransport::with_client(
        http,
        StreamableHttpClientTransportConfig::with_uri(f.endpoint.clone()).auth_header(paired.token),
    ))
    .await
    .unwrap()
}
async fn call(client: &Client, name: &str, value: Value) -> (Value, bool) {
    let r = client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(value.as_object().unwrap().clone()),
        )
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    (r.structured_content.unwrap(), r.is_error == Some(true))
}
fn recipe(f: &Fixture) -> ClipRecipe {
    ClipRecipe {
        asset_id: f.id.clone(),
        asset_version_id: f.version.clone(),
        source_sample_rate_hz: 48000,
        start_frame: "0".into(),
        end_frame: "24000".into(),
        channel_policy: "preserve".into(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    }
}
#[tokio::test]
async fn scoped_discovery_search_facets_parity_and_portable_export() {
    let f = fixture();
    let read = connect(&f, access(&f)).await;
    let tools = read.list_all_tools().await.unwrap();
    assert!(tools.iter().any(|t| t.name == "sounds.search"));
    assert!(!tools.iter().any(|t| t.name == "sounds.annotate"
        || t.name == "clips.export"
        || t.name == "sounds.resolve"));
    let query = json!({"text":"metal","source_ids":[f.source],"limit":1});
    let (result, error) = call(&read, "sounds.search", query.clone()).await;
    assert!(!error);
    assert_eq!(result["total"], 1);
    assert_eq!(result["items"][0]["sound"]["content_hash"], f.version);
    assert_eq!(result["items"][0]["availability"], "available");
    let ui = f
        .library
        .catalog
        .lock()
        .unwrap()
        .search(&serde_json::from_value::<SearchQuery>(query.clone()).unwrap())
        .unwrap();
    assert_eq!(ui.items[0].id, result["items"][0]["sound"]["id"]);
    assert_eq!(json!(ui.facets), result["facets"]);
    assert_eq!(ui.total, result["total"].as_u64().unwrap() as usize);
    assert!(!result.to_string().contains("private tag"));
    let (typo, _) = call(&read, "sounds.search", json!({"text":"zzprivateterm"})).await;
    assert_eq!(typo["interpretation"]["corrected"], json!([]));
    let (denied, error) = call(
        &read,
        "sounds.search",
        json!({"source_ids":[f.hidden_source]}),
    )
    .await;
    assert!(error);
    assert_eq!(denied["code"], "PERMISSION_DENIED");
    let (export, error) = call(&read, "catalog.export", json!({"limit":1})).await;
    assert!(!error);
    assert_eq!(export["catalog"]["sounds"].as_array().unwrap().len(), 1);
    assert!(export["root_map"][0]["root"].is_null());
    assert!(!export.to_string().contains(&f.hidden));
    assert!(!export.to_string().contains(f._dir.path().to_str().unwrap()));
    let portable: soundshelf_core::portable::PortableCatalog =
        serde_json::from_value(export["catalog"].clone()).unwrap();
    portable.validate().unwrap();
    let (empty, _) = call(&read, "sounds.search", json!({"offset":1,"limit":1})).await;
    assert_eq!(empty["items"], json!([]));
    assert!(empty["next_offset"].is_null());
    let no_sources = connect(
        &f,
        Access {
            read: true,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        call(&no_sources, "sounds.search", json!({})).await.0["total"],
        0
    );
    // Hidden malformed profiles must not affect authorized reads.
    f.library
        .catalog
        .lock()
        .unwrap()
        .db_connection()
        .execute(
            "UPDATE analyses SET profile='broken' WHERE content_hash=?",
            [&f.hidden_version],
        )
        .unwrap();
    assert_eq!(call(&read, "sounds.search", json!({})).await.0["total"], 1);
}
#[tokio::test]
async fn edits_versions_idempotency_and_invalid_arguments() {
    let f = fixture();
    let read = connect(&f, access(&f)).await;
    let writer = connect(
        &f,
        Access {
            edit: true,
            ..access(&f)
        },
    )
    .await;
    let annotate = json!({"id":f.id,"version":f.version,"tags":["custom tag"],"comment":"Agent comment","favorite":true});
    let (denied, error) = call(&read, "sounds.annotate", annotate.clone()).await;
    assert!(error);
    assert_eq!(denied["code"], "PERMISSION_DENIED");
    assert!(!call(&writer, "sounds.annotate", annotate).await.1);
    let (get, _) = call(
        &writer,
        "sounds.get",
        json!({"id":f.id,"version":f.version}),
    )
    .await;
    assert_eq!(get["sound"]["user_tags"], json!(["custom tag"]));
    assert_eq!(get["sound"]["comment"], "Agent comment");
    let create =
        json!({"sound_id":f.id,"name":"Half","recipe":recipe(&f),"idempotency_key":"clip-one"});
    let (first, _) = call(&writer, "clips.create", create.clone()).await;
    let (second, _) = call(&writer, "clips.create", create.clone()).await;
    assert_eq!(first["id"], second["id"]);
    let mut conflicting = create.clone();
    conflicting["name"] = json!("Changed");
    assert_eq!(
        call(&writer, "clips.create", conflicting).await.0["code"],
        "ASSET_CHANGED"
    );
    let (update, error) = call(
        &writer,
        "clips.update",
        json!({"id":first["id"],"name":"Updated","recipe":recipe(&f),"revision":1}),
    )
    .await;
    assert!(!error);
    assert_eq!(update["revision"], 2);
    assert_eq!(
        call(
            &writer,
            "clips.update",
            json!({"id":first["id"],"name":"Stale","recipe":recipe(&f),"revision":1})
        )
        .await
        .0["code"],
        "ASSET_CHANGED"
    );
    assert_eq!(
        call(&writer, "sounds.get", json!({"id":f.id,"version":"old"}))
            .await
            .0["code"],
        "ASSET_CHANGED"
    );
    assert_eq!(
        call(
            &writer,
            "sounds.get",
            json!({"id":f.hidden,"version":f.hidden_version})
        )
        .await
        .0["code"],
        "PERMISSION_DENIED"
    );
    for arguments in [
        json!({"limit":101}),
        json!({"offset":1000001}),
        json!({"file":"/etc/passwd"}),
        json!({"text":12}),
    ] {
        assert!(writer
            .call_tool(
                CallToolRequestParams::new("sounds.search")
                    .with_arguments(arguments.as_object().unwrap().clone())
            )
            .await
            .is_err());
    }
    let mut invalid = create;
    invalid["recipe"]["path"] = json!("/etc/passwd");
    assert!(writer
        .call_tool(
            CallToolRequestParams::new("clips.create")
                .with_arguments(invalid.as_object().unwrap().clone())
        )
        .await
        .is_err());
    f.library
        .catalog
        .lock()
        .unwrap()
        .set_status(&f.id, "missing")
        .unwrap();
    assert_eq!(
        call(
            &writer,
            "sounds.get",
            json!({"id":f.id,"version":f.version})
        )
        .await
        .0["code"],
        "ASSET_MISSING"
    );
    assert_eq!(
        call(&writer, "sounds.search", json!({})).await.0["total"],
        0
    );
}
#[tokio::test]
async fn path_grant_containment_offline_and_sanitized_jobs() {
    let f = fixture();
    let read = connect(&f, access(&f)).await;
    let paths = connect(
        &f,
        Access {
            paths: true,
            ..access(&f)
        },
    )
    .await;
    let value = json!({"id":f.id,"version":f.version});
    assert_eq!(
        call(&read, "sounds.resolve", value.clone()).await.0["code"],
        "PERMISSION_DENIED"
    );
    assert!(
        call(&paths, "sounds.resolve", value.clone()).await.0["path"]
            .as_str()
            .unwrap()
            .ends_with("metal_scrape.wav")
    );
    let job = f
        .library
        .catalog
        .lock()
        .unwrap()
        .enqueue_scan(&f.source)
        .unwrap();
    let listed = call(&read, "jobs.list", json!({})).await.0;
    assert_eq!(listed["total"], 1);
    assert!(!listed.to_string().contains(f._dir.path().to_str().unwrap()));
    assert_eq!(
        call(&read, "jobs.get", json!({"id":job.id})).await.0["kind"],
        "import"
    );
    let private_job = f
        .library
        .catalog
        .lock()
        .unwrap()
        .enqueue_scan(&f.hidden_source)
        .unwrap();
    assert_eq!(
        call(&read, "jobs.get", json!({"id":private_job.id}))
            .await
            .0["code"],
        "PERMISSION_DENIED"
    );
    f.library
        .catalog
        .lock()
        .unwrap()
        .set_available(&f.source, false)
        .unwrap();
    assert_eq!(
        call(&paths, "sounds.resolve", value.clone()).await.0["code"],
        "SOURCE_OFFLINE"
    );
    f.library
        .catalog
        .lock()
        .unwrap()
        .set_available(&f.source, true)
        .unwrap();
    fs::remove_file(f._dir.path().join("public/metal_scrape.wav")).unwrap();
    assert_eq!(
        call(&paths, "sounds.resolve", value).await.0["code"],
        "ASSET_MISSING"
    );
}
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe"]
async fn real_clip_export_is_owned_idempotent_and_preserves_original() {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let media = root.join("media");
    fs::create_dir(&media).unwrap();
    let file = media.join("tone.wav");
    let tools = MediaTools::discover().unwrap();
    assert!(std::process::Command::new(&tools.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=0.4",
            "-ar",
            "16000",
            "-ac",
            "2"
        ])
        .arg(&file)
        .status()
        .unwrap()
        .success());
    let profile = analyze(&tools, &file, Arc::new(AtomicBool::new(false))).unwrap();
    let before = hash_file(&file).unwrap();
    let mut c = Catalog::open(&root.join("c.sqlite")).unwrap();
    let source = c.add_source(&media).unwrap();
    let id = c.register(&source, "tone.wav", &before).unwrap();
    c.publish(&source, &id, &before, &profile).unwrap();
    let library = Arc::new(AgentLibrary::new(
        Arc::new(Mutex::new(c)),
        Arc::new(ExportService::new(root.join("journal")).unwrap()),
        Some(tools.clone()),
    ));
    let agent = Agent::with_library(root.join("runtime/mcp.json"), library.clone());
    let endpoint = agent.start(0).unwrap().endpoint.unwrap();
    let scope = Access {
        source_ids: vec![source.id],
        read: true,
        edit: true,
        export: true,
        paths: false,
    };
    let paired = agent
        .pair_with_access("editor".into(), scope.clone())
        .unwrap();
    let other = agent.pair_with_access("other".into(), scope).unwrap();
    let client = ()
        .serve(StreamableHttpClientTransport::with_client(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            StreamableHttpClientTransportConfig::with_uri(endpoint).auth_header(paired.token),
        ))
        .await
        .unwrap();
    let recipe = ClipRecipe {
        asset_id: id.clone(),
        asset_version_id: before.clone(),
        source_sample_rate_hz: 16000,
        start_frame: "800".into(),
        end_frame: "4000".into(),
        channel_policy: "preserve".into(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let clip = call(
        &client,
        "clips.create",
        json!({"sound_id":id,"name":"Tone excerpt","recipe":recipe,"idempotency_key":"clip"}),
    )
    .await
    .0;
    let destination = library
        .approve_destination(
            &paired.client.id,
            &root.join("export.wav"),
            &ExportOptions::default(),
        )
        .unwrap();
    let request = json!({"clip_id":clip["id"],"revision":1,"destination_id":destination.id,"options":{"format":"wav"},"idempotency_key":"export"});
    assert_eq!(
        library
            .execute(
                &other.client.id,
                &other.client.access,
                "clips.export",
                request.clone()
            )
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    let (job, error) = call(&client, "clips.export", request.clone()).await;
    assert!(!error);
    assert_eq!(call(&client, "clips.export", request).await.0, job);
    let snapshot = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let state = call(&client, "jobs.get", json!({"id":job["job_id"]}))
                .await
                .0;
            if state["state"] != "running" {
                break state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(snapshot["state"], "complete", "{snapshot}");
    assert_eq!(snapshot["result"]["frames"], "3200");
    assert_eq!(
        analyze(
            &tools,
            &root.join("export.wav"),
            Arc::new(AtomicBool::new(false))
        )
        .unwrap()
        .frames,
        3200
    );
    assert_eq!(hash_file(&file).unwrap(), before);
    assert_eq!(
        library
            .execute(
                &other.client.id,
                &other.client.access,
                "jobs.get",
                json!({"id":job["job_id"]})
            )
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    assert!(library
        .approve_destination(
            &paired.client.id,
            &root.join("export.wav"),
            &ExportOptions::default()
        )
        .is_err());
    let approved = library
        .approve_destination(
            &paired.client.id,
            &root.join("cancel.wav"),
            &ExportOptions::default(),
        )
        .unwrap();
    let cancel=call(&client,"clips.export",json!({"clip_id":clip["id"],"revision":1,"destination_id":approved.id,"options":{"format":"wav"},"idempotency_key":"cancel"})).await.0;
    assert_eq!(
        library
            .execute(
                &other.client.id,
                &other.client.access,
                "jobs.cancel",
                json!({"id":cancel["job_id"]})
            )
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    call(&client, "jobs.cancel", json!({"id":cancel["job_id"]})).await;
    let cancelled = wait_job(&client, &cancel["job_id"]).await;
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    assert!(!root.join("cancel.wav").exists());
    let approved = library
        .approve_destination(
            &paired.client.id,
            &root.join("collision.wav"),
            &ExportOptions::default(),
        )
        .unwrap();
    fs::write(root.join("collision.wav"), b"keep").unwrap();
    let collision=call(&client,"clips.export",json!({"clip_id":clip["id"],"revision":1,"destination_id":approved.id,"options":{"format":"wav"},"idempotency_key":"collision"})).await.0;
    assert_eq!(
        wait_job(&client, &collision["job_id"]).await["error"]["code"],
        "EXPORT_COLLISION"
    );
    assert_eq!(fs::read(root.join("collision.wav")).unwrap(), b"keep");
    let approved = library
        .approve_destination(
            &paired.client.id,
            &root.join("changed.wav"),
            &ExportOptions::default(),
        )
        .unwrap();
    {
        use std::io::Write;
        fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(b"user replaced media")
            .unwrap();
    }
    let changed=call(&client,"clips.export",json!({"clip_id":clip["id"],"revision":1,"destination_id":approved.id,"options":{"format":"wav"},"idempotency_key":"changed"})).await.0;
    assert_eq!(
        wait_job(&client, &changed["job_id"]).await["error"]["code"],
        "ASSET_CHANGED"
    );
    assert!(!root.join("changed.wav").exists());
    agent.stop();
}

#[tokio::test]
async fn selected_file_scope_never_grants_siblings() {
    let f = fixture();
    let sibling = f._dir.path().join("public/sibling.wav");
    fs::write(&sibling, b"sibling fixture").unwrap();
    let sibling_id = {
        let mut c = f.library.catalog.lock().unwrap();
        let source = c.source(&f.source).unwrap();
        let hash = hash_file(&sibling).unwrap();
        let profile = c.sound(&f.id).unwrap().profile.unwrap();
        let id = c.register(&source, "sibling.wav", &hash).unwrap();
        c.publish(&source, &id, &hash, &profile).unwrap();
        c.db_connection()
            .execute("UPDATE sources SET scope='files' WHERE id=?", [&f.source])
            .unwrap();
        c.db_connection()
            .execute(
                "INSERT INTO source_files(source_id,relative_path) VALUES(?,'metal_scrape.wav')",
                [&f.source],
            )
            .unwrap();
        id
    };
    let client = connect(
        &f,
        Access {
            paths: true,
            ..access(&f)
        },
    )
    .await;
    assert_eq!(
        call(&client, "sounds.search", json!({})).await.0["total"],
        1
    );
    assert_eq!(
        call(
            &client,
            "sounds.get",
            json!({"id":sibling_id,"version":hash_file(&sibling).unwrap()})
        )
        .await
        .0["code"],
        "PERMISSION_DENIED"
    );
    #[cfg(unix)]
    {
        let path = f._dir.path().join("public/metal_scrape.wav");
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(f._dir.path().join("secret/zzprivateterma.wav"), &path).unwrap();
        assert_eq!(
            call(
                &client,
                "sounds.resolve",
                json!({"id":f.id,"version":f.version})
            )
            .await
            .0["code"],
            "PERMISSION_DENIED"
        );
    }
}
#[tokio::test]
async fn compiled_bridge_search_and_disconnect_use_running_catalog() {
    let f = fixture();
    let paired = f
        .agent
        .pair_with_access("bridge".into(), access(&f))
        .unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_soundshelf-mcp"))
        .arg(&f.endpoint)
        .env("SOUNDSHELF_MCP_TOKEN", &paired.token)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = tokio::time::timeout(
        Duration::from_secs(10),
        ().serve((child.stdout.take().unwrap(), child.stdin.take().unwrap())),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        call(&client, "sounds.search", json!({})).await.0["total"],
        1
    );
    f.agent.stop();
    let after = tokio::time::timeout(
        Duration::from_secs(10),
        client.call_tool(CallToolRequestParams::new("sounds.search")),
    )
    .await
    .unwrap();
    assert!(after.is_err());
    drop(client);
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&paired.token));
}

async fn wait_job(client: &Client, id: &Value) -> Value {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let state = call(client, "jobs.get", json!({"id":id})).await.0;
            if state["state"] != "running" {
                break state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
