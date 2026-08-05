//! Shared black-box harness for S1 six-interface e2e tests.
//!
//! Each test spins up an isolated mock upstream (records what the gateway
//! forwarded) and a real `pingogate-core` subprocess wired to it on fresh ports,
//! so tests run in parallel without contending. Secrets reach the subprocess via
//! environment references, exactly as in production.
//!
//! Ported from 001 (`app/pingogate/tests/common/mod.rs`); adapted to the new
//! binary name (`pingogate-core`) and crate structure. The six-interface
//! pipeline (T7/T8 Responses + Interactions extensions) is exercised by the
//! e2e_* test files alongside this harness.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

pub mod client;
pub mod closure;
pub mod mock;

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Gateway key the test client presents; mapped to the "team" principal.
pub const GATEWAY_KEY: &str = "gw-team-secret";
/// Upstream provider keys the gateway must inject (never the gateway key).
pub const OPENAI_UPSTREAM_KEY: &str = "sk-upstream-openai";
pub const ANTHROPIC_UPSTREAM_KEY: &str = "sk-upstream-anthropic";
pub const GEMINI_UPSTREAM_KEY: &str = "sk-upstream-gemini";

/// Reserve an ephemeral port by binding then dropping the listener.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Index of the first `\r\n\r\n` (header/body boundary) in `buf`, if present.
pub fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// A running gateway subprocess plus the mock it forwards to. Killed on drop.
pub struct Harness {
    child: Child,
    config_path: PathBuf,
    public_port: u16,
    admin_port: u16,
    mock_port: u16,
    pub mock: mock::MockUpstream,
}

impl Harness {
    /// Start a mock upstream and a gateway subprocess wired to it, serving the
    /// default model table.
    pub fn start() -> Self {
        Self::spawn(mock::MockUpstream::start())
    }

    /// Start a gateway backed by a gated mock; the returned [`mock::Gate`] holds
    /// a single in-flight upstream response so a reload can be forced to overlap
    /// a request that is provably mid-flight (XII).
    pub fn start_gated() -> (Self, mock::Gate) {
        let (mock, gate) = mock::MockUpstream::start_gated();
        (Self::spawn(mock), gate)
    }

    fn spawn(mock: mock::MockUpstream) -> Self {
        let public_port = free_port();
        let admin_port = free_port();
        let config_path = write_config(public_port, admin_port, mock.port);

        // `CARGO_BIN_EXE_pingogate-core` is set by Cargo when running integration
        // tests. This Cargo toolchain keeps the hyphen in the env var name (rather
        // than replacing it with an underscore), and sets it at runtime not compile
        // time, so we use `std::env::var` instead of `env!`.
        let bin_path = std::env::var("CARGO_BIN_EXE_pingogate-core")
            .expect("CARGO_BIN_EXE_pingogate-core must be set by Cargo for integration tests");
        let child = Command::new(&bin_path)
            .arg("--config")
            .arg(&config_path)
            .env("PINGO_ADMIN_TOKEN", "admin-secret")
            .env("PINGO_GATEWAY_KEY_TEAM", GATEWAY_KEY)
            .env("OPENAI_KEY", OPENAI_UPSTREAM_KEY)
            .env("ANTHROPIC_KEY", ANTHROPIC_UPSTREAM_KEY)
            .env("GEMINI_KEY", GEMINI_UPSTREAM_KEY)
            .env("RUST_LOG", "error")
            .spawn()
            .expect("spawn pingogate-core");

        let harness = Self {
            child,
            config_path,
            public_port,
            admin_port,
            mock_port: mock.port,
            mock,
        };
        harness.await_ready();
        harness
    }

    /// Send a request to the gateway's public listener.
    pub fn send(&self, req: &client::Request) -> client::Resp {
        client::send(self.public_port, req)
    }

    /// The public listener port (for tests that drive their own client thread).
    pub fn public_port(&self) -> u16 {
        self.public_port
    }

    /// The admin listener port (S2 wires endpoints onto it).
    pub fn admin_port(&self) -> u16 {
        self.admin_port
    }

    /// Overwrite the config file with arbitrary YAML, then send `SIGHUP`. Used to
    /// drive a candidate that must be rejected without disturbing the active one.
    pub fn reload_with_raw(&self, yaml: &str) {
        std::fs::write(&self.config_path, yaml).expect("rewrite config");
        self.sighup();
    }

    /// Deliver `SIGHUP` to the gateway subprocess. `kill(1)` keeps the test in
    /// Safe Rust (no `libc`/`unsafe` to raise the signal directly).
    pub fn sighup(&self) {
        let status = Command::new("kill")
            .arg("-HUP")
            .arg(self.child.id().to_string())
            .status()
            .expect("send SIGHUP");
        assert!(status.success(), "kill -HUP failed");
    }

    /// Poll `check` against this harness until it returns true or the deadline
    /// elapses; returns whether it ever passed. Bridges the asynchronous gap
    /// between signalling a reload and the new snapshot becoming active.
    pub fn poll_until(&self, mut check: impl FnMut(&Self) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if check(self) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    /// Block until the public listener accepts connections (or panic on timeout).
    fn await_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", self.public_port)).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "gateway public listener did not come up on :{}",
            self.public_port
        );
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.config_path);
    }
}

/// Render the config template wired to the three mock-backed providers.
/// Models are nested under providers in the template (no external routes block).
fn render_config(public: u16, admin: u16, mock: u16) -> String {
    include_str!("fixtures/gateway.yaml.tmpl")
        .replace("__PUBLIC__", &public.to_string())
        .replace("__ADMIN__", &admin.to_string())
        .replace("__MOCK__", &mock.to_string())
}

/// Write the rendered config to a unique temp path; return that path.
fn write_config(public: u16, admin: u16, mock: u16) -> PathBuf {
    let yaml = render_config(public, admin, mock);
    let path = std::env::temp_dir().join(format!(
        "pingogate-it-{}-{}.yaml",
        std::process::id(),
        public
    ));
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(yaml.as_bytes()).unwrap();
    path
}
