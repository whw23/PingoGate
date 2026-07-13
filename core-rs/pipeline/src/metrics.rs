//! In-process Prometheus metrics registry (FR-032/FR-033; constitution XIX).
//!
//! Self-contained - no metrics-client dependency (constitution III), mirroring
//! the dependency-free approach used for RFC3339 timestamps. Records request
//! counts by status, end-to-end and upstream latency histograms, and token
//! counts, all aggregated under `provider` + `capability_family` labels (FR-033).
//! [`Metrics::render`] emits the Prometheus text exposition format. No secret is
//! ever a label or value - only routing facets and counts (FR-034).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::Duration;

use pingogate_provider::TokenUsage;

/// Latency histogram bucket upper bounds in seconds (Prometheus cumulative `le`).
const BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Token directions reported, in stable render order.
const DIRECTIONS: [&str; 5] = ["input", "output", "reasoning", "cache_read", "cache_write"];

/// Aggregation key: the FR-033 required label set.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RouteKey {
    provider: String,
    capability_family: String,
}

/// A cumulative-bucket latency histogram.
#[derive(Default)]
struct Histogram {
    buckets: [u64; BUCKETS.len()],
    sum: f64,
    count: u64,
}

impl Histogram {
    fn observe(&mut self, seconds: f64) {
        self.sum += seconds;
        self.count += 1;
        for (slot, edge) in self.buckets.iter_mut().zip(BUCKETS.iter()) {
            if seconds <= *edge {
                *slot += 1;
            }
        }
    }
}

/// Per-route aggregated measurements.
#[derive(Default)]
struct RouteAgg {
    by_status: BTreeMap<u16, u64>,
    request_latency: Histogram,
    upstream_latency: Histogram,
    tokens: BTreeMap<&'static str, u64>,
}

/// One completed request's measurements (keeps [`Metrics::record`] within the
/// 4-parameter limit, constitution).
pub struct RequestRecord {
    pub provider: String,
    pub capability_family: String,
    pub status: u16,
    pub duration: Duration,
    pub upstream_duration: Option<Duration>,
    pub tokens: Option<TokenUsage>,
}

/// The process-wide metrics registry, shared by the data plane (records) and the
/// admin `/metrics` endpoint (renders).
pub struct Metrics {
    routes: Mutex<BTreeMap<RouteKey, RouteAgg>>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            routes: Mutex::new(BTreeMap::new()),
        }
    }

    /// Fold one completed request into the registry. A poisoned lock is ignored
    /// rather than propagated - metrics must never fail a request.
    pub fn record(&self, rec: RequestRecord) {
        let key = RouteKey {
            provider: rec.provider,
            capability_family: rec.capability_family,
        };
        let Ok(mut routes) = self.routes.lock() else {
            return;
        };
        let agg = routes.entry(key).or_default();
        *agg.by_status.entry(rec.status).or_insert(0) += 1;
        agg.request_latency.observe(rec.duration.as_secs_f64());
        if let Some(upstream) = rec.upstream_duration {
            agg.upstream_latency.observe(upstream.as_secs_f64());
        }
        if let Some(tokens) = rec.tokens {
            add_tokens(agg, &tokens);
        }
    }

    /// Render the full registry as Prometheus text exposition.
    pub fn render(&self) -> String {
        let Ok(routes) = self.routes.lock() else {
            return String::new();
        };
        let mut out = String::new();
        render_requests(&mut out, &routes);
        render_histogram(
            &mut out,
            "pingogate_request_duration_seconds",
            &routes,
            |a| &a.request_latency,
        );
        render_histogram(
            &mut out,
            "pingogate_upstream_duration_seconds",
            &routes,
            |a| &a.upstream_latency,
        );
        render_tokens(&mut out, &routes);
        out
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

type Routes = BTreeMap<RouteKey, RouteAgg>;

fn add_tokens(agg: &mut RouteAgg, tokens: &TokenUsage) {
    *agg.tokens.entry("input").or_insert(0) += tokens.input;
    *agg.tokens.entry("output").or_insert(0) += tokens.output;
    if let Some(reasoning) = tokens.reasoning {
        *agg.tokens.entry("reasoning").or_insert(0) += reasoning;
    }
    if let Some(read) = tokens.cache_read {
        *agg.tokens.entry("cache_read").or_insert(0) += read;
    }
    if let Some(write) = tokens.cache_write {
        *agg.tokens.entry("cache_write").or_insert(0) += write;
    }
}

/// Escape a label value per the Prometheus exposition format.
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Render the `provider`/`capability_family` label pair shared by every series.
fn route_labels(key: &RouteKey) -> String {
    format!(
        "provider=\"{}\",capability_family=\"{}\"",
        escape(&key.provider),
        escape(&key.capability_family)
    )
}

fn render_requests(out: &mut String, routes: &Routes) {
    let _ = writeln!(
        out,
        "# HELP pingogate_requests_total Proxied requests by status."
    );
    let _ = writeln!(out, "# TYPE pingogate_requests_total counter");
    for (key, agg) in routes {
        for (status, count) in &agg.by_status {
            let _ = writeln!(
                out,
                "pingogate_requests_total{{{},status=\"{status}\"}} {count}",
                route_labels(key)
            );
        }
    }
}

fn render_histogram(
    out: &mut String,
    name: &str,
    routes: &Routes,
    pick: impl Fn(&RouteAgg) -> &Histogram,
) {
    let _ = writeln!(out, "# HELP {name} Latency in seconds.");
    let _ = writeln!(out, "# TYPE {name} histogram");
    for (key, agg) in routes {
        let hist = pick(agg);
        let labels = route_labels(key);
        for (slot, edge) in hist.buckets.iter().zip(BUCKETS.iter()) {
            let _ = writeln!(out, "{name}_bucket{{{labels},le=\"{edge}\"}} {slot}");
        }
        let _ = writeln!(out, "{name}_bucket{{{labels},le=\"+Inf\"}} {}", hist.count);
        let _ = writeln!(out, "{name}_sum{{{labels}}} {}", hist.sum);
        let _ = writeln!(out, "{name}_count{{{labels}}} {}", hist.count);
    }
}

fn render_tokens(out: &mut String, routes: &Routes) {
    let _ = writeln!(
        out,
        "# HELP pingogate_tokens_total Tokens from upstream usage by direction."
    );
    let _ = writeln!(out, "# TYPE pingogate_tokens_total counter");
    for (key, agg) in routes {
        for direction in DIRECTIONS {
            if let Some(total) = agg.tokens.get(direction) {
                let _ = writeln!(
                    out,
                    "pingogate_tokens_total{{{},direction=\"{direction}\"}} {total}",
                    route_labels(key)
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(metrics: &Metrics) {
        metrics.record(RequestRecord {
            provider: "openai-up".to_string(),
            capability_family: "generation.stateless".to_string(),
            status: 200,
            duration: Duration::from_millis(12),
            upstream_duration: Some(Duration::from_millis(8)),
            tokens: Some(TokenUsage {
                input: 11,
                output: 22,
                reasoning: None,
                cache_read: None,
                cache_write: None,
            }),
        });
    }

    #[test]
    fn render_carries_provider_and_capability_family_labels() {
        let metrics = Metrics::new();
        sample(&metrics);
        let out = metrics.render();
        assert!(out.contains("provider=\"openai-up\""));
        assert!(out.contains("capability_family=\"generation.stateless\""));
        assert!(out.contains("pingogate_requests_total"));
        assert!(out.contains("status=\"200\""));
    }

    #[test]
    fn render_emits_latency_histograms_and_tokens() {
        let metrics = Metrics::new();
        sample(&metrics);
        let out = metrics.render();
        assert!(out.contains("pingogate_request_duration_seconds_count{provider=\"openai-up\",capability_family=\"generation.stateless\"} 1"));
        assert!(out.contains("pingogate_upstream_duration_seconds_bucket"));
        assert!(out.contains("pingogate_tokens_total{provider=\"openai-up\",capability_family=\"generation.stateless\",direction=\"input\"} 11"));
        assert!(out.contains("direction=\"output\"} 22"));
    }

    #[test]
    fn counts_accumulate_across_requests() {
        let metrics = Metrics::new();
        sample(&metrics);
        sample(&metrics);
        let out = metrics.render();
        assert!(out.contains("pingogate_requests_total{provider=\"openai-up\",capability_family=\"generation.stateless\",status=\"200\"} 2"));
    }
}
