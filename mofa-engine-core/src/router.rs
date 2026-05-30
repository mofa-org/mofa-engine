//! Multi-dimensional model routing.
//!
//! Scores each candidate model and picks the best match for an inference request.

use mofa_kernel::{Capability, CostTier, InferenceRequest, ModelCard, ModelStatus, ProviderKind};

/// Selects the best model for a given request from a pool of candidates.
pub struct Router;

impl Router {
    /// Score and rank models, returning the best match.
    ///
    /// Returns `None` if no model can serve the request.
    pub fn select<'a>(
        models: &'a [ModelCard],
        request: &InferenceRequest,
        provider_kinds: &[(String, ProviderKind)],
    ) -> Option<&'a ModelCard> {
        let desired_cap = request.capability;

        let mut best: Option<(&ModelCard, i64)> = None;

        for model in models {
            // If a specific model is requested, only consider that model
            if let Some(ref target) = request.model
                && model.name != *target && model.id != *target {
                    continue;
                }

            let score = Self::score(model, desired_cap, provider_kinds);
            if score <= 0 {
                continue;
            }

            match best {
                None => best = Some((model, score)),
                Some((_, best_score)) if score > best_score => {
                    best = Some((model, score));
                }
                Some((current, best_score)) if score == best_score => {
                    // Tie-break: prefer Hot over anything else
                    if model.status == ModelStatus::Hot && current.status != ModelStatus::Hot {
                        best = Some((model, score));
                    }
                }
                _ => {}
            }
        }

        best.map(|(m, _)| m)
    }

    /// Compute a composite score for a model.
    ///
    /// ```text
    /// score = availability * 1000
    ///       + locality * 100
    ///       + cost * 50
    ///       + capability_match * 200
    /// ```
    fn score(
        model: &ModelCard,
        desired_cap: Option<Capability>,
        provider_kinds: &[(String, ProviderKind)],
    ) -> i64 {
        let avail = Self::availability_score(model.status);
        if avail == 0 {
            return 0; // Busy/Failed models are never selected
        }
        let locality = Self::locality_score(&model.provider, provider_kinds);
        let cost = Self::cost_score(model.cost_tier);
        let cap = Self::capability_score(model, desired_cap);

        avail * 1000 + locality * 100 + cost * 50 + cap * 200
    }

    /// Score based on model status.
    fn availability_score(status: ModelStatus) -> i64 {
        match status {
            ModelStatus::Hot => 1000,
            ModelStatus::Warming => 500,
            ModelStatus::Cold => 100,
            ModelStatus::Busy | ModelStatus::Failed => 0,
            _ => 0,
        }
    }

    /// Score based on whether the provider is local or cloud.
    fn locality_score(provider_name: &str, kinds: &[(String, ProviderKind)]) -> i64 {
        let is_local = kinds
            .iter()
            .any(|(n, k)| n == provider_name && *k == ProviderKind::Ollama);
        if is_local { 100 } else { 0 }
    }

    /// Score based on cost tier (cheaper = better).
    fn cost_score(tier: CostTier) -> i64 {
        match tier {
            CostTier::Free => 50,
            CostTier::Low => 30,
            CostTier::Medium => 15,
            CostTier::High => 5,
            _ => 10,
        }
    }

    /// Score based on capability match.
    fn capability_score(model: &ModelCard, desired: Option<Capability>) -> i64 {
        match desired {
            Some(cap) if cap == model.capability => 200,
            None => 200, // No preference = everything matches
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(name: &str, provider: &str, cap: Capability, status: ModelStatus, cost: CostTier) -> ModelCard {
        ModelCard {
            id: format!("{provider}::{name}"),
            name: name.into(),
            provider: provider.into(),
            capability: cap,
            status,
            cost_tier: cost,
            context_window: 4096,
            memory_estimate_bytes: 0,
        }
    }

    #[test]
    fn prefers_hot_over_cold() {
        let models = vec![
            make_model("a", "ollama", Capability::Chat, ModelStatus::Cold, CostTier::Free),
            make_model("b", "ollama", Capability::Chat, ModelStatus::Hot, CostTier::Free),
        ];
        let kinds = vec![("ollama".into(), ProviderKind::Ollama)];
        let req = InferenceRequest {
            capability: Some(Capability::Chat),
            model: None,
            messages: vec![],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
        };
        let selected = Router::select(&models, &req, &kinds).unwrap();
        assert_eq!(selected.name, "b");
    }

    #[test]
    fn prefers_local_over_cloud() {
        let models = vec![
            make_model("cloud", "openai", Capability::Chat, ModelStatus::Cold, CostTier::High),
            make_model("local", "ollama", Capability::Chat, ModelStatus::Cold, CostTier::Free),
        ];
        let kinds = vec![
            ("openai".into(), ProviderKind::OpenAiCompatible),
            ("ollama".into(), ProviderKind::Ollama),
        ];
        let req = InferenceRequest {
            capability: Some(Capability::Chat),
            model: None,
            messages: vec![],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
        };
        let selected = Router::select(&models, &req, &kinds).unwrap();
        assert_eq!(selected.name, "local");
    }

    #[test]
    fn filters_by_capability() {
        let models = vec![
            make_model("chat", "p", Capability::Chat, ModelStatus::Hot, CostTier::Free),
            make_model("tts", "p", Capability::Tts, ModelStatus::Hot, CostTier::Free),
        ];
        let kinds = vec![];
        let req = InferenceRequest {
            capability: Some(Capability::Tts),
            model: None,
            messages: vec![],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
        };
        let selected = Router::select(&models, &req, &kinds).unwrap();
        assert_eq!(selected.name, "tts");
    }

    #[test]
    fn respects_explicit_model() {
        let models = vec![
            make_model("a", "p", Capability::Chat, ModelStatus::Hot, CostTier::Free),
            make_model("b", "p", Capability::Chat, ModelStatus::Cold, CostTier::High),
        ];
        let kinds = vec![];
        let req = InferenceRequest {
            capability: Some(Capability::Chat),
            model: Some("b".into()),
            messages: vec![],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
        };
        let selected = Router::select(&models, &req, &kinds).unwrap();
        assert_eq!(selected.name, "b");
    }

    #[test]
    fn returns_none_for_failed() {
        let models = vec![
            make_model("a", "p", Capability::Chat, ModelStatus::Failed, CostTier::Free),
        ];
        let kinds = vec![];
        let req = InferenceRequest {
            capability: Some(Capability::Chat),
            model: None,
            messages: vec![],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
        };
        assert!(Router::select(&models, &req, &kinds).is_none());
    }
}
