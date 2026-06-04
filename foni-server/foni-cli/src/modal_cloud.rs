use modal_rs::{
    FunctionFromNameOptions, ModalClient, VolumeFromNameOptions, VolumeListFilesOptions,
};
use std::collections::HashMap;

const APP_NAME: &str = "foni-fish-finetune";
const FUNCTION_NAME: &str = "train";
const VOLUME_NAME: &str = "foni-training";
const ENVIRONMENT: &str = "main";

pub async fn spawn_training(model: &str, steps: u32) -> Result<String, String> {
    let mut client = connect().await?;

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
        .spawn_with(&mut client, ((),), kwargs)
        .await
        .map_err(|e| format!("spawn: {e}"))?;

    let call_id = call.id().to_string();
    eprintln!("training job spawned: {call_id}");
    Ok(call_id)
}

pub async fn get_result(call_id: &str) -> Result<Option<String>, String> {
    let mut client = connect().await?;

    let mut call = modal_rs::FunctionCall::from_id(&mut client, call_id)
        .await
        .map_err(|e| format!("get call: {e}"))?;

    match call
        .get::<String>(&mut client, Some(std::time::Duration::from_secs(5)), 0)
        .await
    {
        Ok(result) => Ok(Some(result)),
        Err(e) => {
            let msg = format!("{e}");
            if msg.to_lowercase().contains("timeout") || msg.contains("pending") {
                Ok(None)
            } else {
                Err(msg)
            }
        }
    }
}

pub async fn list_volume_files(path: &str) -> Result<Vec<String>, String> {
    let mut client = connect().await?;

    let vol = client
        .volumes()
        .from_name(VOLUME_NAME, ENVIRONMENT, VolumeFromNameOptions::default())
        .await
        .map_err(|e| format!("lookup volume: {e}"))?;

    let files = client
        .volumes()
        .list_files(vol.id(), path, VolumeListFilesOptions::default())
        .await
        .map_err(|e| format!("list files: {e}"))?;

    Ok(files.iter().map(|f| f.path.clone()).collect())
}

async fn connect() -> Result<ModalClient, String> {
    ModalClient::connect()
        .await
        .map_err(|e| format!("Modal connect: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_modal_token() -> bool {
        let home = std::env::var("HOME").unwrap_or_default();
        std::path::Path::new(&format!("{home}/.modal.toml")).exists()
    }

    #[tokio::test]
    async fn connect_to_modal() {
        if !has_modal_token() {
            eprintln!("skip: no ~/.modal.toml");
            return;
        }
        let result = connect().await;
        assert!(result.is_ok(), "connect failed: {:?}", result.err());
        eprintln!("Modal connected");
    }

    #[tokio::test]
    async fn list_dataset_files() {
        if !has_modal_token() {
            eprintln!("skip: no ~/.modal.toml");
            return;
        }
        match list_volume_files("dataset-raw").await {
            Ok(files) => {
                eprintln!("{} files in dataset-raw/", files.len());
                assert!(files.len() >= 63, "expected 63+ WAV files");
            }
            Err(e) => eprintln!("volume list: {e}"),
        }
    }
}
