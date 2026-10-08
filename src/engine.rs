//! The engine: conductor (pulse clock, layers, variations) + two mbiras + hosho +
//! ngoma + bass guitar + reverb + mastering chain + loudness meter. `process` is real-time safe (no allocation, no locks).

use crate::ambience::Ambience;
use crate::band::{Band, EventLog, BandPulse, BassStyle, DrumStyle};
use crate::compose::{octave_alternative, MusicParams, Piece, MAX_NOTES, PULSES, PULSES_PER_DYAD, VARIATIONS};
use crate::dsp::{Biquad, Limiter, Reverb};
use crate::master::{LoudnessMeter, LoudnessReading, Master, MasterParams};
use crate::instrument::{Hosho, Mbira, TimbreParams};
use crate::rng::{Drift, Rng};
use crate::tuning::{TuningParams, KEYS, NUM_KEYS};

/// Everything the GUI controls. `Copy`, so it can be sent through a lock-free queue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub tuning: TuningParams,
    pub instrument_seed: u64,
    pub timbre: TimbreParams,
    pub music: MusicParams,
    /// Pulses per second (the fast even pulse; 12 per quarter-cycle).
    pub tempo: f32,
    /// 0..1: density and (if `auto_layers`) which parts play.
    pub intensity: f32,
    pub auto_layers: bool,
    pub play_kushaura: bool,
    pub play_kutsinhira: bool,
    pub play_hosho: bool,
    pub hosho_level: f32,
    pub play_ngoma: bool,
    pub play_bass: bool,
    pub ngoma_level: f32,
    pub bass_level: f32,
    pub drum_style: DrumStyle,
    pub bass_style: BassStyle,
    /// How hard the generated parts pluck, 0..1. Softer = mellower and less buzz.
    pub touch: f32,
    /// Master tone (low-pass cutoff, Hz). Lower = warmer.
    pub tone_hz: f32,
    /// Timing/velocity humanisation, 0..1.
    pub humanize: f32,
    /// How much the melody wanders: how often variations change, plus live ornaments (0..1).
    pub variation: f32,
    /// How much the arrangement changes over time: parts drop out and come back in
    /// sections (breakdowns, solo mbira, drums out…). 0 = everything always plays.
    pub arrangement: f32,
    /// Systematic player micro-timing and phrase dynamics (0..1).
    pub groove: f32,
    pub reverb: f32,
    pub volume: f32,
    pub master: MasterParams,
    /// Weather ambience levels, 0..1.
    pub rain: f32,
    pub wind: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            tuning: TuningParams::default(),
            instrument_seed: 7,
            timbre: TimbreParams::default(),
            music: MusicParams::default(),
            tempo: 5.0,
            intensity: 0.4,
            auto_layers: true,
            play_kushaura: true,
            play_kutsinhira: true,
            play_hosho: true,
            hosho_level: 0.3,
            play_ngoma: true,
            play_bass: true,
            ngoma_level: 0.7,
            bass_level: 0.7,
            drum_style: DrumStyle::Bira,
            bass_style: BassStyle::Chimurenga,
            touch: 0.55,
            tone_hz: 7500.0,
            humanize: 0.5,
            variation: 0.5,
            arrangement: 0.5,
            groove: 0.5,
            reverb: 0.3,
            volume: 0.7,
            master: MasterParams::default(),
            rain: 0.0,
            wind: 0.0,
        }
    }
}

impl Settings {
    /// Which parts play, after applying `auto_layers`.
    pub fn active_layers(&self) -> Layers {
        if self.auto_layers {
            let i = self.intensity;
            Layers { kushaura: true, kutsinhira: i >= 0.55, hosho: i >= 0.65, ngoma: i >= 0.65, bass: i >= 0.45 }
        } else {
            Layers {
                kushaura: self.play_kushaura,
                kutsinhira: self.play_kutsinhira,
                hosho: self.play_hosho,
                ngoma: self.play_ngoma,
                bass: self.play_bass,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layers {
    pub kushaura: bool,
    pub kutsinhira: bool,
    pub hosho: bool,
    pub ngoma: bool,
    pub bass: bool,
}

/// Arrangement sections: which of the enabled parts sit out for a few cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Full,
    /// Ngoma and hosho rest.
    NoDrums,
    /// Second mbira rests; lead with the band.
    Groove,
    NoBass,
    /// Lead mbira and bass only.
    Breakdown,
    /// Lead mbira alone.
    Solo,
}

impl Section {
    const CHOICES: [(Section, f32); 6] = [
        (Section::Full, 3.0),
        (Section::NoDrums, 2.0),
        (Section::Groove, 1.5),
        (Section::NoBass, 1.0),
        (Section::Breakdown, 1.0),
        (Section::Solo, 0.6),
    ];

    pub fn name(self) -> &'static str {
        match self {
            Section::Full => "full",
            Section::NoDrums => "drums out",
            Section::Groove => "groove",
            Section::NoBass => "bass out",
            Section::Breakdown => "breakdown",
            Section::Solo => "solo mbira",
        }
    }

    fn apply(self, l: Layers) -> Layers {
        let mut l = l;
        match self {
            Section::Full => {}
            Section::NoDrums => {
                l.ngoma = false;
                l.hosho = false;
            }
            Section::Groove => l.kutsinhira = false,
            Section::NoBass => l.bass = false,
            Section::Breakdown => {
                l.kutsinhira = false;
                l.ngoma = false;
                l.hosho = false;
            }
            Section::Solo => {
                l.kutsinhira = false;
                l.ngoma = false;
                l.hosho = false;
                l.bass = false;
            }
        }
        l
    }
}

/// Every generated note is scheduled this much late, so human timing can push notes
/// early as well as late.
const LATENCY_S: f32 = 0.03;

/// Correlated human-feel generators (one per player).
struct Feel {
    tempo: Drift,
    timing: [Drift; 2],
    dynamics: [Drift; 2],
}

impl Feel {
    fn new(seed: u64) -> Self {
        Feel {
            tempo: Drift::new(seed, 0x7E, [12.0, 48.0, 192.0]),
            timing: [Drift::new(seed, 0x71, [3.0, 12.0, 48.0]), Drift::new(seed, 0x72, [3.0, 12.0, 48.0])],
            dynamics: [Drift::new(seed, 0xD1, [6.0, 24.0, 96.0]), Drift::new(seed, 0xD2, [6.0, 24.0, 96.0])],
        }
    }
}

const DRUM_GAIN: f32 = 1.9;
const BASS_GAIN: f32 = 1.6;

pub enum Command {
    Settings(Settings),
    Play,
    Stop,
    /// Manual pluck on instrument 0 (kushaura mbira).
    Pluck { key: u8, vel: f32 },
    /// Restart the integrated-loudness measurement.
    ResetLoudness,
}

#[derive(Clone, Copy, Debug)]
pub enum UiEvent {
    Key { inst: u8, key: u8, vel: f32 },
    Pulse { pulse: u8, variation: [u8; 2], section: Section, generation: u32 },
    Level(f32),
    /// Loudness reading and total gain reduction (dB) of the mastering chain.
    Loudness(LoudnessReading, f32),
}

#[derive(Clone, Copy, Default)]
struct Pending {
    delay: u32,
    inst: u8,
    key: u8,
    vel: f32,
    live: bool,
}

pub struct Engine {
    sr: f32,
    settings: Settings,
    mbiras: [Mbira; 2],
    hosho: [Hosho; 2],
    band: Band,
    ambience: Ambience,
    reverb: Reverb,
    tone: [Biquad; 2],
    limiter: Limiter,
    master: Master,
    meter: LoudnessMeter,
    /// Extra gain (dB) before the final limiter, used by loudness-normalised export.
    post_gain_db: f32,
    piece: Box<Piece>,
    playing: bool,
    pulse: usize,
    samples_to_pulse: f64,
    variation: [usize; 2],
    cycles_on_variation: [u32; 2],
    stay_for: [u32; 2],
    swell_phase: f32,
    cycle_gain: f32,
    pending: [Pending; 64],
    rng: Rng,
    feel: Feel,
    /// Fingerprints of the cycle in progress and of the last finished cycle
    /// (melody, band), for measuring how often the music repeats exactly.
    melody_fp: u64,
    section: Section,
    phrase_cut: f32,
    phrase_shift: i8,
    /// Slow 1/f drift of how full the playing is, stepped once per cycle.
    arc: f32,
    arc_drift: Drift,
    section_left: u32,
    /// The most recently developed variation slot (the player's newest idea).
    newest: usize,
    pub last_cycle: (u64, u64),
    cycle_events: EventLog,
    /// Every note, drum stroke and bass note of the last finished cycle.
    pub last_cycle_events: Box<EventLog>,
    pub cycles: u64,
    /// Current tempo multiplier from the ensemble's slow tempo drift.
    tempo_mod: f32,
    cmd_rx: Option<rtrb::Consumer<Command>>,
    ui_tx: Option<rtrb::Producer<UiEvent>>,
    peak: f32,
    peak_counter: u32,
}

impl Engine {
    pub fn new(sr: f32, settings: Settings) -> Self {
        let s = settings;
        let mut e = Engine {
            sr,
            settings: s,
            mbiras: [
                Mbira::new(sr, s.instrument_seed, &s.tuning, s.timbre, -0.12),
                Mbira::new(sr, s.instrument_seed.wrapping_add(1), &s.tuning, s.timbre, 0.12),
            ],
            hosho: [Hosho::new(sr, s.music.seed, -0.35, 4200.0), Hosho::new(sr, s.music.seed ^ 1, 0.35, 3600.0)],
            band: Band::new(sr, s.music.seed),
            ambience: Ambience::new(sr, s.instrument_seed),
            reverb: Reverb::new(sr),
            tone: [Biquad::lowpass(s.tone_hz, 0.6, sr); 2],
            limiter: Limiter::new(sr),
            master: Master::new(sr, s.master),
            meter: LoudnessMeter::new(sr),
            post_gain_db: 0.0,
            piece: Piece::generate(&s.music),
            playing: false,
            pulse: 0,
            samples_to_pulse: 0.0,
            variation: [0, 0],
            cycles_on_variation: [0, 0],
            stay_for: [3, 3],
            swell_phase: 0.0,
            cycle_gain: 1.0,
            pending: [Pending::default(); 64],
            rng: Rng::stream(s.music.seed, 0xE),
            feel: Feel::new(s.music.seed),
            melody_fp: 0,
            section: Section::Full,
            phrase_cut: 0.0,
            phrase_shift: 0,
            arc: 0.0,
            arc_drift: Drift::new(s.music.seed, 0xA5C, [2.0, 6.0, 16.0]),
            section_left: 4,
            newest: 0,
            last_cycle: (0, 0),
            cycle_events: EventLog::default(),
            last_cycle_events: Box::default(),
            cycles: 0,
            tempo_mod: 1.0,
            cmd_rx: None,
            ui_tx: None,
            peak: 0.0,
            peak_counter: 0,
        };
        e.reverb.room = 0.7;
        e.ambience.set_levels(s.rain, s.wind);
        e
    }

    /// Attach the lock-free queues used by the live (GUI) path.
    pub fn with_queues(mut self, rx: rtrb::Consumer<Command>, tx: rtrb::Producer<UiEvent>) -> Self {
        self.cmd_rx = Some(rx);
        self.ui_tx = Some(tx);
        self
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn play(&mut self) {
        if !self.playing {
            self.playing = true;
            self.pulse = 0;
            self.samples_to_pulse = 0.0;
            self.variation = [0, 0];
            self.cycles_on_variation = [0, 0];
        }
    }

    pub fn stop(&mut self) {
        self.playing = false;
        for p in self.pending.iter_mut() {
            p.live = false;
        }
    }

    pub fn apply_settings(&mut self, s: Settings) {
        let old = self.settings;
        self.settings = s;
        if old.tuning != s.tuning || old.instrument_seed != s.instrument_seed || old.timbre != s.timbre {
            self.mbiras[0].retune(s.instrument_seed, &s.tuning, s.timbre);
            self.mbiras[1].retune(s.instrument_seed.wrapping_add(1), &s.tuning, s.timbre);
        }
        self.ambience.set_levels(s.rain, s.wind);
        if old.master != s.master {
            self.update_master();
        }
        if old.tone_hz != s.tone_hz {
            for t in self.tone.iter_mut() {
                t.retune(Biquad::lowpass(s.tone_hz, 0.6, self.sr));
            }
        }
        if old.music != s.music {
            self.piece.generate_into(&s.music);
            self.variation = [0, 0];
            self.rng = Rng::stream(s.music.seed, 0xE);
            self.feel = Feel::new(s.music.seed);
            self.band.new_song(s.music.seed);
        }
    }

    fn update_master(&mut self) {
        let mut m = self.settings.master;
        m.output_db += self.post_gain_db;
        self.master.set_params(m);
    }

    /// Extra output gain before the limiter (loudness normalisation).
    pub fn set_post_gain_db(&mut self, db: f32) {
        self.post_gain_db = db;
        self.update_master();
    }

    pub fn loudness(&self) -> LoudnessReading {
        let mut r = self.meter.reading;
        r.integrated = self.meter.integrated();
        r
    }

    pub fn pluck(&mut self, inst: usize, key: usize, vel: f32) {
        if key < NUM_KEYS {
            self.mbiras[inst.min(1)].pluck(key, vel);
            self.send(UiEvent::Key { inst: inst as u8, key: key as u8, vel });
        }
    }

    fn send(&mut self, ev: UiEvent) {
        if let Some(tx) = self.ui_tx.as_mut() {
            let _ = tx.push(ev);
        }
    }

    fn schedule(&mut self, inst: u8, key: u8, vel: f32, delay: u32) {
        if let Some(slot) = self.pending.iter_mut().find(|p| !p.live) {
            *slot = Pending { delay, inst, key, vel, live: true };
        }
    }

    fn drain_commands(&mut self) {
        let Some(mut rx) = self.cmd_rx.take() else { return };
        while let Ok(cmd) = rx.pop() {
            match cmd {
                Command::Settings(s) => self.apply_settings(s),
                Command::Play => self.play(),
                Command::Stop => self.stop(),
                Command::Pluck { key, vel } => self.pluck(0, key as usize, vel),
                Command::ResetLoudness => self.meter.reset(),
            }
        }
        self.cmd_rx = Some(rx);
    }

    fn on_cycle_end(&mut self) {
        let s = self.settings;
        self.last_cycle = (self.melody_fp, self.band.fingerprint);
        for &e in self.band.events.as_slice() {
            self.cycle_events.push(e);
        }
        *self.last_cycle_events = self.cycle_events;
        self.cycle_events.clear();
        self.band.events.clear();
        self.melody_fp = 0;
        self.band.fingerprint = 0;
        self.cycles += 1;
        // A fresh dynamic contour for the next cycle.
        self.swell_phase = self.rng.range(0.0, std::f32::consts::TAU);
        self.cycle_gain = 1.0 + self.rng.gauss() * 0.05 * s.groove;
        self.band.on_cycle_end(s.variation, (s.drum_style, s.bass_style));

        self.arc = self.arc_drift.next().clamp(-1.5, 1.5);

        // Arrangement: hold a section for a few cycles, then move to another one.
        self.section_left = self.section_left.saturating_sub(1);
        if self.section_left == 0 {
            self.section = self.next_section();
            let max = 2 + ((1.0 - 0.5 * s.arrangement) * 6.0) as usize;
            self.section_left = 2 + self.rng.below(max) as u32;
        }

        // Development: the player keeps finding new versions of the song, so the pool of
        // variations evolves instead of looping the same handful forever.
        if self.rng.chance(0.25 + 0.5 * s.variation) {
            let mut slot = 1 + self.rng.below(VARIATIONS - 1);
            if slot == self.variation[0] || slot == self.variation[1] {
                slot = 1 + (slot % (VARIATIONS - 1));
            }
            let from = if self.rng.chance(0.6) { self.variation[0] } else { self.rng.below(VARIATIONS) };
            let mut r = self.rng;
            self.piece.evolve(slot, from, &mut r);
            self.rng = Rng::stream(r.next_u64(), 0xE7);
            self.newest = slot;
        }

        for part in 0..2 {
            self.cycles_on_variation[part] += 1;
            if self.cycles_on_variation[part] >= self.stay_for[part] {
                self.cycles_on_variation[part] = 0;
                // More variation (and intensity) → change sooner.
                let calm = (1.0 - s.variation) * 0.7 + (1.0 - s.intensity) * 0.3;
                let max_stay = 1 + (calm * 6.0) as usize;
                self.stay_for[part] = 1 + self.rng.below(max_stay) as u32;
                self.variation[part] = if self.variation[part] != 0 && self.rng.chance(0.25) {
                    0 // return home to the basic chara
                } else if self.newest != 0 && self.newest != self.variation[part] && self.rng.chance(0.5) {
                    self.newest // play the newest idea
                } else {
                    (self.variation[part] + 1 + self.rng.below(VARIATIONS - 1)) % VARIATIONS
                };
            }
        }
    }

    fn next_section(&mut self) -> Section {
        let a = self.settings.arrangement;
        if a <= 0.0 {
            return Section::Full;
        }
        // Higher arrangement → less time in Full.
        let weight = |sec: Section, w: f32| if sec == Section::Full { w * (1.5 - a) } else { w * a };
        let total: f32 = Section::CHOICES.iter().map(|&(sec, w)| weight(sec, w)).sum();
        let mut x = self.rng.f32() * total;
        for (sec, w) in Section::CHOICES {
            x -= weight(sec, w);
            if x <= 0.0 && sec != self.section {
                return sec;
            }
        }
        Section::Full
    }

    pub fn section(&self) -> Section {
        self.section
    }

    fn on_pulse(&mut self) {
        let s = self.settings;
        let layers = self.section.apply(s.active_layers());
        let (kush, kuts, hosho) = (layers.kushaura, layers.kutsinhira, layers.hosho);
        // Density: low intensity drops the less important notes.
        // Phrasing: each quarter-cycle the player thins out a little differently, and the
        // overall fullness drifts slowly (the long arc of building up and calming down).
        if self.pulse % 12 == 0 {
            self.phrase_cut = self.rng.gauss() * 0.08 * (0.3 + s.variation);
            // Register play for a whole phrase: the right hand sings an octave up (huro),
            // or the left hand moves its notes to the other octave.
            self.phrase_shift = if self.rng.chance(0.1 + 0.25 * s.variation) {
                if self.rng.chance(0.6) { 1 } else { -1 }
            } else {
                0
            };
        }
        let cutoff = 1.0 - (0.35 + 0.65 * s.intensity) + self.phrase_cut - self.arc * 0.18 * (0.3 + s.arrangement);
        let hum = s.humanize;
        // Human feel: the ensemble's tempo breathes slowly, and each player drifts ahead of
        // and behind the beat (correlated, not white jitter), with their own dynamics.
        self.tempo_mod = 1.0 + 0.018 * hum * self.feel.tempo.next();
        let mut timing = [0.0f32; 2];
        let mut dynamics = [1.0f32; 2];
        for i in 0..2 {
            timing[i] = self.feel.timing[i].next() * hum * 0.009 * self.sr;
            dynamics[i] = 1.0 + self.feel.dynamics[i].next() * 0.12 * hum;
        }
        let base_delay = LATENCY_S * self.sr;
        let touch = 0.3 + 0.7 * s.touch;
        let p = self.pulse;
        // Groove: consistent micro-timing inside each 6-pulse group (fraction of a pulse,
        // all ≥ 0 so it can be scheduled), plus a slow swell over the cycle.
        const GROOVE: [f32; 6] = [0.02, 0.09, 0.04, 0.10, 0.0, 0.07];
        let pulse_len = self.sr / s.tempo.clamp(1.0, 20.0);
        let groove_delay = GROOVE[p % 6] * s.groove * pulse_len;
        let phase = p as f32 / PULSES as f32 * std::f32::consts::TAU;
        let swell = self.cycle_gain * (1.0 + 0.12 * s.groove * (phase + self.swell_phase).sin());
        let ornament_p = s.variation * 0.12;
        for (inst, on) in [(0usize, kush), (1usize, kuts)] {
            if !on {
                continue;
            }
            let chara = if inst == 0 {
                &self.piece.kushaura[self.variation[0]]
            } else {
                &self.piece.kutsinhira[self.variation[1]]
            };
            let notes: [Option<_>; MAX_NOTES] = chara.notes[p];
            for n in notes.into_iter().flatten() {
                if n.importance < cutoff {
                    continue;
                }
                let mut key = n.key;
                if self.phrase_shift != 0 {
                    let k = &KEYS[key as usize];
                    if let Some(alt) = octave_alternative(key as usize) {
                        let up = KEYS[alt].degree > k.degree;
                        if (self.phrase_shift > 0 && !k.manual.is_left() && up) || (self.phrase_shift < 0 && k.manual.is_left()) {
                            key = alt as u8;
                        }
                    }
                }
                // Live ornaments: occasional octave substitution or a dropped (ghost) note.
                if self.rng.chance(ornament_p) {
                    if self.rng.chance(0.7) {
                        if let Some(alt) = octave_alternative(key as usize) {
                            key = alt as u8;
                        }
                    } else if n.importance < 0.9 {
                        continue;
                    }
                }
                let vel = (n.vel * touch * swell * dynamics[inst] * (1.0 + self.rng.gauss() * 0.06 * hum)).clamp(0.05, 1.1);
                let delay = (base_delay + groove_delay + timing[inst] + self.rng.gauss() * 0.002 * hum * self.sr).max(0.0) as u32;
                self.schedule(inst as u8, key, vel, delay);
                self.melody_fp = crate::band::mix(self.melody_fp, (p as u64) << 16 | (inst as u64) << 8 | key as u64);
                self.cycle_events.push((p as u32) << 8 | (inst as u32) << 6 | key as u32);
            }
        }
        if hosho {
            // "sh-ka-ka": accented first stroke of each beat.
            const STROKES: [f32; 3] = [1.0, 0.45, 0.6];
            let st = STROKES[p % 3] * s.hosho_level;
            let h = p % 2;
            self.hosho[h].shake(st * (1.0 + self.rng.gauss() * 0.1 * s.humanize));
        }
        // The drummer signals a change: of variation, or of section.
        let change_coming = self.cycles_on_variation[0] + 1 >= self.stay_for[0] || self.section_left == 1;
        self.band.on_pulse(BandPulse {
            pulse: p,
            roots: &self.piece.roots,
            tuning: &s.tuning,
            pulse_len,
            drums: layers.ngoma,
            bass: layers.bass,
            drum_style: s.drum_style,
            bass_style: s.bass_style,
            drum_level: s.ngoma_level,
            bass_level: s.bass_level,
            intensity: s.intensity,
            variation: s.variation,
            humanize: s.humanize,
            groove_delay,
            base_delay,
            swell,
            change_coming,
        });
        let (section, generation) = (self.section, self.piece.generation);
        self.send(UiEvent::Pulse { pulse: p as u8, variation: [self.variation[0] as u8, self.variation[1] as u8], section, generation });
    }

    /// Fill an interleaved buffer with `channels` channels (1 or more).
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        self.drain_commands();
        let channels = channels.max(1);
        let samples_per_pulse = self.sr as f64 / self.settings.tempo.clamp(1.0, 20.0) as f64;
        let wet = self.settings.reverb;
        let vol = self.settings.volume * 0.6;
        let hosho_gain = 0.6;
        for frame in out.chunks_mut(channels) {
            if self.playing {
                self.samples_to_pulse -= 1.0;
                if self.samples_to_pulse <= 0.0 {
                    self.samples_to_pulse += samples_per_pulse * self.tempo_mod as f64;
                    self.on_pulse();
                    self.pulse += 1;
                    if self.pulse == PULSES {
                        self.pulse = 0;
                        self.on_cycle_end();
                    } else if self.pulse == PULSES / 2 && self.rng.chance(self.settings.variation * 0.25) {
                        // Occasionally switch the lead part's variation at the half cycle.
                        self.variation[0] = (self.variation[0] + 1 + self.rng.below(VARIATIONS - 1)) % VARIATIONS;
                    }
                }
            }
            for i in 0..self.pending.len() {
                let pd = self.pending[i];
                if pd.live {
                    if pd.delay == 0 {
                        self.pending[i].live = false;
                        self.pluck(pd.inst as usize, pd.key as usize, pd.vel);
                    } else {
                        self.pending[i].delay -= 1;
                    }
                }
            }
            let (al, ar) = self.mbiras[0].tick();
            let (bl, br) = self.mbiras[1].tick();
            let (h0l, h0r) = self.hosho[0].tick();
            let (h1l, h1r) = self.hosho[1].tick();
            let ((nl, nr), bass) = self.band.tick();
            let (wl, wr) = self.ambience.tick();
            let l = self.tone[0].tick(al + bl + (h0l + h1l) * hosho_gain);
            let r = self.tone[1].tick(ar + br + (h0r + h1r) * hosho_gain);
            // Drum and bass: mostly dry, a little room.
            let (bl2, br2) = (nl * DRUM_GAIN + bass * BASS_GAIN + wl, nr * DRUM_GAIN + bass * BASS_GAIN + wr);
            let (rl, rr) = if wet > 0.0 { self.reverb.tick(l + bl2 * 0.25, r + br2 * 0.25) } else { (0.0, 0.0) };
            let (l, r) = (l + bl2, r + br2);
            let (l, r) = ((l + rl * wet * 3.0) * vol, (r + rr * wet * 3.0) * vol);
            let (l, r) = if self.settings.master.enabled {
                let (l, r) = self.master.tick(l, r);
                (l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0))
            } else {
                let g = 10f32.powf(self.post_gain_db / 20.0);
                self.limiter.tick(l * g, r * g)
            };
            self.meter.tick(l, r);
            self.peak = self.peak.max(l.abs()).max(r.abs());
            if channels == 1 {
                frame[0] = 0.5 * (l + r);
            } else {
                frame[0] = l;
                frame[1] = r;
                for x in frame[2..].iter_mut() {
                    *x = 0.0;
                }
            }
        }
        self.peak_counter += 1;
        if self.peak_counter >= 4 {
            let pk = self.peak;
            self.send(UiEvent::Level(pk));
            let reading = self.meter.reading;
            let gr = if self.settings.master.enabled { self.master.gain_reduction_db() } else { 0.0 };
            self.send(UiEvent::Loudness(reading, gr));
            self.peak = 0.0;
            self.peak_counter = 0;
        }
    }

    pub fn current_pulse(&self) -> usize {
        self.pulse
    }

    pub fn dyad_index(&self) -> usize {
        self.pulse / PULSES_PER_DYAD
    }

    pub fn key_freq(&self, k: usize) -> f32 {
        self.mbiras[0].key_freq(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(settings: Settings, seconds: f32) -> Vec<f32> {
        let sr = 48_000.0;
        let mut e = Engine::new(sr, settings);
        e.play();
        let mut buf = vec![0.0; (sr * seconds) as usize * 2];
        for chunk in buf.chunks_mut(1024) {
            e.process(chunk, 2);
        }
        buf
    }

    #[test]
    fn finite_and_bounded() {
        let mut s = Settings::default();
        s.intensity = 1.0;
        s.timbre.buzz = 1.0;
        s.timbre.sympathetic = 1.0;
        let buf = render(s, 8.0);
        assert!(buf.iter().all(|x| x.is_finite()));
        let peak = buf.iter().fold(0.0f32, |a, &x| a.max(x.abs()));
        assert!(peak <= 1.0, "peak {peak}");
        assert!(peak > 0.05, "too quiet: {peak}");
    }

    #[test]
    fn every_master_preset_respects_its_ceiling() {
        for preset in crate::master::MasterPreset::ALL {
            let mut s = Settings::default();
            s.intensity = 1.0;
            s.volume = 1.0;
            s.master = preset.params();
            let buf = render(s, 6.0);
            let peak = buf.iter().fold(0.0f32, |a, &x| a.max(x.abs()));
            let ceiling = if s.master.enabled { 10f32.powf(s.master.ceiling_db / 20.0) } else { 1.0 };
            assert!(buf.iter().all(|x| x.is_finite()));
            assert!(peak <= ceiling + 1e-4, "{preset:?}: peak {peak} > {ceiling}");
            assert!(peak > 0.05, "{preset:?} too quiet: {peak}");
        }
    }

    #[test]
    fn deterministic_audio() {
        let s = Settings::default();
        assert_eq!(render(s, 2.0), render(s, 2.0));
    }

    /// Goertzel magnitude of `x` at `f`.
    fn goertzel(x: &[f32], f: f32, sr: f32) -> f32 {
        let w = 2.0 * std::f32::consts::PI * f / sr;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &v in x {
            let s0 = v + c * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - c * s1 * s2).sqrt()
    }

    #[test]
    fn pluck_has_inharmonic_fast_decaying_overtone() {
        let sr = 48_000.0;
        let mut s = Settings::default();
        s.timbre.ratio_jitter = 0.0;
        s.timbre.buzz = 0.0;
        s.timbre.click = 0.0;
        s.timbre.sympathetic = 0.0;
        s.reverb = 0.0;
        let mut m = Mbira::new(sr, 1, &s.tuning, s.timbre, 0.0);
        let key = 3;
        let f1 = m.key_freq(key);
        m.pluck(key, 1.0);
        let mono: Vec<f32> = (0..(sr as usize)).map(|_| { let (l, r) = m.tick(); l + r }).collect();
        let n = 4096;
        let early = &mono[..n];
        let late = &mono[mono.len() - n..];
        let f2 = f1 * 5.2;
        let e1 = goertzel(early, f1, sr);
        let e2 = goertzel(early, f2, sr);
        let l1 = goertzel(late, f1, sr);
        let l2 = goertzel(late, f2, sr);
        assert!(e2 > 0.05 * e1, "overtone missing in attack");
        assert!(goertzel(early, f1 * 2.0, sr) < e2, "harmonic 2 should be weaker than inharmonic 5.2x partial");
        assert!(l2 / e2 < 0.1 * (l1 / e1), "overtone should decay much faster than the fundamental");
    }

    #[test]
    fn buzz_is_threshold_dependent() {
        let sr = 48_000.0;
        let mut t = TimbreParams::default();
        t.buzz = 1.0;
        t.click = 0.0;
        t.body = crate::instrument::BodyPreset::None;
        let tuning = TuningParams::default();
        let energy = |vel: f32, buzz: f32| {
            let mut tt = t;
            tt.buzz = buzz;
            let mut m = Mbira::new(sr, 1, &tuning, tt, 0.0);
            m.pluck(0, vel);
            (0..(sr as usize / 2)).map(|_| { let (l, r) = m.tick(); l * l + r * r }).sum::<f32>()
        };
        let buzz_soft = energy(0.15, 1.0) - energy(0.15, 0.0);
        let buzz_loud = energy(1.0, 1.0) - energy(1.0, 0.0);
        assert!(buzz_loud > 20.0 * buzz_soft.abs().max(1e-9), "soft {buzz_soft} loud {buzz_loud}");
    }
}
