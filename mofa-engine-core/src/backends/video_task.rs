//! Task-based video generation provider (TOOL-02): the create → poll →
//! download pattern used by DashScope video-synthesis (Seedance) and
//! similar vendor APIs. One `invoke` covers the whole lifecycle with
//! bounded polling; the artifact lands in `files`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelResidency, Provider,
    ProviderKind,
};

use crate::config::ModelDef;

/// Polling bounds: every 2s, up to 10 minutes (short clips finish in
/// tens of seconds; the cap keeps a hung task from blocking forever).
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_DEADLINE: Duration = Duration::from_secs(600);

pub struct VideoTaskProvider {
    name: String,
    base_url: String,
    api_key: String,
    models: Vec<ModelDef>,
    cost_tier: CostTier,
    output_dir: std::path::PathBuf,
    client: reqwest::Client,
}

impl VideoTaskProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        models: Vec<ModelDef>,
        cost_tier: CostTier,
        output_dir: Option<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("failed to build HTTP client");
        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            models,
            cost_tier,
            output_dir: output_dir
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
            client,
        }
    }

    /// Replace the built-in HTTP client (tests inject a no-proxy client
    /// for loopback mocks).
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    async fn create_task(
        &self,
        model_name: &str,
        prompt: &str,
        params: &Value,
    ) -> Result<String, EngineError> {
        let mut body = json!({
            "model": model_name,
            "input": {
                "prompt": prompt,
            },
            "parameters": {},
        });
        if let Some(size) = params.get("size").and_then(Value::as_str) {
            // DashScope accepts WxH like "1280x720".
            body["input"]["prompt"] = json!(format!("{prompt}"));
            let _ = size; // size maps below for vendors that take it explicitly
        }
        if let Some(duration) = params.get("duration").and_then(Value::as_u64) {
            body["parameters"]["duration"] = json!(duration);
        }
        if let Some(size) = params.get("size").and_then(Value::as_str) {
            body["parameters"]["size"] = json!(size);
        }

        let url = format!("{}/video-synthesis", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("X-DashScope-Async", "enable")
            .json(&body)
            .send()
            .await
            .map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("create task failed: {e}"),
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("create task HTTP {status}: {text}"),
            });
        }
        let payload: Value = resp.json().await.map_err(|e| EngineError::ProviderError {
            provider: self.name.clone(),
            detail: format!("create task parse error: {e}"),
        })?;
        payload
            .pointer("/output/task_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("create task returned no task_id: {payload}"),
            })
    }

    /// Poll until the task leaves PENDING/RUNNING. Returns the final
    /// status payload.
    async fn poll_task(&self, task_id: &str) -> Result<Value, EngineError> {
        let url = format!("{}/tasks/{task_id}", self.base_url);
        let deadline = Instant::now() + POLL_DEADLINE;
        loop {
            if Instant::now() >= deadline {
                return Err(EngineError::Timeout(format!(
                    "video task '{task_id}' did not finish within {}s",
                    POLL_DEADLINE.as_secs()
                )));
            }
            let resp = self
                .client
                .get(&url)
                .bearer_auth(&self.api_key)
                .send()
                .await
                .map_err(|e| EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("poll task failed: {e}"),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("poll task HTTP {status}: {text}"),
                });
            }
            let payload: Value = resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("poll task parse error: {e}"),
            })?;
            match payload
                .pointer("/output/task_status")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN")
            {
                "SUCCEEDED" => return Ok(payload),
                "FAILED" | "CANCELED" | "UNKNOWN" => {
                    let message = payload
                        .pointer("/output/message")
                        .and_then(Value::as_str)
                        .unwrap_or("no failure detail");
                    return Err(EngineError::ProviderError {
                        provider: self.name.clone(),
                        detail: format!("video task '{task_id}' ended unsuccessfully: {message}"),
                    });
                }
                _ => tokio::time::sleep(POLL_INTERVAL).await,
            }
        }
    }

    async fn download_video(&self, url: &str) -> Result<Vec<u8>, EngineError> {
        self.client
            .get(url)
            .send()
            .await
            .map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("video download failed: {e}"),
            })?
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("video download read failed: {e}"),
            })
    }
}

#[derive(Debug, Deserialize)]
struct TaskStatusResponse {
    output: TaskOutput,
}

#[derive(Debug, Deserialize)]
struct TaskOutput {
    task_id: String,
    #[serde(default)]
    task_status: String,
    #[serde(default)]
    video_url: Option<String>,
    #[serde(default)]
    metrics: Option<Value>,
}

#[async_trait]
impl Provider for VideoTaskProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::VideoTask
    }

    fn features(&self) -> Vec<BackendFeature> {
        vec![]
    }

    async fn discover(&self) -> Result<Vec<ModelCard>, EngineError> {
        let cards = self
            .models
            .iter()
            .filter_map(|m| {
                let cap = Capability::from_str_loose(&m.capability)?;
                let mut card =
                    ModelCard::new(self.name.clone(), m.name.clone(), cap, self.cost_tier);
                card.id = format!("{}/{}", self.name, m.name);
                card.availability = ModelAvailability::Configured;
                card.residency = ModelResidency::Remote;
                card.context_window = m.context_window.unwrap_or(0);
                card.memory_estimate_bytes = 0;
                card.execution.max_concurrency = 4;
                card.refresh_status();
                Some(card)
            })
            .collect();
        Ok(cards)
    }

    async fn health(&self) -> Result<BackendHealth, EngineError> {
        if self.api_key.is_empty() {
            return Ok(BackendHealth::Unavailable);
        }
        Ok(BackendHealth::Healthy)
    }

    async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        Ok(LifecycleResult {
            model_id: model_id.into(),
            residency: ModelResidency::Remote,
            memory_bytes: Some(0),
            changed: false,
        })
    }

    async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        Ok(LifecycleResult {
            model_id: model_id.into(),
            residency: ModelResidency::Remote,
            memory_bytes: Some(0),
            changed: false,
        })
    }

    async fn invoke(
        &self,
        model_id: &str,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, EngineError> {
        let start = Instant::now();
        let model_name = mofa_kernel::model_id_name(model_id);
        let prompt = request
            .messages
            .first()
            .map(|m| m.content.clone())
            .ok_or_else(|| {
                EngineError::InvalidRequest("video generation requires a prompt".into())
            })?;

        let task_id = self
            .create_task(model_name, &prompt, &request.params)
            .await?;
        let final_status = self.poll_task(&task_id).await?;
        let video_url = final_status
            .pointer("/output/video_url")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("succeeded task '{task_id}' carried no video_url: {final_status}"),
            })?;
        let bytes = self.download_video(video_url).await?;

        let path = self
            .output_dir
            .join(format!("mofa_video_{}.mp4", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| EngineError::Internal(format!("write error: {e}")))?;
        let file = path.to_string_lossy().to_string();

        Ok(InferenceResponse {
            text: None,
            file: Some(file.clone()),
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms: start.elapsed().as_millis() as u64,
            request_id: request.request_id.clone(),
            tokens_used: None,
            fallback_used: false,
            routing_reason: None,
            reasoning: None,
            files: vec![file],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModelDef;

    #[test]
    fn task_status_response_parses_dashscope_shape() {
        let raw = r#"{
            "output": {
                "task_id": "t-1",
                "task_status": "SUCCEEDED",
                "video_url": "https://cdn.example/v.mp4"
            }
        }"#;
        let parsed: TaskStatusResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.output.task_id, "t-1");
        assert_eq!(parsed.output.task_status, "SUCCEEDED");
        assert_eq!(
            parsed.output.video_url.as_deref(),
            Some("https://cdn.example/v.mp4")
        );
    }

    #[tokio::test]
    async fn full_lifecycle_against_mock_endpoints() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let polls = Arc::new(AtomicUsize::new(0));
        let polls_for_task = polls.clone();
        // addr is known only after binding; use a shared slot the poll
        // handler reads at request time.
        let addr_slot = Arc::new(std::sync::Mutex::new(String::new()));
        let addr_for_poll = addr_slot.clone();

        let app = axum::Router::new()
            .route(
                "/v1/video-synthesis",
                axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
                    assert!(body["input"]["prompt"].as_str().is_some());
                    let task = serde_json::json!({
                        "output": { "task_id": "task-42", "task_status": "PENDING" }
                    });
                    axum::Json(task)
                }),
            )
            .route(
                "/v1/tasks/{task_id}",
                axum::routing::get(
                    move |axum::extract::Path(task_id): axum::extract::Path<String>| async move {
                        let n = polls_for_task.fetch_add(1, Ordering::SeqCst);
                        // PENDING → RUNNING → SUCCEEDED
                        let status = match n {
                            0 => "PENDING",
                            1 => "RUNNING",
                            _ => "SUCCEEDED",
                        };
                        // A real http URL the provider can download.
                        let host = addr_for_poll.lock().unwrap().clone();
                        let video_url = format!("http://{host}/video");
                        let body = if status == "SUCCEEDED" {
                            serde_json::json!({
                                "output": {
                                    "task_id": task_id,
                                    "task_status": status,
                                    "video_url": video_url,
                                }
                            })
                        } else {
                            serde_json::json!({
                                "output": { "task_id": task_id, "task_status": status }
                            })
                        };
                        axum::Json(body)
                    },
                ),
            )
            .route("/video", axum::routing::get(|| async { "VIDEOBYTES" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        *addr_slot.lock().unwrap() = addr.to_string();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let output_dir = tempfile::tempdir().unwrap();
        let provider = VideoTaskProvider::new(
            "mock-video",
            format!("http://{addr}/v1"),
            "sk-test",
            vec![ModelDef {
                name: "seedance".into(),
                capability: "video_gen".into(),
                context_window: None,
                memory_mb: None,
            }],
            CostTier::High,
            Some(output_dir.path().to_string_lossy().to_string()),
        )
        .with_http_client(reqwest::Client::builder().no_proxy().build().unwrap());

        let request = InferenceRequest {
            capability: Some(Capability::VideoGen),
            model: Some("mock-video/seedance".into()),
            app_id: None,
            session_id: None,
            fallback_policy: Default::default(),
            messages: vec![mofa_kernel::Message {
                role: "user".into(),
                content: "一只橘猫追激光笔".into(),
                images: Vec::new(),
            }],
            input_file: None,
            params: serde_json::json!({"size": "1280x720", "duration": 5}),
            hint_next: None,
            request_id: "req-video".into(),
        };

        let resp = provider
            .invoke("mock-video/seedance", &request)
            .await
            .expect("video lifecycle succeeds");
        assert!(resp.file.as_deref().unwrap().ends_with(".mp4"));
        let bytes = std::fs::read(resp.file.as_deref().unwrap()).unwrap();
        assert_eq!(bytes, b"VIDEOBYTES");
        assert_eq!(resp.model_used, "seedance");
        // At least PENDING + RUNNING + SUCCEEDED polls happened.
        assert!(polls.load(Ordering::SeqCst) >= 3);
    }
}
