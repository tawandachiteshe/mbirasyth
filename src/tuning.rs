//! Tunings and key layout (docs/02).

use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningPreset {
    Nyamaropa,
    Dambatsoko,
    Mavembe,
    Katsanzaira,
    Saungweme,
}

impl TuningPreset {
    pub const ALL: [TuningPreset; 5] = [
        TuningPreset::Nyamaropa,
        TuningPreset::Dambatsoko,
        TuningPreset::Mavembe,
        TuningPreset::Katsanzaira,
        TuningPreset::Saungweme,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TuningPreset::Nyamaropa => "Nyamaropa (≈ Mixolydian)",
            TuningPreset::Dambatsoko => "Dambatsoko (≈ Ionian)",
            TuningPreset::Mavembe => "Mavembe / Gandanga (≈ Phrygian)",
            TuningPreset::Katsanzaira => "Katsanzaira (≈ Dorian)",
            TuningPreset::Saungweme => "Saungweme (≈ 7-TET)",
        }
    }

    /// Western-mode approximation in cents above the tonic.
    pub fn mode_cents(self) -> [f32; 7] {
        match self {
            TuningPreset::Nyamaropa => [0., 200., 400., 500., 700., 900., 1000.],
            TuningPreset::Dambatsoko => [0., 200., 400., 500., 700., 900., 1100.],
            TuningPreset::Mavembe => [0., 100., 300., 500., 700., 800., 1000.],
            TuningPreset::Katsanzaira => [0., 200., 300., 500., 700., 900., 1000.],
            TuningPreset::Saungweme => [0., 200., 400., 500., 700., 900., 1000.],
        }
    }

    /// Suggested `equalness` for the preset.
    pub fn default_equalness(self) -> f32 {
        match self {
            TuningPreset::Saungweme => 0.95,
            _ => 0.35,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TuningParams {
    pub preset: TuningPreset,
    /// Frequency of the lowest key (scale degree 0).
    pub tonic_hz: f32,
    /// 0 = Western mode, 1 = 7-tone equal temperament.
    pub equalness: f32,
    /// Octave stretch factor (1.0 = pure octaves).
    pub stretch: f32,
    /// Std-dev of per-key random detune, cents.
    pub detune_cents: f32,
}

impl Default for TuningParams {
    fn default() -> Self {
        TuningParams {
            preset: TuningPreset::Nyamaropa,
            tonic_hz: 110.0,
            equalness: 0.35,
            stretch: 1.003,
            detune_cents: 6.0,
        }
    }
}

impl TuningParams {
    /// Frequency of an absolute scale degree, without per-key detune.
    pub fn degree_hz(&self, degree: i32) -> f32 {
        let d = degree.rem_euclid(7) as usize;
        let oct = degree.div_euclid(7) as f32;
        let mode = self.preset.mode_cents()[d];
        let equal = d as f32 * 1200.0 / 7.0;
        let cents = mode + (equal - mode) * self.equalness + 1200.0 * oct * self.stretch;
        self.tonic_hz * 2f32.powf(cents / 1200.0)
    }

    /// Frequencies of every key, with seeded per-key detune.
    pub fn key_freqs(&self, instrument_seed: u64) -> [f32; NUM_KEYS] {
        let mut rng = Rng::stream(instrument_seed, 0xDE7);
        let mut out = [0.0; NUM_KEYS];
        for (k, key) in KEYS.iter().enumerate() {
            let det = rng.gauss() * self.detune_cents;
            out[k] = self.degree_hz(key.degree) * 2f32.powf(det / 1200.0);
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manual {
    /// Left lower — bass (left thumb).
    Bass,
    /// Left upper — middle register (left thumb).
    Upper,
    /// Right, played by the right thumb (downstroke).
    RightThumb,
    /// Right, played by the right index finger (upstroke from below).
    RightIndex,
}

impl Manual {
    pub fn is_left(self) -> bool {
        matches!(self, Manual::Bass | Manual::Upper)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct KeyDef {
    pub label: &'static str,
    pub manual: Manual,
    /// Absolute scale degree (0 = lowest tonic; +7 per octave).
    pub degree: i32,
    /// Computer-keyboard shortcut shown in the GUI.
    pub shortcut: char,
}

pub const NUM_KEYS: usize = 24;

/// Provisional 24-key *mbira dzavadzimu*-style layout. It follows the documented
/// structure (bass / middle / right manuals, octave duplicates between manuals,
/// ~3 octaves) but is NOT a measured key-by-key chart of a real instrument —
/// see docs/02 §4.
pub const KEYS: [KeyDef; NUM_KEYS] = [
    // Left lower (bass), degrees 0..6
    KeyDef { label: "B1", manual: Manual::Bass, degree: 0, shortcut: 'Z' },
    KeyDef { label: "B2", manual: Manual::Bass, degree: 1, shortcut: 'X' },
    KeyDef { label: "B3", manual: Manual::Bass, degree: 2, shortcut: 'C' },
    KeyDef { label: "B4", manual: Manual::Bass, degree: 3, shortcut: 'V' },
    KeyDef { label: "B5", manual: Manual::Bass, degree: 4, shortcut: 'B' },
    KeyDef { label: "B6", manual: Manual::Bass, degree: 5, shortcut: 'N' },
    KeyDef { label: "B7", manual: Manual::Bass, degree: 6, shortcut: 'M' },
    // Left upper (middle), degrees 7..13
    KeyDef { label: "L1", manual: Manual::Upper, degree: 7, shortcut: 'A' },
    KeyDef { label: "L2", manual: Manual::Upper, degree: 8, shortcut: 'S' },
    KeyDef { label: "L3", manual: Manual::Upper, degree: 9, shortcut: 'D' },
    KeyDef { label: "L4", manual: Manual::Upper, degree: 10, shortcut: 'F' },
    KeyDef { label: "L5", manual: Manual::Upper, degree: 11, shortcut: 'G' },
    KeyDef { label: "L6", manual: Manual::Upper, degree: 12, shortcut: 'H' },
    KeyDef { label: "L7", manual: Manual::Upper, degree: 13, shortcut: 'J' },
    // Right thumb, degrees 9..11 (duplicates of L3..L5)
    KeyDef { label: "R1", manual: Manual::RightThumb, degree: 9, shortcut: 'Q' },
    KeyDef { label: "R2", manual: Manual::RightThumb, degree: 10, shortcut: 'W' },
    KeyDef { label: "R3", manual: Manual::RightThumb, degree: 11, shortcut: 'E' },
    // Right index, degrees 14..20
    KeyDef { label: "R4", manual: Manual::RightIndex, degree: 14, shortcut: 'R' },
    KeyDef { label: "R5", manual: Manual::RightIndex, degree: 15, shortcut: 'T' },
    KeyDef { label: "R6", manual: Manual::RightIndex, degree: 16, shortcut: 'Y' },
    KeyDef { label: "R7", manual: Manual::RightIndex, degree: 17, shortcut: 'U' },
    KeyDef { label: "R8", manual: Manual::RightIndex, degree: 18, shortcut: 'I' },
    KeyDef { label: "R9", manual: Manual::RightIndex, degree: 19, shortcut: 'O' },
    KeyDef { label: "R10", manual: Manual::RightIndex, degree: 20, shortcut: 'P' },
];

/// Stereo position of a key: left manuals left, right manual right, lowest near centre.
pub fn key_pan(k: usize) -> f32 {
    let key = &KEYS[k];
    let spread = (key.degree % 7) as f32 / 6.0;
    match key.manual {
        Manual::Bass => -0.10 - 0.25 * spread,
        Manual::Upper => -0.20 - 0.30 * spread,
        Manual::RightThumb => 0.10 + 0.10 * spread,
        Manual::RightIndex => 0.15 + 0.35 * ((key.degree - 14) as f32 / 6.0),
    }
}

/// Note-ish letter name for a scale degree in Tracey's "C" notation.
pub fn degree_letter(d: i32) -> char {
    ['C', 'D', 'E', 'F', 'G', 'A', 'B'][d.rem_euclid(7) as usize]
}

pub fn hz_to_note_name(hz: f32) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let n = midi.round() as i32;
    let cents = ((midi - n as f32) * 100.0).round() as i32;
    format!("{}{} {:+}c", NAMES[n.rem_euclid(12) as usize], n.div_euclid(12) - 1, cents)
}
