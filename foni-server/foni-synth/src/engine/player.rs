use std::process::Command;

const PLAYERS: &[&str] = &["paplay", "afplay", "mpv", "aplay", "ffplay"];

pub fn play_wav_blocking(wav_data: &[u8]) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!(
        "foni_play_{}.wav",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::write(&tmp, wav_data).map_err(|e| format!("write temp wav: {e}"))?;

    let result = play_file(&tmp);
    let _ = std::fs::remove_file(&tmp);
    result
}

fn play_file(path: &std::path::Path) -> Result<(), String> {
    for player in PLAYERS {
        if let Ok(status) = Command::new(player)
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            if status.success() {
                return Ok(());
            }
        }
    }
    Err("no audio player found".into())
}

pub async fn play_wav_async(wav_data: Vec<u8>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || play_wav_blocking(&wav_data))
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_list_is_nonempty() {
        assert!(!PLAYERS.is_empty());
    }
}
