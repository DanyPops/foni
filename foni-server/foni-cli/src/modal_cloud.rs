use modal_rs::{App, AppLogsOptions, FunctionFromNameOptions, ModalClient, VolumeFromNameOptions};
use std::collections::HashMap;

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

    let kwargs: HashMap<String, serde_json::Value> = HashMap::from([
        ("model".into(), serde_json::Value::String(model.into())),
        ("steps".into(), serde_json::Value::Number(steps.into())),
    ]);

    let call = func
        .spawn_with(client, ((),), kwargs)
        .await
        .map_err(|e| format!("spawn: {e}"))?;

    Ok(call.id().to_string())
}

pub async fn poll_result(
    client: &mut ModalClient,
    call_id: &str,
) -> Result<Option<String>, String> {
    let mut call = modal_rs::FunctionCall::from_id(client, call_id)
        .await
        .map_err(|e| format!("get call: {e}"))?;

    match call.poll::<String>(client).await {
        Ok(Some(result)) => Ok(Some(result)),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("{e}")),
    }
}

pub async fn cancel_job(client: &mut ModalClient, call_id: &str) -> Result<(), String> {
    let call = modal_rs::FunctionCall::from_id(client, call_id)
        .await
        .map_err(|e| format!("get call: {e}"))?;

    call.cancel(client, true)
        .await
        .map_err(|e| format!("cancel: {e}"))
}

pub async fn stream_logs(
    client: &mut ModalClient,
    call_id: &str,
) -> Result<Option<String>, String> {
    let app = client
        .apps()
        .from_name(APP_NAME, ENVIRONMENT, Default::default())
        .await
        .map_err(|e| format!("lookup app: {e}"))?;

    let opts = AppLogsOptions {
        function_call_id: Some(call_id.to_string()),
        ..Default::default()
    };

    let mut apps = client.apps();
    let mut follower = apps.logs_follow(&app, opts, true);

    loop {
        match follower.next_batch().await {
            Ok(Some(batch)) => {
                for entry in &batch.entries {
                    eprint!("{}", entry.message);
                }
            }
            Ok(None) => break,
            Err(e) => return Err(format!("logs: {e}")),
        }
    }

    poll_result(client, call_id).await
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
            eprintln!("skip: no token");
            return;
        }
        assert!(connect().await.is_ok());
    }
}
