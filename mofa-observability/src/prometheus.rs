//! # Prometheus Text Exposition Renderer
//!
//! Converts the in-memory `MetricsState` into valid Prometheus text format.
//!
//! Reference: https://prometheus.io/docs/instrumenting/exposition_formats/
//!
//! The renderer is a pure function: `MetricsState` in, `String` out.
//! No I/O. Minimal allocations for sorting and formatting.

use crate::collector::{CounterFamily, GaugeFamily, HistogramFamily, Labels, MetricsState};
use std::fmt::Write;

// ─── Label Formatting ────────────────────────────────────────────────────────

/// Format labels into Prometheus `{key="value",key2="value2"}` syntax.
/// Returns empty string if there are no labels.
fn format_labels(labels: &Labels) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let mut out = String::from("{");
    for (i, (key, value)) in labels.pairs().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        // Escape backslash, double-quote, and newline in label values.
        let escaped = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        write!(out, "{}=\"{}\"", key, escaped).unwrap();
    }
    out.push('}');
    out
}

/// Format labels with an extra label appended (used for histogram `le` buckets).
/// The extra label is added after existing labels.
fn format_labels_with_extra(labels: &Labels, extra_key: &str, extra_value: &str) -> String {
    let mut out = String::from("{");
    for (key, value) in labels.pairs() {
        let escaped = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        write!(out, "{}=\"{}\",", key, escaped).unwrap();
    }
    write!(out, "{}=\"{}\"", extra_key, extra_value).unwrap();
    out.push('}');
    out
}

// ─── Metric Rendering ────────────────────────────────────────────────────────

/// Render a counter family into Prometheus text format.
fn render_counter(buf: &mut String, family: &CounterFamily) {
    if family.values.is_empty() {
        return;
    }
    writeln!(buf, "# HELP {} {}", family.name, family.help).unwrap();
    writeln!(buf, "# TYPE {} counter", family.name).unwrap();

    // Sort label sets for deterministic output without allocations.
    let mut entries: Vec<_> = family.values.iter().collect();
    entries.sort_by(|a, b| a.0.pairs().cmp(b.0.pairs()));

    for (labels, value) in entries {
        writeln!(buf, "{}{} {}", family.name, format_labels(labels), value).unwrap();
    }
    buf.push('\n');
}

/// Render a gauge family into Prometheus text format.
fn render_gauge(buf: &mut String, family: &GaugeFamily) {
    if family.values.is_empty() {
        return;
    }
    writeln!(buf, "# HELP {} {}", family.name, family.help).unwrap();
    writeln!(buf, "# TYPE {} gauge", family.name).unwrap();

    let mut entries: Vec<_> = family.values.iter().collect();
    entries.sort_by(|a, b| a.0.pairs().cmp(b.0.pairs()));

    for (labels, value) in entries {
        // Render integer values without decimal point for cleanliness.
        if *value == value.floor() && value.is_finite() {
            writeln!(
                buf,
                "{}{} {}",
                family.name,
                format_labels(labels),
                *value as i64
            )
            .unwrap();
        } else {
            writeln!(buf, "{}{} {}", family.name, format_labels(labels), format_float(*value)).unwrap();
        }
    }
    buf.push('\n');
}

/// Render a histogram family into Prometheus text format.
///
/// Each histogram produces:
/// - One line per bucket: `name_bucket{...,le="bound"} count`
/// - A `+Inf` bucket line
/// - A `name_sum{...} value` line
/// - A `name_count{...} value` line
fn render_histogram(buf: &mut String, family: &HistogramFamily) {
    if family.values.is_empty() {
        return;
    }
    writeln!(buf, "# HELP {} {}", family.name, family.help).unwrap();
    writeln!(buf, "# TYPE {} histogram", family.name).unwrap();

    let mut entries: Vec<_> = family.values.iter().collect();
    entries.sort_by(|a, b| a.0.pairs().cmp(b.0.pairs()));

    for (labels, hist) in entries {
        // Render each bucket.
        for (bound, count) in &hist.buckets {
            // Format the bound: remove trailing zeros for cleanliness.
            let bound_str = format_float(*bound);
            writeln!(
                buf,
                "{}_bucket{} {}",
                family.name,
                format_labels_with_extra(labels, "le", &bound_str),
                count
            )
            .unwrap();
        }

        // +Inf bucket (always equals total count).
        writeln!(
            buf,
            "{}_bucket{} {}",
            family.name,
            format_labels_with_extra(labels, "le", "+Inf"),
            hist.count
        )
        .unwrap();

        // Sum and count.
        writeln!(
            buf,
            "{}_sum{} {}",
            family.name,
            format_labels(labels),
            format_float(hist.sum)
        )
        .unwrap();
        writeln!(
            buf,
            "{}_count{} {}",
            family.name,
            format_labels(labels),
            hist.count
        )
        .unwrap();
    }
    buf.push('\n');
}

/// Format a float for Prometheus output.
/// Handles special values (+Inf, -Inf, NaN) strictly per the Prometheus spec.
fn format_float(v: f64) -> String {
    if v.is_nan() {
        "NaN".to_string()
    } else if v == f64::INFINITY {
        "+Inf".to_string()
    } else if v == f64::NEG_INFINITY {
        "-Inf".to_string()
    } else if v == v.floor() && v.is_finite() {
        // Whole number — render with one decimal place (e.g., "1.0", "10.0").
        format!("{:.1}", v)
    } else {
        // Has fractional part — render naturally.
        format!("{}", v)
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Render the full metrics state into Prometheus text exposition format.
///
/// The output is suitable for serving at a `/metrics` HTTP endpoint.
/// Prometheus can scrape this directly.
///
/// # Example
///
/// ```ignore
/// let state = collector.state().read().await;
/// let output = prometheus::render(&state);
/// // Return `output` as text/plain from your HTTP handler.
/// ```
pub fn render(state: &MetricsState) -> String {
    let mut buf = String::with_capacity(4096);

    // ── Counters ─────────────────────────────────────────────────────────
    render_counter(&mut buf, &state.requests_total);
    render_counter(&mut buf, &state.model_loads_total);
    render_counter(&mut buf, &state.model_unloads_total);
    render_counter(&mut buf, &state.failovers_total);
    render_counter(&mut buf, &state.evictions_total);
    render_counter(&mut buf, &state.preflight_predictions_total);
    render_counter(&mut buf, &state.preflight_hits_total);
    render_counter(&mut buf, &state.preflight_misses_total);
    render_counter(&mut buf, &state.tokens_input_total);
    render_counter(&mut buf, &state.tokens_output_total);
    render_counter(&mut buf, &state.events_dropped_total);

    // ── Histograms ───────────────────────────────────────────────────────
    render_histogram(&mut buf, &state.request_duration_seconds);
    render_histogram(&mut buf, &state.model_load_seconds);
    render_histogram(&mut buf, &state.ttft_seconds);

    // ── Gauges ───────────────────────────────────────────────────────────
    render_gauge(&mut buf, &state.memory_used_bytes);
    render_gauge(&mut buf, &state.memory_budget_bytes);
    render_gauge(&mut buf, &state.models_loaded);
    render_gauge(&mut buf, &state.active_requests);

    buf
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::MetricsState;
    use crate::events::*;

    #[test]
    fn test_empty_state_renders_empty() {
        let state = MetricsState::new();
        let output = render(&state);
        // Empty state has no values, so no output.
        assert!(output.is_empty(), "Expected empty output, got:\n{}", output);
    }

    #[test]
    fn test_counter_format() {
        let mut state = MetricsState::new();
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "qwen2.5:7b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 15000,
                ttft_ms: None,
                tokens_in: None,
                tokens_out: None,
                model_was_hot: None,
                success: true,
                error_code: None,
            },
        )));

        let output = render(&state);

        // Must contain HELP and TYPE headers.
        assert!(output.contains("# HELP mofa_requests_total Total inference requests"));
        assert!(output.contains("# TYPE mofa_requests_total counter"));
        // Must contain the labeled counter value.
        assert!(output.contains("mofa_requests_total{capability=\"chat\",status=\"success\"} 1"));
    }

    #[test]
    fn test_gauge_format() {
        let mut state = MetricsState::new();
        state.process_event(&EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            load_duration_ms: 1532,
            memory_bytes: 4_700_000_000,
        })));

        let output = render(&state);

        assert!(output.contains("# HELP mofa_models_loaded Number of models currently loaded"));
        assert!(output.contains("# TYPE mofa_models_loaded gauge"));
        assert!(output.contains("mofa_models_loaded 1"));
        assert!(output.contains("mofa_memory_used_bytes 4700000000"));
    }

    #[test]
    fn test_histogram_format() {
        let mut state = MetricsState::new();
        state.process_event(&EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            load_duration_ms: 1532,
            memory_bytes: 4_700_000_000,
        })));

        let output = render(&state);

        // Must have TYPE histogram.
        assert!(output.contains("# TYPE mofa_model_load_seconds histogram"));
        // Must have bucket lines with le labels.
        assert!(output.contains("mofa_model_load_seconds_bucket{backend=\"ollama\",model=\"qwen2.5:7b\",le=\"0.5\"} 0"));
        assert!(output.contains("mofa_model_load_seconds_bucket{backend=\"ollama\",model=\"qwen2.5:7b\",le=\"2.0\"} 1"));
        // Must have +Inf bucket.
        assert!(output.contains("mofa_model_load_seconds_bucket{backend=\"ollama\",model=\"qwen2.5:7b\",le=\"+Inf\"} 1"));
        // Must have sum and count.
        assert!(output.contains("mofa_model_load_seconds_sum{backend=\"ollama\",model=\"qwen2.5:7b\"} 1.532"));
        assert!(output.contains("mofa_model_load_seconds_count{backend=\"ollama\",model=\"qwen2.5:7b\"} 1"));
    }

    #[test]
    fn test_labels_sorted_alphabetically() {
        let mut state = MetricsState::new();
        // RequestCompleted generates labels: capability, status.
        // These should appear in alphabetical order: capability before status.
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "test".into(),
                backend: "ollama".into(),
                capability: Capability::Tts,
                duration_ms: 1000,
                ttft_ms: None,
                tokens_in: None,
                tokens_out: None,
                model_was_hot: None,
                success: true,
                error_code: None,
            },
        )));

        let output = render(&state);
        // Labels must be sorted: capability comes before status.
        assert!(output.contains("{capability=\"tts\",status=\"success\"}"));
    }

    #[test]
    fn test_label_value_escaping() {
        let mut state = MetricsState::new();
        // Simulate a model ID with a quote character to test escaping.
        state.process_event(&EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "model\"with\"quotes".into(),
            backend: "test\\backend".into(),
            capability: Capability::Chat,
            load_duration_ms: 1000,
            memory_bytes: 1000,
        })));

        let output = render(&state);
        // Quotes and backslashes must be escaped.
        assert!(output.contains("model\\\"with\\\"quotes"));
        assert!(output.contains("test\\\\backend"));
    }

    #[test]
    fn test_multiple_label_combinations() {
        let mut state = MetricsState::new();

        // Chat success
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "qwen".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 1000,
                ttft_ms: None,
                tokens_in: None,
                tokens_out: None,
                model_was_hot: None,
                success: true,
                error_code: None,
            },
        )));

        // TTS error
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "kokoro".into(),
                backend: "ollama".into(),
                capability: Capability::Tts,
                duration_ms: 500,
                ttft_ms: None,
                tokens_in: None,
                tokens_out: None,
                model_was_hot: None,
                success: false,
                error_code: Some("timeout".into()),
            },
        )));

        let output = render(&state);

        // Both label combinations must appear.
        assert!(output.contains("{capability=\"chat\",status=\"success\"} 1"));
        assert!(output.contains("{capability=\"tts\",status=\"error\"} 1"));
    }

    #[test]
    fn test_full_render_is_valid_prometheus() {
        let mut state = MetricsState::new();

        // Generate a mix of events.
        state.process_event(&EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            load_duration_ms: 1532,
            memory_bytes: 4_700_000_000,
        })));
        state.process_event(&EventEnvelope::now(EngineEvent::RequestReceived(
            RequestReceived {
                capability: Capability::Chat,
                model: None,
                hint: None,
            },
        )));
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "qwen2.5:7b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 15000,
                ttft_ms: Some(115),
                tokens_in: Some(50),
                tokens_out: Some(277),
                model_was_hot: Some(true),
                success: true,
                error_code: None,
            },
        )));
        state.process_event(&EventEnvelope::now(EngineEvent::PreflightHit(
            PreflightHit {
                predicted_capability: Capability::Tts,
                cold_start_avoided_ms: 2404,
            },
        )));

        let output = render(&state);

        // Basic validity checks:
        // 1. Every HELP line is followed by a TYPE line.
        let lines: Vec<&str> = output.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.starts_with("# HELP") {
                assert!(
                    i + 1 < lines.len() && lines[i + 1].starts_with("# TYPE"),
                    "HELP line not followed by TYPE: {}",
                    line
                );
            }
        }

        // 2. No empty metric names.
        for line in &lines {
            if !line.starts_with('#') && !line.is_empty() {
                assert!(
                    !line.starts_with('{') && !line.starts_with(' '),
                    "Metric line has no name: {}",
                    line
                );
            }
        }

        // 3. Contains expected sections.
        assert!(output.contains("mofa_requests_total"));
        assert!(output.contains("mofa_model_load_seconds"));
        assert!(output.contains("mofa_memory_used_bytes"));
        assert!(output.contains("mofa_preflight_hits_total"));
    }

    #[test]
    fn test_no_labels_metric_has_no_braces() {
        let mut state = MetricsState::new();
        state.process_event(&EventEnvelope::now(EngineEvent::EvictionTriggered(
            EvictionTriggered {
                evicted_model: "test".into(),
                memory_before_bytes: 100,
                memory_after_bytes: 50,
                budget_bytes: 100,
            },
        )));

        let output = render(&state);
        // The evictions_total counter has no labels.
        // It should render as "mofa_evictions_total 1" without {}.
        assert!(
            output.contains("mofa_evictions_total 1"),
            "Expected no-label metric, got:\n{}",
            output
        );
        // Make sure there's no empty braces.
        assert!(!output.contains("mofa_evictions_total{}"));
    }

    #[test]
    fn test_float_formatting_spec_compliance() {
        let mut state = MetricsState::new();
        state.memory_used_bytes.values.insert(Labels::new(), f64::INFINITY);
        state.active_requests.values.insert(Labels::new(), f64::NAN);
        let output = render(&state);
        assert!(output.contains("mofa_memory_used_bytes +Inf"), "Failed to render +Inf: {}", output);
        assert!(output.contains("mofa_active_requests NaN"), "Failed to render NaN: {}", output);
    }
}
