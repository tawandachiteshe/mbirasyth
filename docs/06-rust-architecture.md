# 06 — Rust architecture

## 1. Goals

- A **pure library core**: no audio backend, no engine dependency. It only fills `&mut [f32]`.
- **Real-time safe** `process()`: no allocation, locks, I/O, or panics on the audio thread.
- **Deterministic**: seed in, identical samples out.
- **Thin adapters** for cpal (standalone/tools), kira, Bevy, and others.

## 2. Crate layout

Start as one crate with modules. Split into a workspace when adapters appear.

```
mbirasyth/
├── Cargo.toml
├── src/
│   ├── lib.rs              // pub use engine::*, config types
│   ├── main.rs             // CLI: render demo WAVs, live play via cpal (feature "cli")
│   ├── dsp/
│   │   ├── modal.rs        // complex-phasor resonator, ModeBank
│   │   ├── filters.rs      // biquad (peaking/BP/LP), one-pole, envelope follower
│   │   ├── noise.rs        // fast RNG noise (no rand on hot path, or rand_pcg)
│   │   └── limiter.rs
│   ├── instrument/
│   │   ├── tine.rs         // Tine = ModeBank + exciter params + dormancy
│   │   ├── exciter.rs      // pluck impulse + click burst, finger presets
│   │   ├── buzz.rs         // stochastic rattle (option A), later physical (option B)
│   │   ├── body.rs         // gourd filters + rim buzz
│   │   ├── hosho.rs        // PhISEM shaker
│   │   └── mbira.rs        // Mbira = tines + board bus + sympathetic + body + pan
│   ├── tuning/
│   │   ├── tuning.rs       // parametric tuning model (doc 02)
│   │   └── layout.rs       // key layout data (RON), KeyId, Manual, Finger
│   ├── compose/
│   │   ├── cycle.rs        // 12-dyad cycle, standards, short cycles
│   │   ├── skeleton.rs     // rhythmic cells
│   │   ├── voicing.rs      // constraint+score note choice
│   │   ├── variation.rs    // zvara operators + Markov playback
│   │   └── conductor.rs    // pulse clock, intensity → layers, quantised changes
│   └── engine.rs           // MbiraEngine: owns everything, process(), command queue
├── assets/layouts/*.ron, assets/tunings/*.ron
├── benches/                // criterion: process() cost per block
└── docs/
```

## 3. Dependencies

| Crate | Version (Oct 2026) | Use | Where |
|---|---|---|---|
| `rand_pcg` + `rand_core` | 0.10 | Seeded deterministic RNG | core |
| `rtrb` | 0.4 | Lock-free SPSC queue (game thread → audio thread) | core |
| `serde` + `ron` | 1 / 0.12 | Layout/tuning/preset files | core (feature `serde`) |
| `cpal` | 0.18 | Audio output for CLI/tools | feature `cli` |
| `hound` | 3.5 | Offline WAV render | feature `cli`, tests |
| `rustfft` | 6 | Spectral tests (overtone ratios, no aliasing) | dev |
| `criterion` | 0.8 | Benchmarks | dev |
| `kira` | 0.12 | Game audio integration | feature `kira` |

Avoid `fundsp` in the core. It is great for prototyping graphs, but hand-written resonators give
exact control and zero overhead. It's still worth using in a scratch prototype to tune parameters fast.

## 4. Public API sketch

```rust
pub struct EngineConfig {
    pub sample_rate: f32,
    pub seed: u64,
    pub layout: Layout,          // from RON or built-in preset
    pub tuning: TuningParams,    // tonic_hz, equalness, stretch, detune...
    pub timbre: TimbreParams,    // mode ratios/T60s, buzz, body preset
}

pub struct MbiraEngine { /* tines, buzz, body, hosho, conductor, cmd_rx, ... */ }

impl MbiraEngine {
    pub fn new(cfg: EngineConfig) -> (Self, EngineHandle);
    /// Interleaved stereo. Real-time safe.
    pub fn process(&mut self, out: &mut [f32]);
}

/// Cheap, Send, owned by the game thread.
pub struct EngineHandle { tx: rtrb::Producer<Command> }

pub enum Command {
    Play { piece_seed: u64, mood: Mood },
    Stop { fade_quarters: u8 },
    SetIntensity(f32),
    SetTempo { pulses_per_sec: f32 },
    SetBuzz(f32),
    SetSpace(Space),
    Pluck { key: KeyId, velocity: f32 },     // player-driven instrument
    Stinger(StingerKind),                    // harmonised one-shot
    Retune(TuningParams),                    // applied at next quarter
}
```

Inside `process`, the conductor advances a **sample-accurate pulse clock**, so composition runs
*inside* the audio callback. It's cheap (a few decisions per pulse), it removes any need for timing sync,
and it guarantees sample-exact interlock between parts. All buffers (event lists per chara,
variation pool) are **pre-allocated** when a piece starts. A new `Play` builds the piece on the
game thread and sends it in a `Box` through the queue. Drop the old one on the game thread too:
send it back through a second ring buffer so the audio thread never frees memory.

## 5. Real-time checklist

- [ ] No `Vec::push`/`Box::new`/`String` in `process`. Debug with `assert_no_alloc` in tests.
- [ ] No `Mutex`. Parameters go via the `rtrb` queue, continuous values smoothed per sample.
- [ ] Flush denormals (FTZ/DAZ) at the top of `process` on x86/ARM.
- [ ] `f32` DSP. Resonator rotation renormalised or computed in `f64` at retune time.
- [ ] Block size agnostic (works for 32–4096 frames).
- [ ] Dormant voices skipped. Benchmark worst case: all 28 keys ringing, full buzz, hosho.

## 6. Engine integration

| Target | How |
|---|---|
| **Standalone / tools** | `cpal` output stream; callback calls `engine.process(buf)` |
| **kira 0.12** | Implement kira's custom `Sound`/`SoundData` traits around `MbiraEngine` (check the exact trait signatures for the pinned version). Kira then handles mixing, spatial tracks, and effects |
| **Bevy** | Via `bevy_kira_audio`, or Bevy's own custom audio source (`Decodable` impl producing samples) |
| **Godot (gdext)** | `AudioStreamGenerator` with a push loop, or a custom `AudioStreamPlayback` |
| **Unity / Unreal / Web** | C ABI (`cdylib` + `#[no_mangle] extern "C"`), or `wasm32` + AudioWorklet for web builds |

## 7. Testing strategy

| Test | What it checks |
|---|---|
| `cycle_standards` | C/E/A/F standards reproduce Tracey's tables; cycle closes after 28 steps |
| `voicing_rules` | All sounding notes ∈ dyad tones (except ≤ p_third); ≤ 1 key per finger per pulse; no parallel 4ths |
| `kutsinhira_offset` | Delay mode equals kushaura shifted by one pulse |
| `determinism` | Same seed + command script ⇒ identical buffer hash |
| `layer_independence` | Toggling hosho/kutsinhira doesn't change kushaura events |
| `pluck_spectrum` | FFT of one pluck: peaks at f₁ and ≈ 5.2 f₁ / 14 f₁ (± jitter); partial 2 decays > 10× faster than f₁ |
| `buzz_threshold` | Buzz RMS ≈ 0 for vel 0.2, clearly > 0 for vel 0.9 |
| `no_nan_no_clip` | 10 min random command fuzzing: finite output, peak ≤ 1.0 after the limiter |
| Bench | `process(512 frames)` worst case < 2 % of the block's real-time duration |

Listening tests count for more than any of these: render A/B WAVs against real recordings
(Hugh Tracey/ILAM, contemporary players) and ask mbira players.
