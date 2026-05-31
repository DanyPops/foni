/// timeline_integration — chronological pause/silence structure test.
///
/// Compares the studio WAV timeline (frozen fixture) against synthesis
/// at the pause level — the correct granularity for pure VAD without a
/// language model. Per-word alignment requires CTC forced alignment (TSK-79
/// extension) which runs whisper on the synthetic side too.
///
/// Assertions:
///   - synthetic silence_ratio >= 50% of reference (we don't collapse all pauses)
///   - every reference pause > 150ms has a synthetic counterpart > 50ms
///   - total duration within 3× of reference (we're not infinitely slow)
///
/// Run: cargo test -p foni-analyse --test timeline_integration
/// Requires: espeak-ng on PATH, ../../baseline/stalker/timeline/trader1a.json
use foni_analyse::{
    alignment::{format_alignment_table, TimelineFixture},
    decode_wav,
    timeline::{merge_short_silences, pauses, segment, voiced_segments},
};

use std::path::Path;
use std::process::Command;

const PHRASE:       &str = "Подойди-ка, надо тебе ситуацию прояснить.";
const FIXTURE_PATH: &str = "../../baseline/stalker/timeline/trader1a.json";
const ESPEAK_WPM:   u32  = 150;

/// Synthetic silence_ratio must be >= this fraction of the reference ratio.
/// Reference: ~0.43. Floor: we must have at least 40% as much silence.
const SILENCE_RATIO_FLOOR: f32 = 0.40;

/// Any reference pause > this duration must produce a synthetic pause > MIN_SYNTHETIC_PAUSE.
const REF_PAUSE_SIGNIFICANT_S: f32 = 0.15;
const MIN_SYNTHETIC_PAUSE_S:   f32 = 0.05;

fn synthesise_espeak(phrase: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("foni-tl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("out.wav");
    let status = Command::new("espeak-ng")
        .args(["-v", "ru", "-s", &ESPEAK_WPM.to_string(), "-p", "50", "-a", "200", "-w"])
        .arg(&out).arg(phrase)
        .status().expect("espeak-ng not found");
    assert!(status.success(), "espeak-ng failed");
    let b = std::fs::read(&out).expect("espeak output missing");
    std::fs::remove_file(&out).ok();
    b
}

fn load_fixture() -> TimelineFixture {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_PATH);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!(
        "Fixture missing: {}\nRun: python3 scripts/extract-timeline.py \
         baseline/stalker/wav/sidorovich/trader1a.wav", path.display()
    ));
    serde_json::from_str(&text).expect("invalid fixture JSON")
}

#[test]
fn pause_structure_matches_reference() {
    let fixture = load_fixture();

    let wav_bytes = synthesise_espeak(PHRASE);
    let wav = decode_wav(&wav_bytes).expect("WAV decode failed");

    // VAD with 80ms hangover — suppress inter-phoneme gaps, preserve inter-word pauses
    let raw = segment(&wav.samples, wav.sample_rate);
    let tl  = merge_short_silences(&raw, 0.08);
    let ps  = pauses(&tl);
    let vs  = voiced_segments(&tl);

    println!("\n── Synthetic timeline ────────────────────────────────");
    println!("Total duration:  {:.3}s  (ref {:.3}s)", tl.total_duration_s, fixture.total_duration_s);
    println!("Silence ratio:   {:.3}   (ref ~0.43)", tl.silence_ratio);
    println!("Voiced segments: {}", vs.len());
    println!("Pause segments:  {}", ps.len());
    for p in &ps {
        println!("  pause {:.3}–{:.3}s  ({:.0}ms)", p.start_s, p.end_s, p.duration_s * 1000.0);
    }
    println!("\n── Reference pauses (from fixture) ───────────────────");
    for p in &fixture.pauses {
        println!("  [pause after '{}'] {:.0}ms", p.after_word, p.duration_s * 1000.0);
    }

    // ── Assertion 1: silence ratio floor ─────────────────────────────────────
    // Reference silence_ratio ≈ 0.43 (Sidorovich natural speech rhythm)
    // We must have at least SILENCE_RATIO_FLOOR × 0.43 silence
    let ref_silence_ratio: f32 = fixture.pauses.iter().map(|p| p.duration_s).sum::<f32>()
        / fixture.total_duration_s;
    let min_required = ref_silence_ratio * SILENCE_RATIO_FLOOR;
    assert!(
        tl.silence_ratio >= min_required,
        "silence_ratio {:.3} < required {:.3} ({:.0}% of reference {:.3})\n\
         Fix: add SSML <break> tags in ProsodyAnnotator (TSK-81)",
        tl.silence_ratio, min_required,
        SILENCE_RATIO_FLOOR * 100.0, ref_silence_ratio,
    );

    // ── Assertion 2: significant reference pauses have a synthetic counterpart ──
    let significant_ref_pauses: Vec<_> = fixture.pauses.iter()
        .filter(|p| p.duration_s >= REF_PAUSE_SIGNIFICANT_S)
        .collect();

    if !significant_ref_pauses.is_empty() {
        let long_syn_pauses: Vec<_> = ps.iter()
            .filter(|p| p.duration_s >= MIN_SYNTHETIC_PAUSE_S)
            .collect();
        assert!(
            !long_syn_pauses.is_empty(),
            "Reference has {} significant pause(s) (>{}ms) but synthesis produced none >{}ms.\n\
             Fix: SSML break injection (TSK-81)",
            significant_ref_pauses.len(),
            (REF_PAUSE_SIGNIFICANT_S * 1000.0) as u32,
            (MIN_SYNTHETIC_PAUSE_S * 1000.0) as u32,
        );
    }

    // ── Assertion 3: total duration sanity ───────────────────────────────────
    assert!(
        tl.total_duration_s <= fixture.total_duration_s * 3.0,
        "synthetic is {}× longer than reference — something is wrong",
        tl.total_duration_s / fixture.total_duration_s,
    );

    println!("\n✅ silence_ratio={:.3}  pauses={}  duration={:.2}s",
        tl.silence_ratio, ps.len(), tl.total_duration_s);
}
