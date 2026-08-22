//! Bounded-cardinality engine metrics and Prometheus text rendering.
//!
//! These counters are the raw signal behind the `/metrics` endpoint. They are
//! deliberately label-free (or labelled only by provider, a small fixed set) so
//! metric cardinality stays bounded regardless of request volume — no per-model,
//! per-request, or user-supplied labels leak into the series.

use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;

/// Upper bounds (in milliseconds) for the request-latency histogram buckets.
/// The final `+Inf` bucket is implicit.
const LATENCY_BUCKETS_MS: [u64; 8] = [50, 100, 250, 500, 1000, 2500, 5000, 10000];

/// Per-provider token and cost aggregation (dual-track observability).
#[derive(Default)]
struct ProviderUsage {
    prompt_tokens: AtomicU64,
    completion_tokens: AtomicU64,
    /// Accumulated cost in micro-USD (millionths of a dollar) to avoid float
    /// atomics; rendered back to USD.
    cost_micro_usd: AtomicU64,
    /// Track this provider serves — `"local"` or `"cloud"` (a provider is always
    /// one or the other). Set on first use; drives the dual-track `locality` label.
    locality: std::sync::OnceLock<&'static str>,
}

/// Process-wide engine counters.
#[derive(Default)]
pub(crate) struct EngineMetrics {
    requests_total: AtomicU64,
    requests_failed: AtomicU64,
    requests_fallback: AtomicU64,
    request_duration_ms_sum: AtomicU64,
    /// Cumulative-per-bucket counts plus a trailing `+Inf` bucket.
    latency_buckets: [AtomicU64; LATENCY_BUCKETS_MS.len() + 1],
    loads_total: AtomicU64,
    unloads_total: AtomicU64,
    evictions_total: AtomicU64,
    idle_unloads_total: AtomicU64,
    load_failures_total: AtomicU64,
    /// Token/cost totals keyed by provider (bounded by the fixed provider set).
    per_provider: DashMap<String, ProviderUsage>,
}

impl EngineMetrics {
    /// Record the outcome of one logical inference request.
    pub(crate) fn record_request(&self, success: bool, duration_ms: u64, fallback_used: bool) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.requests_failed.fetch_add(1, Ordering::Relaxed);
        }
        if fallback_used {
            self.requests_fallback.fetch_add(1, Ordering::Relaxed);
        }
        self.request_duration_ms_sum
            .fetch_add(duration_ms, Ordering::Relaxed);
        // Increment the first bucket whose upper bound covers this observation.
        let mut idx = LATENCY_BUCKETS_MS.len();
        for (i, bound) in LATENCY_BUCKETS_MS.iter().enumerate() {
            if duration_ms <= *bound {
                idx = i;
                break;
            }
        }
        self.latency_buckets[idx].fetch_add(1, Ordering::Relaxed);
    }

    /// Fold a lifecycle event (as recorded in the lifecycle history) into the
    /// relevant lifecycle counter.
    pub(crate) fn record_lifecycle(&self, event: &str) {
        match event {
            "load" => &self.loads_total,
            "unload" => &self.unloads_total,
            "evict" => &self.evictions_total,
            "idle_unload" => &self.idle_unloads_total,
            "load_failed" | "load_timeout" => &self.load_failures_total,
            _ => return,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    /// Record per-provider token usage and cost for one successful request.
    ///
    /// `is_local` tags the series with the dual-track `locality` label so the
    /// dashboard can compare on-device vs cloud spend/throughput (PRD §5.3).
    pub(crate) fn record_usage(
        &self,
        provider: &str,
        is_local: bool,
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
        cost_usd: Option<f64>,
    ) {
        let entry = self.per_provider.entry(provider.to_string()).or_default();
        entry
            .locality
            .get_or_init(|| if is_local { "local" } else { "cloud" });
        if let Some(p) = prompt_tokens {
            entry.prompt_tokens.fetch_add(p as u64, Ordering::Relaxed);
        }
        if let Some(c) = completion_tokens {
            entry
                .completion_tokens
                .fetch_add(c as u64, Ordering::Relaxed);
        }
        if let Some(cost) = cost_usd
            && cost > 0.0
        {
            let micro = (cost * 1_000_000.0).round() as u64;
            entry.cost_micro_usd.fetch_add(micro, Ordering::Relaxed);
        }
    }

    /// Render the Prometheus text-exposition body for these counters.
    ///
    /// `gauges` are point-in-time values pulled from the engine (memory, model
    /// counts, preflight, and per-provider routability) sampled at scrape time.
    pub(crate) fn render_prometheus(&self, gauges: &MetricsGauges) -> String {
        let mut out = String::with_capacity(2048);

        let counter = |out: &mut String, name: &str, help: &str, value: u64| {
            out.push_str(&format!(
                "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}\n"
            ));
        };
        let gauge = |out: &mut String, name: &str, help: &str, value: u64| {
            out.push_str(&format!(
                "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}\n"
            ));
        };

        counter(
            &mut out,
            "mofa_requests_total",
            "Total inference requests handled.",
            self.requests_total.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "mofa_requests_failed_total",
            "Inference requests that returned an error.",
            self.requests_failed.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "mofa_requests_fallback_total",
            "Requests served by a fallback candidate.",
            self.requests_fallback.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "mofa_model_loads_total",
            "Model load operations.",
            self.loads_total.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "mofa_model_unloads_total",
            "Explicit model unload operations.",
            self.unloads_total.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "mofa_model_evictions_total",
            "Models evicted under memory pressure.",
            self.evictions_total.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "mofa_model_idle_unloads_total",
            "Models unloaded by the idle sweep.",
            self.idle_unloads_total.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "mofa_model_load_failures_total",
            "Failed or timed-out model loads.",
            self.load_failures_total.load(Ordering::Relaxed),
        );

        // Latency histogram (cumulative buckets, per Prometheus convention).
        out.push_str(
            "# HELP mofa_request_duration_ms Request latency in milliseconds.\n\
             # TYPE mofa_request_duration_ms histogram\n",
        );
        let mut cumulative = 0u64;
        for (i, bound) in LATENCY_BUCKETS_MS.iter().enumerate() {
            cumulative += self.latency_buckets[i].load(Ordering::Relaxed);
            out.push_str(&format!(
                "mofa_request_duration_ms_bucket{{le=\"{bound}\"}} {cumulative}\n"
            ));
        }
        cumulative += self.latency_buckets[LATENCY_BUCKETS_MS.len()].load(Ordering::Relaxed);
        out.push_str(&format!(
            "mofa_request_duration_ms_bucket{{le=\"+Inf\"}} {cumulative}\n"
        ));
        out.push_str(&format!(
            "mofa_request_duration_ms_sum {}\n",
            self.request_duration_ms_sum.load(Ordering::Relaxed)
        ));
        out.push_str(&format!("mofa_request_duration_ms_count {cumulative}\n"));

        // Point-in-time gauges from the engine.
        gauge(
            &mut out,
            "mofa_models_total",
            "Known models in the registry.",
            gauges.models_total,
        );
        gauge(
            &mut out,
            "mofa_models_loaded",
            "Models currently resident.",
            gauges.models_loaded,
        );
        gauge(
            &mut out,
            "mofa_memory_used_bytes",
            "Reserved model memory in bytes.",
            gauges.memory_used_bytes,
        );
        gauge(
            &mut out,
            "mofa_memory_budget_bytes",
            "Total model memory budget in bytes.",
            gauges.memory_budget_bytes,
        );
        gauge(
            &mut out,
            "mofa_preflight_warms_total",
            "Speculative warm tasks started.",
            gauges.preflight_warms_started,
        );
        gauge(
            &mut out,
            "mofa_preflight_hits_total",
            "Confirmed predictive-warm hits.",
            gauges.preflight_hits,
        );

        // One bounded-cardinality gauge family labelled by provider.
        out.push_str(
            "# HELP mofa_provider_up Provider routability (1 = routable).\n\
             # TYPE mofa_provider_up gauge\n",
        );
        for (provider, up) in &gauges.provider_up {
            out.push_str(&format!(
                "mofa_provider_up{{provider=\"{}\"}} {}\n",
                Self::escape_label_value(provider),
                u8::from(*up)
            ));
        }

        // Dual-track observability: per-provider token and cost totals, tagged by
        // `locality` (local/cloud) so on-device and cloud tracks are comparable.
        out.push_str(
            "# HELP mofa_tokens_total Tokens processed, by provider, locality, and direction.\n\
             # TYPE mofa_tokens_total counter\n",
        );
        for entry in self.per_provider.iter() {
            let provider = Self::escape_label_value(entry.key());
            let locality = entry.locality.get().copied().unwrap_or("unknown");
            out.push_str(&format!(
                "mofa_tokens_total{{provider=\"{provider}\",locality=\"{locality}\",direction=\"prompt\"}} {}\n",
                entry.prompt_tokens.load(Ordering::Relaxed)
            ));
            out.push_str(&format!(
                "mofa_tokens_total{{provider=\"{provider}\",locality=\"{locality}\",direction=\"completion\"}} {}\n",
                entry.completion_tokens.load(Ordering::Relaxed)
            ));
        }
        out.push_str(
            "# HELP mofa_cost_usd_total Estimated spend in USD, by provider and locality.\n\
             # TYPE mofa_cost_usd_total counter\n",
        );
        for entry in self.per_provider.iter() {
            let usd = entry.cost_micro_usd.load(Ordering::Relaxed) as f64 / 1_000_000.0;
            let locality = entry.locality.get().copied().unwrap_or("unknown");
            out.push_str(&format!(
                "mofa_cost_usd_total{{provider=\"{}\",locality=\"{locality}\"}} {usd}\n",
                Self::escape_label_value(entry.key())
            ));
        }

        out
    }
}

impl EngineMetrics {
    /// Escape a Prometheus label value per the exposition format: backslash,
    /// double-quote, and line breaks must be escaped so a provider name cannot
    /// produce malformed output. A carriage return is normalized to `\n` (the
    /// format defines no `\r` escape) so it cannot terminate the line early.
    fn escape_label_value(value: &str) -> String {
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\r', '\n'], "\\n")
    }
}

/// Point-in-time gauge values sampled from the engine at scrape time.
#[derive(Debug, Default, Clone)]
pub(crate) struct MetricsGauges {
    /// Known models in the registry.
    pub models_total: u64,
    /// Models currently resident.
    pub models_loaded: u64,
    /// Reserved model memory in bytes.
    pub memory_used_bytes: u64,
    /// Total model memory budget in bytes.
    pub memory_budget_bytes: u64,
    /// Speculative warm tasks started.
    pub preflight_warms_started: u64,
    /// Confirmed predictive-warm hits.
    pub preflight_hits: u64,
    /// Per-provider routability, `(provider_name, is_routable)`.
    pub provider_up: Vec<(String, bool)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_and_counters_render() {
        let m = EngineMetrics::default();
        m.record_request(true, 30, false); // → le=50 bucket
        m.record_request(false, 1500, true); // → le=2500 bucket, failed + fallback
        m.record_lifecycle("load");
        m.record_lifecycle("idle_unload");

        let gauges = MetricsGauges {
            models_total: 3,
            models_loaded: 1,
            memory_budget_bytes: 1000,
            provider_up: vec![("ollama".into(), true)],
            ..Default::default()
        };
        let text = m.render_prometheus(&gauges);

        assert!(text.contains("mofa_requests_total 2"));
        assert!(text.contains("mofa_requests_failed_total 1"));
        assert!(text.contains("mofa_requests_fallback_total 1"));
        assert!(text.contains("mofa_model_loads_total 1"));
        assert!(text.contains("mofa_model_idle_unloads_total 1"));
        // Cumulative histogram: both observations are <= +Inf.
        assert!(text.contains("mofa_request_duration_ms_bucket{le=\"+Inf\"} 2"));
        assert!(text.contains("mofa_request_duration_ms_count 2"));
        assert!(text.contains("mofa_request_duration_ms_sum 1530"));
        assert!(text.contains("mofa_provider_up{provider=\"ollama\"} 1"));
    }

    #[test]
    fn per_provider_token_and_cost_render() {
        let m = EngineMetrics::default();
        m.record_usage("openai", false, Some(100), Some(40), Some(0.0021));
        m.record_usage("openai", false, Some(50), Some(10), Some(0.0009));
        m.record_usage("ollama", true, Some(30), Some(20), None); // local → no cost

        let text = m.render_prometheus(&MetricsGauges::default());
        assert!(text.contains(
            "mofa_tokens_total{provider=\"openai\",locality=\"cloud\",direction=\"prompt\"} 150"
        ));
        assert!(text.contains(
            "mofa_tokens_total{provider=\"openai\",locality=\"cloud\",direction=\"completion\"} 50"
        ));
        assert!(text.contains(
            "mofa_tokens_total{provider=\"ollama\",locality=\"local\",direction=\"prompt\"} 30"
        ));
        // 0.0021 + 0.0009 = 0.003 USD accumulated for openai; ollama (local) has none.
        assert!(text.contains("mofa_cost_usd_total{provider=\"openai\",locality=\"cloud\"} 0.003"));
        assert!(text.contains("mofa_cost_usd_total{provider=\"ollama\",locality=\"local\"} 0"));
    }

    #[test]
    fn provider_label_values_are_escaped() {
        let m = EngineMetrics::default();
        let gauges = MetricsGauges {
            provider_up: vec![("we\"ird\\name".into(), true)],
            ..Default::default()
        };
        let text = m.render_prometheus(&gauges);
        // The quote and backslash must be escaped so the exposition stays valid.
        assert!(text.contains(r#"mofa_provider_up{provider="we\"ird\\name"} 1"#));
    }

    #[test]
    fn latency_buckets_are_cumulative() {
        let m = EngineMetrics::default();
        m.record_request(true, 10, false); // le=50
        m.record_request(true, 80, false); // le=100
        let text = m.render_prometheus(&MetricsGauges::default());
        // le=50 covers 1, le=100 covers both.
        assert!(text.contains("mofa_request_duration_ms_bucket{le=\"50\"} 1"));
        assert!(text.contains("mofa_request_duration_ms_bucket{le=\"100\"} 2"));
    }
}
