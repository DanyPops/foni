//! Voice index: k-NN blending of ContentVec features with reference speaker vectors.
//!
//! Mirrors Python rvc_python pipeline.py:
//!   score, ix = index.search(npy, k=8)
//!   weight = np.square(1 / score)
//!   weight /= weight.sum(axis=1, keepdims=True)
//!   npy = np.sum(big_npy[ix] * weight[:, :, None], axis=1)
//!   feats = npy * index_rate + (1 - index_rate) * feats
//!
//! Loaded from `<models_dir>/<model>/voice_index_vectors.npy`.
//! Generate with: uv run --with faiss-cpu rvc/export_voice_index.py

use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const K: usize = 8;
const D: usize = 768;

/// Flat array of reference speaker embeddings: [N, D] row-major f32.
#[derive(Clone)]
pub struct VoiceIndex {
    pub vecs: Arc<Vec<f32>>,
    pub n: usize,
}

impl VoiceIndex {
    /// Load from a .npy file exported by export_voice_index.py.
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let vecs = parse_npy_f32(&bytes)?;
        let n = vecs.len() / D;
        if vecs.len() != n * D {
            return Err(format!("npy: expected multiple of {D}, got {}", vecs.len()));
        }
        tracing::info!("voice index: {n} × {D}d from {}", path.display());
        Ok(Self {
            vecs: Arc::new(vecs),
            n,
        })
    }

    /// Resolve the index path for a given model name and models directory.
    pub fn path_for(models_dir: &Path, model_name: &str) -> PathBuf {
        models_dir.join(model_name).join("voice_index_vectors.npy")
    }

    /// Blend `phone` features with K-NN retrieved speaker vectors.
    ///
    /// `phone` — [t_prime, D] row-major f32 (ContentVec output before repeat-by-2).
    /// Returns blended [t_prime, D] row-major f32.
    pub fn blend(&self, phone: &[f32], t_prime: usize, index_rate: f32) -> Vec<f32> {
        assert_eq!(phone.len(), t_prime * D);

        let vecs = self.vecs.as_slice();
        let n = self.n;

        // Precompute squared norms of all reference vectors once.
        let ref_norms: Vec<f32> = (0..n)
            .into_par_iter()
            .map(|i| l2sq(&vecs[i * D..(i + 1) * D]))
            .collect();

        // Process each frame in parallel.
        let out: Vec<f32> = (0..t_prime)
            .into_par_iter()
            .flat_map_iter(|t| {
                let query = &phone[t * D..(t + 1) * D];
                let query_sq = l2sq(query);

                // Compute L2² distances: ||q - v||² = ||q||² - 2 q·v + ||v||²
                let mut dists: Vec<(f32, usize)> = (0..n)
                    .map(|i| {
                        let dot = dot(query, &vecs[i * D..(i + 1) * D]);
                        let dist = (query_sq - 2.0 * dot + ref_norms[i]).max(0.0);
                        (dist, i)
                    })
                    .collect();

                // Partial sort: rearrange so dists[..k] contains the k smallest.
                let k = K.min(dists.len());
                if k < dists.len() {
                    dists.select_nth_unstable_by(k, |a, b| a.0.partial_cmp(&b.0).unwrap());
                }
                let top_k = &dists[..k];

                // Inverse-squared distance weights, normalised.
                let w: Vec<f32> = top_k
                    .iter()
                    .map(|(d, _)| 1.0 / d.max(1e-6f32).powi(2))
                    .collect();
                let w_sum: f32 = w.iter().sum();
                let w: Vec<f32> = w.iter().map(|x| x / w_sum).collect();

                // Weighted centroid of retrieved vectors.
                let mut blended = vec![0.0f32; D];
                for (j, (_, idx)) in top_k.iter().enumerate() {
                    let rv = &vecs[idx * D..(idx + 1) * D];
                    for (b, r) in blended.iter_mut().zip(rv) {
                        *b += w[j] * r;
                    }
                }

                // Final blend with original ContentVec features.
                let out_frame: Vec<f32> = blended
                    .iter()
                    .zip(query)
                    .map(|(b, q)| index_rate * b + (1.0 - index_rate) * q)
                    .collect();
                out_frame.into_iter()
            })
            .collect();

        out
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

#[inline]
fn l2sq(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum()
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Minimal .npy parser — supports only C-order float32 arrays.
fn parse_npy_f32(bytes: &[u8]) -> Result<Vec<f32>, String> {
    // Magic + version
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err("not a .npy file".into());
    }
    let major = bytes[6];
    let header_len_bytes = if major == 1 { 2usize } else { 4 };
    let header_offset = 10;
    if bytes.len() < header_offset + header_len_bytes {
        return Err("npy: truncated header length".into());
    }
    let hlen = if major == 1 {
        u16::from_le_bytes(bytes[header_offset..header_offset + 2].try_into().unwrap()) as usize
    } else {
        u32::from_le_bytes(bytes[header_offset..header_offset + 4].try_into().unwrap()) as usize
    };
    let data_start = header_offset + header_len_bytes + hlen;
    if bytes.len() < data_start {
        return Err("npy: data starts past end of file".into());
    }
    let header = std::str::from_utf8(&bytes[header_offset + header_len_bytes..data_start])
        .map_err(|_| "npy: non-utf8 header")?;
    if !header.contains("'float32'") && !header.contains("float32") {
        return Err(format!("npy: expected float32, got header: {header}"));
    }
    if header.contains("'F'") {
        return Err("npy: Fortran-order arrays not supported".into());
    }
    let data = &bytes[data_start..];
    if data.len() % 4 != 0 {
        return Err("npy: data length not divisible by 4".into());
    }
    let n = data.len() / 4;
    let mut out = vec![0.0f32; n];
    for (i, chunk) in data.chunks_exact(4).enumerate() {
        out[i] = f32::from_le_bytes(chunk.try_into().unwrap());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_identity_at_rate_zero() {
        // index_rate=0 → output == input unchanged
        let n = 10;
        let vecs: Vec<f32> = (0..n * D).map(|i| (i % 100) as f32 / 100.0).collect();
        let idx = VoiceIndex {
            vecs: Arc::new(vecs),
            n,
        };
        let phone: Vec<f32> = (0..D).map(|i| i as f32 / D as f32).collect();
        let out = idx.blend(&phone, 1, 0.0);
        for (a, b) in out.iter().zip(&phone) {
            assert!((a - b).abs() < 1e-6, "rate=0 must be identity");
        }
    }

    #[test]
    fn blend_retrieves_nearest_at_rate_one() {
        // index_rate=1 → output is blended neighbour, not query
        let n = 4;
        let mut vecs = vec![0.0f32; n * D];
        // Ref vector 0: all 1.0 — will be nearest to query of all 0.9
        for v in &mut vecs[..D] {
            *v = 1.0;
        }
        let idx = VoiceIndex {
            vecs: Arc::new(vecs),
            n,
        };
        let phone = vec![0.9f32; D];
        let out = idx.blend(&phone, 1, 1.0);
        // All output values should be close to 1.0 (pulled toward ref[0])
        assert!(
            out.iter().all(|x| (x - 1.0).abs() < 0.1),
            "should be near ref[0]"
        );
    }
}
