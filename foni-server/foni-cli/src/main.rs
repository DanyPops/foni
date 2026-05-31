use std::path::PathBuf;
use foni_analyse::{analyse, compute_gap, decode_wav, format_gap_table, TargetTensor};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: foni-cli <file.wav> [--vs <reference.wav>] [--json]");
        std::process::exit(1);
    }

    let path = PathBuf::from(&args[1]);
    let json_mode = args.contains(&"--json".to_string());

    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("error reading {}: {e}", path.display());
        std::process::exit(1);
    });

    let wav = decode_wav(&bytes).unwrap_or_else(|e| {
        eprintln!("WAV decode error: {e}");
        std::process::exit(1);
    });

    let analysis = analyse(&wav.samples, wav.sample_rate);

    // Optional: compare against a reference WAV
    if let Some(ref_pos) = args.iter().position(|a| a == "--vs") {
        if let Some(ref_path) = args.get(ref_pos + 1) {
            let ref_bytes = std::fs::read(ref_path).expect("cannot read reference WAV");
            let ref_wav   = decode_wav(&ref_bytes).expect("cannot decode reference WAV");
            let ref_analysis = analyse(&ref_wav.samples, ref_wav.sample_rate);
            let tensor = TargetTensor::from_analysis(&ref_analysis, ref_path);
            let gap = compute_gap(path.to_str().unwrap_or("?"), &analysis, &tensor);
            if json_mode {
                println!("{}", serde_json::to_string_pretty(&gap).unwrap());
            } else {
                println!("{}", format_gap_table(&gap));
            }
            return;
        }
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&analysis).unwrap());
    } else {
        println!("Duration:  {:.2}s", analysis.temporal.duration_secs);
        println!("Speech rt: {:.1} frames/s", analysis.temporal.speech_rate);
        println!("Pauses:    {} × {:.0}ms avg", analysis.temporal.pause_count, analysis.temporal.mean_pause_duration * 1000.0);
        println!("RMS:       {:.1} dBFS", analysis.loudness.rms_db);
        println!("Crest:     {:.1} dB", analysis.loudness.crest_factor);
        println!("Centroid:  {:.0} Hz", analysis.spectral.centroid_hz);
        println!("Flatness:  {:.3}", analysis.spectral.flatness);
        println!("ZCR:       {:.0}/s", analysis.spectral.zero_crossing_rate);
    }
}
