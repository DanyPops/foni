//! RunPod REST + Serverless client — typed Rust, no Python.
//!
//! All cloud GPU operations go through this module.
//! REST API (rest.runpod.io/v1) for management; Serverless API (api.runpod.ai/v2) for jobs.
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
    pub docker_args: String,
    pub env: Vec<(String, String)>,
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
    fn stream_progress(
        &self,
        endpoint_id: &str,
        job_id: &str,
        on_chunk: &mut dyn FnMut(serde_json::Value),
    );
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

    /// Create a serverless template.
    pub fn create_template(
        &self,
        name: &str,
        image: &str,
        registry_auth_id: Option<&str>,
    ) -> Result<String, String> {
        let reg = registry_auth_id
            .map(|id| format!(r#", containerRegistryAuthId: "{id}""#))
            .unwrap_or_default();
        let q = format!(
            r#"mutation {{ saveTemplate(input: {{ name: "{name}", imageName: "{image}"{reg}, dockerArgs: "", containerDiskInGb: 30, volumeInGb: 0, isServerless: true, env: [] }}) {{ id name }} }}"#
        );
        let data = self.graphql(&q)?;
        data["saveTemplate"]["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or("no template id".into())
    }

    /// Create a serverless endpoint from a template.
    pub fn create_endpoint(
        &self,
        name: &str,
        template_id: &str,
        gpu_ids: &str,
        execution_timeout_ms: u64,
    ) -> Result<String, String> {
        let q = format!(
            r#"mutation {{ saveEndpoint(input: {{ name: "{name}", templateId: "{template_id}", gpuIds: "{gpu_ids}", workersMin: 0, workersMax: 1, idleTimeout: 30, scalerType: "QUEUE_DELAY", scalerValue: 1, flashBootType: FLASHBOOT, executionTimeoutMs: {execution_timeout_ms} }}) {{ id name }} }}"#
        );
        let data = self.graphql(&q)?;
        data["saveEndpoint"]["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or("no endpoint id".into())
    }

    /// Register container registry credentials.
    pub fn register_registry(
        &self,
        name: &str,
        username: &str,
        password: &str,
    ) -> Result<String, String> {
        let q = format!(
            r#"mutation {{ saveRegistryAuth(input: {{ name: "{name}", username: "{username}", password: "{password}" }}) {{ id }} }}"#
        );
        let data = self.graphql(&q)?;
        data["saveRegistryAuth"]["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or("no registry id".into())
    }

    /// Check endpoint health.
    pub fn endpoint_health(&self, endpoint_id: &str) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .get(format!("https://api.runpod.ai/v2/{endpoint_id}/health"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .map_err(|e| format!("health: {e}"))?;
        resp.json().map_err(|e| e.to_string())
    }

    /// Set SSH public key on the account.
    pub fn set_ssh_key(&self, pub_key: &str) -> Result<(), String> {
        let q = format!(
            r#"mutation {{ updateUserSettings(input: {{ pubKey: "{pub_key}" }}) {{ id }} }}"#
        );
        self.graphql(&q)?;
        Ok(())
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

    // ── REST API (rest.runpod.io/v1) ──────────────────────────────────────────

    fn rest_get(&self, path: &str) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .get(format!("https://rest.runpod.io/v1{path}"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .map_err(|e| format!("REST GET {path}: {e}"))?;
        resp.json().map_err(|e| format!("REST JSON: {e}"))
    }

    fn rest_patch(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .patch(format!("https://rest.runpod.io/v1{path}"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .map_err(|e| format!("REST PATCH {path}: {e}"))?;
        resp.json().map_err(|e| format!("REST JSON: {e}"))
    }

    /// GET /endpoints/{id} — full endpoint details with workers.
    pub fn get_endpoint(&self, endpoint_id: &str) -> Result<serde_json::Value, String> {
        self.rest_get(&format!("/endpoints/{endpoint_id}"))
    }

    /// PATCH /endpoints/{id} — update GPU types, worker counts, etc.
    pub fn update_endpoint(
        &self,
        endpoint_id: &str,
        patch: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.rest_patch(&format!("/endpoints/{endpoint_id}"), patch)
    }

    /// GET /templates/{id} — template details.
    pub fn get_template(&self, template_id: &str) -> Result<serde_json::Value, String> {
        self.rest_get(&format!("/templates/{template_id}"))
    }

    /// PATCH /templates/{id} — update image, disk, etc.
    pub fn update_template(
        &self,
        template_id: &str,
        patch: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.rest_patch(&format!("/templates/{template_id}"), patch)
    }

    /// GET /billing/endpoints — serverless billing history.
    pub fn billing_endpoints(&self) -> Result<serde_json::Value, String> {
        self.rest_get("/billing/endpoints")
    }

    /// GET /pods — list all pods.
    pub fn list_pods(&self) -> Result<serde_json::Value, String> {
        self.rest_get("/pods")
    }

    /// GET /pods/{id} — single pod details (IP, ports, status).
    pub fn get_pod(&self, pod_id: &str) -> Result<serde_json::Value, String> {
        self.rest_get(&format!("/pods/{pod_id}"))
    }

    /// Wait for pod to reach RUNNING status with a public IP.
    pub fn wait_for_pod(
        &self,
        pod_id: &str,
        timeout_secs: u64,
    ) -> Result<serde_json::Value, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            let pod = self.get_pod(pod_id)?;
            let status = pod["desiredStatus"].as_str().unwrap_or("");
            let ip = pod["publicIp"].as_str().unwrap_or("");
            let has_ssh = pod["portMappings"]
                .as_object()
                .map(|m| m.contains_key("22"))
                .unwrap_or(false);

            if status == "RUNNING" && !ip.is_empty() && has_ssh {
                eprintln!();
                return Ok(pod);
            }
            if std::time::Instant::now() > deadline {
                return Err(format!(
                    "Pod {pod_id} not ready after {timeout_secs}s (status={status}, ip={ip})"
                ));
            }
            eprint!("\r  Waiting for pod... {status}  ");
            std::io::Write::flush(&mut std::io::stderr()).ok();
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    }
}

// ── Pod SSH/SCP helpers ─────────────────────────────────────────────────────

/// SSH connection info extracted from a pod.
#[derive(Debug, Clone)]
pub struct PodSsh {
    pub pod_id: String,
    pub account_hash: String,
}

impl PodSsh {
    pub fn new(pod_id: &str) -> Self {
        let hash = std::env::var("RUNPOD_SSH_HASH").unwrap_or_else(|_| "64410b27".into());
        Self {
            pod_id: pod_id.to_string(),
            account_hash: hash,
        }
    }

    pub fn ssh_dest(&self) -> String {
        format!("{}-{}@ssh.runpod.io", self.pod_id, self.account_hash)
    }

    pub fn ssh_opts_static() -> Vec<String> {
        vec![
            "-tt".into(),
            "-o".into(),
            "StrictHostKeyChecking=no".into(),
            "-o".into(),
            "UserKnownHostsFile=/dev/null".into(),
            "-o".into(),
            "LogLevel=ERROR".into(),
        ]
    }

    pub fn run(&self, cmd: &str) -> Result<(), String> {
        eprintln!("  \u{25b6} {}", cmd);
        let status = std::process::Command::new("ssh")
            .args(Self::ssh_opts_static())
            .arg(&self.ssh_dest())
            .arg(cmd)
            .status()
            .map_err(|e| format!("ssh: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("ssh failed: exit {}", status.code().unwrap_or(-1)))
        }
    }

    pub fn upload(&self, local: &str, remote: &str) -> Result<(), String> {
        eprintln!("  \u{2191} {} \u{2192} {}", local, remote);
        self.run(&format!("mkdir -p {remote}"))?;
        let tar = std::process::Command::new("tar")
            .args(["czf", "-", "-C", local, "."])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("tar: {e}"))?;
        let status = std::process::Command::new("ssh")
            .args(Self::ssh_opts_static())
            .arg(&self.ssh_dest())
            .arg(format!("tar xzf - -C {remote}"))
            .stdin(tar.stdout.ok_or("no tar stdout")?)
            .status()
            .map_err(|e| format!("ssh upload: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "upload failed: exit {}",
                status.code().unwrap_or(-1)
            ))
        }
    }

    pub fn download(&self, remote: &str, local: &str) -> Result<(), String> {
        eprintln!("  \u{2193} {} \u{2192} {}", remote, local);
        let local_path = std::path::Path::new(local);
        let local_dir = local_path.parent().unwrap_or(std::path::Path::new("."));
        std::fs::create_dir_all(local_dir).ok();
        let remote_dir = std::path::Path::new(remote)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("/");
        let remote_name = std::path::Path::new(remote)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let ssh = std::process::Command::new("ssh")
            .args(Self::ssh_opts_static())
            .arg(&self.ssh_dest())
            .arg(format!("tar czf - -C {remote_dir} {remote_name}"))
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("ssh download: {e}"))?;
        let status = std::process::Command::new("tar")
            .args(["xzf", "-", "-C", local_dir.to_str().unwrap_or(".")])
            .stdin(ssh.stdout.ok_or("no ssh stdout")?)
            .status()
            .map_err(|e| format!("tar: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "download failed: exit {}",
                status.code().unwrap_or(-1)
            ))
        }
    }

    pub fn wait_for_ssh(&self, timeout_secs: u64) -> Result<(), String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            // run `hostname` not `true` — proxy accepts `true` before container is ready
            let out = std::process::Command::new("ssh")
                .args(Self::ssh_opts_static())
                .arg(&self.ssh_dest())
                .arg("hostname")
                .output();
            if let Ok(o) = out {
                let stdout = String::from_utf8_lossy(&o.stdout);
                // "container not found" means proxy is up but container isn't
                if o.status.success() && !stdout.contains("container not found") {
                    eprintln!("  SSH ready");
                    return Ok(());
                }
            }
            if std::time::Instant::now() > deadline {
                return Err("SSH not reachable".into());
            }
            eprint!("\r  Waiting for container...");
            std::io::Write::flush(&mut std::io::stderr()).ok();
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    }
}

impl RunPodProvider {
    /// DELETE on REST API.
    pub fn rest_delete(&self, path: &str) -> Result<(), String> {
        self.client
            .delete(format!("https://rest.runpod.io/v1{path}"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .map_err(|e| format!("REST DELETE {path}: {e}"))?;
        Ok(())
    }

    /// Purge all queued jobs. Returns count removed.
    pub fn purge_queue(&self, endpoint_id: &str) -> Result<u64, String> {
        let resp = self
            .client
            .post(format!(
                "https://api.runpod.ai/v2/{endpoint_id}/purge-queue"
            ))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .map_err(|e| format!("purge: {e}"))?;
        let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        Ok(data["removed"].as_u64().unwrap_or(0))
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
        let mut body = serde_json::json!({
            "name": opts.name,
            "imageName": opts.image,
            "gpuTypeIds": [opts.gpu_type_id],
            "gpuCount": 1,
            "containerDiskInGb": opts.container_disk_gb,
            "volumeInGb": opts.volume_gb,
            "volumeMountPath": "/workspace",
            "ports": opts.ports.split(',').collect::<Vec<_>>(),
            "supportPublicIp": true,
        });
        if !opts.docker_args.is_empty() {
            body["dockerStartCmd"] = serde_json::json!(["bash", "-c", opts.docker_args]);
        }
        if !opts.env.is_empty() {
            let env_map: serde_json::Map<String, serde_json::Value> = opts
                .env
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            body["env"] = serde_json::Value::Object(env_map);
        }
        let resp = self
            .client
            .post("https://rest.runpod.io/v1/pods")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .map_err(|e| format!("create pod: {e}"))?;
        let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        if let Some(errors) = data.as_array() {
            if let Some(err) = errors.first() {
                return Err(err["error"]
                    .as_str()
                    .unwrap_or(&data.to_string())
                    .to_string());
            }
        }
        Ok(Pod {
            id: data["id"].as_str().unwrap_or("").to_string(),
            cost_per_hr: data["costPerHr"]
                .as_f64()
                .or_else(|| data["costPerHr"].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(0.0),
            status: data["desiredStatus"]
                .as_str()
                .unwrap_or("UNKNOWN")
                .to_string(),
            gpu_name: data["machine"]["gpuType"]["displayName"]
                .as_str()
                .or_else(|| data["gpu"]["displayName"].as_str())
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

    fn stream_progress(
        &self,
        endpoint_id: &str,
        job_id: &str,
        on_chunk: &mut dyn FnMut(serde_json::Value),
    ) {
        // Wait for worker to cold-start — poll status until not IN_QUEUE
        for attempt in 1..=120 {
            match RunPodProvider::job_status(self, endpoint_id, job_id) {
                Ok(s) => {
                    let status = s["status"].as_str().unwrap_or("");
                    match status {
                        "COMPLETED" => {
                            if let Some(output) = s.get("output") {
                                on_chunk(output.clone());
                            }
                            return;
                        }
                        "FAILED" | "CANCELLED" | "TIMED_OUT" => {
                            eprintln!("    Job {status}");
                            return;
                        }
                        "IN_PROGRESS" => {
                            // Worker is running — try streaming
                            let _ = self.stream_job(endpoint_id, job_id, |v| on_chunk(v));
                            return;
                        }
                        _ => {
                            eprint!("\r    Waiting for worker... [{attempt}] {status}   ");
                            std::io::Write::flush(&mut std::io::stderr()).ok();
                        }
                    }
                }
                Err(e) => {
                    eprintln!("    Status error: {e}");
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        eprintln!("    Timed out waiting for worker (10 min)");
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

    /// Cancel a running or queued job.
    pub fn cancel_job(&self, endpoint_id: &str, job_id: &str) -> Result<(), String> {
        self.client
            .post(format!(
                "https://api.runpod.ai/v2/{endpoint_id}/cancel/{job_id}"
            ))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .map_err(|e| format!("cancel: {e}"))?;
        Ok(())
    }

    /// List all endpoints on this account.
    pub fn list_endpoints(&self) -> Result<Vec<serde_json::Value>, String> {
        let data = self.graphql(
            "{ myself { endpoints { id name templateId gpuIds workersMin workersMax } } }",
        )?;
        Ok(data["myself"]["endpoints"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// List all templates on this account.
    pub fn list_templates(&self) -> Result<Vec<serde_json::Value>, String> {
        let data =
            self.graphql("{ myself { podTemplates { id name imageName isServerless } } }")?;
        Ok(data["myself"]["podTemplates"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// Full account overview.
    pub fn account_overview(&self) -> Result<serde_json::Value, String> {
        self.graphql(
            "{ myself { id email clientBalance currentSpendPerHr pods { id name costPerHr desiredStatus } endpoints { id name gpuIds workersMin workersMax } containerRegistryCreds { id name } networkVolumes { id name size } } }"
        ).map(|d| d["myself"].clone())
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

    fn stream_progress(
        &self,
        _endpoint_id: &str,
        _job_id: &str,
        _on_chunk: &mut dyn FnMut(serde_json::Value),
    ) {
        // No-op in production mock. Tests use SimulatedProvider below.
    }
}

/// Test-only provider that simulates a timed training run with progress.
#[cfg(test)]
pub struct SimulatedProvider {
    pub epochs: u32,
    pub delay_ms: u64,
}

#[cfg(test)]
impl CloudProvider for SimulatedProvider {
    fn balance(&self) -> Result<AccountStatus, String> {
        Ok(AccountStatus {
            balance: 10.0,
            spend_per_hr: 0.22,
            active_pods: 1,
        })
    }
    fn gpu_types(&self) -> Result<Vec<GpuType>, String> {
        Ok(vec![GpuType {
            id: "SIM_3090".into(),
            display_name: "RTX 3090 (sim)".into(),
            memory_gb: 24,
            community_price: Some(0.22),
            secure_price: None,
        }])
    }
    fn create_pod(&self, _opts: CreatePodOpts) -> Result<Pod, String> {
        Ok(Pod {
            id: "sim-pod".into(),
            cost_per_hr: 0.22,
            status: "RUNNING".into(),
            gpu_name: "RTX 3090".into(),
        })
    }
    fn pod_status(&self, _pod_id: &str) -> Result<String, String> {
        Ok("RUNNING".into())
    }
    fn terminate_pod(&self, _pod_id: &str) -> Result<(), String> {
        Ok(())
    }
    fn submit_job(
        &self,
        _ep: &str,
        _input: serde_json::Value,
        _wh: Option<&str>,
    ) -> Result<ServerlessJob, String> {
        Ok(ServerlessJob {
            id: "sim-job".into(),
            status: "IN_PROGRESS".into(),
        })
    }
    fn job_status(&self, _ep: &str, _job_id: &str) -> Result<serde_json::Value, String> {
        Ok(
            serde_json::json!({ "status": "COMPLETED", "output": { "model_url": "sim://model.pth" } }),
        )
    }
    fn stream_progress(
        &self,
        _ep: &str,
        _job_id: &str,
        on_chunk: &mut dyn FnMut(serde_json::Value),
    ) {
        for epoch in 1..=self.epochs {
            std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
            let loss = 0.05 * (1.0 - epoch as f64 / self.epochs as f64) + 0.001;
            on_chunk(serde_json::json!({
                "epoch": epoch,
                "total_epochs": self.epochs,
                "loss": loss,
                "status": if epoch == self.epochs { "COMPLETED" } else { "IN_PROGRESS" },
            }));
        }
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
                docker_args: String::new(),
                env: vec![],
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

    #[test]
    fn simulated_provider_streams_all_epochs() {
        let sim = SimulatedProvider {
            epochs: 5,
            delay_ms: 10,
        };
        let mut received = Vec::new();
        sim.stream_progress("ep", "job", &mut |chunk| {
            received.push(chunk);
        });
        assert_eq!(received.len(), 5);
        assert_eq!(received[0]["epoch"], 1);
        assert_eq!(received[4]["epoch"], 5);
        assert_eq!(received[4]["status"], "COMPLETED");
        assert_eq!(received[2]["status"], "IN_PROGRESS");
    }

    #[test]
    fn simulated_provider_loss_decreases() {
        let sim = SimulatedProvider {
            epochs: 10,
            delay_ms: 1,
        };
        let mut losses = Vec::new();
        sim.stream_progress("ep", "job", &mut |chunk| {
            losses.push(chunk["loss"].as_f64().unwrap());
        });
        assert!(losses.len() == 10);
        assert!(
            losses.first().unwrap() > losses.last().unwrap(),
            "loss should decrease: first={} last={}",
            losses.first().unwrap(),
            losses.last().unwrap()
        );
    }

    #[test]
    fn simulated_provider_respects_timing() {
        let sim = SimulatedProvider {
            epochs: 3,
            delay_ms: 50,
        };
        let t0 = std::time::Instant::now();
        sim.stream_progress("ep", "job", &mut |_| {});
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() >= 140,
            "3 epochs × 50ms should take >=140ms, took {}ms",
            elapsed.as_millis()
        );
    }
}

#[test]
fn docker_args_escaping_no_double_quotes() {
    let opts = CreatePodOpts {
        gpu_type_id: "TEST".into(),
        image: "test".into(),
        volume_gb: 0,
        container_disk_gb: 20,
        name: "test".into(),
        ports: "22/tcp".into(),
        docker_args: "bash -c 'echo hello; sleep 10'".into(),
        env: vec![],
    };
    // Verify the escaped string doesn't contain double-escaped quotes
    let escaped = opts.docker_args.replace('"', r#"\""#);
    assert!(
        !escaped.contains(r#"\\""#),
        "double-escaped quotes: {escaped}"
    );
    assert_eq!(escaped, "bash -c 'echo hello; sleep 10'");
}

#[test]
fn docker_args_with_double_quotes_escapes_once() {
    let args = r#"python3 -c "print('hello')""#;
    let escaped = args.replace('"', r#"\""#);
    assert!(
        escaped.contains(r#"\"print"#),
        "should escape double quotes: {escaped}"
    );
}

#[test]
fn env_vars_serialize_to_graphql() {
    let env = vec![
        ("FONI_MODEL".to_string(), "sidorovich".to_string()),
        ("FONI_EPOCHS".to_string(), "500".to_string()),
    ];
    let pairs: Vec<String> = env
        .iter()
        .map(|(k, v)| format!(r#"{{ key: "{k}", value: "{v}" }}"#))
        .collect();
    let env_str = format!(", env: [{}]", pairs.join(", "));
    assert!(env_str.contains(r#"key: "FONI_MODEL""#));
    assert!(env_str.contains(r#"value: "sidorovich""#));
    assert!(env_str.contains(r#"key: "FONI_EPOCHS""#));
}

#[test]
fn pod_ssh_dest_format() {
    std::env::set_var("RUNPOD_SSH_HASH", "abcd1234");
    let ssh = PodSsh::new("mypod123");
    assert_eq!(ssh.ssh_dest(), "mypod123-abcd1234@ssh.runpod.io");
    std::env::remove_var("RUNPOD_SSH_HASH");
}

#[test]
fn pod_ssh_default_hash() {
    std::env::remove_var("RUNPOD_SSH_HASH");
    let ssh = PodSsh::new("testpod");
    assert_eq!(ssh.ssh_dest(), "testpod-64410b27@ssh.runpod.io");
}
