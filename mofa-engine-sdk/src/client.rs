//! Native Rust SDK: embedded and daemon clients.
//!
//! The RFC feedback separates two deployment modes that the prototype conflated:
//!
//! - **Embedded mode** — the engine runs in the caller's process. [`EmbeddedEngine`]
//!   is a small *synchronous* facade over an internally managed Tokio runtime,
//!   so a non-async caller (including a UniFFI-generated Python binding) can drive
//!   the engine without owning a runtime. This is the intended UniFFI target.
//! - **Daemon mode** — the engine runs as a separate process and is reached over
//!   the versioned HTTP API. [`DaemonClient`] is a typed client for that surface.
//!
//! Both speak the same domain request/response types from `mofa-kernel`, so a
//! caller can switch modes without changing how it constructs requests.

use std::sync::Arc;
use std::time::Duration;

use mofa_engine_core::engine::{LifecycleRecord, MemoryReport};
use mofa_engine_core::preflight::PreflightStats;
use mofa_engine_core::subscription::SubscriptionInfo;
use mofa_engine_core::{Engine, EngineConfig};
use mofa_kernel::{
    Capability, EngineError, EngineStatus, ErrorInfo, InferenceRequest, InferenceResponse, Message,
    ModelCard, ResponsesRequest, ResponsesResponse,
};

/// Errors returned by [`DaemonClient`].
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Network/transport failure reaching the daemon.
    #[error("transport error: {0}")]
    Transport(String),
    /// The daemon returned a structured engine error.
    #[error("engine error: {}", .0.message)]
    Engine(ErrorInfo),
    /// The daemon returned a non-success status without a structured body
    /// (e.g. a 401/404/413 from the framework rather than the handler).
    #[error("http {status}: {body}")]
    Status {
        /// HTTP status code.
        status: u16,
        /// Raw response body.
        body: String,
    },
    /// A success response body could not be decoded into the expected type.
    #[error("decode error: {0}")]
    Decode(String),
}

impl ClientError {
    /// The structured engine error, if this was an engine-level failure.
    pub fn engine_error(&self) -> Option<&ErrorInfo> {
        match self {
            Self::Engine(info) => Some(info),
            _ => None,
        }
    }
}

/// A synchronous, in-process facade over the engine for embedded/UniFFI use.
///
/// Owns a multi-threaded Tokio runtime and blocks on each call, so it can be
/// driven from ordinary synchronous code. Background engine tasks (idle
/// eviction, speculative warming) run on the owned runtime for the lifetime of
/// this value.
pub struct EmbeddedEngine {
    runtime: tokio::runtime::Runtime,
    engine: Arc<Engine>,
}

impl EmbeddedEngine {
    /// Build and initialize an embedded engine from configuration.
    pub fn new(config: EngineConfig) -> Result<Self, EngineError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| EngineError::Internal(format!("failed to build runtime: {e}")))?;
        let engine = runtime.block_on(Engine::try_new(config))?;
        Ok(Self { runtime, engine })
    }

    /// List all known model cards.
    pub fn capabilities(&self) -> Vec<ModelCard> {
        self.runtime.block_on(self.engine.capabilities())
    }

    /// Run one inference request to completion.
    pub fn invoke(&self, request: InferenceRequest) -> Result<InferenceResponse, EngineError> {
        self.runtime.block_on(self.engine.invoke(request))
    }

    /// Run one turn of the stateful Responses API (multi-turn deep reasoning).
    /// The returned [`ResponsesResponse::id`] is passed as `previous_response_id`
    /// to continue the conversation on the next turn.
    pub fn respond(&self, request: ResponsesRequest) -> Result<ResponsesResponse, EngineError> {
        self.runtime.block_on(self.engine.respond(request))
    }

    /// Snapshot the engine status.
    pub fn status(&self) -> EngineStatus {
        self.runtime.block_on(self.engine.status())
    }

    /// Re-run discovery and health probes.
    pub fn refresh(&self) {
        self.runtime.block_on(self.engine.refresh_resources());
    }

    /// Register a capability subscription; returns its id.
    pub fn subscribe(
        &self,
        app_id: Option<String>,
        session_id: Option<String>,
        capabilities: Vec<Capability>,
        ttl_secs: Option<u64>,
    ) -> u64 {
        // `Engine::subscribe` spawns background warm tasks, so it must run with
        // the owned runtime entered on this thread — otherwise `tokio::spawn`
        // panics when called from a plain (non-async) caller.
        let _guard = self.runtime.enter();
        self.engine.subscribe(
            app_id,
            session_id,
            capabilities,
            ttl_secs.map(Duration::from_secs),
        )
    }

    /// Remove a subscription by id.
    pub fn unsubscribe(&self, id: u64) -> bool {
        let _guard = self.runtime.enter();
        self.engine.unsubscribe(id)
    }

    /// Access the underlying async engine for advanced, async-native use.
    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }
}

/// JSON string facade — the surface a UniFFI binding exports to Python.
///
/// UniFFI cannot carry `serde_json::Value`, borrowed types, or the full domain
/// structs across the FFI boundary without hand-written type maps. These
/// string-in / string-out methods sidestep that: they are stable, language
/// agnostic, and map onto UniFFI's `string` and `Result<string, string>`
/// natively. A generated binding is a thin wrapper that forwards JSON.
impl EmbeddedEngine {
    /// Capability list as a JSON array.
    pub fn capabilities_json(&self) -> String {
        serde_json::to_string(&self.capabilities()).unwrap_or_else(|_| "[]".into())
    }

    /// Engine status as a JSON object.
    pub fn status_json(&self) -> String {
        serde_json::to_string(&self.status()).unwrap_or_else(|_| "{}".into())
    }

    /// Parse an [`InferenceRequest`] from JSON, invoke, and return the response
    /// as JSON. On failure the `Err` payload is a JSON [`ErrorInfo`] with a
    /// stable code, so a binding can raise a typed exception.
    pub fn invoke_json(&self, request_json: &str) -> Result<String, String> {
        let request: InferenceRequest = serde_json::from_str(request_json)
            .map_err(|e| Self::err_json(EngineError::InvalidRequest(e.to_string())))?;
        self.invoke(request)
            .map(|resp| serde_json::to_string(&resp).unwrap_or_default())
            .map_err(Self::err_json)
    }

    /// Parse a [`ResponsesRequest`] from JSON, run one turn, and return the
    /// [`ResponsesResponse`] as JSON — the UniFFI-friendly stateful surface. On
    /// failure the `Err` payload is a JSON [`ErrorInfo`] with a stable code.
    pub fn respond_json(&self, request_json: &str) -> Result<String, String> {
        let request: ResponsesRequest = serde_json::from_str(request_json)
            .map_err(|e| Self::err_json(EngineError::InvalidRequest(e.to_string())))?;
        self.respond(request)
            .map(|resp| serde_json::to_string(&resp).unwrap_or_default())
            .map_err(Self::err_json)
    }
}

impl EmbeddedEngine {
    /// Serialize an engine error to a JSON `ErrorInfo` string.
    fn err_json(e: EngineError) -> String {
        serde_json::to_string(&e.info()).unwrap_or_else(|_| e.to_string())
    }
}

/// A typed client for the engine's versioned HTTP API (daemon mode).
pub struct DaemonClient {
    base_url: String,
    http: reqwest::Client,
    token: Option<String>,
}

impl DaemonClient {
    /// Create a client for a daemon at `base_url` (e.g. `http://localhost:8420`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_client(base_url, reqwest::Client::new())
    }

    /// Create a client with a caller-provided HTTP client.
    pub fn with_client(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            base_url,
            http,
            token: None,
        }
    }

    /// Attach a bearer token, sent on every request. Required when the daemon
    /// runs with `MOFA_API_TOKEN` set.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    /// Attach the bearer token to a request builder when one is configured.
    fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(token) => rb.bearer_auth(token),
            None => rb,
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let resp = self
            .authed(self.http.get(self.url(path)))
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        Self::decode(resp).await
    }

    async fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, ClientError> {
        let resp = self
            .authed(self.http.post(self.url(path)).json(&body))
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        Self::decode(resp).await
    }

    async fn decode<T: serde::de::DeserializeOwned>(
        resp: reqwest::Response,
    ) -> Result<T, ClientError> {
        let status = resp.status();
        if status.is_success() {
            return resp
                .json::<T>()
                .await
                .map_err(|e| ClientError::Decode(e.to_string()));
        }
        // Read the body once, then prefer a structured ErrorInfo; fall back to a
        // status-bearing error for framework responses (401/404/413/…) so the
        // status and reason are not lost.
        let body = resp.text().await.unwrap_or_default();
        match serde_json::from_str::<ErrorInfo>(&body) {
            Ok(info) => Err(ClientError::Engine(info)),
            Err(_) => Err(ClientError::Status {
                status: status.as_u16(),
                body,
            }),
        }
    }

    /// `GET /v1/capabilities`
    pub async fn capabilities(&self) -> Result<Vec<ModelCard>, ClientError> {
        self.get_json("/v1/capabilities").await
    }

    /// `GET /v1/status`
    pub async fn status(&self) -> Result<EngineStatus, ClientError> {
        self.get_json("/v1/status").await
    }

    /// `GET /v1/memory`
    pub async fn memory(&self) -> Result<MemoryReport, ClientError> {
        self.get_json("/v1/memory").await
    }

    /// `GET /v1/lifecycle`
    pub async fn lifecycle(&self) -> Result<Vec<LifecycleRecord>, ClientError> {
        self.get_json("/v1/lifecycle").await
    }

    /// `GET /v1/preflight`
    pub async fn preflight(&self) -> Result<PreflightStats, ClientError> {
        self.get_json("/v1/preflight").await
    }

    /// `GET /v1/subscriptions`
    pub async fn subscriptions(&self) -> Result<Vec<SubscriptionInfo>, ClientError> {
        self.get_json("/v1/subscriptions").await
    }

    /// `POST /v1/invoke`
    pub async fn invoke(
        &self,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, ClientError> {
        let resp = self
            .authed(self.http.post(self.url("/v1/invoke")).json(request))
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        Self::decode(resp).await
    }

    /// `POST /v1/responses` — one turn of the stateful Responses API. The returned
    /// [`ResponsesResponse::id`] is passed as `previous_response_id` to continue.
    pub async fn respond(
        &self,
        request: &ResponsesRequest,
    ) -> Result<ResponsesResponse, ClientError> {
        let resp = self
            .authed(self.http.post(self.url("/v1/responses")).json(request))
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        Self::decode(resp).await
    }

    /// `GET /v1/responses/{id}` — the stored message history for a conversation.
    pub async fn conversation(&self, id: &str) -> Result<Vec<Message>, ClientError> {
        self.get_json(&format!("/v1/responses/{id}")).await
    }

    /// `DELETE /v1/responses/{id}` — forget a stored conversation; returns whether
    /// one existed. A not-found id is reported as `Ok(false)`, not an error.
    pub async fn delete_conversation(&self, id: &str) -> Result<bool, ClientError> {
        let resp = self
            .authed(self.http.delete(self.url(&format!("/v1/responses/{id}"))))
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        #[derive(serde::Deserialize)]
        struct DeleteResponse {
            removed: bool,
        }
        let resp: DeleteResponse = Self::decode(resp).await?;
        Ok(resp.removed)
    }

    /// `POST /v1/discovery/refresh`
    pub async fn refresh(&self) -> Result<EngineStatus, ClientError> {
        let resp = self
            .authed(self.http.post(self.url("/v1/discovery/refresh")))
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        Self::decode(resp).await
    }

    /// `POST /v1/subscriptions` — register a capability subscription and warm its
    /// models; returns the new subscription id.
    pub async fn subscribe(
        &self,
        app_id: Option<String>,
        session_id: Option<String>,
        capabilities: Vec<Capability>,
        ttl_secs: Option<u64>,
    ) -> Result<u64, ClientError> {
        #[derive(serde::Deserialize)]
        struct SubscribeResponse {
            id: u64,
        }
        let body = serde_json::json!({
            "app_id": app_id,
            "session_id": session_id,
            "capabilities": capabilities,
            "ttl_secs": ttl_secs,
        });
        let resp: SubscribeResponse = self.post_json("/v1/subscriptions", body).await?;
        Ok(resp.id)
    }

    /// `DELETE /v1/subscriptions/{id}` — remove a subscription; returns whether
    /// one existed. A not-found id is reported as `Ok(false)`, not an error.
    pub async fn unsubscribe(&self, id: u64) -> Result<bool, ClientError> {
        let resp = self
            .authed(
                self.http
                    .delete(self.url(&format!("/v1/subscriptions/{id}"))),
            )
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        #[derive(serde::Deserialize)]
        struct UnsubscribeResponse {
            removed: bool,
        }
        let resp: UnsubscribeResponse = Self::decode(resp).await?;
        Ok(resp.removed)
    }

    /// `POST /v1/models/load` — manually warm a model; returns whether backend
    /// state changed.
    pub async fn load_model(&self, model_id: &str) -> Result<bool, ClientError> {
        #[derive(serde::Deserialize)]
        struct ModelActionResponse {
            changed: bool,
        }
        let body = serde_json::json!({ "model_id": model_id });
        let resp: ModelActionResponse = self.post_json("/v1/models/load", body).await?;
        Ok(resp.changed)
    }

    /// `POST /v1/models/unload` — manually unload a model; returns whether the
    /// model was known.
    pub async fn unload_model(&self, model_id: &str) -> Result<bool, ClientError> {
        #[derive(serde::Deserialize)]
        struct ModelActionResponse {
            changed: bool,
        }
        let body = serde_json::json!({ "model_id": model_id });
        let resp: ModelActionResponse = self.post_json("/v1/models/unload", body).await?;
        Ok(resp.changed)
    }

    // Streaming endpoints (`POST /v1/invoke/stream` and `GET /v1/events`) are
    // intentionally not part of this typed client: SSE consumption is
    // caller-specific (framing, backpressure, reconnection), so a caller drives
    // them with `reqwest` directly, or uses the in-process `EmbeddedEngine` /
    // `Engine::invoke_stream` for a typed `StreamChunk` receiver.
}

#[cfg(test)]
mod tests {
    use super::*;
    use mofa_engine_core::config::{
        DiscoveryConfig, ListenConfig, MemoryConfig, PreflightConfig, TimeoutConfig,
    };

    fn empty_config() -> EngineConfig {
        EngineConfig {
            listen: ListenConfig::default(),
            memory: MemoryConfig::default(),
            timeouts: TimeoutConfig::default(),
            preflight: PreflightConfig::default(),
            artifacts: Default::default(),
            security: Default::default(),
            providers: vec![],
            discovery: DiscoveryConfig { refresh_secs: 0 },
        }
    }

    #[test]
    fn embedded_engine_boots_and_reports_status() {
        // A plain (non-async) caller can drive the engine via the facade.
        let engine = EmbeddedEngine::new(empty_config()).expect("embedded engine builds");
        assert!(engine.capabilities().is_empty());
        assert_eq!(engine.status().total_models, 0);
    }

    #[test]
    fn embedded_invoke_without_models_errors() {
        let engine = EmbeddedEngine::new(empty_config()).unwrap();
        let req = InferenceRequest {
            capability: Some(Capability::Chat),
            model: None,
            app_id: None,
            session_id: None,
            fallback_policy: Default::default(),
            messages: vec![],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "t".into(),
            ..Default::default()
        };
        assert!(matches!(
            engine.invoke(req),
            Err(EngineError::NoCapableModel(_))
        ));
    }

    #[test]
    fn embedded_json_facade_round_trips() {
        let engine = EmbeddedEngine::new(empty_config()).unwrap();
        assert_eq!(engine.capabilities_json(), "[]");
        assert!(engine.status_json().contains("total_models"));

        // A malformed request body yields a JSON ErrorInfo in the Err arm.
        let err = engine.invoke_json("not json").unwrap_err();
        assert!(err.contains("invalid_request"));

        // A well-formed request with no models yields a no_capable_model error.
        let err = engine
            .invoke_json(r#"{"capability":"chat","messages":[]}"#)
            .unwrap_err();
        assert!(err.contains("no_capable_model"));
    }

    #[test]
    fn embedded_responds_and_reports_invalid_input() {
        let engine = EmbeddedEngine::new(empty_config()).unwrap();

        // A turn with input but no models still routes and fails with a typed
        // no_capable_model error (the stateful path reaches routing).
        let err = engine
            .respond_json(r#"{"input":"hello","capability":"chat"}"#)
            .unwrap_err();
        assert!(err.contains("no_capable_model"), "got: {err}");

        // An empty turn is rejected before routing.
        let err = engine.respond_json("{}").unwrap_err();
        assert!(err.contains("invalid_request"), "got: {err}");
    }

    #[test]
    fn daemon_client_normalizes_urls() {
        let client = DaemonClient::new("http://localhost:8420/");
        assert_eq!(client.url("/v1/status"), "http://localhost:8420/v1/status");
        assert_eq!(client.url("v1/status"), "http://localhost:8420/v1/status");
    }

    #[test]
    fn with_token_is_retained_and_url_unaffected() {
        let client = DaemonClient::new("http://localhost:8420").with_token("secret");
        assert_eq!(client.token.as_deref(), Some("secret"));
        assert_eq!(client.url("/v1/status"), "http://localhost:8420/v1/status");
    }

    #[test]
    fn client_error_status_preserves_code_and_body() {
        let err = ClientError::Status {
            status: 401,
            body: "unauthorized".into(),
        };
        assert!(err.to_string().contains("401"));
        assert!(err.to_string().contains("unauthorized"));
        assert!(err.engine_error().is_none());
    }
}
