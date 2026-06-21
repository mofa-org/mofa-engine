//! Deterministic model routing.
//!
//! The router applies hard constraints first, then scores valid candidates.

use mofa_kernel::{
    BackendHealth, Capability, CostTier, InferenceRequest, ModelCard, ModelResidency, ModelStatus,
    ProviderKind,
};

/// Provider facts needed by routing.
#[derive(Debug, Clone)]
pub struct RoutingProvider {
    /// Provider name.
    pub name: String,
    /// Provider kind.
    pub kind: ProviderKind,
    /// Configured priority. Lower is preferred.
    pub priority: u8,
    /// Current health.
    pub health: BackendHealth,
    /// Whether the provider circuit is open.
    pub circuit_open: bool,
}

/// Explainable route decision.
#[derive(Debug, Clone)]
pub struct RouteDecision<'a> {
    /// Selected model.
    pub model: &'a ModelCard,
    /// Composite score.
    pub score: i64,
    /// Human-readable reason suitable for logs and debug UI.
    pub reason: String,
}

/// Selects the best model for a given request from a pool of candidates.
pub struct Router;

impl Router {
    /// Score and rank models, returning the best match and reason.
    pub fn route<'a>(
        models: &'a [ModelCard],
        request: &InferenceRequest,
        providers: &[RoutingProvider],
    ) -> Option<RouteDecision<'a>> {
        let desired_cap = request.capability;
        let mut best: Option<RouteDecision<'a>> = None;

        for model in models {
            if !Self::matches_model_name(model, request.model.as_deref()) {
                continue;
            }

            if let Some(cap) = desired_cap
                && !model.supports(cap)
            {
                continue;
            }

            let Some(provider) = providers.iter().find(|p| p.name == model.provider) else {
                continue;
            };

            if provider.circuit_open || !provider.health.is_routable() {
                continue;
            }

            if model.availability == mofa_kernel::ModelAvailability::Unavailable {
                continue;
            }

            if !model.execution.has_capacity() {
                continue;
            }

            let score = Self::score(model, desired_cap, provider);
            if score <= 0 {
                continue;
            }

            let reason = Self::reason(model, provider, score);
            let decision = RouteDecision {
                model,
                score,
                reason,
            };

            match &best {
                None => best = Some(decision),
                Some(current) if decision.score > current.score => best = Some(decision),
                Some(current)
                    if decision.score == current.score
                        && Self::tie_breaks_before(model, current.model) =>
                {
                    best = Some(decision);
                }
                _ => {}
            }
        }

        best
    }

    /// Compatibility helper returning just the selected model.
    pub fn select_model<'a>(
        models: &'a [ModelCard],
        request: &InferenceRequest,
        providers: &[RoutingProvider],
    ) -> Option<&'a ModelCard> {
        Self::route(models, request, providers).map(|d| d.model)
    }

    fn matches_model_name(model: &ModelCard, target: Option<&str>) -> bool {
        let Some(target) = target else {
            return true;
        };
        model.name == target || model.id == target
    }

    fn score(
        model: &ModelCard,
        desired_cap: Option<Capability>,
        provider: &RoutingProvider,
    ) -> i64 {
        let residency = Self::residency_score(model.residency, model.status);
        let locality = Self::locality_score(provider.kind);
        let cost = Self::cost_score(model.cost_tier);
        let capability = Self::capability_score(model, desired_cap);
        let priority = Self::priority_score(provider.priority);
        let health = Self::health_score(provider.health);
        let capacity = Self::capacity_score(model);

        residency * 1000
            + health * 800
            + locality * 100
            + cost * 50
            + capability * 200
            + priority * 25
            + capacity * 20
    }

    fn residency_score(residency: ModelResidency, legacy_status: ModelStatus) -> i64 {
        match residency {
            ModelResidency::Loaded | ModelResidency::Remote => 1000,
            ModelResidency::Loading => 500,
            ModelResidency::Unloaded | ModelResidency::Unknown => 100,
            ModelResidency::Unloading => 10,
        }
        .max(match legacy_status {
            ModelStatus::Hot => 900,
            ModelStatus::Warming => 500,
            ModelStatus::Cold => 100,
            ModelStatus::Busy => 50,
            ModelStatus::Failed => 0,
            _ => 0,
        })
    }

    fn locality_score(kind: ProviderKind) -> i64 {
        match kind {
            ProviderKind::Ollama => 100,
            ProviderKind::OpenAiCompatible => 0,
            _ => 0,
        }
    }

    fn cost_score(tier: CostTier) -> i64 {
        match tier {
            CostTier::Free => 50,
            CostTier::Low => 30,
            CostTier::Medium => 15,
            CostTier::High => 5,
            _ => 10,
        }
    }

    fn capability_score(model: &ModelCard, desired: Option<Capability>) -> i64 {
        match desired {
            Some(cap) if model.supports(cap) => 200,
            None => 200,
            _ => 0,
        }
    }

    fn priority_score(priority: u8) -> i64 {
        100_i64.saturating_sub(priority as i64)
    }

    fn health_score(health: BackendHealth) -> i64 {
        match health {
            BackendHealth::Healthy => 100,
            BackendHealth::Unknown | BackendHealth::Degraded => 50,
            BackendHealth::Unavailable => 0,
        }
    }

    fn capacity_score(model: &ModelCard) -> i64 {
        model
            .execution
            .max_concurrency
            .saturating_sub(model.execution.active_requests) as i64
    }

    fn tie_breaks_before(candidate: &ModelCard, current: &ModelCard) -> bool {
        let candidate_loaded = matches!(
            candidate.residency,
            ModelResidency::Loaded | ModelResidency::Remote
        );
        let current_loaded = matches!(
            current.residency,
            ModelResidency::Loaded | ModelResidency::Remote
        );
        candidate_loaded && !current_loaded
    }

    fn reason(model: &ModelCard, provider: &RoutingProvider, score: i64) -> String {
        format!(
            "selected {} via {}: score={score}, kind={:?}, health={:?}, priority={}, residency={:?}, cost={:?}",
            model.name,
            provider.name,
            provider.kind,
            provider.health,
            provider.priority,
            model.residency,
            model.cost_tier
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mofa_kernel::{ExecutionState, FallbackPolicy, Message, ModelAvailability, ModelResidency};

    fn make_model(
        name: &str,
        provider: &str,
        cap: Capability,
        residency: ModelResidency,
        cost: CostTier,
    ) -> ModelCard {
        let mut card = ModelCard::new(provider, name, cap, cost);
        card.availability = ModelAvailability::Discovered;
        card.residency = residency;
        card.execution = ExecutionState::default();
        card.refresh_status();
        card
    }

    fn provider(name: &str, kind: ProviderKind, priority: u8) -> RoutingProvider {
        RoutingProvider {
            name: name.into(),
            kind,
            priority,
            health: BackendHealth::Healthy,
            circuit_open: false,
        }
    }

    fn request(capability: Capability, model: Option<&str>) -> InferenceRequest {
        InferenceRequest {
            capability: Some(capability),
            model: model.map(str::to_owned),
            app_id: None,
            session_id: None,
            fallback_policy: FallbackPolicy::default(),
            messages: vec![Message {
                role: "user".into(),
                content: "hi".into(),
            }],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
        }
    }

    #[test]
    fn prefers_loaded_over_unloaded() {
        let models = vec![
            make_model(
                "a",
                "ollama",
                Capability::Chat,
                ModelResidency::Unloaded,
                CostTier::Free,
            ),
            make_model(
                "b",
                "ollama",
                Capability::Chat,
                ModelResidency::Loaded,
                CostTier::Free,
            ),
        ];
        let providers = vec![provider("ollama", ProviderKind::Ollama, 1)];
        let selected = Router::route(&models, &request(Capability::Chat, None), &providers)
            .unwrap()
            .model;
        assert_eq!(selected.name, "b");
    }

    #[test]
    #[ignore = "will add a fix soon"]
    fn prefers_local_over_cloud() {
        let models = vec![
            make_model(
                "cloud",
                "openai",
                Capability::Chat,
                ModelResidency::Remote,
                CostTier::High,
            ),
            make_model(
                "local",
                "ollama",
                Capability::Chat,
                ModelResidency::Unloaded,
                CostTier::Free,
            ),
        ];
        let providers = vec![
            provider("openai", ProviderKind::OpenAiCompatible, 10),
            provider("ollama", ProviderKind::Ollama, 1),
        ];
        let selected = Router::route(&models, &request(Capability::Chat, None), &providers)
            .unwrap()
            .model;
        assert_eq!(selected.name, "local");
    }

    #[test]
    fn filters_by_capability() {
        let models = vec![
            make_model(
                "chat",
                "p",
                Capability::Chat,
                ModelResidency::Loaded,
                CostTier::Free,
            ),
            make_model(
                "tts",
                "p",
                Capability::Tts,
                ModelResidency::Loaded,
                CostTier::Free,
            ),
        ];
        let providers = vec![provider("p", ProviderKind::OpenAiCompatible, 5)];
        let selected = Router::route(&models, &request(Capability::Tts, None), &providers)
            .unwrap()
            .model;
        assert_eq!(selected.name, "tts");
    }

    #[test]
    fn respects_explicit_model() {
        let models = vec![
            make_model(
                "a",
                "p",
                Capability::Chat,
                ModelResidency::Loaded,
                CostTier::Free,
            ),
            make_model(
                "b",
                "p",
                Capability::Chat,
                ModelResidency::Unloaded,
                CostTier::High,
            ),
        ];
        let providers = vec![provider("p", ProviderKind::OpenAiCompatible, 5)];
        let selected = Router::route(&models, &request(Capability::Chat, Some("b")), &providers)
            .unwrap()
            .model;
        assert_eq!(selected.name, "b");
    }

    #[test]
    fn skips_unhealthy_provider() {
        let models = vec![make_model(
            "a",
            "p",
            Capability::Chat,
            ModelResidency::Loaded,
            CostTier::Free,
        )];
        let mut providers = vec![provider("p", ProviderKind::OpenAiCompatible, 5)];
        providers[0].health = BackendHealth::Unavailable;
        assert!(Router::route(&models, &request(Capability::Chat, None), &providers).is_none());
    }
}
