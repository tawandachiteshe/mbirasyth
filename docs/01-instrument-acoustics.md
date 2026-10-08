# 01 — How an mbira makes sound

This doc describes the physical sound sources we need to model. Each one maps to a DSP block in
[04-synthesis-design.md](04-synthesis-design.md).

```
 thumb/finger nail ──► tine (lamella) ──► bridge/bar ──► soundboard (gwariva) ──► air
                          │                                   │
                          │ sympathetic coupling ◄────────────┤
                          │ to other tines                    ├──► machachara (buzzers on board)
                          ▼                                   ▼
                                                    gourd resonator (deze) + rim buzzers ──► air
```

## 1. The instrument

| Part | Description | Sound role |
|---|---|---|
| **Keys / tines (lamellae)** | 22–28 forged steel keys on *mbira dzavadzimu*. Hot or cold forged, so geometry varies key to key | The pitched tones |
| **Bridge + bar** | A metal bridge the keys pass over and a bar that clamps them down. Each key is clamped, supported, and then free | Sets boundary conditions → overtone ratios |
| **Soundboard (*gwariva*)** | Hardwood board. Keys are in three manuals: left-lower (bass), left-upper (middle), right (middle–high) | Couples tines together; carries the buzzers |
| **Buzzers (*machachara*)** | Bottle caps or snail shells loosely threaded on a wire/plate on the board, and often on the gourd rim too | The sizzle; culturally essential |
| **Gourd (*deze*)** | Large calabash resonator the mbira is played inside | Amplifies, colours (low-mid resonances), adds more rattles |
| **Hosho** | Pair of gourd shakers with seeds, played alongside | Rhythmic timeline (see doc 03) |

Fundamental range of mbira keys: **about 117 Hz to 948 Hz (≈ B2–B5)**, i.e. a little over three
octaves of a seven-note scale.

## 2. Tine physics: an inharmonic beam

A tine is a thin rectangular steel beam. The Euler–Bernoulli beam theory gives its bending
frequencies:

```
f_n = (β_n L)² / (2π L²) · sqrt(E·I / (ρ·A))
```

For a rectangular cross-section, `I/A = t²/12`, so for the fundamental of an ideal
clamped–free cantilever:

```
f_1 ≈ 0.1615 · (t / L²) · sqrt(E/ρ)        // sqrt(E/ρ) ≈ 5050 m/s for steel
```

- Pitch is **proportional to thickness** and **inversely proportional to the square of the free length**.
  Width doesn't matter, which is why makers tune by sliding keys and hammering/filing the tip.
  - Check: t = 1.5 mm, L = 100 mm → f₁ ≈ 122 Hz. That is about the lowest mbira key, so the formula
  gives sensible values.

### Overtone ratios

| Boundary condition | f₂/f₁ | f₃/f₁ | f₄/f₁ |
|---|---|---|---|
| Ideal clamped–free cantilever (textbook) | 6.27 | 17.55 | 34.39 |
| Kalimba keys, clamped–supported–free (Chapman 2008, measured and modelled) | **5.3–5.9** (5.9–6.8 in a popular summary) | – | – |
| Mbira (measured spectrum, FEM-confirmed) | **≈ 5** | **≈ 14** | – |

Takeaways:

- The overtones are **far from harmonic**. Pairs like 1 : 5.x : 14 give the slightly
  "discordant edge" heard in the attack.
- The ratios **depend on mounting geometry** (where the bridge sits, how long the overhang is,
  the tip shape). So each key on a real instrument has slightly different ratios, and we should
  jitter them per key.
- The FEM study found that the far-field sound is dominated by **transverse (bending) modes**.
  Longitudinal and torsional modes radiate poorly, so we can ignore them. (Torsional modes may add
  faint high partials if a key is plucked off-centre. That's optional detail.)
- The same study argues that expert tunings are **not** chosen by aligning upper keys with lower
  keys' overtones, because the overtones are too high. Tuning is a cultural choice, not acoustic
  matching (see doc 02).

### Why the attack is bright but the tail is pure

The nail pushes the tip down and releases it. A static tip deflection projects onto mode *n*
with *displacement* ∝ 1/ω_n², but radiated pressure tracks *acceleration* ∝ ω_n² × displacement.
So at the instant of release, **overtones can be nearly as loud as the fundamental in the
air**. Damping rises steeply with frequency (internal friction, clamp losses, radiation), so
the 5× and 14× partials die within roughly 0.05–0.4 s while the fundamental rings for seconds. Result:

> "a sharp attack with a slight discordant edge, followed by a slowly decaying pure tone"
> (Chapman's description of the kalimba)

This is the most important timbral feature for the model to get right.

## 3. Attack transients

Besides the modes, the pluck makes non-modal noise:

- **Nail click/scrape:** a 2–10 ms broadband burst as the nail slides off the tip. Brighter
  for harder plucks.
- **Key rattle on the bridge:** slightly loose keys buzz against the bridge or bar. This is often
  deliberate and adds to the sizzle.
- **Right index finger upstrokes** (right-hand keys 4–10 are played from underneath with the index
  finger) give a different attack colour than thumb downstrokes. Use a separate exciter preset.

## 4. Buzzers (machachara)

The buzz is **not optional**. Traditionally it is believed to attract the ancestral spirits, and
it's what makes an mbira sound like an mbira rather than a music box. Scherzinger compares it to
a snare: it "veils" the clean attack.

Mechanism (a *rattle*, a contact nonlinearity):

1. Plucking a tine vibrates the soundboard (and the gourd, through air and contact).
2. Loose caps or shells rest on the board or a wire under gravity. When the board's acceleration
   exceeds about 1 g, a cap lifts off, flies briefly, and **impacts** again.
3. Each impact is a short broadband click shaped by the cap's own ringing modes (thin metal
   bottle caps ring high, around 2–8 kHz; shells are duller).
4. Many caps → a dense, stochastic stream of clicks whose **density and loudness follow the
   board's vibration envelope**, with a threshold.

Behaviour the model must reproduce:

- **Threshold:** soft notes barely buzz, loud notes sizzle. So the buzz is velocity-sensitive in a
  nonlinear way.
- **Envelope-following:** buzz decays with the notes but cuts out (stops being re-triggered)
  once vibration drops under threshold. The sizzle ends before the note does.
- **Low notes buzz more:** they carry more board displacement.
- **Not periodic:** caps bounce chaotically. A pitched noise layer locked to note frequency
  sounds wrong.

Commercial sample libraries record the sizzle separately so users can mix it and set its
decay. That confirms it works as a separable layer, which is what we do.

Closest research on this kind of model: string–obstacle contact for sitar *jawari* and tanpura bridges
(van Walstijn et al., Issanchou et al., DAFx 2016–17). Those are heavier than we need, but they
show stable ways to simulate contact if we ever want a physical cap model (doc 04, option B).

## 5. Gourd resonator (deze)

- A large calabash with an opening. It acts as a **Helmholtz resonator** plus shell modes:
  `f_H = (c / 2π) · sqrt(A / (V · L_eff))`.
  Example: V = 5 L, a 5 cm opening, L_eff ≈ 4.75 cm → **f_H ≈ 157 Hz**. That's in the range of
  the bass keys, so the gourd **reinforces the low register**.
- Wooden kalimba/mbira boxes have reported resonances around 440–520 Hz. That figure comes from a
  student wiki and is only a ballpark.
- Gourd *rim* buzzers add a second, lower, looser rattle layer.
- Modern players often skip the gourd and use a contact mic. That gives a dry, close, less boomy
  sound. Expose it as a preset.

## 6. Sympathetic resonance

When one key is plucked, **neighbouring keys ring faintly** through the soundboard and bridge.
Octave duplicates in particular reinforce each other (doc 02: the manuals share pitches across
octaves). This adds a halo and slight beating. It's cheap in a modal model: feed a little of
the board signal back into every tine's resonators.

## 7. Hosho (shakers)

A pair of gourd shakers with hard seeds inside. Acoustically it is many small impacts. The
standard model is **PhISEM** (Cook 1997, *Physically Informed Stochastic Event Modelling*): a
decaying "shake energy" drives a Poisson stream of seed collisions, filtered by gourd resonances.
Rhythm is in doc 03.

## 8. Summary: what to model, and how much it matters

| Feature | Perceptual importance | Model |
|---|---|---|
| Inharmonic fast-decaying overtones (≈5×, ≈14×) | **Critical** | Modal resonators |
| Long, near-sinusoidal fundamental | **Critical** | Modal resonator, low damping |
| Buzz (threshold, envelope-following) | **Critical** | Stochastic rattle driven by envelope |
| Nail click | High | Filtered noise burst |
| Per-key variation (ratios, decay, detune) | High (avoids "synth" feel) | Seeded jitter |
| Gourd body colour | Medium | 2–4 resonant filters |
| Sympathetic ringing | Medium | Board bus → all tines |
| Hosho | Medium (for full ensemble) | PhISEM |
| Room | Low–medium | Short reverb, optional |
