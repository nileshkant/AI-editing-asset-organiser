use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use soundshelf_agent::Agent;
use std::time::Duration;
// Serialize lifecycle fixtures: OS ephemeral ports can be reassigned immediately after shutdown.
static LIFECYCLE: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}
async fn connect(
    endpoint: &str,
    token: &str,
) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    ().serve(StreamableHttpClientTransport::with_client(
        client(),
        StreamableHttpClientTransportConfig::with_uri(endpoint.to_owned())
            .auth_header(token.to_owned()),
    ))
    .await
    .unwrap()
}
#[tokio::test]
async fn official_sdk_initialize_list_call_revoke_restart_stop() {
    let _fixture = LIFECYCLE.lock().unwrap();
    let agent = Agent::default();
    assert!(agent.status().unwrap().endpoint.is_none());
    assert!(agent.pair("test".into()).is_err());
    let endpoint = agent.start(0).unwrap().endpoint.unwrap();
    let first = agent.pair("Editor".into()).unwrap();
    let second = agent.pair("Agent".into()).unwrap();
    assert_ne!(first.token, second.token);
    let service = connect(&endpoint, &first.token).await;
    let tools = service.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "service_status");
    let result = service
        .call_tool(CallToolRequestParams::new("service_status"))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    assert!(service
        .call_tool(CallToolRequestParams::new("shell"))
        .await
        .is_err());
    assert!(!serde_json::to_string(&agent.status().unwrap())
        .unwrap()
        .contains(&first.token));
    agent.revoke(&first.client.id).unwrap();
    assert!(service.list_all_tools().await.is_err());
    let other = connect(&endpoint, &second.token).await;
    assert_eq!(other.list_all_tools().await.unwrap().len(), 1);
    agent.stop();
    assert!(client().get(&endpoint).send().await.is_err());
    assert!(agent.status().unwrap().clients.is_empty());
    let new_endpoint = agent.start(0).unwrap().endpoint.unwrap();
    assert_eq!(
        client()
            .post(new_endpoint)
            .bearer_auth(&second.token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    agent.stop();
}
#[tokio::test]
async fn headers_rebinding_tokens_and_body_are_bounded() {
    let _fixture = LIFECYCLE.lock().unwrap();
    let agent = Agent::default();
    let endpoint = agent.start(0).unwrap().endpoint.unwrap();
    let paired = agent.pair("test".into()).unwrap();
    let http = client();
    for host in [
        "evil.example",
        "localhost",
        "127.0.0.1:1",
        "127.0.0.1.evil.example",
        "[::1]",
    ] {
        assert_eq!(
            http.post(&endpoint)
                .header("host", host)
                .bearer_auth(&paired.token)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    for origin in [
        "null",
        "https://evil.example",
        "http://localhost",
        "http://127.0.0.1:1",
    ] {
        assert_eq!(
            http.post(&endpoint)
                .header("origin", origin)
                .bearer_auth(&paired.token)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(http.post(&endpoint).send().await.unwrap().status(), 401);
    assert_eq!(
        http.post(&endpoint)
            .bearer_auth("bad")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.post(format!("{endpoint}?token={}", paired.token))
            .bearer_auth(&paired.token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.post(&endpoint)
            .bearer_auth(&paired.token)
            .body(vec![b'x'; 65537])
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    assert_eq!(
        http.post(&endpoint)
            .bearer_auth(&paired.token)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body("{bad")
            .send()
            .await
            .unwrap()
            .status(),
        415
    );
    let response = http
        .post(&endpoint)
        .bearer_auth(&paired.token)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2099-01-01")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert!(!response.text().await.unwrap().contains(&paired.token));
    agent.stop();
}
#[test]
fn occupied_port_and_pairing_limits() {
    let _fixture = LIFECYCLE.lock().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let agent = Agent::default();
    assert!(agent
        .start(listener.local_addr().unwrap().port())
        .err()
        .unwrap()
        .contains("port unavailable"));
    assert!(agent.status().unwrap().endpoint.is_none());
    agent.start(0).unwrap();
    assert!(agent.pair("\n".into()).is_err());
    assert!(agent.pair("x".repeat(81)).is_err());
    for _ in 0..32 {
        agent.pair("test".into()).unwrap();
    }
    assert!(agent.pair("overflow".into()).is_err());
    agent.stop();
}
#[tokio::test]
async fn compiled_bridge_with_official_sdk() {
    let _fixture = LIFECYCLE.lock().unwrap();
    let directory =
        std::env::temp_dir().join(format!("soundshelf-bridge-{}", uuid::Uuid::new_v4()));
    let discovery = directory.join("mcp.json");
    let agent = Agent::with_discovery(discovery.clone());
    let endpoint = agent.start(0).unwrap().endpoint.unwrap();
    let paired = agent.pair("stdio".into()).unwrap();
    // Explicit qualification override exercises a staged/installed executable,
    // while ordinary CI retains Cargo's own compiled fixture binary.
    let bridge = std::env::var_os("CREATIVESHELF_QUALIFICATION_BRIDGE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_soundshelf-mcp").into());
    let mut child = tokio::process::Command::new(&bridge)
        .arg("--discovery")
        .arg(&discovery)
        .env("SOUNDSHELF_MCP_TOKEN", &paired.token)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let transport = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let service = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(service.list_all_tools().await.unwrap().len(), 1);
    assert_ne!(
        service
            .call_tool(CallToolRequestParams::new("service_status"))
            .await
            .unwrap()
            .is_error,
        Some(true)
    );
    service.cancel().await.unwrap();
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&paired.token));
    agent.stop();
    let output = tokio::process::Command::new(&bridge)
        .arg(&endpoint)
        .env("SOUNDSHELF_MCP_TOKEN", &paired.token)
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&paired.token));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn discovery_contains_no_credentials_and_drop_stops_listener() {
    let _fixture = LIFECYCLE.lock().unwrap();
    let directory = std::env::temp_dir().join(format!("soundshelf-mcp-{}", uuid::Uuid::new_v4()));
    let path = directory.join("mcp.json");
    let endpoint;
    {
        let agent = Agent::with_discovery(path.clone());
        endpoint = agent.start(0).unwrap().endpoint.unwrap();
        let paired = agent.pair("test".into()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["endpoint"],
            endpoint
        );
        assert!(!text.contains(&paired.token));
    }
    assert!(!path.exists());
    let address = endpoint
        .strip_prefix("http://")
        .unwrap()
        .strip_suffix("/mcp")
        .unwrap();
    assert!(std::net::TcpStream::connect(address).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn legacy_negotiation_and_same_origin_are_supported() {
    let _fixture = LIFECYCLE.lock().unwrap();
    let agent = Agent::default();
    let endpoint = agent.start(0).unwrap().endpoint.unwrap();
    let paired = agent.pair("legacy".into()).unwrap();
    let response=client().post(&endpoint).bearer_auth(&paired.token)
        .header("origin",endpoint.strip_suffix("/mcp").unwrap())
        .header("accept","application/json, text/event-stream")
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"legacy","version":"1"}}}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["result"]["protocolVersion"], "2025-11-25");
    assert!(!body.to_string().contains(&paired.token));
    agent.stop();
}
