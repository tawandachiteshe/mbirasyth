//! DSP building blocks. Everything here is allocation-free after construction.

use std::f32::consts::PI;

/// A decaying sinusoidal mode in complex-phasor form (docs/04 §2).
/// Unconditionally stable, exact frequency, can be re-excited while ringing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mode {
    re: f32,
    im: f32,
    c: f32,
    s: f32,
    /// 1 - r: input scale that gives ~unity gain at resonance for continuous drive.
    pub one_minus_r: f32,
    pub enabled: bool,
}

impl Mode {
    pub fn set(&mut self, freq: f32, t60: f32, sr: f32) {
        self.enabled = freq > 0.0 && freq < 0.45 * sr;
        if !self.enabled {
            self.re = 0.0;
            self.im = 0.0;
            return;
        }
        let w = 2.0 * PI * freq / sr;
        let r = (-6.907_755 / (t60.max(1e-3) * sr)).exp();
        self.c = r * w.cos();
        self.s = r * w.sin();
        self.one_minus_r = 1.0 - r;
    }

    #[inline]
    pub fn tick(&mut self, input: f32) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let re = self.re * self.c - self.im * self.s;
        let im = self.re * self.s + self.im * self.c + input;
        self.re = re;
        self.im = im;
        im
    }

    /// Change frequency, keeping the decay rate (for pitch glides).
    pub fn set_freq(&mut self, freq: f32, sr: f32) {
        if !self.enabled {
            return;
        }
        let r = (self.c * self.c + self.s * self.s).sqrt();
        let w = 2.0 * PI * freq / sr;
        self.c = r * w.cos();
        self.s = r * w.sin();
    }

    /// Change the decay time, keeping the frequency (e.g. to damp a ringing string).
    pub fn set_t60(&mut self, t60: f32, sr: f32) {
        if !self.enabled {
            return;
        }
        let w = self.s.atan2(self.c);
        let r = (-6.907_755 / (t60.max(1e-3) * sr)).exp();
        self.c = r * w.cos();
        self.s = r * w.sin();
        self.one_minus_r = 1.0 - r;
    }

    /// Add an impulse of amplitude `a` at phase `phase` (radians, 0 = starts at zero crossing).
    pub fn strike(&mut self, a: f32, phase: f32) {
        if self.enabled {
            self.re += a * phase.cos();
            self.im += a * phase.sin();
        }
    }

    pub fn damp(&mut self, k: f32) {
        self.re *= k;
        self.im *= k;
    }

    pub fn energy(&self) -> f32 {
        self.re * self.re + self.im * self.im
    }

    pub fn silence(&mut self) {
        self.re = 0.0;
        self.im = 0.0;
    }
}

/// RBJ biquad, transposed direct form II.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn from_raw(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn wc(freq: f32, q: f32, sr: f32) -> (f32, f32) {
        let w = 2.0 * PI * freq.clamp(10.0, 0.49 * sr) / sr;
        (w.cos(), w.sin() / (2.0 * q))
    }

    pub fn lowpass(freq: f32, q: f32, sr: f32) -> Self {
        let (c, alpha) = Self::wc(freq, q, sr);
        Self::from_raw((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn highpass(freq: f32, q: f32, sr: f32) -> Self {
        let (c, alpha) = Self::wc(freq, q, sr);
        Self::from_raw((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    /// Band-pass with 0 dB peak gain.
    pub fn bandpass(freq: f32, q: f32, sr: f32) -> Self {
        let (c, alpha) = Self::wc(freq, q, sr);
        Self::from_raw(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn peaking(freq: f32, q: f32, gain_db: f32, sr: f32) -> Self {
        let (c, alpha) = Self::wc(freq, q, sr);
        let a = 10f32.powf(gain_db / 40.0);
        Self::from_raw(
            1.0 + alpha * a,
            -2.0 * c,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * c,
            1.0 - alpha / a,
        )
    }

    /// RBJ low shelf (`gain_db` below `freq`).
    pub fn lowshelf(freq: f32, q: f32, gain_db: f32, sr: f32) -> Self {
        let (c, alpha) = Self::wc(freq, q, sr);
        let a = 10f32.powf(gain_db / 40.0);
        let sa = 2.0 * a.sqrt() * alpha;
        Self::from_raw(
            a * ((a + 1.0) - (a - 1.0) * c + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - sa),
            (a + 1.0) + (a - 1.0) * c + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - sa,
        )
    }

    /// RBJ high shelf (`gain_db` above `freq`).
    pub fn highshelf(freq: f32, q: f32, gain_db: f32, sr: f32) -> Self {
        let (c, alpha) = Self::wc(freq, q, sr);
        let a = 10f32.powf(gain_db / 40.0);
        let sa = 2.0 * a.sqrt() * alpha;
        Self::from_raw(
            a * ((a + 1.0) + (a - 1.0) * c + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - sa),
            (a + 1.0) - (a - 1.0) * c + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - sa,
        )
    }

    /// Replace coefficients, keep state (for live parameter changes).
    pub fn retune(&mut self, other: Biquad) {
        let (z1, z2) = (self.z1, self.z2);
        *self = other;
        self.z1 = z1;
        self.z2 = z2;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Peak envelope follower with separate attack/release times.
#[derive(Clone, Copy, Debug, Default)]
pub struct EnvFollower {
    att: f32,
    rel: f32,
    pub value: f32,
}

impl EnvFollower {
    pub fn new(attack_s: f32, release_s: f32, sr: f32) -> Self {
        EnvFollower {
            att: 1.0 - (-1.0 / (attack_s * sr)).exp(),
            rel: 1.0 - (-1.0 / (release_s * sr)).exp(),
            value: 0.0,
        }
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let k = if x > self.value { self.att } else { self.rel };
        self.value += (x - self.value) * k;
        self.value
    }
}

/// Coefficient for a one-pole exponential decay with time constant `tau` seconds.
pub fn decay_coef(tau: f32, sr: f32) -> f32 {
    (-1.0 / (tau.max(1e-5) * sr)).exp()
}

/// Equal-power pan, `p` in [-1, 1].
pub fn pan_gains(p: f32) -> (f32, f32) {
    let a = (p.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    (a.cos(), a.sin())
}

// ---------------------------------------------------------------------------
// Freeverb-style stereo reverb (buffers allocated once in `new`).

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Comb { buf: vec![0.0; len.max(1)], idx: 0, store: 0.0 }
    }
    #[inline]
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = x + self.store * feedback;
        self.idx += 1;
        if self.idx == self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Allpass { buf: vec![0.0; len.max(1)], idx: 0 }
    }
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        self.buf[self.idx] = x + b * 0.5;
        self.idx += 1;
        if self.idx == self.buf.len() {
            self.idx = 0;
        }
        b - x
    }
}

pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    pub room: f32,
    pub damp: f32,
}

impl Reverb {
    pub fn new(sr: f32) -> Self {
        const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
        const APS: [usize; 4] = [556, 441, 341, 225];
        let scale = sr / 44_100.0;
        let mk = |spread: usize| -> (Vec<Comb>, Vec<Allpass>) {
            (
                COMBS.iter().map(|&l| Comb::new(((l + spread) as f32 * scale) as usize)).collect(),
                APS.iter().map(|&l| Allpass::new(((l + spread) as f32 * scale) as usize)).collect(),
            )
        };
        let (cl, al) = mk(0);
        let (cr, ar) = mk(23);
        Reverb { combs: [cl, cr], allpasses: [al, ar], room: 0.75, damp: 0.4 }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let input = (l + r) * 0.015;
        let fb = 0.7 + 0.28 * self.room;
        let mut out = [0.0f32; 2];
        for ch in 0..2 {
            let mut acc = 0.0;
            for c in self.combs[ch].iter_mut() {
                acc += c.tick(input, fb, self.damp);
            }
            for a in self.allpasses[ch].iter_mut() {
                acc = a.tick(acc);
            }
            out[ch] = acc;
        }
        (out[0], out[1])
    }
}

/// Look-ahead-free peak limiter followed by a soft clip.
#[derive(Clone, Copy, Debug)]
pub struct Limiter {
    gain: f32,
    release: f32,
    ceiling: f32,
}

impl Limiter {
    pub fn new(sr: f32) -> Self {
        Limiter { gain: 1.0, release: 1.0 - decay_coef(0.25, sr), ceiling: 0.89 }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let peak = l.abs().max(r.abs());
        let target = if peak * self.gain > self.ceiling { self.ceiling / peak } else { 1.0 };
        if target < self.gain {
            self.gain = target;
        } else {
            self.gain += (1.0 - self.gain) * self.release;
        }
        (soft_clip(l * self.gain), soft_clip(r * self.gain))
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    if x.abs() < 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * ((x.abs() - 0.9) * 10.0).tanh())
    }
}
