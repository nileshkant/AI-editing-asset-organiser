//! Loopback HTTP server shared by provider tests: records each request and
//! answers from a handler.
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Value,
}
pub type Handler = Arc<dyn Fn(&Seen) -> (u16, Vec<(String, String)>, Vec<u8>) + Send + Sync>;
pub struct Server {
    pub address: SocketAddr,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}
impl Server {
    pub fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(vec![]));
        let log = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let (handler, log) = (handler.clone(), log.clone());
                thread::spawn(move || serve(stream, &handler, &log));
            }
        });
        Self { address, seen }
    }
    pub fn json(handler: impl Fn(&Seen) -> (u16, Value) + Send + Sync + 'static) -> Self {
        Self::start(Arc::new(move |s| {
            let (status, body) = handler(s);
            (status, vec![], body.to_string().into_bytes())
        }))
    }
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}
pub fn serve(stream: TcpStream, handler: &Handler, log: &Mutex<Vec<Seen>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        headers.insert(name.to_lowercase(), value.trim().to_owned());
    }
    let length: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let seen = Seen { method, path, headers, body: serde_json::from_slice(&body).unwrap_or(Value::Null) };
    log.lock().unwrap().push(seen.clone());
    let (status, extra, body) = handler(&seen);
    let mut out = stream;
    let mut head = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n", body.len());
    for (name, value) in extra {
        head += &format!("{name}: {value}\r\n");
    }
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(b"\r\n");
    let _ = out.write_all(&body);
}

