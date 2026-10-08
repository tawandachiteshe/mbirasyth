//! Procedural composition (docs/03 and docs/05).
//!
//! Everything is fixed-size and `Copy` so a piece can be generated inside the
//! audio thread without allocating.

use crate::rng::Rng;
use crate::tuning::{Manual, KEYS};

pub const PULSES: usize = 48;
pub const DYADS: usize = 12;
pub const PULSES_PER_DYAD: usize = PULSES / DYADS;
pub const MAX_NOTES: usize = 3;
pub const VARIATIONS: usize = 8;

/// Andrew Tracey's universal Shona sequence as chord movements:
/// 3 = up a third (+2 scale steps), 4 = up a fourth (+3 steps). "334 343 434 333".
pub const INTERVALS: [u8; DYADS] = [3, 3, 4, 3, 4, 3, 4, 3, 4, 3, 3, 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standard {
    /// C standard (Nyamaropa, Nhemamusasa…).
    C,
    E,
    F,
    A,
    /// 6-chord karimba "short" sequence `C E G C F A`, played twice per cycle.
    Short,
}

impl Standard {
    pub const ALL: [Standard; 5] = [Standard::C, Standard::E, Standard::F, Standard::A, Standard::Short];

    pub fn name(self) -> &'static str {
        match self {
            Standard::C => "C standard (Nyamaropa)",
            Standard::E => "E standard",
            Standard::F => "F standard",
            Standard::A => "A standard (pop-friendly)",
            Standard::Short => "Short karimba cycle",
        }
    }

    fn rotation(self) -> usize {
        match self {
            Standard::C => 0,
            Standard::E => 1,
            Standard::A => 5,
            Standard::F => 7,
            Standard::Short => 0,
        }
    }
}

/// Dyad roots (scale degrees, may exceed 7) for the 12 chords of a cycle.
pub fn cycle_roots(standard: Standard, start: i32) -> [i32; DYADS] {
    let mut roots = [0; DYADS];
    if standard == Standard::Short {
        // C E G C F A  (steps 2 2 3 3 2 2 = 14 = 2 octaves)
        const SHORT: [i32; 6] = [0, 2, 4, 7, 10, 12];
        for i in 0..DYADS {
            roots[i] = start + SHORT[i % 6];
        }
        return roots;
    }
    let rot = standard.rotation();
    roots[0] = start;
    for i in 1..DYADS {
        let step = if INTERVALS[(i - 1 + rot) % DYADS] == 3 { 2 } else { 3 };
        roots[i] = roots[i - 1] + step;
    }
    roots
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Note {
    pub key: u8,
    pub vel: f32,
    /// 0..1; notes below the density cutoff are skipped at low intensity.
    pub importance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chara {
    pub notes: [[Option<Note>; MAX_NOTES]; PULSES],
}

impl Default for Chara {
    fn default() -> Self {
        Chara { notes: [[None; MAX_NOTES]; PULSES] }
    }
}

impl Chara {
    fn push(&mut self, p: usize, n: Note) {
        if let Some(slot) = self.notes[p].iter_mut().find(|s| s.is_none()) {
            *slot = Some(n);
        }
    }

    /// The same part, played `by` pulses later (kutsinhira delay mode).
    pub fn shifted(&self, by: usize) -> Chara {
        let mut c = Chara::default();
        for p in 0..PULSES {
            c.notes[(p + by) % PULSES] = self.notes[p];
        }
        c
    }

    pub fn note_count(&self) -> usize {
        self.notes.iter().flatten().filter(|n| n.is_some()).count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KutsinhiraMode {
    /// Kushaura delayed by one pulse.
    Delay,
    /// A new, bass-heavier part interlocking with the kushaura.
    Complement,
}

impl KutsinhiraMode {
    pub const ALL: [KutsinhiraMode; 2] = [KutsinhiraMode::Delay, KutsinhiraMode::Complement];
    pub fn name(self) -> &'static str {
        match self {
            KutsinhiraMode::Delay => "Delay (1 pulse behind)",
            KutsinhiraMode::Complement => "Complement (bass part)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MusicParams {
    pub seed: u64,
    pub standard: Standard,
    /// Scale degree the cycle starts on (0..6).
    pub start_degree: i32,
    pub kutsinhira: KutsinhiraMode,
}

impl Default for MusicParams {
    fn default() -> Self {
        MusicParams { seed: 1, standard: Standard::C, start_degree: 0, kutsinhira: KutsinhiraMode::Delay }
    }
}

// ---------------------------------------------------------------------------
// Figure-based part generation.
//
// Modelled on a real transcription (kushaura of "Nhemamusasa", Wikimedia Commons
// "Mbira cross-rhythm.mid"): in every 6-pulse group the right hand plays pulses
// 0,2,4 and the left hand 0,3,5 (B . R L R L). Each hand repeats a 3-note
// *figure*; every slot of the figure has a fixed target pitch (anchor), and on each
// pulse it sounds the dyad tone nearest that anchor. Common tones therefore stay
// put and only one or two notes move — by step — when the chord changes.

/// Rhythm cell over 6 pulses: `x` = the hand plays.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cell {
    right: &'static [u8; 6],
    left: &'static [u8; 6],
}

const CELLS: [Cell; 3] = [
    // Nhemamusasa kushaura: together . right left right left
    Cell { right: b"x.x.x.", left: b"x..x.x" },
    // Same, left hand pushed one pulse later
    Cell { right: b"x.x.x.", left: b".x.x.x" },
    // Sparser: right on beats 0 and 2, left answers
    Cell { right: b"x...x.", left: b"x.x..x" },
];

/// Alternative 6-pulse rhythms a variation may give one hand (max 3 notes each).
const RIGHT_RHYTHMS: [&[u8; 6]; 5] = [b"x.x.x.", b"x..xx.", b"xx..x.", b"x.xx..", b".x.x.x"];
const LEFT_RHYTHMS: [&[u8; 6]; 5] = [b"x..x.x", b".x.x.x", b"x.x..x", b"x..xx.", b"x...x."];

/// Right-hand figure shapes: (anchor offset, contour) per slot, relative to the figure's peak.
const SHAPES: [[(i32, Contour); 3]; 6] = [
    [(0, Contour::Any), (-2, Contour::Below), (0, Contour::Above)],  // dip & return
    [(0, Contour::Any), (-2, Contour::Below), (1, Contour::Above)],  // dip & rise past
    [(0, Contour::Any), (2, Contour::Above), (-1, Contour::Below)],  // rise & fall
    [(-3, Contour::Any), (-1, Contour::Above), (1, Contour::Above)], // climbing
    [(1, Contour::Any), (-1, Contour::Below), (-3, Contour::Below)], // falling
    [(0, Contour::Any), (0, Contour::Any), (-2, Contour::Below)],    // repeated then dip
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    Bass,
    Upper,
    Right,
}

impl Reach {
    fn allows(self, m: Manual) -> bool {
        match self {
            Reach::Bass => m == Manual::Bass,
            Reach::Upper => m == Manual::Upper,
            Reach::Right => matches!(m, Manual::RightThumb | Manual::RightIndex),
        }
    }
}

/// Melodic direction of a slot relative to the hand's previous note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Contour {
    Any,
    Below,
    Above,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Slot {
    anchor: i32,
    reach: Reach,
    contour: Contour,
    importance: f32,
    vel: f32,
}

/// A part = rhythm cell + 3-slot figure per hand (+ optional right-thumb fills).
#[derive(Clone, Copy, Debug, PartialEq)]
struct PartSpec {
    cell: Cell,
    right: [Slot; 3],
    left: [Slot; 3],
    /// Fill the right hand's rests with a low right-thumb line (high intensity only).
    fill: Option<Slot>,
    /// Pulses the whole part is delayed by.
    shift: usize,
}

fn dyad_at(roots: &[i32; DYADS], pulse: i32) -> i32 {
    roots[(pulse.rem_euclid(PULSES as i32) as usize) / PULSES_PER_DYAD]
}

/// The nearest key on the same side of the instrument with the same pitch class
/// (an octave substitute), used for live ornamentation.
pub fn octave_alternative(k: usize) -> Option<usize> {
    let d = KEYS[k].degree;
    let left = KEYS[k].manual.is_left();
    KEYS.iter()
        .enumerate()
        .filter(|(j, key)| *j != k && key.manual.is_left() == left && key.degree.rem_euclid(7) == d.rem_euclid(7) && key.degree != d)
        .min_by_key(|(_, key)| (key.degree - d).abs())
        .map(|(j, _)| j)
}

/// Notes that are played at normal density (excludes optional fills).
fn core_notes(c: &Chara) -> usize {
    c.notes.iter().flatten().flatten().filter(|n| n.importance > 0.3).count()
}

/// Is `degree` one of the dyad tones (root or fifth) of `root`?
pub fn in_dyad(degree: i32, root: i32) -> bool {
    let d = degree.rem_euclid(7);
    d == root.rem_euclid(7) || d == (root + 4).rem_euclid(7)
}

/// The key in reach that sounds a tone of `root` nearest the anchor.
/// Ties go to the key this slot played last time (common-tone stickiness).
fn realize(slot: &Slot, root: i32, last: Option<usize>, prev: Option<usize>) -> Option<usize> {
    let mut best = None;
    let mut best_cost = f32::MAX;
    for (k, key) in KEYS.iter().enumerate() {
        if !slot.reach.allows(key.manual) || !in_dyad(key.degree, root) {
            continue;
        }
        let mut cost = (key.degree - slot.anchor).abs() as f32;
        // Keep the figure's shape: breaking the contour is possible but expensive.
        if let Some(pk) = prev {
            let pd = KEYS[pk].degree;
            let broken = match slot.contour {
                Contour::Any => false,
                Contour::Below => key.degree >= pd,
                Contour::Above => key.degree <= pd,
            };
            if broken {
                cost += 4.0;
            }
        }
        if Some(k) == last {
            cost -= 0.6;
        }
        // Prefer the right index over the thumb for the same pitch region.
        if key.manual == Manual::RightThumb {
            cost += 0.1;
        }
        if cost < best_cost {
            best_cost = cost;
            best = Some(k);
        }
    }
    best
}

/// Render a part; `second` (if different) is used for the second half of the cycle.
fn render_part(roots: &[i32; DYADS], first: &PartSpec, second: &PartSpec, rng: &mut Rng) -> Chara {
    let mut c = Chara::default();
    let mut last_r = [None::<usize>; 3];
    let mut last_l = [None::<usize>; 3];
    let mut last_fill = None;
    let mut prev_r = None::<usize>;
    // Two passes so stickiness is already warmed up when the cycle wraps.
    for pass in 0..2 {
        for p in 0..PULSES {
            let spec = if p < PULSES / 2 { first } else { second };
            let q = p % 6;
            let root = dyad_at(roots, p as i32);
            let group_start = q == 0;
            let mut emit = |c: &mut Chara, k: usize, slot: &Slot| {
                if pass == 1 {
                    let accent = if group_start { 0.06 } else { 0.0 };
                    let vel = (slot.vel + accent + rng.gauss() * 0.03).clamp(0.2, 1.0);
                    c.push((p + spec.shift) % PULSES, Note { key: k as u8, vel, importance: slot.importance });
                }
            };
            let ri = spec.cell.right[..=q].iter().filter(|&&b| b == b'x').count();
            if spec.cell.right[q] == b'x' {
                let s = (ri - 1).min(2);
                if let Some(k) = realize(&spec.right[s], root, last_r[s], prev_r) {
                    last_r[s] = Some(k);
                    prev_r = Some(k);
                    emit(&mut c, k, &spec.right[s]);
                }
            } else if let Some(fill) = spec.fill {
                if let Some(k) = realize(&fill, root, last_fill, None) {
                    last_fill = Some(k);
                    emit(&mut c, k, &fill);
                }
            }
            let li = spec.cell.left[..=q].iter().filter(|&&b| b == b'x').count();
            if spec.cell.left[q] == b'x' {
                let s = (li - 1).min(2);
                if let Some(k) = realize(&spec.left[s], root, last_l[s], None) {
                    last_l[s] = Some(k);
                    emit(&mut c, k, &spec.left[s]);
                }
            }
        }
    }
    c
}

fn slot(anchor: i32, reach: Reach, importance: f32, vel: f32) -> Slot {
    Slot { anchor, reach, contour: Contour::Any, importance, vel }
}

fn shaped(mut s: Slot, contour: Contour) -> Slot {
    s.contour = contour;
    s
}

/// The basic kushaura: melodic right hand, bass-on-the-beat left hand.
fn kushaura_spec(rng: &mut Rng) -> PartSpec {
    let cell = if rng.chance(0.7) { CELLS[0] } else { CELLS[1] };
    let top = 16 + rng.below(3) as i32; // right-hand peak, degrees 16..18
    let dip = if rng.chance(0.75) { -2 } else { -3 }; // a third (or fourth) below
    let last = if rng.chance(0.6) { 0 } else { 1 };
    let bass = 2 + rng.below(3) as i32; // degrees 2..4
    let mid = 8 + rng.below(3) as i32; // degrees 8..10
    PartSpec {
        cell,
        right: [
            slot(top, Reach::Right, 1.0, 0.55),
            shaped(slot(top + dip, Reach::Right, 0.85, 0.5), Contour::Below),
            shaped(slot(top + last, Reach::Right, 0.9, 0.55), Contour::Above),
        ],
        left: [
            slot(bass, Reach::Bass, 0.95, 0.5),
            slot(mid, Reach::Upper, 0.6, 0.6),
            slot(mid + 3, Reach::Upper, 0.5, 0.55),
        ],
        fill: None,
        shift: 0,
    }
}

/// A bass-heavier answering part, one pulse behind.
fn complement_spec(base: &PartSpec, rng: &mut Rng) -> PartSpec {
    let mut s = *base;
    s.cell = CELLS[if rng.chance(0.5) { 1 } else { 2 }];
    for r in s.right.iter_mut() {
        r.anchor -= 2; // a third lower
    }
    s.left[0].anchor = 1 + rng.below(3) as i32;
    s.left[1] = slot(5 + rng.below(2) as i32, Reach::Bass, 0.7, 0.55);
    s.shift = 1;
    s
}

/// Variation (zvara) operators. Each keeps the chord cycle and most of the figure,
/// so variations sound like the same song played another way.
fn vary_spec(base: &PartSpec, rng: &mut Rng) -> PartSpec {
    vary_spec_n(base, 1 + rng.below(3), rng)
}

fn vary_spec_n(base: &PartSpec, ops: usize, rng: &mut Rng) -> PartSpec {
    let mut s = *base;
    for _ in 0..ops {
        match rng.below(8) {
            // New melodic shape around the same peak.
            0 => {
                let shape = SHAPES[rng.below(SHAPES.len())];
                let peak = base.right[0].anchor - SHAPES[0][0].0;
                for (slot, (off, contour)) in s.right.iter_mut().zip(shape) {
                    slot.anchor = peak + off;
                    slot.contour = contour;
                }
            }
            // Whole right-hand figure moves to another register.
            1 => {
                let d = [-2, 2, 3][rng.below(3)];
                for r in s.right.iter_mut() {
                    r.anchor += d;
                }
            }
            // Right hand plays a different rhythm.
            2 => s.cell.right = RIGHT_RHYTHMS[rng.below(RIGHT_RHYTHMS.len())],
            // Left hand plays a different rhythm.
            3 => s.cell.left = LEFT_RHYTHMS[rng.below(LEFT_RHYTHMS.len())],
            // Left-hand inner voices move.
            4 => {
                s.left[1].anchor += if rng.chance(0.5) { 2 } else { -2 };
                s.left[2].anchor += if rng.chance(0.5) { 2 } else { -1 };
            }
            // Bass takes the other chord tone (inversion / walking bass).
            5 => s.left[0].anchor += if rng.chance(0.5) { 2 } else { -2 },
            // High singing line on the last note of the figure.
            6 => s.right[2] = shaped(slot(19 + rng.below(2) as i32, Reach::Right, 0.8, 0.5), Contour::Above),
            // Busier texture: right thumb fills the rests (heard at higher intensity).
            _ => s.fill = Some(slot(9 + rng.below(3) as i32, Reach::Right, 0.3, 0.4)),
        }
    }
    s
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub roots: [i32; DYADS],
    pub kushaura: [Chara; VARIATIONS],
    pub kutsinhira: [Chara; VARIATIONS],
    /// The figures behind each variation, kept so the song can keep developing.
    specs: [PartSpec; VARIATIONS],
    cspecs: [PartSpec; VARIATIONS],
    mode: KutsinhiraMode,
    /// How many times the variation pool has been developed since generation.
    pub generation: u32,
}

impl Default for Piece {
    fn default() -> Self {
        let mut rng = Rng::new(0);
        let spec = kushaura_spec(&mut rng);
        Piece {
            roots: [0; DYADS],
            kushaura: [Chara::default(); VARIATIONS],
            kutsinhira: [Chara::default(); VARIATIONS],
            specs: [spec; VARIATIONS],
            cspecs: [spec; VARIATIONS],
            mode: KutsinhiraMode::Delay,
            generation: 0,
        }
    }
}

impl Piece {
    /// Generate in place (no allocation, no large stack temporaries).
    pub fn generate_into(&mut self, m: &MusicParams) {
        self.roots = cycle_roots(m.standard, m.start_degree);
        let roots = self.roots;
        let mut rng = Rng::stream(m.seed, 0xC0);
        let base = kushaura_spec(&mut rng);
        let comp = complement_spec(&base, &mut rng);
        let mut specs = [base; VARIATIONS];
        let mut cspecs = [comp; VARIATIONS];
        for v in 0..VARIATIONS {
            let mut vr = Rng::stream(m.seed, 0x200 + v as u64);
            // Variation 0 is the basic chara; later ones sometimes develop the previous one.
            let (first, second) = if v == 0 {
                (base, base)
            } else {
                let parent = if v > 1 && vr.chance(0.4) { specs[v - 1] } else { base };
                specs[v] = vary_spec(&parent, &mut vr);
                cspecs[v] = vary_spec(&cspecs[if v > 1 && vr.chance(0.4) { v - 1 } else { 0 }], &mut vr);
                // Half-cycle variations: only one half changes.
                match vr.below(4) {
                    0 => (base, specs[v]),
                    1 => (specs[v], base),
                    _ => (specs[v], specs[v]),
                }
            };
            self.kushaura[v] = render_part(&roots, &first, &second, &mut vr);
            self.kutsinhira[v] = match m.kutsinhira {
                KutsinhiraMode::Delay => self.kushaura[v].shifted(1),
                KutsinhiraMode::Complement => {
                    let mut cr = Rng::stream(m.seed, 0x400 + v as u64);
                    render_part(&roots, &cspecs[v], &cspecs[v], &mut cr)
                }
            };
        }
        self.specs = specs;
        self.cspecs = cspecs;
        self.mode = m.kutsinhira;
        self.generation = 0;
    }

    /// Develop variation `slot` into a new one, the way a player keeps finding new versions
    /// over a long performance: it grows out of an existing variation (often the one being
    /// played, sometimes the basic chara) with 1–3 operators, so it stays the same song.
    /// Allocation-free; safe to call from the audio thread.
    pub fn evolve(&mut self, slot: usize, from: usize, rng: &mut Rng) {
        let slot = slot.clamp(1, VARIATIONS - 1); // variation 0 stays the song's home
        let base = self.specs[0];
        let parent = self.specs[from % VARIATIONS];
        let mut spec = vary_spec_n(&parent, 2 + rng.below(2), rng);
        // Don't drift too far from home: occasionally pull a slot back toward the basic figure.
        if rng.chance(0.25) {
            spec.left = base.left;
        }
        // Fills come and go rather than accumulating.
        if parent.fill.is_some() && rng.chance(0.5) {
            spec.fill = None;
        }
        let mut cspec = vary_spec(&self.cspecs[from % VARIATIONS], rng);
        if self.cspecs[from % VARIATIONS].fill.is_some() && rng.chance(0.5) {
            cspec.fill = None;
        }
        let (first, second) = match rng.below(4) {
            0 => (base, spec),
            1 => (spec, base),
            _ => (spec, spec),
        };
        let mut kush = render_part(&self.roots, &first, &second, rng);
        // Keep the density of a real kushaura (~1 core note per pulse): if the developments
        // have piled up into something busier, fall back to the song's own rhythm.
        if core_notes(&kush) > 54 {
            spec.cell = base.cell;
            spec.fill = None;
            kush = render_part(&self.roots, &spec, &spec, rng);
        }
        self.specs[slot] = spec;
        self.cspecs[slot] = cspec;
        self.kushaura[slot] = kush;
        self.kutsinhira[slot] = match self.mode {
            KutsinhiraMode::Delay => self.kushaura[slot].shifted(1),
            KutsinhiraMode::Complement => render_part(&self.roots, &cspec, &cspec, rng),
        };
        self.generation += 1;
    }

    pub fn generate(m: &MusicParams) -> Box<Piece> {
        let mut p = Box::<Piece>::default();
        p.generate_into(m);
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letters(roots: [i32; DYADS]) -> String {
        roots.iter().map(|&r| crate::tuning::degree_letter(r)).collect()
    }

    #[test]
    fn standards_match_tracey() {
        assert_eq!(letters(cycle_roots(Standard::C, 0)), "CEGCEACFADFA");
        assert_eq!(letters(cycle_roots(Standard::E, 0)), "CEACFADFBDFA");
        assert_eq!(letters(cycle_roots(Standard::F, 0)), "CEACEGBDGBEG");
        assert_eq!(letters(cycle_roots(Standard::A, 0)), "CEACFACEGBEG");
    }

    #[test]
    fn cycle_closes_after_four_octaves() {
        let steps: i32 = INTERVALS.iter().map(|&i| if i == 3 { 2 } else { 3 }).sum();
        assert_eq!(steps, 28);
    }

    #[test]
    fn notes_mostly_in_dyad() {
        for seed in 0..20 {
            let m = MusicParams { seed, ..Default::default() };
            let piece = Piece::generate(&m);
            let mut total = 0;
            let mut outside = 0;
            for p in 0..PULSES {
                let root = dyad_at(&piece.roots, p as i32);
                let next = dyad_at(&piece.roots, p as i32 + 1);
                for n in piece.kushaura[0].notes[p].iter().flatten() {
                    total += 1;
                    let d = KEYS[n.key as usize].degree;
                    if !in_dyad(d, root) && !in_dyad(d, next) && !in_dyad(d, dyad_at(&piece.roots, p as i32 - 1)) {
                        outside += 1;
                    }
                }
            }
            assert!(total > 30, "seed {seed}: too few notes ({total})");
            assert!(outside * 10 <= total, "seed {seed}: {outside}/{total} notes outside the dyad");
        }
    }

    /// Real kushaura moves by common tones and steps: the same figure slot six
    /// pulses later should almost always be within a third.
    #[test]
    fn smooth_voice_leading() {
        for seed in 0..20 {
            let piece = Piece::generate(&MusicParams { seed, ..Default::default() });
            let c = &piece.kushaura[0];
            let (mut moves, mut smooth) = (0, 0);
            for p in 0..PULSES {
                for a in c.notes[p].iter().flatten() {
                    for b in c.notes[(p + 6) % PULSES].iter().flatten() {
                        let (ma, mb) = (KEYS[a.key as usize].manual, KEYS[b.key as usize].manual);
                        if ma.is_left() == mb.is_left() && ma == mb {
                            moves += 1;
                            if (KEYS[a.key as usize].degree - KEYS[b.key as usize].degree).abs() <= 2 {
                                smooth += 1;
                            }
                        }
                    }
                }
            }
            assert!(smooth * 10 >= moves * 7, "seed {seed}: only {smooth}/{moves} smooth moves");
        }
    }

    /// After many rounds of live development, every variation is still in harmony,
    /// still moves smoothly, still has a realistic density, and differs from the original.
    #[test]
    fn evolved_variations_stay_idiomatic() {
        for seed in 0..10 {
            let mut piece = Piece::generate(&MusicParams { seed, ..Default::default() });
            let original = piece.kushaura;
            let mut rng = Rng::new(seed);
            for _ in 0..200 {
                let slot = 1 + rng.below(VARIATIONS - 1);
                let from = rng.below(VARIATIONS);
                piece.evolve(slot, from, &mut rng);
            }
            let mut changed = 0;
            for v in 0..VARIATIONS {
                let c = &piece.kushaura[v];
                let (mut total, mut outside, mut moves, mut smooth) = (0, 0, 0, 0);
                for p in 0..PULSES {
                    let near = [-1, 0, 1].map(|o| dyad_at(&piece.roots, p as i32 + o));
                    for n in c.notes[p].iter().flatten() {
                        total += 1;
                        let d = KEYS[n.key as usize].degree;
                        if !near.iter().any(|&r| in_dyad(d, r)) {
                            outside += 1;
                        }
                        for b in c.notes[(p + 6) % PULSES].iter().flatten() {
                            if KEYS[n.key as usize].manual == KEYS[b.key as usize].manual {
                                moves += 1;
                                if (d - KEYS[b.key as usize].degree).abs() <= 2 {
                                    smooth += 1;
                                }
                            }
                        }
                    }
                }
                let core = core_notes(c);
                assert!(outside * 10 <= total, "seed {seed} v{v}: {outside}/{total} outside the dyad");
                assert!(smooth * 10 >= moves * 6, "seed {seed} v{v}: {smooth}/{moves} smooth");
                assert!((24..=60).contains(&core), "seed {seed} v{v}: {core} core notes");
                if *c != original[v] {
                    changed += 1;
                }
            }
            assert_eq!(piece.kushaura[0], original[0], "the basic chara must stay the song's home");
            assert!(changed >= VARIATIONS - 2, "seed {seed}: only {changed} variations developed");
        }
    }

    /// About one note per pulse (two hands interleaved), like the transcription.
    #[test]
    fn realistic_density() {
        for seed in 0..20 {
            let piece = Piece::generate(&MusicParams { seed, ..Default::default() });
            let core = piece.kushaura[0].notes.iter().flatten().flatten().filter(|n| n.importance >= 0.3).count();
            assert!((30..=48).contains(&core), "seed {seed}: {core} core notes per 48 pulses");
        }
    }

    #[test]
    fn delay_mode_is_shifted_kushaura() {
        let m = MusicParams { kutsinhira: KutsinhiraMode::Delay, ..Default::default() };
        let piece = Piece::generate(&m);
        for p in 0..PULSES {
            assert_eq!(piece.kutsinhira[0].notes[(p + 1) % PULSES], piece.kushaura[0].notes[p]);
        }
    }

    #[test]
    fn deterministic() {
        let m = MusicParams { seed: 42, ..Default::default() };
        assert_eq!(*Piece::generate(&m), *Piece::generate(&m));
    }
}
