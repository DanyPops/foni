/// DSP chain — Rust replacement for ffmpeg SmoothingProcessor.
///
/// Signal order mirrors pipeline/processors.ts buildPostFilter() exactly:
///   silence_trim → fade → highpass → tilt → de-ess → vibrato →
///   corrective EQ → compression → warmth → presence → air →
///   reverb → loudnorm → hard clip

use super::{
    dynamics::{hard_clip, loudnorm, Compressor},
    effects::{echo, fade_in, fade_out, silence_trim, vibrato},
    filters::Biquad,
};

/// All DSP parameters — mirrors TypeScript SmoothingOptions field names.
#[derive(Debug, Clone)]
pub struct SmoothingOptions {
    // Edge
    pub pad_secs:  f32,
    pub fade_secs: f32,

    // Mud
    pub highpass_freq: f32,

    // Corrective EQ
    pub de_box_freq: f32, pub de_box_db: f32, pub de_box_q: f32,
    pub de_harsh_freq: f32, pub de_harsh_db: f32, pub de_harsh_q: f32,

    // Dynamics
    pub compression_ratio: f32,
    pub compression_attack_ms: f32,
    pub compression_release_ms: f32,
    pub compression_threshold_db: f32,
    pub compression_makeup_db: f32,

    // Creative EQ
    pub warmth_boost_db: f32, pub warmth_freq: f32,
    pub presence_db: f32,
    pub air_boost_db: f32, pub air_freq: f32,

    // De-robotisation
    pub tilt_low_db: f32,
    pub tilt_high_db: f32,
    pub de_ess_db: f32,
    pub vibrato_freq: f32,
    pub vibrato_depth: f32,

    // Reverb
    pub reverb_ms: f32,
    pub reverb_decay: f32,
    pub reverb_in_gain: f32,
    pub reverb_out_gain: f32,

    // Output
    pub rms_target_lufs: f32,
    pub limiter_db: f32,
    pub silence_trim_db: f32,
    pub normalize: bool,
}

impl Default for SmoothingOptions {
    fn default() -> Self {
        SmoothingOptions {
            pad_secs:  0.3,
            fade_secs: 0.04,
            highpass_freq: 80.,
            de_box_freq: 900., de_box_db: 0., de_box_q: 0.9,
            de_harsh_freq: 3500., de_harsh_db: -2., de_harsh_q: 0.7,
            compression_ratio: 3.,
            compression_attack_ms: 10.,
            compression_release_ms: 80.,
            compression_threshold_db: -12.,
            compression_makeup_db: 4.,
            warmth_boost_db: 0., warmth_freq: 200.,
            presence_db: 0.,
            air_boost_db: 0., air_freq: 8000.,
            tilt_low_db: 8.,
            tilt_high_db: -6.,
            de_ess_db: 4.,
            vibrato_freq: 6.,
            vibrato_depth: 0.003,
            reverb_ms: 8.,
            reverb_decay: 0.04,
            reverb_in_gain: 0.8,
            reverb_out_gain: 0.88,
            rms_target_lufs: -11.,
            limiter_db: -1.,
            silence_trim_db: -40.,
            normalize: true,
        }
    }
}

/// Apply the full DSP chain to mono f32 samples. Returns processed samples.
pub fn apply(mut samples: Vec<f32>, sr: u32, opts: &SmoothingOptions) -> Vec<f32> {

    // 0. Silence trim (two-pass, preserves interior pauses)
    samples = silence_trim(samples, opts.silence_trim_db, sr);

    // 1. Fade in / out
    if opts.fade_secs > 0. {
        fade_in(&mut samples, opts.fade_secs, sr);
        fade_out(&mut samples, opts.fade_secs, sr);
    }

    // 2. Highpass
    if opts.highpass_freq > 0. {
        Biquad::highpass(opts.highpass_freq, sr).process(&mut samples);
    }

    // 3. Spectral tilt
    if opts.tilt_low_db != 0. {
        Biquad::lowshelf(100., opts.tilt_low_db, sr).process(&mut samples);
    }
    if opts.tilt_high_db != 0. {
        Biquad::highshelf(8000., opts.tilt_high_db, sr).process(&mut samples);
    }

    // 4. De-esser
    if opts.de_ess_db > 0. {
        Biquad::peaking(7000., -opts.de_ess_db, 1.4, sr).process(&mut samples);
    }

    // 5. Vibrato
    if opts.vibrato_freq > 0. && opts.vibrato_depth > 0. {
        vibrato(&mut samples, opts.vibrato_freq, opts.vibrato_depth, sr);
    }

    // 6. Corrective EQ
    if opts.de_box_db != 0. {
        Biquad::peaking(opts.de_box_freq, opts.de_box_db, opts.de_box_q, sr).process(&mut samples);
    }
    if opts.de_harsh_db != 0. {
        Biquad::peaking(opts.de_harsh_freq, opts.de_harsh_db, opts.de_harsh_q, sr).process(&mut samples);
    }

    // 7. Compression
    if opts.compression_ratio > 1. {
        Compressor::new(
            opts.compression_threshold_db, opts.compression_ratio,
            opts.compression_attack_ms, opts.compression_release_ms,
            opts.compression_makeup_db, sr,
        ).process(&mut samples);
    }

    // 8. Warmth
    if opts.warmth_boost_db != 0. {
        Biquad::lowshelf(opts.warmth_freq, opts.warmth_boost_db, sr).process(&mut samples);
    }

    // 9. Presence
    if opts.presence_db != 0. {
        Biquad::peaking(2500., opts.presence_db, 1.5, sr).process(&mut samples);
    }

    // 10. Air
    if opts.air_boost_db != 0. {
        Biquad::highshelf(opts.air_freq, opts.air_boost_db, sr).process(&mut samples);
    }

    // 11. Reverb
    if opts.reverb_ms > 0. && opts.reverb_decay > 0. {
        echo(&mut samples, opts.reverb_ms, opts.reverb_decay,
             opts.reverb_in_gain, opts.reverb_out_gain, sr);
    }

    // 12. Loudnorm
    if opts.normalize && opts.rms_target_lufs != 0. {
        loudnorm(&mut samples, sr, opts.rms_target_lufs);
    }

    // 13. Hard clip
    if opts.limiter_db < 0. {
        hard_clip(&mut samples, opts.limiter_db);
    }

    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n).map(|i| (2. * std::f32::consts::PI * freq * i as f32 / sr as f32).sin() * 0.5).collect()
    }

    fn peak(s: &[f32]) -> f32 { s.iter().map(|&x| x.abs()).fold(0f32, f32::max) }
    fn rms(s: &[f32])  -> f32 { (s.iter().map(|&x| x*x).sum::<f32>() / s.len() as f32).sqrt() }
    fn db(r: f32)       -> f32 { 20. * r.log10() }

    #[test]
    fn default_chain_does_not_clip() {
        let sig = sine(200., 1., 22050);
        let out = apply(sig, 22050, &SmoothingOptions::default());
        assert!(!out.is_empty());
        assert!(peak(&out) <= 0.892, "clipped: peak={:.3}", peak(&out)); // -1 dBFS ceiling
    }

    #[test]
    fn default_chain_raises_quiet_signal() {
        let mut sig = sine(200., 1., 22050);
        for s in sig.iter_mut() { *s *= 0.05; } // quiet but above silence threshold
        let rms_in = rms(&sig);
        let out = apply(sig, 22050, &SmoothingOptions::default());
        assert!(rms(&out) > rms_in, "loudnorm didn't raise level");
    }

    #[test]
    fn de_harsh_cuts_3500hz() {
        let sr = 22050u32;
        let opts = SmoothingOptions {
            normalize: false,
            limiter_db: 0.,
            silence_trim_db: 0.,
            fade_secs: 0.,
            reverb_ms: 0.,
            vibrato_freq: 0.,
            de_harsh_db: -6.,
            compression_ratio: 1.,
            tilt_low_db: 0., tilt_high_db: 0.,
            de_ess_db: 0.,
            de_box_db: 0.,
            pad_secs: 0.,
            ..SmoothingOptions::default()
        };
        let sig_3500 = sine(3500., 0.3, sr);
        let sig_200  = sine(200.,  0.3, sr);
        let out_3500 = apply(sig_3500.clone(), sr, &opts);
        let out_200  = apply(sig_200.clone(),  sr, &opts);
        // 3500 Hz should be attenuated relative to 200 Hz
        let delta = db(rms(&out_3500)) - db(rms(&out_200))
                  - (db(rms(&sig_3500)) - db(rms(&sig_200)));
        assert!(delta < -3., "de-harsh not cutting 3500Hz: delta={:.1}dB", delta);
    }
}
