//! Mock upstream provider for the integration harness.
//!
//! Default mode (S1): accepts one request per connection, echoes the
//! method/path/credential headers the gateway forwarded back as `x-observed-*`
//! response headers, and returns the request body verbatim so tests can assert
//! the gateway stripped the gateway key, injected the upstream credential, and
//! passed the body through unchanged. A streaming request gets an SSE response.
//!
//! Gated mode (in-flight test): the handler parks after reading the request,
//! signalling the test that a request is in-flight at the upstream, and waits
//! for the test to release it before responding. This lets a test reload the
//! gateway config while a request is provably mid-flight, then prove the
//! in-flight request still completes against the snapshot it bound (XII).
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use super::find_double_crlf;

pub struct MockUpstream {
    pub port: u16,
}

/// Coordination handle for a gated mock: the test waits for a request to arrive,
/// performs a reload, then releases the parked upstream response.
pub struct Gate {
    arrived: Receiver<()>,
    release: Sender<()>,
}

impl Gate {
    /// Block until the mock has read an in-flight request and parked it.
    pub fn wait_for_request(&self) {
        self.arrived.recv().expect("upstream request never arrived");
    }

    /// Release the parked upstream so it sends its response.
    pub fn release(&self) {
        self.release.send(()).expect("release the parked upstream");
    }
}

/// Shared sender/receiver the gated handler uses to talk to the [`Gate`].
struct GateInner {
    arrived: Sender<()>,
    release: Mutex<Receiver<()>>,
}

impl MockUpstream {
    pub fn start() -> Self {
        Self::start_inner(None)
    }

    /// Start a gated mock; the returned [`Gate`] controls a single in-flight
    /// upstream response.
    pub fn start_gated() -> (Self, Gate) {
        let (arrived_tx, arrived_rx) = channel();
        let (release_tx, release_rx) = channel();
        let inner = Arc::new(GateInner {
            arrived: arrived_tx,
            release: Mutex::new(release_rx),
        });
        let mock = Self::start_inner(Some(inner));
        (
            mock,
            Gate {
                arrived: arrived_rx,
                release: release_tx,
            },
        )
    }

    fn start_inner(gate: Option<Arc<GateInner>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let gate = gate.clone();
                thread::spawn(move || handle(&mut stream, gate.as_deref()));
            }
        });
        Self { port }
    }
}

struct Observed {
    method: String,
    path: String,
    authorization: String,
    x_api_key: String,
    anthropic_version: String,
    host: String,
    body: Vec<u8>,
}

fn handle(stream: &mut TcpStream, gate: Option<&GateInner>) {
    let Some(observed) = read_request(stream) else {
        return;
    };
    // Park the response until the test releases it, signalling arrival first so
    // the test knows the request is genuinely in-flight at the upstream.
    if let Some(gate) = gate {
        let _ = gate.arrived.send(());
        if let Ok(rx) = gate.release.lock() {
            let _ = rx.recv();
        }
    }
    let is_stream =
        observed.path.contains(":streamGenerateContent") || body_requests_stream(&observed.body);
    let response = if is_stream {
        stream_response()
    } else {
        echo_response(&observed)
    };
    let _ = stream.write_all(&response);
    let _ = stream.flush();
}

fn read_request(stream: &mut TcpStream) -> Option<Observed> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let headers_end = loop {
        if let Some(pos) = find_double_crlf(&buf) {
            break pos;
        }
        match stream.read(&mut tmp) {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    };

    let head = String::from_utf8_lossy(&buf[..headers_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    let mut obs = Observed {
        method,
        path,
        authorization: String::new(),
        x_api_key: String::new(),
        anthropic_version: String::new(),
        host: String::new(),
        body: Vec::new(),
    };
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match key.trim().to_ascii_lowercase().as_str() {
            "content-length" => content_length = value.parse().unwrap_or(0),
            "authorization" => obs.authorization = value,
            "x-api-key" => obs.x_api_key = value,
            "anthropic-version" => obs.anthropic_version = value,
            "host" => obs.host = value,
            _ => {}
        }
    }

    let mut body = buf[(headers_end + 4).min(buf.len())..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
        }
    }
    body.truncate(content_length);
    obs.body = body;
    Some(obs)
}

fn body_requests_stream(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

fn echo_response(obs: &Observed) -> Vec<u8> {
    let mut head = String::new();
    head.push_str("HTTP/1.1 200 OK\r\n");
    head.push_str("content-type: application/json\r\n");
    head.push_str(&format!("x-observed-method: {}\r\n", obs.method));
    head.push_str(&format!("x-observed-path: {}\r\n", obs.path));
    head.push_str(&format!(
        "x-observed-authorization: {}\r\n",
        obs.authorization
    ));
    head.push_str(&format!("x-observed-x-api-key: {}\r\n", obs.x_api_key));
    head.push_str(&format!(
        "x-observed-anthropic-version: {}\r\n",
        obs.anthropic_version
    ));
    head.push_str(&format!("x-observed-host: {}\r\n", obs.host));
    head.push_str(&format!("content-length: {}\r\n", obs.body.len()));
    head.push_str("connection: close\r\n\r\n");
    let mut out = head.into_bytes();
    out.extend_from_slice(&obs.body);
    out
}

fn stream_response() -> Vec<u8> {
    let body = "data: {\"delta\":\"he\"}\n\ndata: {\"delta\":\"llo\"}\n\ndata: [DONE]\n\n";
    let mut head = String::new();
    head.push_str("HTTP/1.1 200 OK\r\n");
    head.push_str("content-type: text/event-stream\r\n");
    head.push_str(&format!("content-length: {}\r\n", body.len()));
    head.push_str("connection: close\r\n\r\n");
    let mut out = head.into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}
