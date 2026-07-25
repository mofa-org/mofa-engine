//! Deterministic model routing.
//!
//! The router applies hard constraints first, then scores valid candidates.

use mofa_kernel::{
    BackendHealth, Capability, CostTier, InferenceRequest, ModelCard, ModelResidency, ModelStatus,
    Prefer, ProviderKind,
};

/// Bonus applied to a candidate whose reasoning tier matches the request's
/// `reasoning.effort`. Sized to sit *below* the locality term (≈10 000, so
/// local-first still holds) yet *above* the cost/priority terms (≤2 500 each, so
/// effort selects the right tier among same-locality candidates).
const TIER_MATCH_BONUS: i64 = 5_000;

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
    ///
    /// Equivalent to the first entry of [`route_ranked`] with no memory budget.
    pub fn route<'a>(
        models: &'a [ModelCard],
        request: &InferenceRequest,
        providers: &[RoutingProvider],
    ) -> Option<RouteDecision<'a>> {
        Self::route_ranked(models, request, providers, None)
            .into_iter()
            .next()
    }

    /// Apply hard constraints, then score and rank every valid candidate,
    /// best first. The engine uses the same ordered plan for both primary
    /// selection and failover so the two never diverge.
    ///
    /// `budget_bytes` enables a static memory-feasibility filter: a local model
    /// whose estimated footprint exceeds the entire budget can never be admitted
    /// even after evicting everything else, so it is dropped here rather than
    /// failing later at load time. Pass `None` to skip this filter.
    pub fn route_ranked<'a>(
        models: &'a [ModelCard],
        request: &InferenceRequest,
        providers: &[RoutingProvider],
        budget_bytes: Option<u64>,
    ) -> Vec<RouteDecision<'a>> {
        let desired_cap = request.capability;
        let mut decisions: Vec<RouteDecision<'a>> = Vec::new();

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

            // Hard backend-locality constraint (privacy / data-residency
            // guardrail). `prefer=local` — and any `confidential` request — must
            // never leave the device; `prefer=cloud` restricts to cloud. When
            // filtering leaves no candidate the engine turns the empty result into
            // a typed `NoCapableModel` (fail-not-fallback). Confidential wins over
            // a conflicting `prefer=cloud`: privacy is never traded for preference.
            let force_local =
                request.prefer == Prefer::Local || request.data_class.requires_local();
            if force_local && !provider.kind.is_local() {
                continue;
            }
            if !force_local && request.prefer == Prefer::Cloud && provider.kind.is_local() {
                continue;
            }

            if model.availability == mofa_kernel::ModelAvailability::Unavailable {
                continue;
            }

            // Static memory feasibility: a local model that cannot fit the budget
            // even on an empty device is never a viable candidate. Remote models
            // report a zero footprint and are unaffected.
            if let Some(budget) = budget_bytes
                && model.residency != ModelResidency::Remote
                && model.memory_estimate_bytes > budget
            {
                continue;
            }

            // Under `prefer=auto` (the default), the seven-dimensional score already
            // biases toward local via the locality term while keeping cloud as a
            // fallback; the hard cases above have removed the ineligible candidates.
            let mut score = Self::score(model, desired_cap, provider);
            // Reasoning-tier routing (S2): when the request declares an effort, a
            // model whose configured tier matches gets a bonus placed *below*
            // locality (so local-first still holds) but *above* cost/priority, so
            // effort selects the right tier among same-locality candidates.
            if let Some(reasoning) = request.reasoning
                && model.reasoning_tier == Some(reasoning.effort)
            {
                score = score.saturating_add(TIER_MATCH_BONUS);
            }
            if score <= 0 {
                continue;
            }

            let reason = Self::reason(model, provider, score);
            decisions.push(RouteDecision {
                model,
                score,
                reason,
            });
        }

        // Highest score first; ties resolved toward already-resident models, then
        // by canonical id for a fully deterministic, stable ordering.
        decisions.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| Self::residency_rank(b.model).cmp(&Self::residency_rank(a.model)))
                .then_with(|| a.model.id.cmp(&b.model.id))
        });
        decisions
    }

    /// Compatibility helper returning just the selected model.
    pub fn select_model<'a>(
        models: &'a [ModelCard],
        request: &InferenceRequest,
        providers: &[RoutingProvider],
    ) -> Option<&'a ModelCard> {
        Self::route(models, request, providers).map(|d| d.model)
    }

    /// Tie-break rank: resident models (loaded locally or cloud-backed) rank
    /// above those that would require a cold start.
    fn residency_rank(model: &ModelCard) -> u8 {
        u8::from(matches!(
            model.residency,
            ModelResidency::Loaded | ModelResidency::Remote
        ))
    }

    fn matches_model_name(model: &ModelCard, target: Option<&str>) -> bool {
        let Some(target) = target else {
            return true;
        };
        if model.name == target || model.id == target {
            return true;
        }
        // Also accept the legacy `provider::name` form.
        if let Some((provider, name)) = target.split_once("::") {
            return provider == model.provider && name == model.name;
        }
        false
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
        // Remote models are cloud-backed; score them below local-loaded so that
        // locality and cost can tip routing toward a local provider.
        // We do NOT apply the legacy-status override for Remote because its
        // derived status is always Hot, which would otherwise push it back to 900.
        if residency == ModelResidency::Remote {
            return 50;
        }
        let primary = match residency {
            ModelResidency::Loaded => 1000,
            ModelResidency::Loading => 500,
            ModelResidency::Unloaded | ModelResidency::Unknown => 100,
            ModelResidency::Unloading => 10,
            ModelResidency::Remote => unreachable!(),
        };
        primary.max(match legacy_status {
            ModelStatus::Hot => 900,
            ModelStatus::Warming => 500,
            ModelStatus::Cold => 100,
            ModelStatus::Busy => 50,
            ModelStatus::Failed => 0,
            _ => 0,
        })
    }

    fn locality_score(kind: ProviderKind) -> i64 {
        if kind.is_local() { 100 } else { 0 }
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
        // Lower configured priority is preferred; floor at 0 so a priority above
        // 100 cannot contribute a negative term to the score.
        (100_i64 - priority as i64).max(0)
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
    use mofa_kernel::{
        DataClass, ExecutionState, FallbackPolicy, Message, ModelAvailability, ModelResidency,
        Reasoning, ReasoningEffort,
    };

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
                ..Default::default()
            }],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
            ..Default::default()
        }
    }

    #[test]
    #[ignore = "timing-sensitive perf guard; run explicitly with `--ignored` or as a benchmark"]
    fn routing_decision_meets_latency_target() {
        // RFC budget: a scheduling decision should take < 1ms (excluding load).
        // Wall-clock assertions are sensitive to CI host load, so this is a
        // perf guard run on demand rather than a gating unit test.
        // Build a realistic pool: three providers, several models each.
        let providers = vec![
            provider("ollama", ProviderKind::Ollama, 1),
            provider("openai", ProviderKind::OpenAiCompatible, 10),
            provider("deepseek", ProviderKind::OpenAiCompatible, 8),
        ];
        let mut models = Vec::new();
        for (prov, count) in [("ollama", 8), ("openai", 6), ("deepseek", 6)] {
            for i in 0..count {
                models.push(make_model(
                    &format!("m{i}"),
                    prov,
                    Capability::Chat,
                    if i % 2 == 0 {
                        ModelResidency::Unloaded
                    } else {
                        ModelResidency::Loaded
                    },
                    CostTier::Low,
                ));
            }
        }
        let req = request(Capability::Chat, None);
        let budget = Some(16 * 1024 * 1024 * 1024);

        let iterations = 2000;
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let plan = Router::route_ranked(&models, &req, &providers, budget);
            std::hint::black_box(&plan);
        }
        let avg = start.elapsed() / iterations;
        assert!(
            avg < std::time::Duration::from_millis(1),
            "routing averaged {avg:?}/decision over {} models, exceeding the 1ms target",
            models.len()
        );
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
    fn route_ranked_orders_local_before_cloud() {
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
                ModelResidency::Loaded,
                CostTier::Free,
            ),
        ];
        let providers = vec![
            provider("openai", ProviderKind::OpenAiCompatible, 10),
            provider("ollama", ProviderKind::Ollama, 1),
        ];
        let ranked =
            Router::route_ranked(&models, &request(Capability::Chat, None), &providers, None);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].model.name, "local");
        assert_eq!(ranked[1].model.name, "cloud");
        // Scores are strictly ordered best-first.
        assert!(ranked[0].score >= ranked[1].score);
    }

    #[test]
    fn route_ranked_filters_models_larger_than_budget() {
        let mut big = make_model(
            "big",
            "ollama",
            Capability::Chat,
            ModelResidency::Unloaded,
            CostTier::Free,
        );
        big.memory_estimate_bytes = 32 * 1024 * 1024 * 1024; // 32 GB
        let small = make_model(
            "small",
            "ollama",
            Capability::Chat,
            ModelResidency::Unloaded,
            CostTier::Free,
        );
        let models = vec![big, small];
        let providers = vec![provider("ollama", ProviderKind::Ollama, 1)];

        let budget = Some(8 * 1024 * 1024 * 1024); // 8 GB
        let ranked = Router::route_ranked(
            &models,
            &request(Capability::Chat, None),
            &providers,
            budget,
        );
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].model.name, "small");
    }

    #[test]
    fn route_ranked_budget_does_not_filter_remote() {
        let mut cloud = make_model(
            "cloud",
            "openai",
            Capability::Chat,
            ModelResidency::Remote,
            CostTier::High,
        );
        // A nonsensical estimate must not exclude a cloud model, which holds no local memory.
        cloud.memory_estimate_bytes = u64::MAX;
        let models = vec![cloud];
        let providers = vec![provider("openai", ProviderKind::OpenAiCompatible, 10)];
        let ranked = Router::route_ranked(
            &models,
            &request(Capability::Chat, None),
            &providers,
            Some(1024),
        );
        assert_eq!(ranked.len(), 1);
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

    #[test]
    fn local_only_excludes_cloud_candidates() {
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
                ModelResidency::Loaded,
                CostTier::Free,
            ),
        ];
        let providers = vec![
            provider("openai", ProviderKind::OpenAiCompatible, 10),
            provider("ollama", ProviderKind::Ollama, 1),
        ];
        let mut req = request(Capability::Chat, None);
        req.prefer = Prefer::Local;
        let ranked = Router::route_ranked(&models, &req, &providers, None);
        // Only the local candidate survives the hard constraint.
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].model.name, "local");
    }

    #[test]
    fn local_only_with_no_local_returns_empty() {
        let models = vec![make_model(
            "cloud",
            "openai",
            Capability::Chat,
            ModelResidency::Remote,
            CostTier::High,
        )];
        let providers = vec![provider("openai", ProviderKind::OpenAiCompatible, 10)];
        let mut req = request(Capability::Chat, None);
        req.prefer = Prefer::Local;
        // No local candidate → no route (engine turns this into NoCapableModel).
        assert!(Router::route_ranked(&models, &req, &providers, None).is_empty());
    }

    #[test]
    fn prefer_auto_keeps_both_local_and_cloud_candidates() {
        // The default (`prefer=auto`) applies no hard filter: both a local and a
        // cloud candidate remain routable, with the local one biased ahead via the
        // locality score while cloud stays available as a fallback.
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
                ModelResidency::Loaded,
                CostTier::Free,
            ),
        ];
        let providers = vec![
            provider("openai", ProviderKind::OpenAiCompatible, 10),
            provider("ollama", ProviderKind::Ollama, 1),
        ];
        let mut req = request(Capability::Chat, None);
        req.prefer = Prefer::Auto;
        let ranked = Router::route_ranked(&models, &req, &providers, None);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].model.name, "local");
    }

    #[test]
    fn cloud_only_filters_out_local_candidates() {
        // `prefer=cloud` is the mirror hard constraint: local candidates are dropped
        // entirely, leaving only the cloud model.
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
                ModelResidency::Loaded,
                CostTier::Free,
            ),
        ];
        let providers = vec![
            provider("openai", ProviderKind::OpenAiCompatible, 10),
            provider("ollama", ProviderKind::Ollama, 1),
        ];
        let mut req = request(Capability::Chat, None);
        req.prefer = Prefer::Cloud;
        let ranked = Router::route_ranked(&models, &req, &providers, None);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].model.name, "cloud");
    }

    #[test]
    fn confidential_data_pins_local_even_when_prefer_is_cloud() {
        // S5 moat: a confidential request must never leave the device, and privacy
        // wins over a conflicting `prefer=cloud` — only the local model survives.
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
                ModelResidency::Loaded,
                CostTier::Free,
            ),
        ];
        let providers = vec![
            provider("openai", ProviderKind::OpenAiCompatible, 10),
            provider("ollama", ProviderKind::Ollama, 1),
        ];
        let mut req = request(Capability::Chat, None);
        req.data_class = DataClass::Confidential;
        req.prefer = Prefer::Cloud; // conflicting preference is overridden
        let ranked = Router::route_ranked(&models, &req, &providers, None);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].model.name, "local");
    }

    #[test]
    fn reasoning_effort_routes_to_matching_tier() {
        // Two local chat models identical except for reasoning tier; a high-effort
        // request must select the high-tier model, a low-effort one the low-tier.
        let mut small = make_model(
            "small",
            "ollama",
            Capability::Chat,
            ModelResidency::Loaded,
            CostTier::Free,
        );
        small.reasoning_tier = Some(ReasoningEffort::Low);
        let mut big = make_model(
            "big",
            "ollama",
            Capability::Chat,
            ModelResidency::Loaded,
            CostTier::Free,
        );
        big.reasoning_tier = Some(ReasoningEffort::High);
        let models = vec![small, big];
        let providers = vec![provider("ollama", ProviderKind::Ollama, 1)];

        let mut high = request(Capability::Chat, None);
        high.reasoning = Some(Reasoning {
            effort: ReasoningEffort::High,
            include: false,
        });
        let ranked = Router::route_ranked(&models, &high, &providers, None);
        assert_eq!(ranked[0].model.name, "big");

        let mut low = request(Capability::Chat, None);
        low.reasoning = Some(Reasoning {
            effort: ReasoningEffort::Low,
            include: false,
        });
        let ranked = Router::route_ranked(&models, &low, &providers, None);
        assert_eq!(ranked[0].model.name, "small");
    }

    #[test]
    fn confidential_with_no_local_fails_rather_than_leaking() {
        let models = vec![make_model(
            "cloud",
            "openai",
            Capability::Chat,
            ModelResidency::Remote,
            CostTier::High,
        )];
        let providers = vec![provider("openai", ProviderKind::OpenAiCompatible, 10)];
        let mut req = request(Capability::Chat, None);
        req.data_class = DataClass::Confidential;
        // No local candidate → no route (engine turns this into NoCapableModel),
        // never a silent cloud fallback.
        assert!(Router::route_ranked(&models, &req, &providers, None).is_empty());
    }
}
