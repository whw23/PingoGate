//! Shared black-box harness for US1 contract/integration tests.
//!
//! Each test spins up an isolated mock upstream (records what the gateway
//! forwarded) and a real `pingogate` subprocess wired to it on fresh ports, so
//! tests run in parallel without contending. Secrets reach the subprocess via
//! environment references, exactly as in production.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

pub mod client;
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

/// The three routes the default config ships with (US1 fixtures). Reload tests
/// append to this to grow the route table across a SIGHUP.
pub const DEFAULT_ROUTES: &str = include_str!("fixtures/routes-default.yaml");

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
    /// default route table.
    pub fn start() -> Self {
        Self::start_with_routes(DEFAULT_ROUTES)
    }

    /// Start a gateway whose initial `routes:` block is `routes`.
    pub fn start_with_routes(routes: &str) -> Self {
        Self::spawn(routes, mock::MockUpstream::start())
    }

    /// Start a gateway backed by a gated mock; the returned [`mock::Gate`] holds
    /// a single in-flight upstream response so a reload can be forced to overlap
    /// a request that is provably mid-flight (XII).
    pub fn start_gated() -> (Self, mock::Gate) {
        let (mock, gate) = mock::MockUpstream::start_gated();
        (Self::spawn(DEFAULT_ROUTES, mock), gate)
    }

    fn spawn(routes: &str, mock: mock::MockUpstream) -> Self {
        let public_port = free_port();
        let admin_port = free_port();
        let config_path = write_config(public_port, admin_port, mock.port, routes);

        let child = Command::new(env!("CARGO_BIN_EXE_pingogate"))
            .arg("--config")
            .arg(&config_path)
            .env("PINGO_ADMIN_TOKEN", "admin-secret")
            .env("PINGO_GATEWAY_KEY_TEAM", GATEWAY_KEY)
            .env("OPENAI_KEY", OPENAI_UPSTREAM_KEY)
            .env("ANTHROPIC_KEY", ANTHROPIC_UPSTREAM_KEY)
            .env("GEMINI_KEY", GEMINI_UPSTREAM_KEY)
            .env("RUST_LOG", "error")
            .spawn()
            .expect("spawn pingogate");

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

    /// The admin listener port (US3 wires endpoints onto it).
    pub fn admin_port(&self) -> u16 {
        self.admin_port
    }

    /// Rewrite the on-disk config with a new `routes:` block, then signal the
    /// running gateway to reload it via `SIGHUP` (FR-019). The providers,
    /// listeners, and gateway keys are unchanged.
    pub fn reload_with_routes(&self, routes: &str) {
        let yaml = render_config(self.public_port, self.admin_port, self.mock_port, routes);
        std::fs::write(&self.config_path, yaml).expect("rewrite config");
        self.sighup();
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

/// Render the config template wired to the three mock-backed providers, with the
/// given `routes:` block substituted in.
fn render_config(public: u16, admin: u16, mock: u16, routes: &str) -> String {
    include_str!("fixtures/gateway.yaml.tmpl")
        .replace("__PUBLIC__", &public.to_string())
        .replace("__ADMIN__", &admin.to_string())
        .replace("__MOCK__", &mock.to_string())
        .replace("__ROUTES__", routes.trim_end_matches('\n'))
}

/// Write the rendered config to a unique temp path; return that path.
fn write_config(public: u16, admin: u16, mock: u16, routes: &str) -> PathBuf {
    let yaml = render_config(public, admin, mock, routes);
    let path = std::env::temp_dir().join(format!(
        "pingogate-it-{}-{}.yaml",
        std::process::id(),
        public
    ));
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(yaml.as_bytes()).unwrap();
    path
}
