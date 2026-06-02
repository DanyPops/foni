//! RunPod GraphQL client — typed Rust, no Python.
//!
//! All cloud GPU operations go through this module.
//! Implements `CloudProvider` trait for real and mock backends.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuType {
    pub id: String,
    pub display_name: String,
    pub memory_gb: u32,
    pub community_price: Option<f64>,
    pub secure_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pod {
    pub id: String,
    pub cost_per_hr: f64,
    pub status: String,
    pub gpu_name: String,
}

#[derive(Debug, Clone)]
pub struct CreatePodOpts {
    pub gpu_type_id: String,
    pub image: String,
    pub volume_gb: u32,
    pub container_disk_gb: u32,
    pub name: String,
    pub ports: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStatus {
    pub balance: f64,
    pub spend_per_hr: f64,
    pub active_pods: usize,
}

/// The seam: real RunPod or mock for dry-run.
pub trait CloudProvider: Send + Sync {
    fn balance(&self) -> Result<AccountStatus, String>;
    fn gpu_types(&self) -> Result<Vec<GpuType>, String>;
    fn create_pod(&self, opts: CreatePodOpts) -> Result<Pod, String>;
    fn pod_status(&self, pod_id: &str) -> Result<String, String>;
    fn terminate_pod(&self, pod_id: &str) -> Result<(), String>;
    fn submit_job(
        &self,
        endpoint_id: &str,
        input: serde_json::Value,
        webhook: Option<&str>,
    ) -> Result<ServerlessJob, String>;
    fn job_status(&self, endpoint_id: &str, job_id: &str) -> Result<serde_json::Value, String>;
}

/// Real RunPod backend — calls the GraphQL API.
pub struct RunPodProvider {
    api_key: String,
    client: reqwest::blocking::Client,
}

impl RunPodProvider {
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            client: reqwest::blocking::Client::new(),
        }
    }

    fn graphql(&self, query: &str) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .post("https://api.runpod.io/graphql")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({"query": query}))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .map_err(|e| format!("RunPod request failed: {e}"))?;

        let body: serde_json::Value = resp.json().map_err(|e| format!("RunPod JSON: {e}"))?;

        if let Some(errors) = body.get("errors") {
            let msg = errors[0]["message"]
                .as_str()
                .unwrap_or("unknown RunPod error");
            return Err(msg.to_string());
        }

        body.get("data")
            .cloned()
            .ok_or_else(|| "no data in response".into())
    }
}

impl CloudProvider for RunPodProvider {
    fn balance(&self) -> Result<AccountStatus, String> {
        let data = self.graphql("{ myself { clientBalance currentSpendPerHr pods { id } } }")?;
        let m = &data["myself"];
        Ok(AccountStatus {
            balance: m["clientBalance"].as_f64().unwrap_or(0.0),
            spend_per_hr: m["currentSpendPerHr"].as_f64().unwrap_or(0.0),
            active_pods: m["pods"].as_array().map(|a| a.len()).unwrap_or(0),
        })
    }

    fn gpu_types(&self) -> Result<Vec<GpuType>, String> {
        let data =
            self.graphql("{ gpuTypes { id displayName memoryInGb communityPrice securePrice } }")?;
        let raw = data["gpuTypes"].as_array().ok_or("no gpuTypes")?;
        Ok(raw
            .iter()
            .filter_map(|g| {
                Some(GpuType {
                    id: g["id"].as_str()?.to_string(),
                    display_name: g["displayName"].as_str()?.to_string(),
                    memory_gb: g["memoryInGb"].as_u64()? as u32,
                    community_price: g["communityPrice"].as_f64(),
                    secure_price: g["securePrice"].as_f64(),
                })
            })
            .collect())
    }

    fn create_pod(&self, opts: CreatePodOpts) -> Result<Pod, String> {
        let query = format!(
            r#"mutation {{ podFindAndDeployOnDemand(input: {{
                cloudType: COMMUNITY, gpuCount: 1,
                volumeInGb: {vol}, containerDiskInGb: {disk},
                minVcpuCount: 2, minMemoryInGb: 15,
                gpuTypeId: "{gpu}",
                name: "{name}",
                imageName: "{image}",
                ports: "{ports}",
                volumeMountPath: "/workspace"
            }}) {{ id costPerHr desiredStatus machine {{ gpuDisplayName }} }} }}"#,
            vol = opts.volume_gb,
            disk = opts.container_disk_gb,
            gpu = opts.gpu_type_id,
            name = opts.name,
            image = opts.image,
            ports = opts.ports,
        );
        let data = self.graphql(&query)?;
        let p = &data["podFindAndDeployOnDemand"];
        Ok(Pod {
            id: p["id"].as_str().unwrap_or("").to_string(),
            cost_per_hr: p["costPerHr"].as_f64().unwrap_or(0.0),
            status: p["desiredStatus"].as_str().unwrap_or("UNKNOWN").to_string(),
            gpu_name: p["machine"]["gpuDisplayName"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        })
    }

    fn pod_status(&self, pod_id: &str) -> Result<String, String> {
        let query = format!(r#"{{ pod(input: {{podId: "{pod_id}"}}) {{ desiredStatus }} }}"#);
        let data = self.graphql(&query)?;
        Ok(data["pod"]["desiredStatus"]
            .as_str()
            .unwrap_or("UNKNOWN")
            .to_string())
    }

    fn terminate_pod(&self, pod_id: &str) -> Result<(), String> {
        let query = format!(r#"mutation {{ podTerminate(input: {{podId: "{pod_id}"}}) }}"#);
        self.graphql(&query)?;
        Ok(())
    }

    fn submit_job(
        &self,
        endpoint_id: &str,
        input: serde_json::Value,
        webhook: Option<&str>,
    ) -> Result<ServerlessJob, String> {
        RunPodProvider::submit_job(self, endpoint_id, input, webhook)
    }

    fn job_status(&self, endpoint_id: &str, job_id: &str) -> Result<serde_json::Value, String> {
        RunPodProvider::job_status(self, endpoint_id, job_id)
    }
}

// ── Serverless API ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerlessJob {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub output: serde_json::Value,
}

impl RunPodProvider {
    /// Submit an async serverless job. Returns job ID.
    pub fn submit_job(
        &self,
        endpoint_id: &str,
        input: serde_json::Value,
        webhook: Option<&str>,
    ) -> Result<ServerlessJob, String> {
        let mut body = serde_json::json!({ "input": input });
        if let Some(url) = webhook {
            body["webhook"] = serde_json::json!(url);
        }
        let resp = self
            .client
            .post(format!("https://api.runpod.ai/v2/{endpoint_id}/run"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .map_err(|e| format!("submit failed: {e}"))?;
        let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        Ok(ServerlessJob {
            id: data["id"].as_str().unwrap_or("").to_string(),
            status: data["status"].as_str().unwrap_or("UNKNOWN").to_string(),
        })
    }

    /// Get job status (and result if completed).
    pub fn job_status(&self, endpoint_id: &str, job_id: &str) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .get(format!(
                "https://api.runpod.ai/v2/{endpoint_id}/status/{job_id}"
            ))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .map_err(|e| format!("status failed: {e}"))?;
        resp.json().map_err(|e| e.to_string())
    }

    /// Stream incremental results. Calls `on_chunk` for each progress update.
    /// Blocks until the job completes or the stream ends.
    pub fn stream_job<F>(
        &self,
        endpoint_id: &str,
        job_id: &str,
        mut on_chunk: F,
    ) -> Result<(), String>
    where
        F: FnMut(serde_json::Value),
    {
        let resp = self
            .client
            .get(format!(
                "https://api.runpod.ai/v2/{endpoint_id}/stream/{job_id}"
            ))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(3600))
            .send()
            .map_err(|e| format!("stream failed: {e}"))?;
        let text = resp.text().map_err(|e| e.to_string())?;
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(stream) = data.get("stream").and_then(|s| s.as_array()) {
                for chunk in stream {
                    if let Some(output) = chunk.get("output") {
                        on_chunk(output.clone());
                    }
                }
            }
        }
        Ok(())
    }
}

/// Mock backend for dry-run testing — no network, no cost.
pub struct MockProvider {
    pub created_pods: std::sync::Mutex<Vec<String>>,
    pub terminated_pods: std::sync::Mutex<Vec<String>>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            created_pods: std::sync::Mutex::new(Vec::new()),
            terminated_pods: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl CloudProvider for MockProvider {
    fn balance(&self) -> Result<AccountStatus, String> {
        Ok(AccountStatus {
            balance: 10.0,
            spend_per_hr: 0.0,
            active_pods: 0,
        })
    }

    fn gpu_types(&self) -> Result<Vec<GpuType>, String> {
        Ok(vec![GpuType {
            id: "MOCK_RTX_3090".into(),
            display_name: "RTX 3090 (mock)".into(),
            memory_gb: 24,
            community_price: Some(0.22),
            secure_price: None,
        }])
    }

    fn create_pod(&self, opts: CreatePodOpts) -> Result<Pod, String> {
        let id = format!("mock-pod-{}", opts.name);
        self.created_pods.lock().unwrap().push(id.clone());
        Ok(Pod {
            id,
            cost_per_hr: 0.0,
            status: "RUNNING".into(),
            gpu_name: "RTX 3090 (mock)".into(),
        })
    }

    fn pod_status(&self, _pod_id: &str) -> Result<String, String> {
        Ok("EXITED".into())
    }

    fn terminate_pod(&self, pod_id: &str) -> Result<(), String> {
        self.terminated_pods
            .lock()
            .unwrap()
            .push(pod_id.to_string());
        Ok(())
    }

    fn submit_job(
        &self,
        _endpoint_id: &str,
        _input: serde_json::Value,
        _webhook: Option<&str>,
    ) -> Result<ServerlessJob, String> {
        Ok(ServerlessJob {
            id: "mock-job-001".into(),
            status: "IN_QUEUE".into(),
        })
    }

    fn job_status(&self, _endpoint_id: &str, _job_id: &str) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({
            "status": "COMPLETED",
            "output": { "model_url": "mock://model.pth" }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_balance_returns_ten() {
        let m = MockProvider::new();
        let b = m.balance().unwrap();
        assert_eq!(b.balance, 10.0);
    }

    #[test]
    fn mock_create_and_terminate() {
        let m = MockProvider::new();
        let pod = m
            .create_pod(CreatePodOpts {
                gpu_type_id: "MOCK".into(),
                image: "test".into(),
                volume_gb: 20,
                container_disk_gb: 20,
                name: "test-pod".into(),
                ports: "22/tcp".into(),
            })
            .unwrap();
        assert!(pod.id.contains("test-pod"));
        assert_eq!(pod.cost_per_hr, 0.0);

        m.terminate_pod(&pod.id).unwrap();
        assert_eq!(m.terminated_pods.lock().unwrap().len(), 1);
    }

    #[test]
    fn mock_gpu_types_non_empty() {
        let m = MockProvider::new();
        let gpus = m.gpu_types().unwrap();
        assert!(!gpus.is_empty());
        assert!(gpus[0].community_price.unwrap() > 0.0);
    }

    #[test]
    fn mock_pod_status_returns_exited() {
        let m = MockProvider::new();
        assert_eq!(m.pod_status("any").unwrap(), "EXITED");
    }

    #[test]
    fn mock_submit_job_returns_id() {
        let m = MockProvider::new();
        let job = m
            .submit_job("ep-123", serde_json::json!({"epochs": 500}), None)
            .unwrap();
        assert!(!job.id.is_empty());
        assert_eq!(job.status, "IN_QUEUE");
    }

    #[test]
    fn mock_job_status_returns_completed() {
        let m = MockProvider::new();
        let status = m.job_status("ep-123", "job-1").unwrap();
        assert_eq!(status["status"], "COMPLETED");
        assert!(status["output"]["model_url"].as_str().is_some());
    }
}
