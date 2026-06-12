use std::process::Command;

use tokio::sync::watch;

const PLAYERS: &[&str] = &["paplay", "afplay", "mpv", "aplay", "ffplay"];

pub fn play_wav_blocking(wav_data: &[u8]) -> Result<(), String> {
    let tmp = tmp_path();
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

/// Spawn the first available audio player for `path` and return the child handle.
///
/// The caller owns the child and is responsible for `child.wait()` and temp-file cleanup.
pub fn play_wav_spawn(path: &std::path::Path) -> Result<tokio::process::Child, String> {
    for player in PLAYERS {
        if let Ok(child) = tokio::process::Command::new(player)
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            return Ok(child);
        }
    }
    Err("no audio player found".into())
}

/// Play `wav_data` through the first available player.
///
/// Monitors `kill_rx`: if the watch value changes while the player is running,
/// the subprocess is killed and the function returns immediately.
pub async fn play_wav_killable(
    wav_data: Vec<u8>,
    kill_rx: &mut watch::Receiver<u64>,
) -> Result<(), String> {
    let tmp = tmp_path();
    std::fs::write(&tmp, &wav_data).map_err(|e| format!("write temp wav: {e}"))?;
    let result = play_file_killable(&tmp, kill_rx).await;
    let _ = std::fs::remove_file(&tmp);
    result
}

async fn play_file_killable(
    path: &std::path::Path,
    kill_rx: &mut watch::Receiver<u64>,
) -> Result<(), String> {
    for player in PLAYERS {
        if kill_rx.has_changed().unwrap_or(false) {
            return Ok(());
        }
        let Ok(mut child) = tokio::process::Command::new(player)
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(s) if s.success() => return Ok(()),
                    Ok(_) => continue,
                    Err(e) => return Err(e.to_string()),
                }
            }
            _ = kill_rx.changed() => {
                let _ = child.kill().await;
                let _ = kill_rx.borrow_and_update();
                return Ok(());
            }
        }
    }
    Err("no audio player found".into())
}

fn tmp_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "foni_play_{}.wav",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_list_is_nonempty() {
        assert!(!PLAYERS.is_empty());
    }
}
