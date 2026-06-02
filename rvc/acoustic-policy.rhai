// Acoustic correction policy — hot-reloadable, no recompile needed.
//
// Receives:
//   measured  — what the RVC output actually sounds like (read-only)
//   target    — what Sidorovich should sound like (read-only)
//   knobs     — current DSP settings (read-write, return modified)
//   damping   — how aggressively to correct (0.0–1.0)
//
// Edit this file, then POST /controller {"reload": true} to apply.

fn correct(measured, target, knobs, damping) {
    let err_bright = target.brightness_hz - measured.brightness_hz;
    let err_loud   = target.loudness_db - measured.loudness_db;
    let err_bass   = target.bass_balance_db - measured.bass_balance_db;
    let err_dark   = target.vocal_darkness_db_oct - measured.vocal_darkness_db_oct;

    // Sensitivity constants (from fonictl calibrate)
    let bright_per_tilt = 92.0;
    let loud_per_rms    = 0.975;
    let bass_per_tilt   = 0.21;
    let bass_per_pres   = -0.36;
    let dark_per_tilt   = 0.073;

    // Proportional corrections
    let d_tilt_bright = err_bright / bright_per_tilt;
    let d_tilt_dark   = err_dark / dark_per_tilt;
    let d_tilt_high   = (d_tilt_bright * 0.7 + d_tilt_dark * 0.3) * damping;
    let d_rms         = err_loud / loud_per_rms * damping;
    let d_tilt_low    = err_bass / bass_per_tilt * damping;
    let d_presence    = err_bass / bass_per_pres * damping * 0.5;

    // Conditional: surgical de-harsh when brightness is very far off
    let d_de_harsh = if err_bright < -200.0 {
        -4.0 * damping
    } else if err_bright < -100.0 {
        -2.0 * damping
    } else {
        0.0
    };

    // Apply with clamping
    knobs.tilt_high_db    = clamp(knobs.tilt_high_db + d_tilt_high, -24.0, 0.0);
    knobs.rms_target_lufs = clamp(knobs.rms_target_lufs + d_rms, -22.0, -4.0);
    knobs.tilt_low_db     = clamp(knobs.tilt_low_db + d_tilt_low, -4.0, 16.0);
    knobs.presence_db     = clamp(knobs.presence_db + d_presence, -6.0, 6.0);
    knobs.de_harsh_db     = clamp(knobs.de_harsh_db + d_de_harsh, -12.0, 0.0);

    knobs
}

// Clamp a value between lo and hi
fn clamp(val, lo, hi) {
    if val < lo { lo } else if val > hi { hi } else { val }
}
