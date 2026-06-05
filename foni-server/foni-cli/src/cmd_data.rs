use std::path::PathBuf;

pub fn cmd_clean(dir: &PathBuf, out: &PathBuf) {
    use foni_analyse::decode_wav;
    use tabled::{settings::Style, Table, Tabled};

    std::fs::create_dir_all(out).expect("create output dir");

    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("wav"))
        .collect();
    files.sort();

    #[derive(Tabled)]
    struct CleanRow {
        #[tabled(rename = "File")]
        file: String,
        #[tabled(rename = "Before")]
        before: String,
        #[tabled(rename = "After")]
        after: String,
        #[tabled(rename = "Clipping")]
        clipping: String,
    }

    let mut rows = Vec::new();
    let mut total_before = 0.0f32;
    let mut total_after = 0.0f32;

    for path in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let wav = match decode_wav(&bytes) {
            Ok(w) => w,
            Err(_) => continue,
        };
        let sr = wav.sample_rate;
        let mut samples = wav.samples;
        let before_dur = samples.len() as f32 / sr as f32;
        total_before += before_dur;

        // Trim silence (head/tail below -40 dBFS)
        let threshold = 10.0f32.powf(-40.0 / 20.0);
        let first = samples
            .iter()
            .position(|&s| s.abs() > threshold)
            .unwrap_or(0);
        let last = samples
            .iter()
            .rposition(|&s| s.abs() > threshold)
            .unwrap_or(samples.len());
        if first < last {
            samples = samples[first..=last].to_vec();
        }

        // Skip very short clips
        let after_dur = samples.len() as f32 / sr as f32;
        if after_dur < 0.5 {
            continue;
        }

        // Normalize RMS to -14 dBFS
        let rms = (samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        let target_rms = 10.0f32.powf(-14.0 / 20.0);
        if rms > 1e-8 {
            let gain = target_rms / rms;
            for s in samples.iter_mut() {
                *s *= gain;
            }
        }

        // Detect clipping
        let clipped = samples.iter().filter(|&&s| s.abs() > 0.99).count();

        // Write cleaned WAV
        let out_path = out.join(path.file_name().unwrap());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&out_path, spec).expect("create WAV");
        for &s in &samples {
            writer
                .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .ok();
        }
        writer.finalize().ok();

        total_after += after_dur;
        rows.push(CleanRow {
            file: path.file_name().unwrap().to_string_lossy().into_owned(),
            before: format!("{before_dur:.1}s"),
            after: format!("{after_dur:.1}s"),
            clipping: if clipped > 0 {
                format!("{clipped} samples")
            } else {
                String::new()
            },
        });
    }

    println!("{}", Table::new(&rows).with(Style::rounded()));
    eprintln!(
        "\n  {files} files, {before:.1}s \u{2192} {after:.1}s  \u{2192} {out}",
        files = rows.len(),
        before = total_before,
        after = total_after,
        out = out.display()
    );
}

pub fn cmd_augment(dir: &PathBuf, out: &PathBuf, speeds_csv: &str) {
    use foni_analyse::decode_wav;

    std::fs::create_dir_all(out).expect("create output dir");

    let speeds: Vec<f32> = speeds_csv
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if speeds.is_empty() {
        eprintln!("No valid speed factors");
        return;
    }

    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("wav"))
        .collect();
    files.sort();

    let mut total_files = 0usize;
    let mut total_dur = 0.0f32;

    for path in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let wav = match decode_wav(&bytes) {
            Ok(w) => w,
            Err(_) => continue,
        };
        let sr = wav.sample_rate;
        let stem = path.file_stem().unwrap().to_string_lossy();

        for &speed in &speeds {
            let suffix = format!("_s{}", (speed * 100.0) as u32);
            let out_name = format!("{stem}{suffix}.wav");

            // Resample: change duration without pitch shift
            let ratio = 1.0 / speed as f64;
            let out_len = (wav.samples.len() as f64 * ratio).ceil() as usize;
            let resampled: Vec<f32> = (0..out_len)
                .map(|i| {
                    let pos = i as f64 / ratio;
                    let lo = pos.floor() as usize;
                    let hi = (lo + 1).min(wav.samples.len() - 1);
                    let frac = (pos - lo as f64) as f32;
                    wav.samples[lo] * (1.0 - frac) + wav.samples[hi] * frac
                })
                .collect();

            let out_path = out.join(&out_name);
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: sr,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(&out_path, spec).expect("create WAV");
            for &s in &resampled {
                writer
                    .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                    .ok();
            }
            writer.finalize().ok();

            total_files += 1;
            total_dur += resampled.len() as f32 / sr as f32;
        }
    }

    eprintln!(
        "  {total_files} files ({:.1} min) \u{2192} {}",
        total_dur / 60.0,
        out.display()
    );
}

pub fn cmd_corpus(dir: &PathBuf, vs: Option<&PathBuf>) -> Result<(), String> {
    use foni_analyse::{
        analyse, analyse_fast, compute_gap, decode_wav, fast_f0_stats, format_gap_table,
        TargetTensor,
    };
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read dir: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("wav"))
        .collect();
    files.sort();

    if files.is_empty() {
        return Err(format!("No WAV files in {}", dir.display()));
    }

    let t0 = std::time::Instant::now();
    println!(
        "  Analysing {} files in parallel (fast F0 / McLeod, no pyin)…",
        files.len()
    );

    // Parallel accumulation — each file decoded and analysed independently.
    #[derive(Default)]
    struct Row {
        rms: f64,
        crest: f64,
        centroid: f64,
        f0: f64,
        f0_std: f64,
        voiced: f64,
    }
    let acc = Mutex::new(Vec::<Row>::new());
    let errors = AtomicU64::new(0);

    files.par_iter().for_each(|path| {
        match std::fs::read(path).and_then(|b| Ok(b)) {
            Ok(bytes) => match decode_wav(&bytes) {
                Ok(wav) => {
                    // Fast path: loudness + spectral + temporal (cheap) + McLeod F0.
                    // analyse_fast() skips pyin (1400 ms/file) and voice metrics.
                    let r = analyse_fast(&wav.samples, wav.sample_rate);
                    let (f0, f0_std, vr) = fast_f0_stats(&wav.samples, wav.sample_rate);
                    let row = Row {
                        rms: r.loudness.rms_db as f64,
                        crest: r.loudness.crest_factor as f64,
                        centroid: r.spectral.brightness_hz as f64,
                        f0: f0 as f64,
                        f0_std: f0_std as f64,
                        voiced: vr as f64,
                    };
                    acc.lock().unwrap().push(row);
                }
                Err(e) => {
                    eprintln!("  skip {}: {e}", path.display());
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            },
            Err(e) => {
                eprintln!("  skip {}: {e}", path.display());
                errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    let rows = acc.into_inner().unwrap();
    let n = rows.len();
    if n == 0 {
        return Err("All files failed.".into());
    }

    let mean = |f: fn(&Row) -> f64| rows.iter().map(|r| f(r)).sum::<f64>() / n as f64;
    let rms = mean(|r| r.rms);
    let crest = mean(|r| r.crest);
    let centroid = mean(|r| r.centroid);
    let f0 = mean(|r| r.f0);
    let f0_std = mean(|r| r.f0_std);
    let voiced = mean(|r| r.voiced);
    let elapsed = t0.elapsed().as_millis();
    let errs = errors.load(Ordering::Relaxed);

    eprintln!("  Done in {elapsed} ms  ({} files, {} skipped)\n", n, errs);

    // ── Sidorovich acoustic identity (bass-baritone deep Russian voice) ────────
    //
    // Parameters from literature (Kob et al. 2022, PMC9605961; Sundberg 1987;
    // SwiftF0 benchmark 2025):
    //
    //   F0 (speaking)  Bass:      75–100 Hz   Baritone: 96–130 Hz
    //   Spectral cent. Bass:    <2400 Hz      Baritone: 2400–2700 Hz
    //   FHE (1–5 kHz)  Bass:  2384±164 Hz     Baritone: 2454±206 Hz
    //   Crest factor   Conversational speech: 12–16 dB
    //   Voiced ratio   Clean studio speech:    60–85 %
    {
        use tabled::{settings::Style, Table, Tabled};
        #[derive(Tabled)]
        struct CorpusRow {
            #[tabled(rename = "Metric")]
            metric: &'static str,
            #[tabled(rename = "Value")]
            value: String,
            #[tabled(rename = "Target")]
            target: &'static str,
        }
        let rows = vec![
            CorpusRow {
                metric: "Pitch",
                value: format!("{f0:.1} Hz"),
                target: "bass-baritone: 80-130 Hz",
            },
            CorpusRow {
                metric: "Pitch variation",
                value: format!("{f0_std:.1} Hz"),
                target: "higher = more expressive",
            },
            CorpusRow {
                metric: "Brightness",
                value: format!("{centroid:.0} Hz"),
                target: "bass<2400, baritone 2400-2700",
            },
            CorpusRow {
                metric: "RMS level",
                value: format!("{rms:.1} dBFS"),
                target: "studio: -13 to -15 dBFS",
            },
            CorpusRow {
                metric: "Crest factor",
                value: format!("{crest:.1} dB"),
                target: "speech: 12-16 dB",
            },
            CorpusRow {
                metric: "Voiced ratio",
                value: format!("{:.1}%", voiced * 100.0),
                target: "studio: 60-85%",
            },
        ];
        eprintln!("\n  Sidorovich corpus fingerprint ({n} files, {elapsed} ms, {errs} skipped)");
        println!("{}", Table::new(&rows).with(Style::rounded()));
    }

    if let Some(ref_path) = vs {
        let ref_bytes = std::fs::read(ref_path).expect("cannot read reference");
        let ref_wav = decode_wav(&ref_bytes).expect("cannot decode reference");
        let ref_an = analyse(&ref_wav.samples, ref_wav.sample_rate);
        let tensor = TargetTensor::from_analysis(&ref_an, ref_path.to_str().unwrap_or("?"));
        let med_bytes = std::fs::read(&files[files.len() / 2]).expect("cannot read median file");
        let med_wav = decode_wav(&med_bytes).expect("cannot decode median");
        let med_an = analyse(&med_wav.samples, med_wav.sample_rate);
        let gap = compute_gap("corpus-median", &med_an, &tensor);
        println!("\n(median file vs reference WAV)");
        println!("{}", format_gap_table(&gap));
    }
    Ok(())
}
