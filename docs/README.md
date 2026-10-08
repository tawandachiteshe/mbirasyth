# mbirasyth — research & design docs

A procedural mbira sound generator for games: it **synthesises** the sound of a Shona
mbira (no samples) and **composes** idiomatic, endlessly varying mbira music driven by
game state.

The work splits into two halves that meet at a note-event interface:

```
 game state ──► Composer (music system)  ──► note events ──► Synth (instrument model) ──► f32 audio
                 tuning · 48-pulse cycle        (key, time,      tines · buzz · gourd · hosho
                 kushaura / kutsinhira           velocity)
```

| Doc | What it covers |
|---|---|
| [01-instrument-acoustics.md](01-instrument-acoustics.md) | How a real mbira makes sound: tine physics, partials, decay, buzzers, gourd, sympathetic ringing, hosho |
| [02-tuning-and-layout.md](02-tuning-and-layout.md) | Named tunings, non-Western intervals, key layout, a parametric tuning model |
| [03-music-system.md](03-music-system.md) | Andrew Tracey's chord system, the 48-pulse cycle, kushaura/kutsinhira interlock, hosho rhythm |
| [04-synthesis-design.md](04-synthesis-design.md) | DSP architecture: modal tines, pluck exciter, buzz model, body filter, starting parameter tables |
| [05-procedural-composition.md](05-procedural-composition.md) | Algorithm that generates parts, variations, and responds to game intensity |
| [06-rust-architecture.md](06-rust-architecture.md) | Crate layout, real-time rules, APIs, engine integration (cpal / kira / Bevy), testing |
| [07-roadmap.md](07-roadmap.md) | Milestones from first tine to adaptive game music |
| [references.md](references.md) | Sources, with notes on how reliable each one is |

## Key findings, short version

1. **A tine is an inharmonic cantilever.** Measured mbira spectra show the strongest
   overtones at roughly **5× and 14×** the fundamental (not 2×, 3×…). These overtones are loud
   in the attack and die within a fraction of a second, leaving a near-pure sine.
   Modal synthesis (a few decaying resonators per tine) reproduces this cheaply.
2. **The buzz is the soul of the sound.** Bottle caps/shells (*machachara*) rattle against
   the soundboard and the gourd. It is a *nonlinear, threshold* effect, so loud playing buzzes
   much more than soft playing. A plain noise layer won't sound right, so we model it as
   amplitude-triggered stochastic impacts.
3. **Shona harmony runs on rules simple enough to code directly.** Most long mbira pieces use one
   48-pulse cycle of 12 two-note chords (root + fifth), moving only **up a third or up a fourth**:
   `C E G C E A C F A D F A` (interval pattern `334 343 434 333`). Pieces differ mainly in
   where you start in the cycle and in their rhythm and fingering patterns.
4. **The music emerges from interlocking parts.** *Kushaura* (lead) and *kutsinhira* (follow)
   are offset by about one pulse. Their combination produces "inherent patterns" that no single part
   plays. This is how one simple generator can produce music with a lot of variety.
5. **Tuning isn't 12-TET.** Traditional tunings lean toward equal spacing (200–300 cent steps)
   and vary per instrument. A tuning model that blends between a Western mode and 7-tone equal
   temperament, plus per-key detune, captures this, and players can hear the difference.

## Assumptions

- **Target instrument:** *mbira dzavadzimu* (*mbira huru*), 22–24 keys, Nyamaropa tuning,
  since it is the best-documented one. *Nyunga nyunga* (15 keys) and *matepe* use the same system
  and can be added later as layout presets.
- **Engine-agnostic core:** the generator is a pure Rust library that fills `f32` buffers.
  Adapters for specific engines are thin layers on top (see doc 06).

## A note on cultural context

*Mbira dzavadzimu* means "mbira of the ancestral spirits". The instrument and much of its
repertoire (e.g. *Nyamaropa*) belong to Shona religious practice (*bira* ceremonies). The docs
treat the harmonic system as a generative grammar rather than reproducing specific sacred pieces
note-for-note. Credit Shona musical tradition in the game, and consider consulting Zimbabwean
mbira players before shipping. Doing so is also the best way to validate the sound.
