//! # Vendor Pricing Matrix & Cost Engine
//!
//! Calculates estimated USD costs for cloud vendor API calls based on prompt and completion token counts.
//! Local execution providers (e.g., `ollama`, `kokoro`, `funasr`) yield $0.00 by default.

/// Calculates estimated USD cost for a given provider, model, prompt tokens, and completion tokens.
pub fn estimate_cost_usd(
    provider: &str,
    model: &str,
    prompt_tokens: u32,
    completion_tokens: u32,
) -> f64 {
    let provider_lower = provider.to_lowercase();
    let model_lower = model.to_lowercase();

    // Local providers are free ($0.00)
    if provider_lower == "ollama"
        || provider_lower == "kokoro"
        || provider_lower == "funasr"
        || provider_lower == "local"
    {
        return 0.0;
    }

    // Rates per 1,000 tokens: (prompt_rate_per_k, completion_rate_per_k)
    let (prompt_rate, completion_rate) = match (provider_lower.as_str(), model_lower.as_str()) {
        ("openai", m) if m.contains("gpt-4o") => (0.0025, 0.0100),
        ("openai", m) if m.contains("gpt-4") => (0.0300, 0.0600),
        ("openai", m) if m.contains("gpt-3.5") => (0.0005, 0.0015),
        (p, m) if p == "deepseek" || m.contains("deepseek") => (0.00055, 0.00219),
        (p, m) if p == "anthropic" || m.contains("claude") => (0.0030, 0.0150),
        (p, m) if p == "dashscope" || m.contains("qwen") => (0.0028, 0.0084),
        // Generic cloud provider fallback rates
        _ => (0.0020, 0.0060),
    };

    let prompt_cost = (prompt_tokens as f64 / 1000.0) * prompt_rate;
    let completion_cost = (completion_tokens as f64 / 1000.0) * completion_rate;

    prompt_cost + completion_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_provider_is_free() {
        assert_eq!(estimate_cost_usd("ollama", "llama3:8b", 1000, 1000), 0.0);
        assert_eq!(estimate_cost_usd("kokoro", "kokoro", 500, 500), 0.0);
    }

    #[test]
    fn gpt4o_cost_calculation() {
        let cost = estimate_cost_usd("openai", "gpt-4o", 1000, 1000);
        assert!((cost - 0.0125).abs() < 1e-5);
    }

    #[test]
    fn deepseek_cost_calculation() {
        let cost = estimate_cost_usd("deepseek", "deepseek-r1", 1000, 1000);
        assert!((cost - 0.00274).abs() < 1e-5);
    }
}
