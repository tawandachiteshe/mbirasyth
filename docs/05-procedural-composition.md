# 05 — Procedural composition

This doc turns the grammar in [03-music-system.md](03-music-system.md) into an algorithm that
generates endless, idiomatic, game-reactive mbira music, deterministically from a seed.

## 1. Core types

```rust
type Degree = i32;                 // scale degree, absolute: degree + 7*octave

struct Cycle {                     // the 12-dyad harmonic cycle
    roots: [Degree; 12],           // dyad roots, e.g. C standard in C → [0,2,4,0,2,5,0,3,5,1,3,5]
    pulses_per_dyad: u8,           // 4  → 48 pulses
}

enum Hand { Left, Right }
enum Finger { LeftThumb, RightThumb, RightIndex }

struct Event { pulse: u16, key: KeyId, velocity: f32, finger: Finger }

struct Chara {                     // one 48-pulse "version"
    events: Vec<Event>,
    skeleton: [HandSlot; 48],      // Rest | L | R | Both, the rhythmic identity
}

struct Piece {                     // the generated "song"
    cycle: Cycle,
    kushaura: Vec<Chara>,          // basic chara + variations
    kutsinhira: Vec<Chara>,
}
```

## 2. Step 1: build the harmonic cycle

```
INTERVALS = [3,3,4, 3,4,3, 4,3,4, 3,3,3]      // in "chord names": 3rd = +2 steps, 4th = +3 steps
standard  = choose { C: rot 0, E: rot 1, A: rot 5, F: rot 7 }   // verified against Tracey's tables
start     = tonic degree (e.g. 0 for "C-mode", 4 for "G-mode")
roots[0]  = start
roots[i+1]= roots[i] + (INTERVALS[(i+rot) % 12] == 3 ? 2 : 3)
```

Unit tests: starting on C, rot 0/1/5/7 must reproduce Tracey's C/E/A/F standard tables in
doc 03 (e.g. C standard → `C E G C E A C F A D F A`), and the
sum of steps over 12 dyads must be 28 (4 octaves), so the cycle closes.

Dyad tones at pulse p: `{root, root+4}` (root + fifth) in any octave, where
`root = roots[(p / 4) % 12]`. "Third" tone (low probability): `root + 2`.

For short pieces/ambient moments: a 6-dyad short cycle (`C E G C F A`, or Tapera's
`G B D G B E`) with 24 pulses.

> **Implementation note (validated against a real transcription).** The shipped composer
> (`src/compose.rs`) uses a *figure-based* method instead of per-pulse scoring. The model is the
> kushaura of "Nhemamusasa" (Wikimedia Commons, *Mbira cross-rhythm.mid*). In every 6 pulses the right hand
> plays pulses 0,2,4 and the left 0,3,5 (`B . R L R L`). Each hand repeats a 3-note figure whose
> slots have fixed target pitches and contours. On each pulse a slot sounds the dyad tone nearest its
> target, so common tones hold and notes move by step when the chord changes. That's ~1 note
> per pulse at ~5 pulses/s. Per-pulse random scoring (below) sounded aimless and twice as dense.

## 3. Step 2: generate the rhythmic skeleton (per part)

The skeleton gives a part its identity and should stay fixed across variations.

- Work in 12-pulse quarters. Build a quarter from **cells** such as `L·R·`, `B·L·R·`, `LR`,
  `B··L`, where `·` = rest and `B` = both hands. Nyamaropa's "together–left–right–left–together"
  is one such cell shape.
- Constraints:
  - Hands **alternate** most of the time (hocket). Avoid more than 2 consecutive same-hand slots.
  - A "Both" slot every 3–6 pulses (it anchors the dyad).
  - Density target from game intensity: 0.4–0.9 of the pulses filled.
- Usually all 4 quarters use the **same** or near-same skeleton (the mbira's rhythmic repetition),
  with an optional variation in quarter 4.

## 4. Step 3: choose notes (constraint + scoring)

For each skeleton slot, for each hand active at that pulse:

```
candidates = keys reachable by that hand's finger(s)
           ∩ { keys whose degree mod 7 ∈ dyad_tones(pulse + harmonic_offset[hand]) }
           ∪ (with p_third ≈ 0.05) third-tone keys
score(key) =
     − w_leap   · |pitch(key) − last_pitch[hand]|           // smooth voice leading
     + w_reg    · register_preference(hand, key)            // left: bass/mid, right: mid/high
     − w_repeat · (key == last_key[hand] && repeat_count>1)
     + w_contour· continues_contour(hand, key)              // keeps lines going up/down
     − w_par4   · forms_parallel_fourth(prev_dyad_pair, key) // Shona avoids parallel 4ths
pick = softmax(score / temperature)  using the part's RNG
```

- `harmonic_offset[hand]` ∈ {−1, 0, +1} pulses: shifting a hand's chord change slightly makes
  the interlock and the harmony sound more "mbira-like" and less block-chordal. The left (bass) hand
  often moves at a different point than the right.
- One key per finger per pulse. Right thumb and right index may sound together. The left thumb plays
  one key, or rarely two adjacent keys at once (a "thumb slide").
- Velocity: base by hand (bass slightly louder), accent the first pulse of each beat
  (pulse % 3 == 0) by +1–2 dB, and add jitter.

## 5. Step 4: interlocking second part (kutsinhira)

Two modes, chosen by seed or intensity:

1. **Delay mode:** `kutsinhira = kushaura shifted by +1 pulse`. Trivial, and authentically used.
2. **Complement mode:** generate a new part on the same cycle with
   - skeleton = kushaura's skeleton shifted by 1 (or its complement: fill kushaura's rests),
   - `register_preference` biased to the **bass** (B manual), more octave/fifth leaps,
   - a check that the combined parts fill ≥ 85 % of pulses and that no pulse has > 4 simultaneous notes.

Optional **resultant-pattern** bonus: extract the top note at each pulse of the *combined* parts.
Reward candidate choices that make this composite line stepwise. That is the "inherent melody"
listeners latch onto.

## 6. Step 5: variations (zvara)

Generate a pool of 3–6 variations per part from the basic chara using operators that **keep the
cycle and skeleton**:

| Operator | Effect |
|---|---|
| Octave swap | Move a note to its octave duplicate on the other manual |
| Inversion | Root ↔ fifth |
| Fill / thin | Add a note on a rest, or remove one (density control) |
| High line | Right index plays an upper-octave melodic line in 1–2 quarters |
| Bass walk | Left thumb plays more bass-manual notes |
| Re-seed quarter | Regenerate notes of one quarter with a new sub-seed |

Playback is a Markov process over the pool: stay on a chara for 2–8 cycles (weighted by intensity),
then move to a neighbouring variation. Return to the basic chara now and then, since that's where
the tune's identity lives.

## 7. Game-facing control

| Control | Range | Mapping |
|---|---|---|
| `intensity` | 0–1 | 0.0: single sparse kushaura · 0.3: full kushaura · 0.5: + kutsinhira · 0.7: + hosho · 0.85: denser variations, high line, more buzz · 1.0: + second mbira pair |
| `mood` | enum | picks cycle standard (C/E/F/A), tuning preset, tempo range |
| `tempo` | 5–9 pulses/s | smoothed ramps only between quarters |
| `buzz` | 0–1 | buzz mix and threshold |
| `space` | enum | gourd/room preset (hut, open, cave) |
| `stinger(kind)` | event | one-shot pluck/run built from **current dyad tones**, so it's always in harmony |
| `pluck(key, vel)` | event | direct play (player-controlled in-game instrument) |

Rules for responsiveness:

- **Quantise structural changes to quarter boundaries** (every 12 pulses ≈ 1.5–2.5 s). Changes to
  layering can happen at the next beat (3 pulses).
- Fade layers in over a quarter instead of hard-cutting.
- Keep the cycle running across intensity changes. The music never "restarts", it just
  thickens or thins, which is true to the tradition (mbira performances fade in and out).

## 8. Determinism and seeds

- `world_seed → piece_seed → per-part RNGs` (e.g. `Pcg32::seed_from_u64(hash(piece_seed, part_id))`).
- **Each part, layer, and humaniser gets its own RNG stream**, so adding hosho or a kutsinhira
  never changes the kushaura's note choices.
- The same seed and the same control timeline give the same audio, bit for bit (on the same platform), which makes regression tests possible.

## 9. Possible extensions

- Learn cell libraries and scoring weights from transcriptions (Tracey 1970, Berliner,
  sympathetic-resonances.org notations) instead of hand-tuning.
- A "player personality": temperature, leap tolerance, buzz preference, and favourite variation
  operators per NPC musician.
- A spatialised ensemble: several engine instances placed in the game world, sharing one pulse clock.

## 10. Long-form development (implemented)

Listeners notice repetition fast, so the shipped engine measures it. `examples/repetition.rs` records
every 48-pulse cycle and asks how much of it (Jaccard overlap of note/drum/bass events) was already
heard in the most similar earlier cycle. Exact repeats are rare because of live ornaments, so the
useful number is the share of cycles that are ≥ 80 % like something already heard:

| 20 min render | before | after |
|---|---|---|
| default (solo kushaura) | 82 % | 25 % |
| full band | 29 % | 10 % |
| home director (60 min) | 69 % | 12 % |

Consecutive cycles still share ~50 % of their events, so it stays one recognisable song. What changed:

- **Evolving variation pool:** each cycle the player may develop a new variation out of the one being
  played (2–3 operators). Variation 0 never changes (the song's home). A density guard keeps it at
  about 1 core note per pulse.
- **Phrasing:** each 12-pulse phrase thins out differently, an octave-up "huro" line or a bass-octave
  move can last a whole phrase, and fullness drifts slowly (1/f) over minutes.
- **Arrangement sections:** full / drums out / groove / bass out / breakdown / solo mbira, 2–8
  cycles each. The drummer plays a fill before section changes.
- **Generated grooves:** each song gets a classic pattern plus generated grooves that follow the beat
  hierarchy and mutate one stroke at a time. Fills are generated too.
- **Home songs:** each new song draws its own chord standard, start degree (mode), kutsinhira style
  and a ±7 % tempo.
