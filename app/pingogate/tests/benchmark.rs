//! Phase 7 polish (T064): throughput / latency benchmark against a stub upstream.
//!
//! Marked `#[ignore]` so it never runs in the normal `cargo test` gate (latency
//! assertions are inherently machine-dependent and would flake CI). Run it
//! explicitly to record p50/p95:
//!
//! ```bash
//! cargo test --release --test benchmark -- --ignored --nocapture
//! ```
//!
//! It drives sequential requests through the gateway to the echo mock and reports
//! the percentile latencies plus throughput. The recorded numbers — not the loose
//! guard assertion — are the deliverable (see `benchmark-results.md`).
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

mod common;

use std::time::{Duration, Instant};

use common::client::Request;
use common::{Harness, GATEWAY_KEY};

/// Requests discarded before measuring, to warm connection/route caches.
const WARMUP: usize = 200;
/// Measured requests.
const ITERS: usize = 3000;

#[test]
#[ignore = "performance benchmark; run explicitly with --ignored --nocapture"]
fn latency_percentiles_against_stub_upstream() {
    let gw = Harness::start();
    let auth = format!("Bearer {GATEWAY_KEY}");
    let body = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;
    let request = || Request::post("/v1/chat/completions", body).header("authorization", &auth);

    for _ in 0..WARMUP {
        assert_eq!(gw.send(&request()).status, 200);
    }

    let mut samples = Vec::with_capacity(ITERS);
    let wall = Instant::now();
    for _ in 0..ITERS {
        let started = Instant::now();
        let resp = gw.send(&request());
        let elapsed = started.elapsed();
        assert_eq!(resp.status, 200);
        samples.push(elapsed);
    }
    let total = wall.elapsed();

    samples.sort_unstable();
    let p = |q: usize| samples[(samples.len() * q / 100).min(samples.len() - 1)];
    let rps = ITERS as f64 / total.as_secs_f64();

    println!("PINGOGATE_BENCH n={ITERS} wall={total:?} rps={rps:.0}");
    println!(
        "PINGOGATE_BENCH p50={:?} p90={:?} p95={:?} p99={:?} max={:?}",
        p(50),
        p(90),
        p(95),
        p(99),
        samples[samples.len() - 1]
    );

    // Loose guard only — the per-connection client (fresh TCP per request) is an
    // upper bound on gateway overhead, so this must not encode the tight target.
    assert!(
        p(95) < Duration::from_millis(200),
        "p95 latency regressed badly: {:?}",
        p(95)
    );
}
