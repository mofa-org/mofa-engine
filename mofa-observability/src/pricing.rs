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
    is_local: bool,
) -> f64 {
    let provider_lower = provider.to_lowercase();
    let model_lower = model.to_lowercase();

    // Local providers/backends are free ($0.00)
    if is_local
        || provider_lower == "ollama"
        || provider_lower == "kokoro"
        || provider_lower == "local-tts"
        || provider_lower == "funasr"
        || provider_lower == "local-asr"
        || provider_lower == "local"
        || provider_lower.contains("stable-diffusion")
        || model_lower.contains("kokoro")
        || model_lower.contains("funasr")
        || model_lower.contains("whisper-local")
    {
        return 0.0;
    }

    // Rates per 1,000 tokens / units: (prompt_rate_per_k, completion_rate_per_k)
    let (prompt_rate, completion_rate) = match (provider_lower.as_str(), model_lower.as_str()) {
        // [AUDIO] Speech & Audio Models (TTS & ASR)
        (p, m) if p == "gemini-tts" || m.contains("preview-tts") => (0.000100, 0.000100),
        (_, m) if m.contains("chirp") => (0.01600, 0.01600),
        ("openai", m) if m.contains("tts-1-hd") => (0.03000, 0.03000),
        ("openai", m) if m.contains("tts-1") || m.contains("tts") => (0.01500, 0.01500),
        ("openai", m) if m.contains("whisper-1") || m.contains("whisper-large") => (0.00600, 0.00600),
        ("openai", m) if m.contains("gpt-transcribe") => (0.00450, 0.00450),

        // [IMAGE] Image & Video Generation Models
        ("openai", m) if m.contains("dall-e-3") => (0.04000, 0.04000),
        (_, m) if m.contains("flux-schnell") => (0.00300, 0.00300),
        (_, m) if m.contains("flux-dev") => (0.02500, 0.02500),
        (_, m) if m.contains("veo-3.1-lite") => (0.05000, 0.05000),
        (_, m) if m.contains("veo-3.1-fast") => (0.10000, 0.10000),
        (_, m) if m.contains("veo-3.1") || m.contains("veo") => (0.40000, 0.40000),
        ("openai", m) if m.contains("sora") => (0.30000, 0.30000),

        // [LOCAL] OpenAI Family (From OpenRouter Matrix) - Order: Most specific first!
        ("openai", m) if m.contains("o1-pro") => (0.07500, 0.30000),
        ("openai", m) if m.contains("gpt-5.5-pro") || m.contains("gpt-5.4-pro") => (0.01500, 0.09000),
        ("openai", m) if m.contains("gpt-5.2-pro") => (0.01050, 0.08400),
        ("openai", m) if m.contains("gpt-5-pro") || m.contains("o3-pro") => (0.00750, 0.06000),
        ("openai", m) if m.contains("gpt-5-codex") => (0.000625, 0.00500),
        ("openai", m) if m.contains("o1-mini") || m.contains("o3-mini") || m.contains("o4-mini") => (0.00055, 0.00220),
        ("openai", m) if m.contains("o1") => (0.00750, 0.03000),
        ("openai", m) if m.contains("gpt-4o-mini") => (0.00015, 0.00060),
        ("openai", m) if m.contains("gpt-mini-latest") => (0.00075, 0.00450),
        ("openai", m) if m.contains("gpt-4o") => (0.00250, 0.01000),
        ("openai", m) if m.contains("gpt-latest") => (0.00200, 0.01000),
        ("openai", m) if m.contains("gpt-4-turbo") || m.contains("gpt-4") => (0.01000, 0.03000),
        ("openai", m) if m.contains("gpt-3.5") => (0.00050, 0.00150),
        ("openai", _) => (0.00250, 0.01000),

        // [LOCAL] Anthropic Claude Family (From OpenRouter Matrix)
        (p, m) if (p == "anthropic" || m.contains("claude")) && (m.contains("fable-5") || m.contains("opus-5-fast") || m.contains("opus-4.8-fast")) => (0.01000, 0.05000),
        (p, m) if (p == "anthropic" || m.contains("claude")) && (m.contains("opus-4") && !m.contains("opus-4.") && !m.contains("opus-4.5") && !m.contains("opus-4.6") && !m.contains("opus-4.7") && !m.contains("opus-4.8")) => (0.01500, 0.07500),
        (p, m) if (p == "anthropic" || m.contains("claude")) && (m.contains("opus-5") || m.contains("opus-4.8") || m.contains("opus-4.7") || m.contains("opus-4.6") || m.contains("opus-4.5")) => (0.00500, 0.02500),
        (p, m) if (p == "anthropic" || m.contains("claude")) && m.contains("sonnet-5") => (0.00200, 0.01000),
        (p, m) if (p == "anthropic" || m.contains("claude")) && (m.contains("sonnet-4") || m.contains("3-5-sonnet") || m.contains("3.5-sonnet") || m.contains("sonnet")) => (0.00300, 0.01500),
        (p, m) if (p == "anthropic" || m.contains("claude")) && m.contains("haiku-4.5") => (0.00100, 0.00500),
        (p, m) if (p == "anthropic" || m.contains("claude")) && (m.contains("haiku") || m.contains("3-haiku")) => (0.00025, 0.00125),
        (p, m) if (p == "anthropic" || m.contains("claude")) && m.contains("3-opus") => (0.01500, 0.07500),
        ("anthropic", _) => (0.00300, 0.01500),

        // [LOCAL] DeepSeek Family (From OpenRouter Matrix)
        (p, m) if (p == "deepseek" || m.contains("deepseek")) && m.contains("v4-flash") => (0.000065, 0.000180),
        (p, m) if (p == "deepseek" || m.contains("deepseek")) && m.contains("v4-pro") => (0.000414, 0.000828),
        (p, m) if (p == "deepseek" || m.contains("deepseek")) && (m.contains("v3.2") || m.contains("v3.1") || m.contains("v3")) => (0.000209, 0.000310),
        (p, m) if (p == "deepseek" || m.contains("deepseek")) && (m.contains("reasoner") || m.contains("r1")) => (0.00055, 0.00219),
        (p, m) if (p == "deepseek" || m.contains("deepseek")) && m.contains("chat") => (0.00014, 0.00028),
        (p, m) if p == "deepseek" || m.contains("deepseek") => (0.00055, 0.00219),

        // [LOCAL] Google Gemini Family (From OpenRouter Matrix)
        (p, m) if (p.contains("gemini") || m.contains("gemini")) && (m.contains("flash") || m.contains("3.7-flash") || m.contains("2.5-flash") || m.contains("2.0-flash") || m.contains("1.5-flash")) => (0.000075, 0.000300),
        (p, m) if (p.contains("gemini") || m.contains("gemini")) && (m.contains("3-flash-preview")) => (0.000250, 0.001500),
        (p, m) if (p.contains("gemini") || m.contains("gemini")) && (m.contains("pro-latest") || m.contains("3.1-pro") || m.contains("2.5-pro") || m.contains("1.5-pro")) => (0.001250, 0.005000),
        (_p, m) if m.contains("gemma-2-27b") => (0.000650, 0.000650),
        (p, m) if p.contains("gemini") || m.contains("gemini") => (0.000100, 0.000400),

        // [LOCAL] Open Weights Cloud Hostings (Fireworks, Together, Groq)
        (_p, m) if m.contains("llama-3.3-70b") || m.contains("llama-3.1-70b") || m.contains("llama-3-70b") => (0.00090, 0.00090),
        (_p, m) if m.contains("llama-3.1-8b") || m.contains("llama-3-8b") => (0.00020, 0.00020),
        (p, m) if p == "dashscope" || m.contains("qwen") => (0.00280, 0.00840),

        // Generic cloud provider fallback rates
        _ => (0.00200, 0.00600),
    };

    let prompt_cost = (prompt_tokens as f64 / 1000.0) * prompt_rate;
    let completion_cost = (completion_tokens as f64 / 1000.0) * completion_rate;

    prompt_cost + completion_cost
}

/// Calculate estimated USD cost with optional custom override rates `(prompt_rate_per_k, completion_rate_per_k)`.
pub fn estimate_cost_usd_with_override(
    provider: &str,
    model: &str,
    prompt_tokens: u32,
    completion_tokens: u32,
    is_local: bool,
    override_rate: Option<(f64, f64)>,
) -> f64 {
    if is_local {
        return 0.0;
    }
    if let Some((prompt_rate, completion_rate)) = override_rate {
        let prompt_cost = (prompt_tokens as f64 / 1000.0) * prompt_rate;
        let completion_cost = (completion_tokens as f64 / 1000.0) * completion_rate;
        return prompt_cost + completion_cost;
    }
    estimate_cost_usd(provider, model, prompt_tokens, completion_tokens, is_local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_provider_is_free() {
        assert_eq!(
            estimate_cost_usd("ollama", "llama3:8b", 1000, 1000, true),
            0.0
        );
        assert_eq!(estimate_cost_usd("kokoro", "kokoro", 500, 500, true), 0.0);
        assert_eq!(estimate_cost_usd("funasr", "paraformer", 500, 500, false), 0.0);
    }

    #[test]
    fn gpt4o_cost_calculation() {
        let cost = estimate_cost_usd("openai", "gpt-4o", 1000, 1000, false);
        assert!((cost - 0.0125).abs() < 1e-5);
    }

    #[test]
    fn gpt4o_mini_cost_calculation() {
        let cost = estimate_cost_usd("openai", "gpt-4o-mini", 1000, 1000, false);
        assert!((cost - 0.00075).abs() < 1e-5);
    }

    #[test]
    fn claude_sonnet_cost_calculation() {
        let cost = estimate_cost_usd("anthropic", "claude-3-5-sonnet", 1000, 1000, false);
        assert!((cost - 0.0180).abs() < 1e-5);
    }

    #[test]
    fn deepseek_cost_calculation() {
        let cost = estimate_cost_usd("deepseek", "deepseek-r1", 1000, 1000, false);
        assert!((cost - 0.00274).abs() < 1e-5);

        let chat_cost = estimate_cost_usd("deepseek", "deepseek-chat", 1000, 1000, false);
        assert!((chat_cost - 0.00042).abs() < 1e-5);
    }

    #[test]
    fn gemini_flash_cost_calculation() {
        let cost = estimate_cost_usd("gemini", "gemini-2.5-flash", 1000, 1000, false);
        assert!((cost - 0.000375).abs() < 1e-6);

        let flash37_cost = estimate_cost_usd("gemini", "gemini-3.7-flash", 1000, 1000, false);
        assert!((flash37_cost - 0.000375).abs() < 1e-6);
    }

    #[test]
    fn new_cloud_models_pricing_verification() {
        // Claude Opus 5 ($5 / $25 per M => 0.005 / 0.025 per k)
        let opus_cost = estimate_cost_usd("anthropic", "claude-opus-5", 1000, 1000, false);
        assert!((opus_cost - 0.0300).abs() < 1e-5);

        // DeepSeek V4 Flash ($0.065 / $0.18 per M => 0.000065 / 0.000180 per k)
        let v4_cost = estimate_cost_usd("deepseek", "deepseek-v4-flash-0731", 1000, 1000, false);
        assert!((v4_cost - 0.000245).abs() < 1e-6);

        // OpenAI GPT-5.5 Pro ($15 / $90 per M => 0.015 / 0.090 per k)
        let gpt5_cost = estimate_cost_usd("openai", "gpt-5.5-pro", 1000, 1000, false);
        assert!((gpt5_cost - 0.1050).abs() < 1e-5);
    }

    #[test]
    fn multimodal_models_pricing_verification() {
        // Gemini TTS ($0.000100 / 1k chars)
        let tts_cost = estimate_cost_usd("gemini-tts", "gemini-2.5-flash-preview-tts", 1000, 0, false);
        assert!((tts_cost - 0.000100).abs() < 1e-6);

        // OpenAI Whisper ($0.006 / min)
        let whisper_cost = estimate_cost_usd("openai", "whisper-1", 1000, 0, false);
        assert!((whisper_cost - 0.006000).abs() < 1e-6);

        // DALL-E 3 ($0.040 / image)
        let dalle_cost = estimate_cost_usd("openai", "dall-e-3", 1000, 0, false);
        assert!((dalle_cost - 0.040000).abs() < 1e-6);

        // Google Veo 3.1 ($0.40 / video)
        let veo_cost = estimate_cost_usd("google", "veo-3.1", 1000, 0, false);
        assert!((veo_cost - 0.400000).abs() < 1e-6);

        // Local Kokoro TTS and FunASR are free ($0.00)
        assert_eq!(estimate_cost_usd("local-tts", "kokoro", 1000, 1000, true), 0.0);
        assert_eq!(estimate_cost_usd("local-asr", "funasr", 1000, 1000, true), 0.0);
    }
}
