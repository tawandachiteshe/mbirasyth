# 02 — Tunings and key layout

## 1. Tunings are relative, heptatonic, and per-instrument

- Mbira tunings describe **relative intervals**, not absolute pitch. Every instrument is
  pitched differently.
- Scales are **heptatonic** (7 notes per octave). Some older or regional ones are hexatonic.
- The tuning of each instrument is personal. Berliner documents two instruments of the "same"
  tuning whose intervals differ by **more than a semitone** at some points, and others that
  agree within a quarter-tone.
- Andrew Tracey: in the central Shona region there is "a tendency towards equi-spacing and
  intervals of between **200 and 300 cents**". This differs from the Western scale, where one step
  (semitone, 100 c) is half the size of the other (tone, 200 c).
- Octaves can be **stretched**, i.e. the seven-note pattern does not have to repeat at exactly 1200 cents.

## 2. Named tunings (Western approximations)

| Tuning | Western approximation | Notes |
|---|---|---|
| **Nyamaropa** | Mixolydian (major with ♭7) | The most common, "standard/regular" tuning |
| **Dambatsoko** | Ionian (major) | Named after the Mujuru family's village |
| **Mavembe / Gandanga** | Phrygian (minor with ♭2) | Some players' gandanga sounded closer to Lydian. Tracey says *gandanga* is the re-tuning closest to a Western major scale, so the names are used inconsistently |
| **Nemakonde** | Low-pitched Phrygian | |
| **Katsanzaira** | Dorian | High pitched, "very sweet" |
| **Saungweme** | ≈ 7-tone equal temperament (171.4 c steps) | "Non-Western"; a flattened whole-tone feel |
| **Dongonda** | Layout variant | Right-side keys an octave lower to match the left upper keys |

Treat these as **presets**, not as authoritative definitions: sources disagree, and real
instruments differ.

## 3. A parametric tuning model

Each scale degree *d* (0–6) gets a cent offset from the tonic:

```
mode_cents  = the Western mode (e.g. Mixolydian: 0 200 400 500 700 900 1000)
equal_cents = 7-TET:                            0 171 343 514 686 857 1029

cents[d]    = lerp(mode_cents[d], equal_cents[d], equalness)   // equalness ∈ [0,1]
            + preset_offsets[d]                                // hand-tuned colour
pitch(key)  = tonic_hz · 2^((cents[deg] + 1200·octave·stretch) / 1200)
            · 2^(detune_cents[key] / 1200)                     // per-key seeded jitter
```

Suggested defaults:

| Param | Range | Default | Effect |
|---|---|---|---|
| `tonic_hz` | 100–200 Hz for the lowest tonic | ~ G2/A2 | Overall register |
| `equalness` | 0 (Western) – 1 (7-TET) | **0.35** | How "Shona" vs "Western" it sounds |
| `stretch` | 0.99–1.02 | 1.003 | Octave stretch |
| `detune σ` | 0–20 c | 6 c | Per-key imperfection |
| `octave_pair_detune` | 0–8 c | 3 c | Beating between octave duplicates |

`equalness` is a good **world-building knob**. Villages/regions in the game can each have their own
tuning, and old or sacred instruments can be tuned closer to equal spacing. Tracey notes that the
B-mode is only usable when the fifth is reasonably in tune, so test fifths at extreme settings.

Because the harmony (doc 03) is defined in **scale degrees**, any tuning works with the
same composer. A tuning is just a table of 22–28 frequencies.

## 4. Key layout (mbira dzavadzimu)

```
          LEFT HAND (thumb)                     RIGHT HAND
   ┌──────────────────────────────┐   ┌───────────────────────────┐
   │ L  (left upper) — middle reg.│   │ R thumb (keys 1–3, down)  │
   │    ~7 keys                   │   │ R index (keys 4–10, up)   │
   │ B  (left lower) — bass reg.  │   │ ~9–10 keys, middle-high   │
   │    ~6–8 keys                 │   │                           │
   └──────────────────────────────┘   └───────────────────────────┘
        longest (lowest) keys toward the centre, rising outward
```

- 22–28 keys in three manuals. The left lower (B) is the bass, the left upper (L) the middle,
  and the right (R) the middle-to-high register.
- Low notes sit in the **centre**, with pitches rising toward both edges (a V-shaped array). Not all
  pitches are in strict order. Tracey notes that the "irregularities" are characteristic.
- Berliner: the **R manual holds a basic scale whose pitches have octave/unison counterparts in L
  and B**. This duplication explains both the sympathetic ringing and the beating between
  octave pairs.
- Some keys are placed next to each other for idiomatic leaps of a fifth or third, which shapes
  the patterns players fall into.
- The left thumb plays B and L. The right thumb strokes R1–R3 downward, and the right index strokes
  R4–R10 upward from underneath.
- Total range is a little over 3 octaves (≈ 117–948 Hz).

### Recommended data format

The exact key-to-pitch chart varies between instruments, and the sources found didn't agree on
one key-by-key table. So the layout is **data**, loaded from a file. Get a real instrument's chart
from Berliner (*The Soul of Mbira*) or a teacher's tablature, or measure an instrument.

```ron
// layouts/dzavadzimu_22.ron  (structure. Fill degrees from a reference instrument)
Layout(
    name: "mbira dzavadzimu 22",
    keys: [
        // id,   manual,   finger,       degree, octave
        Key("B1", Bass,    LeftThumb,    0,      0),
        Key("L1", Upper,   LeftThumb,    ...),
        Key("R1", Right,   RightThumb,   ...),
        Key("R4", Right,   RightIndex,   ...),
        // ...
    ],
)
```

The composer works on `(degree, octave)` and asks the layout which key and finger can play it.
The finger matters for (a) the exciter preset (thumb down vs index up) and (b) the rule that one
finger can only play one note per pulse (doc 05).

## 5. Other lamellophones to add later

| Instrument | Keys | Notes |
|---|---|---|
| *Nyunga nyunga* / karimba | 15 | Two ranks, hexatonic layout (degree 4 missing on the original). Developed at Kwanongoma College from Jege Tapera's karimba |
| *Matepe / hera* | ~26 | Uses the same 48-pulse system, played with 4 fingers |
| *Njari* | 30–32 | |
| *Mbira dzaVaNdau*, *timbila* | | Hexatonic, short cycles, low-to-high left-to-right layout |
