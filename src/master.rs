//! Mastering: the processing on the final stereo mix, and a loudness meter.
//!
//! Chain: low cut → 3-band EQ (low shelf, mid bell, air shelf) → glue compressor →
//! tape saturation → stereo width (with mono bass) → output gain → look-ahead limiter.
//! Loudness follows ITU-R BS.1770-4 / EBU R128 (K-weighting, 400 ms momentary,
//! 3 s short-term, gated integrated). Everything is allocation-free after `new`.

use crate::dsp::{decay_coef, Biquad};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MasterPreset {
    Bypass,
    Gentle,
    WarmTape,
    LoudPunchy,
    WideAiry,
    GameHeadroom,
}

impl MasterPreset {
    pub const ALL: [MasterPreset; 6] = [
        MasterPreset::Bypass,
        MasterPreset::Gentle,
        MasterPreset::WarmTape,
        MasterPreset::LoudPunchy,
        MasterPreset::WideAiry,
        MasterPreset::GameHeadroom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MasterPreset::Bypass => "Bypass",
            MasterPreset::Gentle => "Gentle glue",
            MasterPreset::WarmTape => "Warm tape",
            MasterPreset::LoudPunchy => "Loud & punchy",
            MasterPreset::WideAiry => "Wide & airy",
            MasterPreset::GameHeadroom => "Game (headroom)",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            MasterPreset::Bypass => "No mastering: the raw mix with the safety limiter only",
            MasterPreset::Gentle => "Light compression and a touch of air; keeps the dynamics",
            MasterPreset::WarmTape => "Rounder lows, softer highs and tape saturation",
            MasterPreset::LoudPunchy => "Harder compression and limiting for a loud, upfront sound",
            MasterPreset::WideAiry => "Open top end and a wider stereo image",
            MasterPreset::GameHeadroom => "Controlled peaks and -2 dB ceiling so it sits under game SFX and dialogue",
        }
    }

    pub fn params(self) -> MasterParams {
        let base = MasterParams { preset: self, ..MasterParams::default() };
        match self {
            MasterPreset::Bypass => MasterParams { enabled: false, ..base },
            MasterPreset::Gentle => base,
            MasterPreset::WarmTape => MasterParams {
                low_db: 2.0,
                mid_db: -1.5,
                mid_hz: 2500.0,
                air_db: -1.0,
                threshold_db: -20.0,
                ratio: 2.5,
                attack_ms: 20.0,
                release_ms: 250.0,
                makeup_db: 3.0,
                saturation: 0.55,
                ..base
            },
            MasterPreset::LoudPunchy => MasterParams {
                low_cut_hz: 35.0,
                low_db: 1.5,
                mid_db: 1.0,
                mid_hz: 1200.0,
                air_db: 2.5,
                threshold_db: -24.0,
                ratio: 3.5,
                attack_ms: 10.0,
                release_ms: 120.0,
                makeup_db: 6.0,
                saturation: 0.3,
                width: 1.15,
                output_db: 3.0,
                ..base
            },
            MasterPreset::WideAiry => MasterParams {
                air_db: 3.0,
                mid_db: -1.0,
                mid_hz: 400.0,
                width: 1.4,
                threshold_db: -20.0,
                ratio: 1.8,
                ..base
            },
            MasterPreset::GameHeadroom => MasterParams {
                low_cut_hz: 40.0,
                threshold_db: -22.0,
                ratio: 3.0,
                attack_ms: 15.0,
                release_ms: 180.0,
                makeup_db: 3.0,
                saturation: 0.1,
                width: 1.0,
                ceiling_db: -2.0,
                ..base
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasterParams {
    pub enabled: bool,
    /// Which preset these values came from (for the GUI).
    pub preset: MasterPreset,
    pub low_cut_hz: f32,
    /// Low shelf at 110 Hz.
    pub low_db: f32,
    pub mid_db: f32,
    pub mid_hz: f32,
    /// Air shelf at 9 kHz.
    pub air_db: f32,
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_db: f32,
    /// Tape-style soft saturation, 0..1.
    pub saturation: f32,
    /// Stereo width: 0 = mono, 1 = unchanged, 2 = very wide.
    pub width: f32,
    /// Keep everything under ~120 Hz in mono (tighter low end, safe on any speaker).
    pub mono_bass: bool,
    pub output_db: f32,
    /// Limiter ceiling, dBFS.
    pub ceiling_db: f32,
}

impl Default for MasterParams {
    fn default() -> Self {
        MasterParams {
            enabled: true,
            preset: MasterPreset::Gentle,
            low_cut_hz: 30.0,
            low_db: 1.0,
            mid_db: 0.0,
            mid_hz: 800.0,
            air_db: 1.5,
            threshold_db: -18.0,
            ratio: 2.0,
            attack_ms: 30.0,
            release_ms: 200.0,
            makeup_db: 2.0,
            saturation: 0.15,
            width: 1.1,
            mono_bass: true,
            output_db: 0.0,
            ceiling_db: -1.0,
        }
    }
}

#[inline]
fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}

/// Look-ahead peak limiter: hold the minimum required gain over the look-ahead window,
/// release exponentially, then smooth with a box filter of the same length. The audio is
/// delayed by the window, so the gain is already down when a peak arrives (no overshoot).
struct LookaheadLimiter {
    len: usize,
    delay: [Vec<f32>; 2],
    didx: usize,
    req: Vec<f32>,
    smooth: Vec<f32>,
    idx: usize,
    sum: f64,
    released: f32,
    release: f32,
    pub gain: f32,
}

impl LookaheadLimiter {
    fn new(sr: f32) -> Self {
        let len = ((0.002 * sr) as usize).max(4);
        LookaheadLimiter {
            len,
            // Audio is delayed by len-1 so every gain value averaged at the output
            // already "saw" that sample's requirement.
            delay: [vec![0.0; len - 1], vec![0.0; len - 1]],
            didx: 0,
            req: vec![1.0; len],
            smooth: vec![1.0; len],
            idx: 0,
            sum: len as f64,
            released: 1.0,
            release: 1.0 - decay_coef(0.12, sr),
            gain: 1.0,
        }
    }

    #[inline]
    fn tick(&mut self, l: f32, r: f32, ceiling: f32) -> (f32, f32) {
        let peak = l.abs().max(r.abs());
        let need = if peak > ceiling { ceiling / peak } else { 1.0 };
        let i = self.idx;
        self.req[i] = need;
        let held = self.req.iter().fold(1.0f32, |a, &b| a.min(b));
        self.released = (self.released + (1.0 - self.released) * self.release).min(held);
        self.sum += self.released as f64 - self.smooth[i] as f64;
        self.smooth[i] = self.released;
        let g = (self.sum / self.len as f64) as f32;
        let d = self.didx;
        let out = (self.delay[0][d] * g, self.delay[1][d] * g);
        self.delay[0][d] = l;
        self.delay[1][d] = r;
        self.didx = (d + 1) % self.delay[0].len();
        self.idx = (i + 1) % self.len;
        if self.idx == 0 {
            // Re-sum now and then so float error can't accumulate.
            self.sum = self.smooth.iter().map(|&x| x as f64).sum();
        }
        self.gain = g;
        out
    }
}

pub struct Master {
    sr: f32,
    params: MasterParams,
    low_cut: [[Biquad; 2]; 2],
    low: [Biquad; 2],
    mid: [Biquad; 2],
    air: [Biquad; 2],
    side_low: Biquad,
    env_db: f32,
    att: f32,
    rel: f32,
    limiter: LookaheadLimiter,
    /// Current compressor gain reduction in dB (≥ 0), for metering.
    pub comp_gr_db: f32,
}

impl Master {
    pub fn new(sr: f32, params: MasterParams) -> Self {
        let mut m = Master {
            sr,
            params,
            low_cut: [[Biquad::default(); 2]; 2],
            low: [Biquad::default(); 2],
            mid: [Biquad::default(); 2],
            air: [Biquad::default(); 2],
            side_low: Biquad::lowpass(120.0, 0.7, sr),
            env_db: 0.0,
            att: 0.0,
            rel: 0.0,
            limiter: LookaheadLimiter::new(sr),
            comp_gr_db: 0.0,
        };
        m.set_params(params);
        m
    }

    pub fn set_params(&mut self, p: MasterParams) {
        let sr = self.sr;
        self.params = p;
        for ch in 0..2 {
            // 4th-order low cut (two Butterworth sections).
            self.low_cut[ch][0].retune(Biquad::highpass(p.low_cut_hz.max(10.0), 0.541, sr));
            self.low_cut[ch][1].retune(Biquad::highpass(p.low_cut_hz.max(10.0), 1.307, sr));
            self.low[ch].retune(Biquad::lowshelf(110.0, 0.7, p.low_db, sr));
            self.mid[ch].retune(Biquad::peaking(p.mid_hz, 0.9, p.mid_db, sr));
            self.air[ch].retune(Biquad::highshelf(9000.0, 0.7, p.air_db, sr));
        }
        self.att = 1.0 - decay_coef(p.attack_ms.max(0.1) / 1000.0, sr);
        self.rel = 1.0 - decay_coef(p.release_ms.max(1.0) / 1000.0, sr);
    }

    /// Total gain reduction right now (compressor + limiter), dB.
    pub fn gain_reduction_db(&self) -> f32 {
        self.comp_gr_db - gain_to_db(self.limiter.gain)
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let p = self.params;
        let mut x = [l, r];
        for ch in 0..2 {
            let mut v = x[ch];
            v = self.low_cut[ch][0].tick(v);
            v = self.low_cut[ch][1].tick(v);
            v = self.low[ch].tick(v);
            v = self.mid[ch].tick(v);
            v = self.air[ch].tick(v);
            x[ch] = v;
        }

        // Glue compressor: stereo-linked, peak detector, 6 dB soft knee.
        let level_db = gain_to_db(x[0].abs().max(x[1].abs()));
        let k = if level_db > self.env_db { self.att } else { self.rel };
        self.env_db += (level_db - self.env_db) * k;
        let over = self.env_db - p.threshold_db;
        const KNEE: f32 = 6.0;
        let slope = 1.0 - 1.0 / p.ratio.max(1.0);
        let gr = if over <= -KNEE / 2.0 {
            0.0
        } else if over >= KNEE / 2.0 {
            over * slope
        } else {
            slope * (over + KNEE / 2.0).powi(2) / (2.0 * KNEE)
        };
        self.comp_gr_db = gr;
        let g = db_to_gain(p.makeup_db - gr);
        x[0] *= g;
        x[1] *= g;

        // Tape saturation: unity gain for small signals, rounds off peaks.
        if p.saturation > 0.0 {
            let d = 1.0 + 3.0 * p.saturation;
            for v in x.iter_mut() {
                *v = (*v * d).tanh() / d;
            }
        }

        // Width (mid/side), keeping the bass centred.
        let mid = 0.5 * (x[0] + x[1]);
        let mut side = 0.5 * (x[0] - x[1]) * p.width;
        if p.mono_bass {
            side -= self.side_low.tick(side);
        }
        let out = db_to_gain(p.output_db);
        let (l, r) = ((mid + side) * out, (mid - side) * out);
        self.limiter.tick(l, r, db_to_gain(p.ceiling_db))
    }
}

// ---------------------------------------------------------------------------
// Loudness (BS.1770-4)

#[derive(Clone, Copy, Default)]
struct Biquad64 {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad64 {
    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// K-weighting filters for any sample rate (same parameterisation as pyloudnorm).
fn k_weighting(sr: f64) -> [Biquad64; 2] {
    use std::f64::consts::PI;
    // Stage 1: high shelf, +4 dB above ~1.7 kHz (head acoustics).
    let (g, q, fc) = (3.999_843_853_973_347, 0.707_175_236_955_419_6, 1_681.974_450_955_533);
    let a = 10f64.powf(g / 40.0);
    let w0 = 2.0 * PI * fc / sr;
    let (c, alpha) = (w0.cos(), w0.sin() / (2.0 * q));
    let sa = 2.0 * a.sqrt() * alpha;
    let a0 = (a + 1.0) - (a - 1.0) * c + sa;
    let shelf = Biquad64 {
        b: [
            a * ((a + 1.0) + (a - 1.0) * c + sa) / a0,
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c) / a0,
            a * ((a + 1.0) + (a - 1.0) * c - sa) / a0,
        ],
        a: [2.0 * ((a - 1.0) - (a + 1.0) * c) / a0, ((a + 1.0) - (a - 1.0) * c - sa) / a0],
        z: [0.0; 2],
    };
    // Stage 2: RLB high-pass at ~38 Hz.
    let (q, fc) = (0.500_327_037_323_877_3, 38.135_470_876_024_44);
    let w0 = 2.0 * PI * fc / sr;
    let (c, alpha) = (w0.cos(), w0.sin() / (2.0 * q));
    let a0 = 1.0 + alpha;
    let hp = Biquad64 {
        b: [(1.0 + c) / 2.0 / a0, -(1.0 + c) / a0, (1.0 + c) / 2.0 / a0],
        a: [-2.0 * c / a0, (1.0 - alpha) / a0],
        z: [0.0; 2],
    };
    [shelf, hp]
}

const SUB_BLOCKS: usize = 30; // 30 × 100 ms = 3 s short-term window
const HIST_MIN: f64 = -70.0;
const HIST_BINS: usize = 800; // 0.1 LU bins from -70 to +10 LUFS

#[derive(Clone, Copy, Debug)]
pub struct LoudnessReading {
    pub momentary: f32,
    pub short_term: f32,
    pub integrated: f32,
    /// Highest sample peak since reset, dBFS.
    pub peak_db: f32,
}

impl Default for LoudnessReading {
    /// "Nothing measured yet" (shown as —).
    fn default() -> Self {
        LoudnessReading { momentary: -99.0, short_term: -99.0, integrated: -99.0, peak_db: -99.0 }
    }
}

pub struct LoudnessMeter {
    filters: [[Biquad64; 2]; 2],
    sub_len: usize,
    sub_count: usize,
    sub_acc: f64,
    subs: [f64; SUB_BLOCKS],
    sub_idx: usize,
    subs_filled: usize,
    hist_count: Vec<u32>,
    hist_power: Vec<f64>,
    peak: f32,
    pub reading: LoudnessReading,
}

fn lufs(power: f64) -> f32 {
    if power <= 1e-12 { -99.0 } else { (-0.691 + 10.0 * power.log10()) as f32 }
}

impl LoudnessMeter {
    pub fn new(sr: f32) -> Self {
        let k = k_weighting(sr as f64);
        LoudnessMeter {
            filters: [k, k],
            sub_len: (sr as usize / 10).max(1),
            sub_count: 0,
            sub_acc: 0.0,
            subs: [0.0; SUB_BLOCKS],
            sub_idx: 0,
            subs_filled: 0,
            hist_count: vec![0; HIST_BINS],
            hist_power: vec![0.0; HIST_BINS],
            peak: 0.0,
            reading: LoudnessReading::default(),
        }
    }

    pub fn reset(&mut self) {
        self.subs = [0.0; SUB_BLOCKS];
        self.subs_filled = 0;
        self.sub_acc = 0.0;
        self.sub_count = 0;
        self.hist_count.iter_mut().for_each(|c| *c = 0);
        self.hist_power.iter_mut().for_each(|p| *p = 0.0);
        self.peak = 0.0;
        self.reading = LoudnessReading::default();
    }

    fn window(&self, n: usize) -> f64 {
        let n = n.min(self.subs_filled).max(1);
        (0..n).map(|i| self.subs[(self.sub_idx + SUB_BLOCKS - 1 - i) % SUB_BLOCKS]).sum::<f64>() / n as f64
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) {
        self.peak = self.peak.max(l.abs()).max(r.abs());
        let mut sum = 0.0;
        for (ch, &x) in [l, r].iter().enumerate() {
            let [a, b] = &mut self.filters[ch];
            let y = b.tick(a.tick(x as f64));
            sum += y * y;
        }
        self.sub_acc += sum;
        self.sub_count += 1;
        if self.sub_count == self.sub_len {
            self.subs[self.sub_idx] = self.sub_acc / self.sub_len as f64;
            self.sub_idx = (self.sub_idx + 1) % SUB_BLOCKS;
            self.subs_filled = (self.subs_filled + 1).min(SUB_BLOCKS);
            self.sub_acc = 0.0;
            self.sub_count = 0;
            self.on_sub_block();
        }
    }

    fn on_sub_block(&mut self) {
        let momentary = self.window(4);
        self.reading.momentary = lufs(momentary);
        self.reading.short_term = lufs(self.window(SUB_BLOCKS));
        self.reading.peak_db = gain_to_db(self.peak);
        if self.subs_filled >= 4 {
            // 400 ms gating blocks with 75 % overlap → one every 100 ms.
            let l = lufs(momentary) as f64;
            if l > HIST_MIN {
                let bin = (((l - HIST_MIN) * 10.0) as usize).min(HIST_BINS - 1);
                self.hist_count[bin] += 1;
                self.hist_power[bin] += momentary;
            }
            self.reading.integrated = self.integrated();
        }
    }

    /// Gated integrated loudness: absolute gate -70 LUFS, relative gate -10 LU.
    pub fn integrated(&self) -> f32 {
        let (n, p) = self.gated_sum(0);
        if n == 0 {
            return -99.0;
        }
        let rel = lufs(p / n as f64) as f64 - 10.0;
        let start = (((rel - HIST_MIN) * 10.0).max(0.0) as usize).min(HIST_BINS);
        let (n, p) = self.gated_sum(start);
        if n == 0 { -99.0 } else { lufs(p / n as f64) }
    }

    fn gated_sum(&self, from: usize) -> (u64, f64) {
        let mut n = 0u64;
        let mut p = 0.0;
        for i in from..HIST_BINS {
            n += self.hist_count[i] as u64;
            p += self.hist_power[i];
        }
        (n, p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(sr: f32, f: f32, amp: f32, secs: f32) -> impl Iterator<Item = f32> {
        (0..(sr * secs) as usize).map(move |i| amp * (std::f32::consts::TAU * f * i as f32 / sr).sin())
    }

    #[test]
    fn meter_reads_reference_tone() {
        // BS.1770: a 0 dBFS 997 Hz sine in one channel reads -3.01 LKFS; in both channels
        // 0 LKFS. So -20 dBFS in both reads ≈ -20 LUFS.
        let sr = 48_000.0;
        let mut m = LoudnessMeter::new(sr);
        for x in sine(sr, 997.0, 0.1, 10.0) {
            m.tick(x, x);
        }
        let i = m.integrated();
        assert!((i - -20.0).abs() < 0.3, "integrated {i}");
        assert!((m.reading.short_term - -20.0).abs() < 0.3);
    }

    #[test]
    fn gating_ignores_silence() {
        let sr = 48_000.0;
        let mut m = LoudnessMeter::new(sr);
        for x in sine(sr, 997.0, 0.1, 5.0).chain(std::iter::repeat_n(0.0, 48_000 * 20)) {
            m.tick(x, x);
        }
        assert!((m.integrated() - -20.0).abs() < 0.5, "{}", m.integrated());
    }

    #[test]
    fn limiter_holds_ceiling_without_overshoot() {
        let sr = 48_000.0;
        let p = MasterParams { output_db: 12.0, ceiling_db: -1.0, ..MasterParams::default() };
        let mut m = Master::new(sr, p);
        let ceil = db_to_gain(-1.0) + 1e-4;
        for (i, x) in sine(sr, 220.0, 0.5, 2.0).enumerate() {
            // Add sharp transients too.
            let x = if i % 9_000 == 0 { 1.0 } else { x };
            let (l, r) = m.tick(x, x);
            assert!(l.abs() <= ceil && r.abs() <= ceil, "sample {i}: {l} {r}");
        }
    }

    #[test]
    fn compressor_reduces_loud_signals_only() {
        let sr = 48_000.0;
        let p = MasterParams { saturation: 0.0, makeup_db: 0.0, ..MasterParams::default() };
        let mut m = Master::new(sr, p);
        sine(sr, 440.0, 0.01, 1.0).for_each(|x| {
            m.tick(x, x);
        });
        assert!(m.comp_gr_db < 0.1, "quiet signal compressed by {}", m.comp_gr_db);
        sine(sr, 440.0, 0.8, 1.0).for_each(|x| {
            m.tick(x, x);
        });
        assert!(m.comp_gr_db > 3.0, "loud signal only compressed by {}", m.comp_gr_db);
    }
}
