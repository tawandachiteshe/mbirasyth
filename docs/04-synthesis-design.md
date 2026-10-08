# 04 — Synthesis design

The synthesis approach is physically informed **modal synthesis** for the tines, a **stochastic
rattle** for the buzzers, resonant filters for the gourd, and **PhISEM** for the hosho. The whole
thing is cheap enough to run dozens of instances in a game.

## 1. Signal flow

```
                       ┌──────────── sympathetic feedback (g_sym, 1-sample delay) ───────────┐
                       ▼                                                                      │
 note event ──► Exciter ──► Tine k: modal bank (3–4 resonators) ──┬──► key pan ──► dry mix ───┤
 (key, vel,      (impulse      × 22–28 tines                      │                           │
  finger)        + click)                                         └──► BOARD BUS ─────────────┘
                                                                         │
                                         ┌───────────────────────────────┼─────────────────────┐
                                         ▼                               ▼                     ▼
                              Board buzz (machachara)          Gourd body filter      Rim buzz (shells)
                              envelope→stochastic clicks        (Helmholtz + shell)    lower, looser
                                         │                               │                     │
                                         └──────────────► MIX ◄──────────┴─────────────────────┘
                                                           │
                                Hosho (PhISEM) ──────────► + ──► short room reverb (opt.) ──► limiter ──► out
```

## 2. Tine: modal resonator bank

Each tine has M = 3 (optionally 4) **decaying sinusoidal modes**. Use the **complex phasor** form,
which is unconditionally stable, has exact frequency, and accepts re-excitation while it rings:

```rust
// per mode, per sample
z = z * rot + excite;          // z, rot: Complex<f32>;  rot = r · e^{iω}
y += amp * z.im;
// ω = 2π f / fs,   r = exp(-6.9078 / (T60 · fs))   // −60 dB after T60 seconds
```

Every ~1024 samples, renormalise `rot` (or compute it in f64) to avoid drift on very long decays.
Skip modes with `f > 0.45·fs`. A voice goes **dormant** when all its |z| fall below −100 dB, which
makes CPU proportional to the number of ringing keys.

**Re-pluck damping:** a real finger touches a ringing key before plucking it. Before adding
the new excitation, multiply that key's `z` by `repluck_damp` (≈ 0.1–0.3). Without this,
repeated notes pile up unnaturally.

### Starting parameters

These are estimates derived from the acoustics in doc 01. **Tune them by ear and against
spectrograms of real recordings.**

| Mode | Freq ratio (base ± per-key jitter) | Initial amp (pressure) | T60 at ~120 Hz key | T60 at ~950 Hz key |
|---|---|---|---|---|
| 1 (fundamental) | 1.0 | 1.0 | 5–7 s | 1.2–2 s |
| 2 | **5.2 ± 0.35** | 0.35–0.7 × vel^0.7 | 0.3–0.5 s | 0.06–0.12 s |
| 3 | **14.0 ± 1.0** | 0.15–0.35 × vel^1.0 | 0.08–0.15 s | 0.02–0.05 s (often above Nyquist budget, drop) |
| 4 (opt.) | ~ 25–30 | 0.05 × vel^1.5 | 0.03 s | – |

- Interpolate T60 on a log-frequency axis between the low and high anchors.
- Jitter is **seeded per key** (deterministic per instrument seed), so every virtual instrument
  is a different, consistent "hand-made" mbira.
- Make the ratios a per-instrument preset, since mounting geometry changes them (Chapman). Kalimba
  preset: mode 2 ≈ 5.6–6.2.
- Add a tiny initial phase offset per mode so the summed attack isn't a perfect click.

## 3. Exciter (the pluck)

```
excite(mode m) = amp_m(vel) · e^{iφ_m}            // one-sample impulse into each mode
click          = noise · env(τ = 2–6 ms) → bandpass(2.5–6 kHz, Q≈1) · vel²
```

| Finger preset | Click centre | Click level | Overtone tilt |
|---|---|---|---|
| Thumb, downstroke (left B/L, right R1–R3) | 2.5 kHz | medium | neutral |
| Right index, upstroke (R4–R10) | 4 kHz | lower, sharper | +2 dB on mode 2 |
| Soft (fleshy) pluck (UI/ambient use) | 1.5 kHz | very low | −6 dB on modes 2–3 |

Velocity mapping: louder means **brighter** (more modes 2/3 and click) and **buzzier** (through
the threshold, §5), not just louder. This nonlinearity is what makes it sound like an instrument.

Humanisation: ±(3–8) ms timing jitter, ±2 dB velocity jitter, seeded.

## 4. Board bus and sympathetic resonance

- `board = Σ_k tine_k_output` (pre-pan, mono). This is the soundboard vibration proxy.
- **Sympathetic ringing:** feed `g_sym · board[n−1]` (g_sym ≈ 0.002–0.01, i.e. −54 to −40 dB) into
  the excitation of **every** tine's modes 1 and 2. The resonators pick out their own
  frequencies, so octave duplicates and near-coincident partials light up naturally. Keep total
  loop gain < 1. The 1-sample delay keeps it explicit and stable.
- Optional cheap alternative: only couple tines that are within ±20 cents of an octave/unison of each other.

## 5. Buzzers (machachara)

### Option A: stochastic rattle (recommended, ship this)

```
acc     = highpass(board, 80 Hz)                       // board "acceleration"
env     = envelope_follower(|acc|, attack 0.5 ms, release 20 ms)
drive   = max(0, env − threshold)                      // threshold ⇒ soft notes don't buzz
rate    = k_rate · drive        (clamp ≤ ~3000 impacts/s)
gate    = lerp(1, max(0, sign(acc)), phase_lock)       // caps hit near board peaks
each sample: if rng() < rate·gate/fs:
     impulse(amp = k_amp · drive · (0.4 + 0.6·rng()))  → cap resonator bank
cap resonator bank = 3–5 two-pole bandpasses
     bottle caps: ~2.2, 3.6, 5.1, 7.4 kHz, Q 8–20
     snail shells: ~0.9, 1.6, 2.8 kHz, Q 4–8
```

| Param | Meaning | Default |
|---|---|---|
| `threshold` | Board level below which caps stay seated | tune so pp notes barely buzz |
| `k_rate`, `k_amp` | Density, loudness | – |
| `phase_lock` 0–1 | 0 = pure noise-like sizzle, 1 = hits locked to note cycles (rougher, more "pitched") | 0.4 |
| `buzz_mix` | Player/game control. 0 = clean mbira, 1 = ceremonial | 0.6 |

Run **two instances**: board caps (bright, threshold low) and gourd-rim shells/caps (duller,
fed from the gourd filter output, slightly higher threshold, longer cap Q).

### Option B: physical rattle (research / "deluxe" mode)

N = 8–20 caps, each a mass bouncing on the board under gravity:

```
x_b  = board displacement (leaky-integrate board twice, scaled)
δ    = x_b − y_cap                                       // penetration if > 0
F    = δ > 0 ? k·δ^α + c·δ^α·δ̇ : 0                       // Hunt–Crossley contact, α≈1.5
y¨   = −g + F/m
```

Integrate with semi-implicit (symplectic) Euler at 2× oversampling, and feed contact forces into the cap
resonator bank. You get the threshold, chaos, and envelope-following for free, at about 10× the cost of
option A. If you go further, stable contact schemes are covered in the tanpura/sitar literature
(van Walstijn et al. DAFx 2016; Bridges & van Walstijn DAFx 2017; Issanchou et al. 2018).

## 6. Gourd (deze) body

Applied to the board bus, then mixed with the direct tine sound (the dry part represents keys
heard directly):

| Filter | Freq | Gain | Q | Represents |
|---|---|---|---|---|
| Peaking | 140–220 Hz | +6–9 dB | 2–4 | Helmholtz air mode |
| Peaking | 350–550 Hz | +3 dB | 3 | Shell/board mode |
| Peaking | 900–1300 Hz | +2 dB | 4 | Shell mode |
| Low-pass | 7–9 kHz | – | 0.7 | Gourd absorbs extreme highs |

Presets: **No gourd / contact mic** (dry, bright, almost no rim buzz). **Small gourd**
(Helmholtz ~250 Hz). **Big ceremonial gourd** (~150 Hz, more rim buzz, slight 30–60 ms early-reflection
reverb).

## 7. Hosho (PhISEM, Cook 1997)

```
on shake stroke: shake_energy += stroke_strength
every sample:
    shake_energy *= system_decay          // ≈ 0.999
    if rng() < n_seeds · shake_energy · p: sound_level += gain · shake_energy
    sound_level *= sound_decay            // ≈ 0.95
    out = resonator(noise · sound_level)  // 2-pole BP ~ 3–5 kHz, + gentle 800 Hz body peak
```

Use two hosho, panned slightly L/R, with strokes per doc 03 (3 per beat, accent on the first).
Gourd shakers have a softer, lower centre than metal shakers.

## 8. Output stage

- Stereo: pan keys by physical position (left manual left, right manual right), width 30–50 %.
  Keep buzz and hosho near the centre.
- Optional short room (Schroeder/FDN, 0.4–0.8 s) for "inside a hut" vs. dry "outdoors".
- **Soft limiter** at the end. The modal sum plus a buzz burst can peak hard.
- Denormals: enable flush-to-zero on x86 (`_MM_SET_FLUSH_ZERO_MODE`) or add ±1e-20 noise. The
  dormant-voice check also limits the problem.

## 9. CPU budget (rough)

| Block | Cost per sample |
|---|---|
| 28 tines × 4 modes, complex phasor | ~ 112 × 6 flops ≈ 700 |
| Exciter clicks (active only) | small |
| 2 buzz instances (5 BP each) | ~ 60 |
| Gourd 4 biquads | ~ 40 |
| Hosho ×2 | ~ 30 |
| **Total** | **~ 1 k flops ≈ 50 MFLOP/s at 48 kHz.** Well under 1 % of one core, and less with dormant voices |

That leaves room for several instruments (an ensemble, or spatialised mbiras in the world).

## 10. Alternatives considered

| Approach | Verdict |
|---|---|
| **Sampling** | Realistic, but static: no per-instrument tuning, buzz interaction, or infinite variation, and needs licensing/recording. Use real samples as a *reference* for tuning parameters |
| **Modal synthesis** (chosen) | Physically meaningful parameters, cheap, decays are exact, works well with seeded variation |
| **Banded waveguides** (Essl & Cook) | Good for bars/bowls. Gives no real benefit over modal for 3–4 modes |
| **FDTD beam simulation** | Most accurate, far too costly for a game, and hard to tune to exact pitches |
| **FM** | Can fake inharmonic bells (1:5 / 1:14 operator ratios), but the decays and buzz are less controllable. Fine for a retro/lo-fi preset |
