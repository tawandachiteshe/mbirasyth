//! The backing band: an ngoma (hand drum) and a bass guitar, both following the
//! mbira's 48-pulse cycle. Allocation-free after construction.
//!
//! Rhythm: the pulse is the fast mbira pulse; 3 pulses make one beat (12/8 feel),
//! 12 pulses one quarter of the cycle. The drum plays on the beat grid while the
//! harmony changes every 4 pulses, so the bass and drum cross against each other
//! the way they do in chimurenga and mbira-pop.

use crate::compose::{DYADS, PULSES, PULSES_PER_DYAD};
use crate::dsp::{decay_coef, pan_gains, Biquad, Mode};
use crate::rng::{Drift, Rng};
use crate::tuning::TuningParams;

const GLIDE_BLOCK: u32 = 32;

// ---------------------------------------------------------------------------
// Ngoma

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stroke {
    /// Open palm in the centre: deep and booming.
    Bass,
    /// Fingers at the edge: pitched, singing.
    Tone,
    /// Sharp cracking slap.
    Slap,
    /// Damped ghost note.
    Mute,
}

/// Circular-membrane mode ratios (Bessel zeros, normalised to the (0,1) mode).
const MEMBRANE: [f32; 8] = [1.0, 1.59, 2.14, 2.30, 2.65, 2.92, 3.16, 3.50];

struct StrokeShape {
    amps: [f32; 8],
    t60: f32,
    glide: f32,
    noise: f32,
    noise_hz: f32,
    noise_decay: f32,
}

fn shape(s: Stroke) -> StrokeShape {
    match s {
        Stroke::Bass => StrokeShape {
            amps: [1.0, 0.22, 0.12, 0.06, 0.04, 0.03, 0.02, 0.02],
            t60: 0.6,
            glide: 0.22,
            noise: 0.08,
            noise_hz: 400.0,
            noise_decay: 0.012,
        },
        Stroke::Tone => StrokeShape {
            amps: [0.3, 1.0, 0.65, 0.45, 0.3, 0.22, 0.16, 0.12],
            t60: 0.38,
            glide: 0.07,
            noise: 0.18,
            noise_hz: 1400.0,
            noise_decay: 0.010,
        },
        Stroke::Slap => StrokeShape {
            amps: [0.08, 0.35, 0.45, 0.4, 0.38, 0.34, 0.3, 0.3],
            t60: 0.13,
            glide: 0.04,
            noise: 0.75,
            noise_hz: 2600.0,
            noise_decay: 0.022,
        },
        Stroke::Mute => StrokeShape {
            amps: [0.2, 0.55, 0.35, 0.2, 0.15, 0.1, 0.08, 0.06],
            t60: 0.07,
            glide: 0.03,
            noise: 0.12,
            noise_hz: 1100.0,
            noise_decay: 0.006,
        },
    }
}

#[derive(Clone, Copy, Default)]
struct Hit {
    delay: u32,
    stroke: Option<Stroke>,
    vel: f32,
}

pub struct Ngoma {
    sr: f32,
    modes: [Mode; 8],
    base_hz: f32,
    glide: f32,
    glide_decay: f32,
    glide_counter: u32,
    noise_env: f32,
    noise_decay: f32,
    noise_amp: f32,
    noise_bp: Biquad,
    out_lp: Biquad,
    pending: [Hit; 8],
    rng: Rng,
    pan: (f32, f32),
    /// Pitch factor from hand pressure on the skin, per stroke.
    press: f32,
    right_hand: bool,
}

impl Ngoma {
    pub fn new(sr: f32, seed: u64, pan: f32) -> Self {
        let mut rng = Rng::stream(seed, 0x96);
        let base_hz = 74.0 * (1.0 + rng.bipolar() * 0.05);
        let mut n = Ngoma {
            sr,
            modes: [Mode::default(); 8],
            base_hz,
            glide: 0.0,
            glide_decay: decay_coef(0.035, sr).powi(GLIDE_BLOCK as i32),
            glide_counter: 0,
            noise_env: 0.0,
            noise_decay: 0.0,
            noise_amp: 0.0,
            noise_bp: Biquad::bandpass(1000.0, 0.9, sr),
            out_lp: Biquad::lowpass(7000.0, 0.7, sr),
            pending: [Hit::default(); 8],
            rng,
            pan: pan_gains(pan),
            press: 1.0,
            right_hand: true,
        };
        for (i, m) in n.modes.iter_mut().enumerate() {
            m.set(base_hz * MEMBRANE[i], 0.3, sr);
        }
        n
    }

    /// Schedule a stroke `delay` samples from now.
    pub fn hit(&mut self, stroke: Stroke, vel: f32, delay: u32) {
        if let Some(h) = self.pending.iter_mut().find(|h| h.stroke.is_none()) {
            *h = Hit { delay, stroke: Some(stroke), vel };
        }
    }

    fn strike(&mut self, stroke: Stroke, vel: f32) {
        let sh = shape(stroke);
        let vel = vel.clamp(0.0, 1.3);
        // Hands alternate; they never hit quite the same spot or with the same weight.
        self.right_hand = !self.right_hand;
        let hand = if self.right_hand { 1.0 } else { -1.0 };
        // Pressure from the other hand / fingers on the skin raises the pitch a little.
        self.press = 1.0 + 0.008 * hand + self.rng.gauss() * 0.006;
        // Harder hits bend the pitch up more (tension modulation).
        self.glide = sh.glide * vel * (0.8 + 0.4 * self.rng.f32());
        // Strike position: off-centre hits excite the higher (nodal-diameter) modes more.
        let off_centre = (0.5 + 0.12 * hand + self.rng.gauss() * 0.2).clamp(0.05, 1.0);
        let t60_var = 1.0 + self.rng.gauss() * 0.1;
        for (i, m) in self.modes.iter_mut().enumerate() {
            let f = self.base_hz * MEMBRANE[i] * self.press * (1.0 + self.glide);
            // Re-setting the decay also damps whatever is still ringing (the hand stays on the skin for a mute).
            m.set(f, sh.t60 * t60_var / (1.0 + 0.45 * i as f32), self.sr);
            let pos = if i == 0 { 1.2 - 0.4 * off_centre } else { 0.5 + off_centre };
            let var = (self.rng.gauss() * 0.3).exp() * pos;
            m.strike(0.5 * vel * sh.amps[i] * var, 0.0);
        }
        self.noise_env = 1.0;
        self.noise_decay = decay_coef(sh.noise_decay, self.sr);
        self.noise_amp = sh.noise * vel * vel;
        self.noise_bp.retune(Biquad::bandpass(sh.noise_hz * (self.rng.gauss() * 0.15).exp(), 0.9, self.sr));
    }

    #[inline]
    pub fn tick(&mut self) -> (f32, f32) {
        for i in 0..self.pending.len() {
            if let Some(st) = self.pending[i].stroke {
                if self.pending[i].delay == 0 {
                    self.pending[i].stroke = None;
                    let v = self.pending[i].vel;
                    self.strike(st, v);
                } else {
                    self.pending[i].delay -= 1;
                }
            }
        }
        self.glide_counter += 1;
        if self.glide_counter >= GLIDE_BLOCK {
            self.glide_counter = 0;
            if self.glide > 1e-4 {
                self.glide *= self.glide_decay;
                for (i, m) in self.modes.iter_mut().enumerate() {
                    m.set_freq(self.base_hz * MEMBRANE[i] * self.press * (1.0 + self.glide), self.sr);
                }
            }
        }
        let mut y = 0.0;
        for m in self.modes.iter_mut() {
            y += m.tick(0.0);
        }
        if self.noise_env > 1e-4 {
            self.noise_env *= self.noise_decay;
            y += self.noise_bp.tick(self.rng.bipolar() * self.noise_env * self.noise_amp) * 2.0;
        }
        let y = self.out_lp.tick(y);
        (y * self.pan.0, y * self.pan.1)
    }
}

// ---------------------------------------------------------------------------
// Bass guitar

const HARMONICS: usize = 10;

#[derive(Clone, Copy, Default)]
struct BassVoice {
    modes: [Mode; HARMONICS],
    gate: u32,
}

#[derive(Clone, Copy, Default)]
struct BassNote {
    delay: u32,
    freq: f32,
    vel: f32,
    dur: u32,
    slide: bool,
    live: bool,
}

/// Finger-plucked electric bass: a slightly stiff string as harmonic modes,
/// plucked near the bridge, released (palm/finger damping) at the end of each note.
pub struct BassGuitar {
    sr: f32,
    voices: [BassVoice; 2],
    current: usize,
    pending: [BassNote; 8],
    tone: Biquad,
    body: Biquad,
    rng: Rng,
    freq: f32,
    /// Pitch offset (fraction) of the sounding voice, decaying to 0: slides into a note.
    glide: f32,
    glide_decay: f32,
    glide_counter: u32,
}

impl BassGuitar {
    pub fn new(sr: f32, seed: u64) -> Self {
        BassGuitar {
            sr,
            voices: [BassVoice::default(); 2],
            current: 0,
            pending: [BassNote::default(); 8],
            tone: Biquad::lowpass(1600.0, 0.7, sr),
            body: Biquad::peaking(110.0, 1.0, 3.0, sr),
            rng: Rng::stream(seed, 0xBA55),
            freq: 55.0,
            glide: 0.0,
            glide_decay: decay_coef(0.045, sr).powi(GLIDE_BLOCK as i32),
            glide_counter: 0,
        }
    }

    /// Schedule a note `delay` samples from now, held for `dur` samples.
    pub fn note(&mut self, freq: f32, vel: f32, dur: u32, delay: u32, slide: bool) {
        if let Some(n) = self.pending.iter_mut().find(|n| !n.live) {
            *n = BassNote { delay, freq, vel, dur, slide, live: true };
        }
    }

    fn partial_hz(freq: f32, n: f32) -> f32 {
        const B: f32 = 0.00025; // string stiffness
        n * freq * (1.0 + B * n * n).sqrt()
    }

    fn release(v: &mut BassVoice, sr: f32) {
        for m in v.modes.iter_mut() {
            if m.enabled {
                // Keep the frequency, shorten the decay.
                m.set_t60(0.05, sr);
            }
        }
    }

    fn start(&mut self, freq: f32, vel: f32, dur: u32, slide: bool) {
        let sr = self.sr;
        Self::release(&mut self.voices[self.current], sr);
        self.current ^= 1;
        let v = &mut self.voices[self.current];
        let vel = vel.clamp(0.0, 1.2);
        // Pluck position: fingerstyle ~ 1/5 of the string, wandering per pluck.
        let pos = (0.2 + self.rng.gauss() * 0.035).clamp(0.1, 0.32);
        // A slide starts a semitone (or two) below and settles onto the note.
        self.glide = if slide { -0.056 * (1.0 + self.rng.below(2) as f32) } else { 0.0 };
        self.freq = freq;
        for (i, m) in v.modes.iter_mut().enumerate() {
            let n = (i + 1) as f32;
            let f = Self::partial_hz(freq * (1.0 + self.glide), n);
            m.silence();
            m.set(f, 2.4 / (1.0 + 0.45 * (n - 1.0)), sr);
            let a = (n * std::f32::consts::PI * pos).sin() / n.powf(1.15);
            // Harder plucks are brighter.
            let bright = (1.0 - (1.0 - vel) * 0.08 * (n - 1.0)).max(0.1);
            m.strike(0.35 * vel * a * bright, 0.0);
        }
        v.gate = dur.max(1);
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        for i in 0..self.pending.len() {
            let n = self.pending[i];
            if n.live {
                if n.delay == 0 {
                    self.pending[i].live = false;
                    self.start(n.freq, n.vel, n.dur, n.slide);
                } else {
                    self.pending[i].delay -= 1;
                }
            }
        }
        self.glide_counter += 1;
        if self.glide_counter >= GLIDE_BLOCK {
            self.glide_counter = 0;
            if self.glide.abs() > 1e-4 {
                self.glide *= self.glide_decay;
                let f = self.freq * (1.0 + self.glide);
                for (i, m) in self.voices[self.current].modes.iter_mut().enumerate() {
                    m.set_freq(Self::partial_hz(f, (i + 1) as f32), self.sr);
                }
            }
        }
        let mut y = 0.0;
        for v in self.voices.iter_mut() {
            if v.gate > 0 {
                v.gate -= 1;
                if v.gate == 0 {
                    Self::release(v, self.sr);
                }
            }
            for m in v.modes.iter_mut() {
                y += m.tick(0.0);
            }
        }
        // A touch of amp/pickup saturation.
        let y = (y * 1.5).tanh() / 1.5;
        self.body.tick(self.tone.tick(y))
    }
}

/// A fixed-size list of the events in one cycle (allocation-free), for analysis tools.
#[derive(Clone, Copy)]
pub struct EventLog {
    pub len: usize,
    pub ev: [u32; 384],
}

impl Default for EventLog {
    fn default() -> Self {
        EventLog { len: 0, ev: [0; 384] }
    }
}

impl EventLog {
    pub fn push(&mut self, e: u32) {
        if self.len < self.ev.len() {
            self.ev[self.len] = e;
            self.len += 1;
        }
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.ev[..self.len]
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

/// Order-dependent hash step for fingerprints.
pub fn mix(h: u64, x: u64) -> u64 {
    (h ^ x.wrapping_mul(0x9E37_79B9_7F4A_7C15)).rotate_left(23).wrapping_mul(0xBF58_476D_1CE4_E5B9)
}

// ---------------------------------------------------------------------------
// Patterns

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrumStyle {
    /// Traditional bira feel: bass on the beat, singing tones in between.
    Bira,
    /// Busier, with slaps and ghost notes (mbira-pop / chimurenga).
    Chimurenga,
    /// Just the downbeats: calm.
    Sparse,
}

impl DrumStyle {
    pub const ALL: [DrumStyle; 3] = [DrumStyle::Bira, DrumStyle::Chimurenga, DrumStyle::Sparse];
    pub fn name(self) -> &'static str {
        match self {
            DrumStyle::Bira => "Bira (traditional)",
            DrumStyle::Chimurenga => "Chimurenga (busy)",
            DrumStyle::Sparse => "Sparse",
        }
    }
    /// 12-pulse patterns (one quarter = 4 beats of 3 pulses).
    /// B bass, T tone, S slap, M mute/ghost, `.` rest.
    fn patterns(self) -> &'static [&'static [u8; 12]] {
        match self {
            DrumStyle::Bira => &[b"B..T.TB..T.T", b"B..T.TB.TT.T", b"B.MT.TB..T.S"],
            DrumStyle::Chimurenga => &[b"B.MS.TB.MS.T", b"B.TS.TBMTS.S", b"BM.SMTB.MSTT"],
            DrumStyle::Sparse => &[b"B.....T.....", b"B.....B..T.."],
        }
    }
}

/// Drum fill for the last quarter before the melody changes variation.
const FILL: &[u8; 12] = b"B.TS.SB.SSTT";

/// One stroke of a generated groove at position `q` of the 12-pulse quarter, following
/// the beat hierarchy (q % 3 == 0 are beats; 6 is the half-bar).
fn groove_stroke(style: DrumStyle, q: usize, rng: &mut Rng) -> u8 {
    let pick = |rng: &mut Rng, opts: &[(u8, f32)]| {
        let mut x = rng.f32();
        for &(c, p) in opts {
            if x < p {
                return c;
            }
            x -= p;
        }
        b'.'
    };
    if q == 0 {
        return b'B';
    }
    match style {
        DrumStyle::Bira => match q % 3 {
            _ if q == 6 => pick(rng, &[(b'B', 0.6), (b'T', 0.4)]),
            0 => pick(rng, &[(b'T', 0.75)]),
            2 => pick(rng, &[(b'T', 0.55), (b'M', 0.15)]),
            _ => pick(rng, &[(b'M', 0.12)]),
        },
        DrumStyle::Chimurenga => match q % 3 {
            _ if q == 6 => pick(rng, &[(b'B', 0.65), (b'S', 0.35)]),
            0 => pick(rng, &[(b'S', 0.5), (b'T', 0.3)]),
            2 => pick(rng, &[(b'T', 0.5), (b'M', 0.25), (b'S', 0.1)]),
            _ => pick(rng, &[(b'M', 0.3), (b'T', 0.1)]),
        },
        DrumStyle::Sparse => match q % 3 {
            _ if q == 6 => pick(rng, &[(b'T', 0.5), (b'B', 0.3)]),
            0 => pick(rng, &[(b'T', 0.2)]),
            _ => b'.',
        },
    }
}

/// A fill: the groove's first half, then a denser run into the next section.
fn make_fill(groove: &[u8; 12], rng: &mut Rng) -> [u8; 12] {
    let mut f = *groove;
    let start = if rng.chance(0.5) { 6 } else { 8 };
    for (q, x) in f.iter_mut().enumerate().skip(start) {
        *x = if q == 6 && rng.chance(0.5) {
            b'B'
        } else if rng.chance(0.8) {
            [b'S', b'T', b'S', b'M'][rng.below(4)]
        } else {
            b'.'
        };
    }
    f
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BassStyle {
    /// One root per chord.
    Roots,
    /// Locks to the 12/8 beat: a note every 3 pulses.
    Beat,
    /// Syncopated mbira-pop line: roots, fifths, octaves and anticipations.
    Chimurenga,
    /// Moving line: root–fifth–octave.
    Walking,
}

impl BassStyle {
    pub const ALL: [BassStyle; 4] = [BassStyle::Roots, BassStyle::Beat, BassStyle::Chimurenga, BassStyle::Walking];
    pub fn name(self) -> &'static str {
        match self {
            BassStyle::Roots => "Roots",
            BassStyle::Beat => "On the beat",
            BassStyle::Chimurenga => "Chimurenga",
            BassStyle::Walking => "Walking",
        }
    }
    /// Per-chord (4-pulse) patterns. R root, F fifth, O octave, L fifth below,
    /// A anticipate the next chord's root, `.` hold.
    fn patterns(self) -> &'static [&'static [u8; 4]] {
        match self {
            BassStyle::Roots => &[b"R..."],
            BassStyle::Beat => &[b"R..."], // unused: beat style is generated on the 3-pulse grid
            BassStyle::Chimurenga => &[b"R..A", b"R.OF", b"R.LA", b"R..F"],
            BassStyle::Walking => &[b"R.F.", b"R.OF", b"R.FO"],
        }
    }
}

/// Settings the band reads from the engine each pulse.
#[derive(Clone, Copy)]
pub struct BandPulse<'a> {
    pub pulse: usize,
    pub roots: &'a [i32; DYADS],
    pub tuning: &'a TuningParams,
    pub pulse_len: f32,
    pub drums: bool,
    pub bass: bool,
    pub drum_style: DrumStyle,
    pub bass_style: BassStyle,
    pub drum_level: f32,
    pub bass_level: f32,
    pub intensity: f32,
    pub variation: f32,
    pub humanize: f32,
    pub groove_delay: f32,
    /// Fixed scheduling latency (samples) so human timing can be early as well as late.
    pub base_delay: f32,
    pub swell: f32,
    /// The melody is about to change variation at the end of this cycle.
    pub change_coming: bool,
}

pub struct Band {
    /// Fingerprint of the drum and bass events this cycle (for repetition analysis).
    pub fingerprint: u64,
    /// The drum/bass events of the cycle in progress (for repetition analysis).
    pub events: EventLog,
    pub ngoma: Ngoma,
    pub bass: BassGuitar,
    drum_pattern: usize,
    /// This song's drum grooves: one classic pattern plus generated ones that slowly mutate.
    grooves: [[u8; 12]; 4],
    groove_style: Option<DrumStyle>,
    fill: [u8; 12],
    bass_choice: [usize; DYADS],
    last_bass_degree: i32,
    rng: Rng,
    drum_time: Drift,
    drum_dyn: Drift,
    bass_time: Drift,
    bass_dyn: Drift,
    /// How long the bassist holds notes (legato ↔ staccato) wanders slowly.
    bass_legato: Drift,
}

impl Band {
    fn log(&mut self, ev: u32) {
        self.events.push(ev);
    }

    pub fn new(sr: f32, seed: u64) -> Self {
        let mut b = Band {
            ngoma: Ngoma::new(sr, seed, 0.15),
            bass: BassGuitar::new(sr, seed),
            drum_pattern: 0,
            grooves: [*b"B..T.TB..T.T"; 4],
            groove_style: None,
            fill: *FILL,
            bass_choice: [0; DYADS],
            last_bass_degree: 0,
            fingerprint: 0,
            events: EventLog::default(),
            rng: Rng::stream(seed, 0xBA4D),
            drum_time: Drift::new(seed, 0xD7, [3.0, 12.0, 48.0]),
            drum_dyn: Drift::new(seed, 0xDD, [6.0, 24.0, 96.0]),
            bass_time: Drift::new(seed, 0xB7, [3.0, 12.0, 48.0]),
            bass_dyn: Drift::new(seed, 0xBD, [6.0, 24.0, 96.0]),
            bass_legato: Drift::new(seed, 0xB1, [12.0, 48.0, 96.0]),
        };
        b.new_song(seed);
        b
    }

    /// Pick this song's riffs (deterministic from the seed).
    pub fn new_song(&mut self, seed: u64) {
        self.rng = Rng::stream(seed, 0xBA4D);
        self.drum_pattern = 0;
        self.groove_style = None;
        // A bass riff repeats every 3 chords (one quarter), like a real player's figure.
        let a = self.rng.below(8);
        let b = self.rng.below(8);
        let c = self.rng.below(8);
        for i in 0..DYADS {
            self.bass_choice[i] = [a, b, c][i % 3];
        }
    }

    fn make_grooves(&mut self, style: DrumStyle) {
        let classic = style.patterns();
        self.grooves[0] = *classic[self.rng.below(classic.len())];
        for g in 1..self.grooves.len() {
            let mut pat = [b'.'; 12];
            for (q, x) in pat.iter_mut().enumerate() {
                *x = groove_stroke(style, q, &mut self.rng);
            }
            // Mostly repeat the first half, like real hand-drum patterns (6-pulse phrases).
            if self.rng.chance(0.6) {
                for q in 7..12 {
                    if self.rng.chance(0.8) {
                        pat[q] = pat[q - 6];
                    }
                }
            }
            self.grooves[g] = pat;
        }
        self.groove_style = Some(style);
        self.drum_pattern = 0;
    }

    pub fn on_cycle_end(&mut self, variation: f32, styles: (DrumStyle, BassStyle)) {
        if self.rng.chance(0.15 + 0.4 * variation) {
            self.drum_pattern = self.rng.below(self.grooves.len());
        }
        // The drummer's grooves drift: one stroke at a time, never on the downbeat.
        if self.groove_style.is_some() && self.rng.chance(0.15 + 0.4 * variation) {
            let g = self.rng.below(self.grooves.len());
            let q = 1 + self.rng.below(11);
            self.grooves[g][q] = groove_stroke(styles.0, q, &mut self.rng);
        }
        if self.rng.chance(0.15 + 0.35 * variation) {
            let i = self.rng.below(3);
            let v = self.rng.below(8);
            for d in (i..DYADS).step_by(3) {
                self.bass_choice[d] = v;
            }
        }
    }

    pub fn on_pulse(&mut self, c: BandPulse) {
        let p = c.pulse;
        let q = p % 12;
        let hum = c.humanize;
        // One step of each player's drift per pulse, whether or not they play.
        let sr = self.ngoma.sr;
        let drum_off = self.drum_time.next() * hum * 0.006;
        let drum_dyn = 1.0 + self.drum_dyn.next() * 0.12 * hum;
        let bass_off = self.bass_time.next() * hum * 0.008;
        let bass_dyn = 1.0 + self.bass_dyn.next() * 0.1 * hum;
        let legato = (0.75 + 0.2 * self.bass_legato.next()).clamp(0.4, 1.0);
        if self.groove_style != Some(c.drum_style) {
            self.make_grooves(c.drum_style);
        }
        if p == PULSES - 12 {
            self.fill = make_fill(&self.grooves[self.drum_pattern % self.grooves.len()], &mut self.rng);
        }
        if c.drums && c.drum_level > 0.0 {
            let last_quarter = p >= PULSES - 12;
            let pat = if last_quarter && c.change_coming && c.drum_style != DrumStyle::Sparse {
                self.fill
            } else {
                self.grooves[self.drum_pattern % self.grooves.len()]
            };
            let stroke = match pat[q] {
                b'B' => Some((Stroke::Bass, 1.0)),
                b'T' => Some((Stroke::Tone, 0.75)),
                b'S' => Some((Stroke::Slap, 0.8)),
                b'M' => Some((Stroke::Mute, 0.45)),
                _ => {
                    // Ghost notes fill some rests when the music is intense.
                    let ghost = (c.intensity - 0.6).max(0.0) * 0.6 * (0.5 + c.variation);
                    if c.drum_style != DrumStyle::Sparse && self.rng.chance(ghost) {
                        Some((Stroke::Mute, 0.25))
                    } else {
                        None
                    }
                }
            };
            // The drummer improvises around the pattern: swaps tones and slaps,
            // drops an off-beat note now and then, never loops it exactly.
            let loose = 0.07 * (0.4 + c.variation) * (0.3 + hum);
            let stroke = match stroke {
                Some((Stroke::Tone, v)) if self.rng.chance(loose) => Some((Stroke::Slap, v * 0.9)),
                Some((Stroke::Slap, v)) if self.rng.chance(loose) => Some((Stroke::Tone, v)),
                Some((Stroke::Tone, v)) if self.rng.chance(loose * 0.5) => Some((Stroke::Mute, v * 0.6)),
                Some((Stroke::Bass, _)) | None => stroke,
                Some(_) if q % 3 != 0 && self.rng.chance(loose * 0.5) => None,
                other => other,
            };
            if let Some((st, v)) = stroke {
                // Beat hierarchy inside the quarter: 1 strongest, 3 next.
                const BEAT: [f32; 4] = [1.12, 0.95, 1.03, 0.95];
                let accent = if q % 3 == 0 { BEAT[q / 3] } else { 0.9 };
                let vel = v * accent * c.swell * drum_dyn * (1.0 + self.rng.gauss() * 0.07 * hum);
                // The drummer sits close to the beat (they are the timekeeper).
                let t = c.base_delay + c.groove_delay * 0.4 + drum_off * sr
                    + self.rng.gauss() * 0.0015 * hum * sr;
                let delay = t.max(0.0) as u32;
                // Occasional flam: a quiet grace note from the other hand just before.
                if matches!(st, Stroke::Tone | Stroke::Slap) && self.rng.chance(0.05 * (0.3 + c.variation)) {
                    let grace = (0.018 + 0.012 * self.rng.f32()) * sr;
                    self.ngoma.hit(st, vel * 0.35 * c.drum_level, (t - grace).max(0.0) as u32);
                }
                self.ngoma.hit(st, vel * c.drum_level, delay);
                self.fingerprint = mix(self.fingerprint, (p as u64) << 8 | st as u64);
                self.log(0x1_0000 | (p as u32) << 4 | st as u32);
            }
        }
        if c.bass && c.bass_level > 0.0 {
            let dyad = (p / PULSES_PER_DYAD) % DYADS;
            let step = p % PULSES_PER_DYAD;
            let root = c.roots[dyad].rem_euclid(7);
            let next_root = c.roots[(dyad + 1) % DYADS].rem_euclid(7);
            let (token, gap) = if c.bass_style == BassStyle::Beat {
                if p % 3 == 0 {
                    // Root on the beat; if the chord hasn't changed, move to the fifth or octave.
                    let t = if root == self.last_bass_degree.rem_euclid(7) && self.rng.chance(0.5) {
                        if self.rng.chance(0.5) { b'F' } else { b'O' }
                    } else {
                        b'R'
                    };
                    (t, 3)
                } else {
                    (b'.', 0)
                }
            } else {
                let pats = c.bass_style.patterns();
                let pat = pats[self.bass_choice[dyad] % pats.len()];
                let gap = (step + 1..PULSES_PER_DYAD).find(|&i| pat[i] != b'.').unwrap_or(PULSES_PER_DYAD) - step;
                (pat[step], gap)
            };
            let degree = match token {
                b'R' => Some(root),
                b'F' => Some(root + 4),
                b'O' => Some(root + 7),
                b'L' => Some(root - 3),
                b'A' => Some(next_root),
                _ => None,
            };
            if let Some(d) = degree {
                self.last_bass_degree = d;
                // Bass guitar register: an octave below the mbira's lowest key.
                let mut f = c.tuning.degree_hz(d) * 0.5;
                while f < 38.0 {
                    f *= 2.0;
                }
                while f > 140.0 {
                    f *= 0.5;
                }
                let accent = if step == 0 { 1.0 } else { 0.8 };
                let vel = accent * c.swell * bass_dyn * (1.0 + self.rng.gauss() * 0.07 * hum);
                let dur = gap as f32 * c.pulse_len * legato;
                // The bassist sits slightly behind the beat, and drifts.
                let delay = c.base_delay + c.groove_delay * 0.7 + 0.012 * sr
                    + bass_off * sr;
                // Slide into chord changes now and then.
                let slide = step == 0 && self.rng.chance(0.08 * (0.3 + c.variation));
                self.bass.note(f, vel * c.bass_level, dur as u32, delay.max(0.0) as u32, slide);
                self.fingerprint = mix(self.fingerprint, 0xB000 | (p as u64) << 16 | d.rem_euclid(14) as u64);
                self.log(0x2_0000 | (p as u32) << 4 | d.rem_euclid(14) as u32);
            } else if token == b'.' && step == PULSES_PER_DYAD - 1 && self.rng.chance(0.05 * (0.3 + c.variation) * hum) {
                // Ghost note: a muted, pitchless-ish thump that keeps the line moving.
                let mut f = c.tuning.degree_hz(self.last_bass_degree) * 0.5;
                while f < 38.0 {
                    f *= 2.0;
                }
                while f > 140.0 {
                    f *= 0.5;
                }
                let delay = c.base_delay + c.groove_delay * 0.7 + bass_off * sr;
                self.bass.note(f, 0.3 * bass_dyn * c.bass_level, (0.04 * sr) as u32, delay.max(0.0) as u32, false);
            }
        }
    }

    #[inline]
    pub fn tick(&mut self) -> ((f32, f32), f32) {
        (self.ngoma.tick(), self.bass.tick())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{cycle_roots, in_dyad, Standard};

    #[test]
    fn bass_follows_the_chords() {
        let tuning = TuningParams::default();
        let roots = cycle_roots(Standard::C, 0);
        for style in BassStyle::ALL {
            let mut band = Band::new(48_000.0, 3);
            let mut notes = 0;
            for p in 0..PULSES * 2 {
                let p = p % PULSES;
                band.bass.pending = [BassNote::default(); 8];
                band.on_pulse(BandPulse {
                    pulse: p,
                    roots: &roots,
                    tuning: &tuning,
                    pulse_len: 9600.0,
                    drums: false,
                    bass: true,
                    drum_style: DrumStyle::Bira,
                    bass_style: style,
                    drum_level: 1.0,
                    bass_level: 1.0,
                    intensity: 1.0,
                    variation: 1.0,
                    humanize: 1.0,
                    groove_delay: 0.0,
                    base_delay: 0.0,
                    swell: 1.0,
                    change_coming: false,
                });
                for n in band.bass.pending.iter().filter(|n| n.live) {
                    notes += 1;
                    let dyad = p / PULSES_PER_DYAD;
                    let ok = (-14..21).any(|d| {
                        let hz = tuning.degree_hz(d);
                        let same_pc = [0.25f32, 0.5, 1.0, 2.0].iter().any(|k| (hz * k / n.freq - 1.0).abs() < 1e-3);
                        same_pc && (in_dyad(d, roots[dyad]) || in_dyad(d, roots[(dyad + 1) % DYADS]))
                    });
                    assert!(ok, "{style:?} pulse {p}: {} Hz not in the chord", n.freq);
                    assert!((38.0..=140.0).contains(&n.freq));
                }
            }
            assert!(notes >= 24, "{style:?}: only {notes} notes");
        }
    }

    #[test]
    fn instruments_are_finite_and_decay() {
        let sr = 48_000.0;
        let mut ng = Ngoma::new(sr, 1, 0.0);
        let mut bass = BassGuitar::new(sr, 1);
        for (i, st) in [Stroke::Bass, Stroke::Tone, Stroke::Slap, Stroke::Mute].into_iter().enumerate() {
            ng.hit(st, 1.2, i as u32 * 2000);
        }
        bass.note(55.0, 1.2, 12_000, 0, true);
        let mut peak = 0.0f32;
        let mut tail = 0.0f32;
        for i in 0..(sr as usize * 3) {
            let (l, r) = ng.tick();
            let b = bass.tick();
            assert!(l.is_finite() && r.is_finite() && b.is_finite());
            peak = peak.max(l.abs()).max(b.abs());
            if i > sr as usize * 2 {
                tail = tail.max(l.abs()).max(b.abs());
            }
        }
        assert!(peak > 0.05 && peak < 2.0, "peak {peak}");
        assert!(tail < 0.01 * peak, "notes should die away (tail {tail})");
    }
}
