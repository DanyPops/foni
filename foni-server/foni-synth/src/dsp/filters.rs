/// Biquad IIR filters — Audio EQ Cookbook (Robert Bristow-Johnson).
/// All coefficients computed at filter construction; apply() is O(n).

#[derive(Debug, Clone)]
pub struct Biquad {
    b0: f64, b1: f64, b2: f64,
    a1: f64, a2: f64,
    x1: f64, x2: f64,   // input  delay line
    y1: f64, y2: f64,   // output delay line
}

impl Biquad {
    fn new(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Biquad { b0: b0/a0, b1: b1/a0, b2: b2/a0, a1: a1/a0, a2: a2/a0,
                 x1: 0., x2: 0., y1: 0., y2: 0. }
    }

    pub fn process_sample(&mut self, x: f32) -> f32 {
        let x = x as f64;
        let y = self.b0*x + self.b1*self.x1 + self.b2*self.x2
                           - self.a1*self.y1 - self.a2*self.y2;
        self.x2 = self.x1; self.x1 = x;
        self.y2 = self.y1; self.y1 = y;
        y as f32
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() { *s = self.process_sample(*s); }
    }

    // ── Constructors ──────────────────────────────────────────────────────────

    /// High-pass filter (2nd order Butterworth).
    pub fn highpass(cutoff_hz: f32, sample_rate: u32) -> Self {
        let w0 = 2. * std::f64::consts::PI * cutoff_hz as f64 / sample_rate as f64;
        let cos_w0 = w0.cos();
        let q = std::f64::consts::FRAC_1_SQRT_2; // 0.707 = maximally flat
        let alpha = w0.sin() / (2. * q);
        Self::new(
            (1. + cos_w0) / 2.,
            -(1. + cos_w0),
            (1. + cos_w0) / 2.,
            1. + alpha,
            -2. * cos_w0,
            1. - alpha,
        )
    }

    /// Low-shelf filter (±dBgain at dc→shelf_hz).
    pub fn lowshelf(freq_hz: f32, gain_db: f32, sample_rate: u32) -> Self {
        let w0    = 2. * std::f64::consts::PI * freq_hz as f64 / sample_rate as f64;
        let a     = 10f64.powf(gain_db as f64 / 40.);
        let cos_w0 = w0.cos();
        let s     = 1.0_f64; // shelf slope = 1
        let alpha = w0.sin() / 2. * ((a + 1./a) * (1./s - 1.) + 2.).sqrt();
        let sq    = 2. * a.sqrt() * alpha;
        Self::new(
            a  * ((a+1.) - (a-1.)*cos_w0 + sq),
            2. * a  * ((a-1.) - (a+1.)*cos_w0),
            a  * ((a+1.) - (a-1.)*cos_w0 - sq),
            (a+1.) + (a-1.)*cos_w0 + sq,
            -2. * ((a-1.) + (a+1.)*cos_w0),
            (a+1.) + (a-1.)*cos_w0 - sq,
        )
    }

    /// High-shelf filter (±dBgain above freq_hz).
    pub fn highshelf(freq_hz: f32, gain_db: f32, sample_rate: u32) -> Self {
        let w0    = 2. * std::f64::consts::PI * freq_hz as f64 / sample_rate as f64;
        let a     = 10f64.powf(gain_db as f64 / 40.);
        let cos_w0 = w0.cos();
        let s     = 1.0_f64;
        let alpha = w0.sin() / 2. * ((a + 1./a) * (1./s - 1.) + 2.).sqrt();
        let sq    = 2. * a.sqrt() * alpha;
        Self::new(
            a  * ((a+1.) + (a-1.)*cos_w0 + sq),
           -2. * a  * ((a-1.) + (a+1.)*cos_w0),
            a  * ((a+1.) + (a-1.)*cos_w0 - sq),
            (a+1.) - (a-1.)*cos_w0 + sq,
            2.  * ((a-1.) - (a+1.)*cos_w0),
            (a+1.) - (a-1.)*cos_w0 - sq,
        )
    }

    /// Peaking EQ (±dBgain centred at freq_hz, Q controls bandwidth).
    pub fn peaking(freq_hz: f32, gain_db: f32, q: f32, sample_rate: u32) -> Self {
        let w0    = 2. * std::f64::consts::PI * freq_hz as f64 / sample_rate as f64;
        let a     = 10f64.powf(gain_db as f64 / 40.);
        let alpha = w0.sin() / (2. * q as f64);
        Self::new(
            1. + alpha * a,
            -2. * w0.cos(),
            1. - alpha * a,
            1. + alpha / a,
            -2. * w0.cos(),
            1. - alpha / a,
        )
    }
}

/// Apply a chain of biquads in sequence (mono f32 samples in-place).
pub fn apply_chain(filters: &mut [Biquad], samples: &mut [f32]) {
    for f in filters.iter_mut() { f.process(samples); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq_hz: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n).map(|i| (2. * std::f32::consts::PI * freq_hz * i as f32 / sr as f32).sin()).collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|&x| x*x).sum::<f32>() / s.len() as f32).sqrt()
    }

    fn db(r: f32) -> f32 { 20. * r.log10() }

    #[test]
    fn highpass_attenuates_below_cutoff() {
        let sr = 22050u32;
        let mut sig = sine(40., 0.5, sr);
        let mut hp  = Biquad::highpass(200., sr);
        hp.process(&mut sig);
        // 40 Hz should be attenuated > 10 dB below a 400 Hz reference
        let mut ref_sig = sine(400., 0.5, sr);
        hp = Biquad::highpass(200., sr);
        hp.process(&mut ref_sig);
        let atten = db(rms(&sig)) - db(rms(&ref_sig));
        assert!(atten < -10., "attenuation={:.1}dB", atten);
    }

    #[test]
    fn lowshelf_boosts_below_shelf() {
        let sr = 22050u32;
        let freq = 200f32;
        let gain_db = 6.;
        let mut sig = sine(100., 0.5, sr);
        let rms_before = rms(&sig);
        let mut f = Biquad::lowshelf(freq, gain_db, sr);
        f.process(&mut sig);
        let rms_after = rms(&sig);
        assert!(db(rms_after) - db(rms_before) > 3., "low-shelf boost insufficient");
    }

    #[test]
    fn peaking_cuts_at_target_frequency() {
        let sr = 22050u32;
        let cut_hz = 3500f32;
        let mut sig = sine(cut_hz, 0.5, sr);
        let rms_before = rms(&sig);
        let mut f = Biquad::peaking(cut_hz, -6., 1.41, sr);
        f.process(&mut sig);
        let cut_db = db(rms_before) - db(rms(&sig));
        assert!(cut_db > 3., "peaking cut insufficient: {:.1}dB", cut_db);
    }
}
