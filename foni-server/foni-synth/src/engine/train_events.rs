use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TrainEvent {
    CheckpointCached { path: String },
    CheckpointDownloading,
    DatasetReady { wav_files: u32, transcripts: u32 },
    VqStarted,
    VqProgress { files: u32, total: u32, hours: f32 },
    VqComplete { files: u32, hours: f32 },
    DatasetBuilt { shards: u32 },
    TrainingStarted { steps: u32 },
    ModelLoading { params: String },
    ModelLoaded { trainable: String, total: String },
    SanityCheck,
    TrainingStep { step: u32, loss: Option<f32> },
    TrainingComplete { elapsed_secs: Option<u32> },
    MergingLora { checkpoint: String },
    ModelSaved { path: String },
    Done,
    Error { message: String },
    Oom { message: String },
    Log { message: String },
}

pub fn parse_log_line(line: &str) -> Option<TrainEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Checkpoint
    if line.contains("already cached at") {
        let path = line.rsplit("at ").next().unwrap_or("").to_string();
        return Some(TrainEvent::CheckpointCached { path });
    }
    if line.contains("downloading") && line.contains("HuggingFace") {
        return Some(TrainEvent::CheckpointDownloading);
    }

    // Dataset
    if let Some(caps) = Regex::new(r"\[train\] (\d+) WAV files")
        .ok()
        .and_then(|r| r.captures(line))
    {
        return Some(TrainEvent::DatasetReady {
            wav_files: caps[1].parse().unwrap_or(0),
            transcripts: 0,
        });
    }
    if let Some(caps) = Regex::new(r"\[train\] (\d+) files with transcripts")
        .ok()
        .and_then(|r| r.captures(line))
    {
        return Some(TrainEvent::DatasetReady {
            wav_files: 0,
            transcripts: caps[1].parse().unwrap_or(0),
        });
    }

    // VQ extraction
    if line.contains("[train] extracting semantic tokens") {
        return Some(TrainEvent::VqStarted);
    }
    if let Some(caps) = Regex::new(r"Processed (\d+)/(\d+) files.*?(\d+\.\d+) hours")
        .ok()
        .and_then(|r| r.captures(line))
    {
        return Some(TrainEvent::VqProgress {
            files: caps[1].parse().unwrap_or(0),
            total: caps[2].parse().unwrap_or(0),
            hours: caps[3].parse().unwrap_or(0.0),
        });
    }
    if let Some(caps) = Regex::new(r"Finished processing (\d+) files.*?(\d+\.\d+) hours")
        .ok()
        .and_then(|r| r.captures(line))
    {
        return Some(TrainEvent::VqComplete {
            files: caps[1].parse().unwrap_or(0),
            hours: caps[2].parse().unwrap_or(0.0),
        });
    }

    // Dataset build
    if let Some(caps) = Regex::new(r"Finished writing (\d+) shards")
        .ok()
        .and_then(|r| r.captures(line))
    {
        return Some(TrainEvent::DatasetBuilt {
            shards: caps[1].parse().unwrap_or(0),
        });
    }

    // Training
    if let Some(caps) = Regex::new(r"\[train\] fine-tuning (\d+) steps")
        .ok()
        .and_then(|r| r.captures(line))
    {
        return Some(TrainEvent::TrainingStarted {
            steps: caps[1].parse().unwrap_or(0),
        });
    }
    if line.contains("Loading model from") {
        let params = line.rsplit("config: ").next().unwrap_or("").to_string();
        return Some(TrainEvent::ModelLoading { params });
    }
    if let Some(caps) =
        Regex::new(r"(\d+[\.\d]*\s*[MBK])\s+Trainable.*?(\d+[\.\d]*\s*[MBG])\s+Total params")
            .ok()
            .and_then(|r| r.captures(line))
    {
        return Some(TrainEvent::ModelLoaded {
            trainable: caps[1].to_string(),
            total: caps[2].to_string(),
        });
    }
    if line.contains("Sanity Checking") {
        return Some(TrainEvent::SanityCheck);
    }
    if let Some(caps) = Regex::new(r"step[= ](\d+).*?loss[= ]([\d.]+)")
        .ok()
        .and_then(|r| r.captures(line))
    {
        return Some(TrainEvent::TrainingStep {
            step: caps[1].parse().unwrap_or(0),
            loss: caps[2].parse().ok(),
        });
    }
    if let Some(caps) = Regex::new(r"\[train\] training done in (\d+)s")
        .ok()
        .and_then(|r| r.captures(line))
    {
        return Some(TrainEvent::TrainingComplete {
            elapsed_secs: caps[1].parse().ok(),
        });
    }

    // LoRA merge
    if line.contains("[train] merging LoRA") {
        let ckpt = line.rsplit(": ").next().unwrap_or("").to_string();
        return Some(TrainEvent::MergingLora { checkpoint: ckpt });
    }
    if line.contains("[train] model saved") {
        let path = line.rsplit("to ").next().unwrap_or("").to_string();
        return Some(TrainEvent::ModelSaved { path });
    }
    if line.contains("[train] DONE") {
        return Some(TrainEvent::Done);
    }

    // Errors
    if line.contains("CUDA out of memory") || line.contains("OutOfMemoryError") {
        return Some(TrainEvent::Oom {
            message: line.to_string(),
        });
    }
    if line.contains("CalledProcessError")
        || line.contains("RuntimeError")
        || line.contains("Error executing job")
        || line.contains("crash-looping")
    {
        return Some(TrainEvent::Error {
            message: line.to_string(),
        });
    }

    // Skip noise (warnings, deprecation notices, blank lines)
    if line.contains("SyntaxWarning")
        || line.contains("FutureWarning")
        || line.contains("UserWarning")
        || line.contains("deprecat")
        || line.starts_with("[1A")
        || line.starts_with("  ")
    {
        return None;
    }

    Some(TrainEvent::Log {
        message: line.to_string(),
    })
}

pub fn parse_log_batch(text: &str) -> Vec<TrainEvent> {
    text.lines().filter_map(parse_log_line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_checkpoint_cached() {
        let e = parse_log_line("[checkpoint] s2-pro already cached at /data/checkpoints/s2-pro");
        assert!(matches!(e, Some(TrainEvent::CheckpointCached { .. })));
    }

    #[test]
    fn parse_dataset_ready() {
        let e = parse_log_line("[train] 63 WAV files in /data/dataset-raw");
        assert!(matches!(
            e,
            Some(TrainEvent::DatasetReady { wav_files: 63, .. })
        ));
    }

    #[test]
    fn parse_vq_progress() {
        let e = parse_log_line("2026-06-04 13:23:14.891 | INFO     | __main__:main:229 | RANK: 0 / 1 - Processed 20/126 files, 0.08 hours of audio, ETA: 0:01:36s");
        assert!(matches!(
            e,
            Some(TrainEvent::VqProgress {
                files: 20,
                total: 126,
                ..
            })
        ));
    }

    #[test]
    fn parse_vq_complete() {
        let e = parse_log_line("2026-06-04 13:23:26.504 | INFO     | __main__:main:234 | RANK: 0 / 1 - Finished processing 126 files, 0.31 hours of audio");
        assert!(matches!(e, Some(TrainEvent::VqComplete { files: 126, .. })));
    }

    #[test]
    fn parse_training_started() {
        let e = parse_log_line("[train] fine-tuning 10 steps (A100)...");
        assert!(matches!(e, Some(TrainEvent::TrainingStarted { steps: 10 })));
    }

    #[test]
    fn parse_training_complete() {
        let e = parse_log_line("[train] training done in 342s");
        assert!(matches!(
            e,
            Some(TrainEvent::TrainingComplete {
                elapsed_secs: Some(342)
            })
        ));
    }

    #[test]
    fn parse_done() {
        let e = parse_log_line("[train] DONE");
        assert!(matches!(e, Some(TrainEvent::Done)));
    }

    #[test]
    fn parse_oom() {
        let e = parse_log_line(
            "torch.OutOfMemoryError: CUDA out of memory. Tried to allocate 7.98 GiB.",
        );
        assert!(matches!(e, Some(TrainEvent::Oom { .. })));
    }

    #[test]
    fn parse_error() {
        let e = parse_log_line("subprocess.CalledProcessError: Command failed");
        assert!(matches!(e, Some(TrainEvent::Error { .. })));
    }

    #[test]
    fn parse_model_saved() {
        let e = parse_log_line("[train] model saved to /data/output/sidorovich");
        assert!(matches!(e, Some(TrainEvent::ModelSaved { .. })));
    }

    #[test]
    fn skip_warnings() {
        assert!(parse_log_line("SyntaxWarning: invalid escape sequence").is_none());
        assert!(parse_log_line("FutureWarning: deprecated").is_none());
    }

    #[test]
    fn skip_empty() {
        assert!(parse_log_line("").is_none());
        assert!(parse_log_line("   ").is_none());
    }

    #[test]
    fn parse_batch() {
        let text = "[checkpoint] s2-pro already cached at /data/checkpoints/s2-pro\n\
                    [train] 63 WAV files in /data/dataset-raw\n\
                    SyntaxWarning: blah\n\
                    [train] extracting semantic tokens...\n\
                    [train] DONE";
        let events = parse_log_batch(text);
        assert_eq!(events.len(), 4); // cached, dataset, vq_started, done (warning skipped)
    }

    #[test]
    fn events_serialize_to_json() {
        let e = TrainEvent::VqProgress {
            files: 40,
            total: 126,
            hours: 0.11,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"event\":\"vq_progress\""));
        assert!(json.contains("\"files\":40"));
    }
}
