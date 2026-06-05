use modal_rs::{AppLogsOptions, FunctionFromNameOptions, ModalClient, VolumeFromNameOptions};
use tracing::info;

const APP_NAME: &str = "foni-fish-finetune";
const FUNCTION_NAME: &str = "train";
const VOLUME_NAME: &str = "foni-training";
const ENVIRONMENT: &str = "main";

pub async fn connect() -> Result<ModalClient, String> {
    ModalClient::connect()
        .await
        .map_err(|e| format!("Modal connect: {e}"))
}

pub async fn spawn_training(
    client: &mut ModalClient,
    model: &str,
    steps: u32,
) -> Result<String, String> {
    let func = client
        .functions()
        .from_name(APP_NAME, FUNCTION_NAME, FunctionFromNameOptions::default())
        .await
        .map_err(|e| format!("lookup function: {e}"))?;

    let call = func
        .spawn(client, (model, steps))
        .await
        .map_err(|e| format!("spawn: {e}"))?;

    Ok(call.id().to_string())
}

pub enum JobStatus {
    Running,
    Success(String),
    Failed(String),
}

pub async fn job_status(client: &mut ModalClient, call_id: &str) -> Result<JobStatus, String> {
    let call = modal_rs::FunctionCall::from_id(client, call_id)
        .await
        .map_err(|e| format!("get call: {e}"))?;

    let graph = call
        .call_graph(client)
        .await
        .map_err(|e| format!("call graph: {e}"))?;

    for input in &graph.inputs {
        match input.status {
            modal_rs::FunctionCallGraphStatus::Success => {
                let mut call2 = modal_rs::FunctionCall::from_id(client, call_id)
                    .await
                    .map_err(|e| format!("{e}"))?;
                match call2
                    .get::<String>(client, Some(std::time::Duration::from_secs(3)), 0)
                    .await
                {
                    Ok(result) => return Ok(JobStatus::Success(result)),
                    Err(e) => return Ok(JobStatus::Success(format!("(result decode: {e})"))),
                }
            }
            modal_rs::FunctionCallGraphStatus::Failure
            | modal_rs::FunctionCallGraphStatus::InitFailure
            | modal_rs::FunctionCallGraphStatus::InternalFailure => {
                return Ok(JobStatus::Failed(format!("{:?}", input.status)));
            }
            modal_rs::FunctionCallGraphStatus::Terminated => {
                return Ok(JobStatus::Failed("terminated".into()));
            }
            modal_rs::FunctionCallGraphStatus::Timeout => {
                return Ok(JobStatus::Failed("timeout".into()));
            }
            _ => {} // Unspecified = still running
        }
    }

    Ok(JobStatus::Running)
}

pub async fn cancel_job(client: &mut ModalClient, call_id: &str) -> Result<(), String> {
    let call = modal_rs::FunctionCall::from_id(client, call_id)
        .await
        .map_err(|e| format!("get call: {e}"))?;

    call.cancel(client, true)
        .await
        .map_err(|e| format!("cancel: {e}"))
}

pub async fn tail_logs(
    client: &mut ModalClient,
    call_id: &str,
    max_batches: usize,
) -> Result<Vec<String>, String> {
    let app = client
        .apps()
        .from_name(APP_NAME, ENVIRONMENT, Default::default())
        .await
        .map_err(|e| format!("lookup app: {e}"))?;

    let opts = AppLogsOptions {
        function_call_id: Some(call_id.to_string()),
        timeout_secs: 3.0,
        ..Default::default()
    };

    let mut apps = client.apps();
    let mut stream = apps
        .logs(&app, opts)
        .await
        .map_err(|e| format!("logs: {e}"))?;

    let mut lines = Vec::new();
    let mut batches = 0;
    while batches < max_batches {
        match stream.next_batch().await {
            Ok(Some(batch)) => {
                for entry in &batch.entries {
                    lines.push(entry.message.clone());
                }
                batches += 1;
            }
            Ok(None) | Err(_) => break,
        }
    }

    Ok(lines)
}

pub async fn list_volume_files(
    client: &mut ModalClient,
    path: &str,
) -> Result<Vec<String>, String> {
    let vol = client
        .volumes()
        .from_name(VOLUME_NAME, ENVIRONMENT, VolumeFromNameOptions::default())
        .await
        .map_err(|e| format!("lookup volume: {e}"))?;

    let files = client
        .volumes()
        .list_files(vol.id(), path, Default::default())
        .await
        .map_err(|e| format!("list files: {e}"))?;

    Ok(files.iter().map(|f| f.path.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_token() -> bool {
        let home = std::env::var("HOME").unwrap_or_default();
        std::path::Path::new(&format!("{home}/.modal.toml")).exists()
    }

    #[tokio::test]
    async fn connect_to_modal() {
        if !has_token() {
            info!("skip: no token");
            return;
        }
        assert!(connect().await.is_ok());
    }
}
