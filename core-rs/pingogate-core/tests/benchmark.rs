//! S3 Task 33: performance benchmark for stateless passthrough (spec §10/§12D
//! SC-7: p50 < 5 ms / p95 < 20 ms). Marked `#[ignore]` so it never runs in the
//! normal `cargo test` gate (latency assertions are machine-dependent and would
//! flake CI). Run it explicitly:
//!
//! ```bash
//! cargo test --release -p pingogate-core --test benchmark -- --ignored --nocapture
//! ```
//!
//! ## Standalone vs platform mode
//!
//! - **Standalone** (`bench_standalone_passthrough`): the S1/S2 harness
//!   (`common::Harness`) spawns a real `pingogate-core` subprocess in
//!   standalone mode with a mock upstream. Each request goes through the full
//!   Pingora pipeline (protocol detect -> gateway-key auth -> route ->
//!   upstream-credential inject -> passthrough -> mock echo). This is the
//!   stateless passthrough the SC-7 budget applies to.
//!
//! - **Platform** (`bench_platform_passthrough_with_decrypt`): TODO. The
//!   platform-mode Pingora data plane is not yet wired (T32 documented gap:
//!   `run_platform` in `main.rs` only runs the gRPC server; the public
//!   listener is not started in platform mode). When the data plane lands,
//!   this test will start Rust + Go + mock upstream, push a snapshot with a
//!   BYOK key + virtual key, present the vkey, and measure the hot path
//!   including the per-request `KeyVault::decrypt` (~0.2 µs, constitution XXI
//!   budget 0.001%) + inline `UsageExtractor` (O(1) memory, T27). The budget
//!   is the same p50 < 5 ms / p95 < 20 ms; the decrypt + extraction are
//!   expected to add < 0.5 ms, well within budget.
//!
//! Ported from 001 (`app/pingogate/tests/benchmark.rs` on the `redesign`
//! branch); adapted to the new harness + crate structure. The 001 recorded
//! p50 ≈ 1.5 ms / p95 ≈ 1.7 ms against a stub upstream; SC-7 allows up to
//! 5/20 ms (constitution XXI budget), so the assertion is loose enough to
//! not flake on slower CI machines while still catching regressions.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

mod common;

use std::time::{Duration, Instant};

use common::client::Request;
use common::{Harness, GATEWAY_KEY};

/// Warmup requests discarded before measuring (stabilizes connection / route
/// caches so the first few requests' connection-setup cost doesn't skew p50).
const WARMUP: usize = 200;
/// Measured requests. 3000 is enough for a stable p95 without making the
/// test take > 10 s on a warm machine.
const ITERS: usize = 3000;

/// Standalone-mode stateless passthrough: p50 < 5 ms / p95 < 20 ms (SC-7).
///
/// Each iteration opens a fresh TCP connection (per-connection client, like
/// 001's benchmark), sends a POST `/v1/chat/completions` with the gateway
/// key, and waits for the mock upstream's echo response. The measured
/// latency includes: TCP setup, Pingora pipeline (detect + gateway-key auth
/// + route + upstream-credential inject), mock upstream round-trip, response
/// read. It does NOT include TLS (plain HTTP to a local mock) or DNS
/// (127.0.0.1).
///
/// The assertion is the SC-7 budget (constitution XXI: < 5 ms p50 / < 20 ms
/// p95 for stateless passthrough, excluding upstream RTT). The mock upstream
/// echoes instantly (sub-millisecond), so upstream RTT is negligible and
/// the measurement is effectively "gateway overhead + local TCP".
#[test]
#[ignore = "performance benchmark; run explicitly with --ignored --nocapture"]
fn bench_standalone_passthrough_p50_p95() {
    let gw = Harness::start();
    let auth = format!("Bearer {GATEWAY_KEY}");
    let body = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;
    let request = || Request::post("/v1/chat/completions", body).header("authorization", &auth);

    // Warmup: stabilizes route / connection caches.
    for _ in 0..WARMUP {
        assert_eq!(gw.send(&request()).status, 200, "warmup request failed");
    }

    // Measure.
    let mut samples = Vec::with_capacity(ITERS);
    let wall = Instant::now();
    for _ in 0..ITERS {
        let started = Instant::now();
        let resp = gw.send(&request());
        let elapsed = started.elapsed();
        assert_eq!(resp.status, 200, "measured request failed");
        samples.push(elapsed);
    }
    let total = wall.elapsed();

    samples.sort_unstable();
    let p = |q: usize| samples[(samples.len() * q / 100).min(samples.len() - 1)];
    let rps = ITERS as f64 / total.as_secs_f64();

    println!(
        "PINGOGATE_BENCH_STANDALONE n={ITERS} wall={total:?} rps={rps:.0} \
         p50={:?} p90={:?} p95={:?} p99={:?} max={:?}",
        p(50),
        p(90),
        p(95),
        p(99),
        samples[samples.len() - 1]
    );

    // SC-7 assertion (constitution XXI): p50 < 5 ms, p95 < 20 ms.
    assert!(
        p(50) < Duration::from_millis(5),
        "SC-7 p50 regression: {:?} (budget 5ms)",
        p(50)
    );
    assert!(
        p(95) < Duration::from_millis(20),
        "SC-7 p95 regression: {:?} (budget 20ms)",
        p(95)
    );
}

/// Platform-mode passthrough with per-request KeyVault decrypt + inline usage
/// extraction. TODO: the platform-mode Pingora data plane is not yet wired
/// (T32 documented gap; `run_platform` only runs the gRPC server). When the
/// data plane lands, this test will:
///
/// 1. Start Rust in platform mode + Go control plane + mock upstream
///    (reuse `e2e_m0m1_closure.rs::ClosureEnv`).
/// 2. Push a snapshot with one BYOK provider key (encrypted) + one virtual key
///    (linked to the provider key).
/// 3. Present the vkey, send N requests through the public listener.
/// 4. Measure p50/p95 of the hot path including:
///    - `VirtualKeyAuth::authenticate` (SHA-256 + snapshot lookup, T26).
///    - `KeyVault::decrypt` (AES-GCM, ~0.2 µs per request, constitution XXI).
///    - `inject_include_usage` (T28, OpenAI Chat streaming only).
///    - `UsageExtractor::on_body_chunk` (O(1) memory, T27).
/// 5. Assert p50 < 5 ms / p95 < 20 ms (same budget; decrypt + extraction add
///    < 0.5 ms per the constitution XXI budget analysis).
///
/// The expected result is p50 ≈ 2-3 ms / p95 ≈ 4-6 ms (standalone baseline +
/// ~0.5 ms for decrypt + extraction), well within the 5/20 ms budget. This
/// test is the platform-mode dual of `bench_standalone_passthrough_p50_p95`;
/// together they verify SC-7 holds in both deployment modes (SC-11).
#[test]
#[ignore = "platform-mode data plane not yet wired (T32 gap); TODO when it lands"]
fn bench_platform_passthrough_with_decrypt() {
    eprintln!(
        "bench_platform_passthrough_with_decrypt: SKIPPED - the platform-mode \
         Pingora data plane is not yet wired (T32 documented gap; run_platform \
         only runs the gRPC server). When the data plane lands, this test \
         will measure the hot path including per-request KeyVault decrypt \
         (~0.2 us) + inline UsageExtractor (O(1) memory) and assert SC-7 \
         (p50 < 5ms / p95 < 20ms)."
    );
    // TODO (T32 follow-up): implement the platform-mode benchmark when the
    // Pingora data plane is wired into `run_platform`. See e2e_m0m1_closure.rs
    // for the spawn + snapshot-push harness that this test will reuse.
}
