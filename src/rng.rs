//! Small deterministic RNG (SplitMix64). Allocation-free and `Copy`, so it can live
//! in the audio thread and be split into independent per-part streams.

#[derive(Clone, Copy, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut r = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
        r.next_u64();
        r
    }

    /// An independent stream derived from `seed` and a stream id.
    pub fn stream(seed: u64, id: u64) -> Self {
        Rng::new(seed ^ id.wrapping_mul(0x2545_F491_4F6C_DD1D).rotate_left(17))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in [-1, 1).
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        self.f32() * 2.0 - 1.0
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }

    /// Approximately standard-normal (Irwin–Hall, 4 terms).
    pub fn gauss(&mut self) -> f32 {
        (self.f32() + self.f32() + self.f32() + self.f32() - 2.0) * 1.732
    }
}

/// Long-range-correlated ("1/f-like") noise for human timing and dynamics.
///
/// Human timing deviations are not white jitter: a player drifts ahead and behind over
/// seconds, and listeners prefer that (Hennig et al., 2011, "The nature and perception of
/// fluctuations in human musical rhythms", PLoS ONE). A sum of AR(1) processes with
/// time constants spread over decades approximates 1/f noise. `next()` is roughly unit variance.
#[derive(Clone, Copy, Debug)]
pub struct Drift {
    state: [f32; 3],
    k: [f32; 3],
    rng: Rng,
}

impl Drift {
    /// Time constants are in steps (e.g. pulses).
    pub fn new(seed: u64, id: u64, taus: [f32; 3]) -> Self {
        Drift { state: [0.0; 3], k: taus.map(|t| (-1.0 / t.max(1.0)).exp()), rng: Rng::stream(seed, id) }
    }

    pub fn next(&mut self) -> f32 {
        let mut sum = 0.0;
        for i in 0..3 {
            let k = self.k[i];
            self.state[i] = k * self.state[i] + (1.0 - k * k).sqrt() * self.rng.gauss();
            sum += self.state[i];
        }
        sum / 3f32.sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drift_is_correlated_and_unit_scale() {
        let mut d = Drift::new(1, 2, [3.0, 12.0, 48.0]);
        let xs: Vec<f32> = (0..20_000).map(|_| d.next()).collect();
        let var = xs.iter().map(|x| x * x).sum::<f32>() / xs.len() as f32;
        let lag1 = xs.windows(2).map(|w| w[0] * w[1]).sum::<f32>() / xs.len() as f32 / var;
        assert!((0.5..1.6).contains(&var), "variance {var}");
        assert!(lag1 > 0.6, "should be strongly correlated, lag-1 {lag1}");
    }
}
