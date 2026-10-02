//! Compiled transport bridge only. Never opens a catalog or starts the app.
use rmcp::{
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, IntoTransport,
        StreamableHttpClientTransport, Transport,
    },
    RoleClient, RoleServer,
};
#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("SoundShelf MCP bridge unavailable. Open SoundShelf, start MCP, and check your endpoint and client credential.");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), ()> {
    let mut args = std::env::args().skip(1);
    let argument = args.next().ok_or(())?;
    let endpoint = if argument == "--discovery" {
        use std::io::Read;
        let mut text = String::new();
        std::fs::File::open(args.next().ok_or(())?)
            .map_err(|_| ())?
            .take(513)
            .read_to_string(&mut text)
            .map_err(|_| ())?;
        if text.len() > 512 {
            return Err(());
        }
        serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|_| ())?
            .get("endpoint")
            .and_then(|s| s.as_str())
            .ok_or(())?
            .to_owned()
    } else {
        argument
    };
    if args.next().is_some() {
        return Err(());
    }
    let uri = reqwest::Url::parse(&endpoint).map_err(|_| ())?;
    if uri.scheme() != "http"
        || uri.host_str() != Some("127.0.0.1")
        || uri.port().is_none()
        || uri.path() != "/mcp"
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        return Err(());
    }
    let token = std::env::var("SOUNDSHELF_MCP_TOKEN").map_err(|_| ())?;
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|_| ())?;
    // Fail clearly before reading stdin when the app is closed or access revoked.
    let probe = client
        .get(uri)
        .bearer_auth(&token)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map_err(|_| ())?;
    if probe.status() != reqwest::StatusCode::METHOD_NOT_ALLOWED {
        return Err(());
    }
    let config = StreamableHttpClientTransportConfig::with_uri(endpoint).auth_header(token);
    let mut remote = StreamableHttpClientTransport::with_client(client, config);
    let mut local = IntoTransport::<RoleServer, _, _>::into_transport(rmcp::transport::stdio());
    loop {
        tokio::select! {
            msg=local.receive()=>match msg {Some(msg)=>remote.send(msg).await.map_err(|_|())?,None=>break},
            msg=remote.receive()=>match msg {Some(msg)=>local.send(msg).await.map_err(|_|())?,None=>break},
        }
    }
    <_ as Transport<RoleClient>>::close(&mut remote)
        .await
        .map_err(|_| ())?;
    local.close().await.map_err(|_| ())?;
    Ok(())
}
