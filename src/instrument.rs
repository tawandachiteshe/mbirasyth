//! The instrument model (docs/04): modal tines, pluck exciter, sympathetic board bus,
//! machachara buzz, gourd body, and hosho shakers.

use crate::dsp::{decay_coef, pan_gains, Biquad, EnvFollower, Mode};
use crate::rng::Rng;
use crate::tuning::{key_pan, Manual, TuningParams, KEYS, NUM_KEYS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyPreset {
    /// Contact mic / no gourd: dry and bright.
    None,
    SmallGourd,
    BigGourd,
}

impl BodyPreset {
    pub const ALL: [BodyPreset; 3] = [BodyPreset::None, BodyPreset::SmallGourd, BodyPreset::BigGourd];
    pub fn name(self) -> &'static str {
        match self {
            BodyPreset::None => "No gourd (contact mic)",
            BodyPreset::SmallGourd => "Small gourd",
            BodyPreset::BigGourd => "Big ceremonial gourd (deze)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimbreParams {
    /// Overtone level (modes 2 & 3).
    pub brightness: f32,
    /// Multiplier on all decay times.
    pub decay: f32,
    /// Second / third mode frequency ratios (≈5 and ≈14 measured on mbira).
    pub ratio2: f32,
    pub ratio3: f32,
    /// Std-dev of per-key ratio jitter (fraction).
    pub ratio_jitter: f32,
    /// Nail click level.
    pub click: f32,
    /// Buzz (machachara) amount and threshold (0..1).
    pub buzz: f32,
    pub buzz_threshold: f32,
    /// 0 = noisy sizzle, 1 = hits locked to the board's vibration.
    pub buzz_phase_lock: f32,
    pub body: BodyPreset,
    pub body_mix: f32,
    pub sympathetic: f32,
    /// How much a re-plucked key is damped by the finger first (0 = none, 1 = full).
    pub repluck_damp: f32,
    /// Slow beating between two nearly-equal vibrations of each key (0..1).
    pub shimmer: f32,
    /// Dense wooden soundboard resonances (0..1).
    pub wood: f32,
    /// Pluck-to-pluck variation of attack colour and pitch settle (0..1).
    pub pluck_variation: f32,
}

impl Default for TimbreParams {
    fn default() -> Self {
        TimbreParams {
            brightness: 0.55,
            decay: 1.0,
            ratio2: 5.2,
            ratio3: 14.0,
            ratio_jitter: 0.05,
            click: 0.1,
            buzz: 0.15,
            buzz_threshold: 0.6,
            buzz_phase_lock: 0.4,
            body: BodyPreset::BigGourd,
            body_mix: 0.5,
            sympathetic: 0.3,
            repluck_damp: 0.75,
            shimmer: 0.5,
            wood: 0.5,
            pluck_variation: 0.6,
        }
    }
}

const MODES: usize = 3;

/// Log-frequency interpolation of a decay time between a 120 Hz and a 950 Hz anchor.
fn t60_at(freq: f32, low: f32, high: f32) -> f32 {
    let x = ((freq / 120.0).log2() / (950.0f32 / 120.0).log2()).clamp(-0.5, 1.5);
    low * (high / low).powf(x)
}

#[derive(Clone, Copy, Default)]
struct Tine {
    modes: [Mode; MODES],
    freq: f32,
    ratios: [f32; MODES],
    phases: [f32; MODES],
    pan: (f32, f32),
    last: f32,
    /// Second, slightly detuned vibration of the fundamental → slow beating.
    twin: Mode,
    twin_ratio: f32,
    /// Pitch offset in cents right after a hard pluck; settles to 0.
    glide: f32,
}

const WOOD_MODES: usize = 20;
const GLIDE_BLOCK: u32 = 32;

#[derive(Clone, Copy, Default)]
struct Click {
    env: f32,
    decay: f32,
    bp: Biquad,
    amp: f32,
    pan: (f32, f32),
}

/// Stochastic rattle (docs/04 §5, option A).
#[derive(Clone, Copy)]
struct Buzz {
    hp: Biquad,
    env: EnvFollower,
    caps: [Mode; 6],
    n_caps: usize,
    rng: Rng,
    /// Absolute-threshold scale; rim shells sit higher than board caps.
    thr_offset: f32,
}

impl Buzz {
    fn new(sr: f32, freqs: &[f32], t60: f32, thr_offset: f32, seed: u64) -> Self {
        let mut caps = [Mode::default(); 6];
        let mut rng = Rng::stream(seed, 0xB022);
        for (m, &f) in caps.iter_mut().zip(freqs) {
            m.set(f * rng.range(0.94, 1.06), t60 * rng.range(0.7, 1.3), sr);
        }
        Buzz {
            hp: Biquad::highpass(80.0, 0.7, sr),
            env: EnvFollower::new(0.0005, 0.02, sr),
            caps,
            n_caps: freqs.len().min(6),
            rng,
            thr_offset,
        }
    }

    #[inline]
    fn tick(&mut self, input: f32, p: &TimbreParams, sr: f32) -> f32 {
        let acc = self.hp.tick(input);
        let e = self.env.tick(acc.abs());
        let threshold = self.thr_offset + 0.015 + p.buzz_threshold * 0.5;
        let drive = e - threshold;
        if drive > 0.0 {
            let rate = (drive * 9000.0).min(3500.0);
            let gate = if acc > 0.0 { 1.0 } else { 1.0 - p.buzz_phase_lock };
            if self.rng.f32() < rate * gate / sr {
                let a = drive.min(1.0) * (0.4 + 0.6 * self.rng.f32());
                let c = self.rng.below(self.n_caps);
                let ph = self.rng.f32() * std::f32::consts::TAU;
                self.caps[c].strike(a, ph);
            }
        }
        let mut out = 0.0;
        for m in self.caps[..self.n_caps].iter_mut() {
            out += m.tick(0.0);
        }
        out
    }
}

/// One mbira: 24 tines, clicks, board bus, buzzers and gourd.
pub struct Mbira {
    sr: f32,
    tines: [Tine; NUM_KEYS],
    clicks: [Click; 8],
    next_click: usize,
    board_prev: f32,
    board_buzz: Buzz,
    rim_buzz: Buzz,
    body: [Biquad; 4],
    body_gain: f32,
    wood: [Mode; WOOD_MODES],
    wood_gain: [f32; WOOD_MODES],
    glide_decay: f32,
    glide_counter: u32,
    rng: Rng,
    pub timbre: TimbreParams,
    /// Overall stereo offset of this instrument (ensemble placement).
    pub offset_pan: f32,
}

impl Mbira {
    pub fn new(sr: f32, seed: u64, tuning: &TuningParams, timbre: TimbreParams, offset_pan: f32) -> Self {
        let mut m = Mbira {
            sr,
            tines: [Tine::default(); NUM_KEYS],
            clicks: [Click::default(); 8],
            next_click: 0,
            board_prev: 0.0,
            board_buzz: Buzz::new(sr, &[2200.0, 2950.0, 3650.0, 4400.0, 5100.0, 7400.0], 0.03, 0.0, seed),
            rim_buzz: Buzz::new(sr, &[850.0, 1350.0, 1900.0, 2700.0], 0.014, 0.05, seed ^ 0x51),
            body: [Biquad::default(); 4],
            body_gain: 1.0,
            wood: [Mode::default(); WOOD_MODES],
            wood_gain: [0.0; WOOD_MODES],
            glide_decay: decay_coef(0.09, sr).powi(GLIDE_BLOCK as i32),
            glide_counter: 0,
            rng: Rng::stream(seed, 0x9),
            timbre,
            offset_pan,
        };
        m.retune(seed, tuning, timbre);
        m
    }

    /// Recompute tine frequencies, decays and body filters. Keeps ringing state.
    pub fn retune(&mut self, seed: u64, tuning: &TuningParams, timbre: TimbreParams) {
        self.timbre = timbre;
        let freqs = tuning.key_freqs(seed);
        let mut rng = Rng::stream(seed, 0x7135);
        for k in 0..NUM_KEYS {
            let t = &mut self.tines[k];
            let f = freqs[k];
            t.freq = f;
            t.ratios = [
                1.0,
                timbre.ratio2 * (1.0 + rng.gauss() * timbre.ratio_jitter),
                timbre.ratio3 * (1.0 + rng.gauss() * timbre.ratio_jitter),
            ];
            t.phases = [rng.range(-0.3, 0.3), rng.range(-1.0, 1.0), rng.range(-1.5, 1.5)];
            let d = timbre.decay;
            let t60 = [
                t60_at(f, 6.0, 1.6) * d,
                t60_at(f, 0.40, 0.09) * d,
                t60_at(f, 0.12, 0.035) * d,
            ];
            for i in 0..MODES {
                t.modes[i].set(f * t.ratios[i], t60[i], self.sr);
            }
            let beat_hz = rng.range(0.35, 1.6) * if rng.chance(0.5) { 1.0 } else { -1.0 };
            t.twin_ratio = 1.0 + beat_hz / f;
            t.twin.set(f * t.twin_ratio, t60[0] * rng.range(0.6, 0.9), self.sr);
            t.glide = 0.0;
            t.pan = pan_gains(key_pan(k) * 0.9 + self.offset_pan);
        }
        let sr = self.sr;
        // Soundboard: a dense cloud of short wooden resonances (log-spread, seeded).
        let mut wr = Rng::stream(seed, 0x600D);
        for i in 0..WOOD_MODES {
            let x = (i as f32 + wr.range(0.0, 0.9)) / WOOD_MODES as f32;
            let f = 160.0 * (4200.0f32 / 160.0).powf(x);
            self.wood[i].set(f, wr.range(0.025, 0.09), sr);
            self.wood_gain[i] = wr.range(0.4, 1.0) * if wr.chance(0.5) { 1.0 } else { -1.0 };
        }
        let (filters, gain) = match timbre.body {
            BodyPreset::None => (
                [
                    Biquad::peaking(200.0, 1.0, 0.0, sr),
                    Biquad::peaking(500.0, 1.0, 0.0, sr),
                    Biquad::peaking(1000.0, 1.0, 0.0, sr),
                    Biquad::lowpass(16000.0, 0.7, sr),
                ],
                0.0,
            ),
            BodyPreset::SmallGourd => (
                [
                    Biquad::peaking(250.0, 3.0, 7.0, sr),
                    Biquad::peaking(520.0, 3.0, 3.0, sr),
                    Biquad::peaking(1250.0, 4.0, 2.0, sr),
                    Biquad::lowpass(8500.0, 0.7, sr),
                ],
                1.0,
            ),
            BodyPreset::BigGourd => (
                [
                    Biquad::peaking(150.0, 2.5, 9.0, sr),
                    Biquad::peaking(420.0, 3.0, 3.0, sr),
                    Biquad::peaking(1000.0, 4.0, 2.0, sr),
                    Biquad::lowpass(7000.0, 0.7, sr),
                ],
                1.0,
            ),
        };
        for (b, f) in self.body.iter_mut().zip(filters) {
            b.retune(f);
        }
        self.body_gain = gain;
    }

    pub fn key_freq(&self, k: usize) -> f32 {
        self.tines[k].freq
    }

    /// Pluck key `k` with velocity in 0..1.
    pub fn pluck(&mut self, k: usize, vel: f32) {
        let vel = vel.clamp(0.0, 1.2);
        let p = self.timbre;
        let index_finger = KEYS[k].manual == Manual::RightIndex;
        let t = &mut self.tines[k];
        let damp = 1.0 - p.repluck_damp * 0.85;
        for m in t.modes.iter_mut() {
            m.damp(damp);
        }
        t.twin.damp(damp);
        let tilt = if index_finger { 1.25 } else { 1.0 };
        // Every pluck differs a little: nail angle and position change the overtone mix.
        let pv = p.pluck_variation;
        let var2 = (self.rng.gauss() * 0.35 * pv).exp();
        let var3 = (self.rng.gauss() * 0.5 * pv).exp();
        let amps = [
            vel,
            vel * p.brightness * 0.55 * vel.powf(0.7) * tilt * var2,
            vel * p.brightness * 0.12 * vel * var3,
        ];
        for i in 0..MODES {
            let ph = t.phases[i] + self.rng.range(-0.2, 0.2);
            t.modes[i].strike(amps[i], ph);
        }
        t.twin.strike(vel * 0.35 * p.shimmer, self.rng.range(0.0, std::f32::consts::TAU));
        // A hard pluck starts a few cents sharp and settles (geometric stiffening).
        t.glide = vel * vel * (4.0 + 4.0 * pv * self.rng.f32());
        // Nail click.
        let ci = self.next_click;
        self.next_click = (ci + 1) % self.clicks.len();
        let c = &mut self.clicks[ci];
        let centre = if index_finger { 4000.0 } else { 2500.0 };
        c.bp = Biquad::bandpass(centre * self.rng.range(0.85, 1.15), 1.2, self.sr);
        c.env = 1.0;
        c.decay = decay_coef(self.rng.range(0.0015, 0.004), self.sr);
        c.amp = p.click * vel * vel * 1.6;
        c.pan = t.pan;
    }

    /// Render one stereo sample.
    #[inline]
    pub fn tick(&mut self) -> (f32, f32) {
        let p = self.timbre;
        let sym = p.sympathetic * 0.35;
        let mut l = 0.0;
        let mut r = 0.0;
        let mut board = 0.0;
        self.glide_counter += 1;
        let update_glide = self.glide_counter >= GLIDE_BLOCK;
        if update_glide {
            self.glide_counter = 0;
        }
        for t in self.tines.iter_mut() {
            if update_glide && t.glide > 0.05 {
                t.glide *= self.glide_decay;
                let g = 2f32.powf(t.glide / 1200.0);
                t.modes[0].set_freq(t.freq * g, self.sr);
                t.twin.set_freq(t.freq * t.twin_ratio * g, self.sr);
            }
            let drive = (self.board_prev - t.last) * sym * t.modes[0].one_minus_r;
            let mut y = t.modes[0].tick(drive);
            y += t.twin.tick(0.0);
            y += t.modes[1].tick(0.0);
            y += t.modes[2].tick(0.0);
            t.last = y;
            board += y;
            l += y * t.pan.0;
            r += y * t.pan.1;
        }
        self.board_prev = board;

        for c in self.clicks.iter_mut() {
            if c.env > 1e-4 {
                let n = self.rng.bipolar();
                let y = c.bp.tick(n * c.env * c.amp);
                c.env *= c.decay;
                l += y * c.pan.0;
                r += y * c.pan.1;
            }
        }

        let sr = self.sr;
        // Board buzzers.
        let bz = self.board_buzz.tick(board, &p, sr) * p.buzz * 0.25;
        // Gourd body + rim buzz.
        let mut body = board;
        for b in self.body.iter_mut() {
            body = b.tick(body);
        }
        body *= self.body_gain;
        let rim = if self.body_gain > 0.0 { self.rim_buzz.tick(body * 0.5, &p, sr) * p.buzz * 0.2 } else { 0.0 };

        let mut wood = 0.0;
        if p.wood > 0.0 {
            for (m, g) in self.wood.iter_mut().zip(self.wood_gain.iter()) {
                wood += m.tick(board * m.one_minus_r) * g;
            }
        }
        let dry = 1.0 - 0.5 * p.body_mix * self.body_gain;
        let centre = body * p.body_mix * 0.6 + wood * p.wood * 0.9 + bz + rim;
        let (cl, cr) = pan_gains(self.offset_pan);
        (l * dry + centre * cl, r * dry + centre * cr)
    }

    pub fn energy(&self) -> f32 {
        self.tines.iter().map(|t| t.modes[0].energy()).sum()
    }
}

/// PhISEM gourd shaker (Cook 1997), docs/04 §7.
pub struct Hosho {
    sr: f32,
    energy: f32,
    level: f32,
    energy_decay: f32,
    level_decay: f32,
    bp: Biquad,
    body: Biquad,
    rng: Rng,
    pan: (f32, f32),
}

impl Hosho {
    pub fn new(sr: f32, seed: u64, pan: f32, centre: f32) -> Self {
        Hosho {
            sr,
            energy: 0.0,
            level: 0.0,
            energy_decay: decay_coef(0.045, sr),
            level_decay: decay_coef(0.0025, sr),
            bp: Biquad::bandpass(centre, 1.4, sr),
            body: Biquad::peaking(800.0, 2.0, 5.0, sr),
            rng: Rng::stream(seed, 0x4054),
            pan: pan_gains(pan),
        }
    }

    pub fn shake(&mut self, strength: f32) {
        self.energy += strength;
    }

    #[inline]
    pub fn tick(&mut self) -> (f32, f32) {
        self.energy *= self.energy_decay;
        if self.energy < 1e-5 && self.level < 1e-5 {
            return (0.0, 0.0);
        }
        // Seed collisions: a Poisson stream whose rate follows the shake energy.
        if self.rng.f32() < (self.energy * 4000.0 / self.sr).min(1.0) {
            self.level += 0.3 * self.energy * self.rng.range(0.5, 1.0);
        }
        self.level *= self.level_decay;
        let n = self.rng.bipolar() * self.level;
        let y = self.body.tick(self.bp.tick(n)) * 2.5;
        (y * self.pan.0, y * self.pan.1)
    }
}
