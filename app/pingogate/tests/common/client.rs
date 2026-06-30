//! Minimal blocking HTTP/1.1 client for the integration harness.
//!
//! Forces `Connection: close` so each response is read to EOF; de-chunks the
//! body when the gateway forwards a chunked response. Intentionally tiny — it
//! only needs to drive single request/response exchanges against the gateway.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::find_double_crlf;

/// A request to send to the gateway.
pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub headers: Vec<(&'a str, &'a str)>,
    pub body: &'a [u8],
}

impl<'a> Request<'a> {
    pub fn get(path: &'a str) -> Self {
        Self {
            method: "GET",
            path,
            headers: Vec::new(),
            body: &[],
        }
    }

    pub fn post(path: &'a str, body: &'a [u8]) -> Self {
        Self {
            method: "POST",
            path,
            headers: vec![("content-type", "application/json")],
            body,
        }
    }

    pub fn header(mut self, name: &'a str, value: &'a str) -> Self {
        self.headers.push((name, value));
        self
    }
}

/// A parsed response.
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn body_str(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

/// Send `req` to `127.0.0.1:port` and read the full response.
pub fn send(port: u16, req: &Request) -> Resp {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();

    let mut raw = format!("{} {} HTTP/1.1\r\n", req.method, req.path);
    raw.push_str(&format!("host: 127.0.0.1:{port}\r\n"));
    raw.push_str("connection: close\r\n");
    let mut has_content_length = false;
    for (key, value) in &req.headers {
        if key.eq_ignore_ascii_case("content-length") {
            has_content_length = true;
        }
        raw.push_str(&format!("{key}: {value}\r\n"));
    }
    if !req.body.is_empty() && !has_content_length {
        raw.push_str(&format!("content-length: {}\r\n", req.body.len()));
    }
    raw.push_str("\r\n");

    let mut bytes = raw.into_bytes();
    bytes.extend_from_slice(req.body);
    stream.write_all(&bytes).unwrap();
    stream.flush().unwrap();

    let buf = read_full(&mut stream);
    parse_response(&buf)
}

/// Read the response, stopping as soon as the full body is in hand (by
/// `content-length` or the chunked terminator) rather than waiting for the peer
/// to close — so a kept-alive downstream connection does not hang the test.
fn read_full(stream: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        if let Some(end) = find_double_crlf(&buf) {
            if response_complete(&buf, end) {
                break;
            }
        }
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }
    buf
}

/// Whether `buf` holds a complete response, given the header boundary at `end`.
fn response_complete(buf: &[u8], end: usize) -> bool {
    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
    let body_start = end + 4;
    if let Some(line) = head.lines().find(|l| l.starts_with("content-length:")) {
        let len: usize = line[15..].trim().parse().unwrap_or(0);
        return buf.len() >= body_start + len;
    }
    if head.contains("transfer-encoding:") && head.contains("chunked") {
        return buf[body_start..].windows(5).any(|w| w == b"0\r\n\r\n");
    }
    false
}

fn parse_response(buf: &[u8]) -> Resp {
    let split = find_double_crlf(buf).unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..split]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);

    let mut headers = Vec::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
    }

    let body_start = (split + 4).min(buf.len());
    let raw_body = buf[body_start..].to_vec();
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    let body = if chunked {
        dechunk(&raw_body)
    } else {
        raw_body
    };

    Resp {
        status,
        headers,
        body,
    }
}

/// Decode an HTTP/1.1 chunked body into its raw bytes.
fn dechunk(buf: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = buf;
    while let Some(pos) = rest.windows(2).position(|w| w == b"\r\n") {
        let size_str = String::from_utf8_lossy(&rest[..pos]);
        let size = usize::from_str_radix(size_str.trim(), 16).unwrap_or(0);
        let chunk_start = pos + 2;
        if size == 0 || chunk_start + size > rest.len() {
            break;
        }
        out.extend_from_slice(&rest[chunk_start..chunk_start + size]);
        rest = &rest[(chunk_start + size + 2).min(rest.len())..];
    }
    out
}
