/// onnx_validation — TSK-9: validate RVC generator ONNX loads and runs in Rust via ort.
///
/// Requires: rvc/models/bandit/onnx/generator.onnx (built by rvc/export_onnx.py)
///
/// cargo test -p foni-synth --test onnx_validation -- --nocapture
use std::path::Path;

#[test]
fn generator_onnx_loads_and_produces_audio() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let generator_path = manifest.join("../../rvc/models/bandit/onnx/generator.onnx");

    if !generator_path.exists() {
        println!(
            "⚠  Generator ONNX not found at {}\n   Run: python3 rvc/export_onnx.py bandit",
            generator_path.display()
        );
        return; // soft skip — don't fail CI without the model
    }

    println!(
        "Loading generator ONNX from {}...",
        generator_path.display()
    );

    let shape = foni_synth::routes::convert::validate_generator_onnx(&generator_path)
        .expect("ONNX validation failed");

    println!("✅ Generator output shape: {:?}", shape);

    // Output is [1, 1, N_samples]
    assert_eq!(shape.len(), 3, "expected 3-dim audio output");
    assert_eq!(shape[0], 1, "batch dim should be 1");
    assert_eq!(shape[1], 1, "channel dim should be 1");
    assert!(
        shape[2] > 1000,
        "should produce > 1000 audio samples, got {}",
        shape[2]
    );

    println!("✅ TSK-9 complete: RVC ONNX generator validated in Rust via ort");
}

#[test]
fn rmvpe_onnx_loads() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let rmvpe_path = manifest.join("../../rvc/models/pretrained/rmvpe.onnx");

    if !rmvpe_path.exists() {
        println!("⚠  RMVPE ONNX not found — download from HuggingFace lj1995/VoiceConversionWebUI");
        return;
    }

    use ort::session::Session;
    let sess = Session::builder()
        .expect("ort builder")
        .commit_from_file(&rmvpe_path)
        .expect("load rmvpe.onnx");

    let inputs: Vec<_> = sess.inputs().iter().map(|i| i.name().to_string()).collect();
    let outputs: Vec<_> = sess
        .outputs()
        .iter()
        .map(|o| o.name().to_string())
        .collect();
    println!("RMVPE inputs:  {:?}", inputs);
    println!("RMVPE outputs: {:?}", outputs);
    println!("✅ RMVPE ONNX loads successfully");
}
