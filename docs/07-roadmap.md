# 07 — Roadmap

Each milestone ends with something you can **hear** (a rendered WAV in `renders/`).

| # | Milestone | Deliverable | Done when |
|---|---|---|---|
| M0 | Scaffolding | Modules from doc 06, `hound` WAV render CLI | `cargo run -- render test.wav` writes a sine |
| M1 | **One tine** | Modal bank + pluck exciter + click | A single pluck has a bright, slightly clangy attack and a long pure tail. FFT test passes |
| M2 | Full instrument | Tuning model, 22-key layout, per-key jitter, pan, re-pluck damping | Scale and arpeggio renders sound like one consistent hand-made instrument |
| M3 | **Buzz + gourd** | Stochastic rattle ×2, body filters, sympathetic bus | Soft vs. loud renders: loud ones sizzle and soft ones are clean. Presets: dry / gourd / ceremonial |
| M4 | Harmony engine | 12-dyad cycle, standards, short cycles, unit tests | Block-chord render of the cycle sounds "mbira-like" even on plain sines |
| M5 | **Kushaura generator** | Skeleton cells, constraint+score voicing | 30 s of generated kushaura that sounds idiomatic |
| M6 | Interlock | Kutsinhira (delay + complement), resultant scoring, hosho | Two-part + hosho renders, with clear inherent patterns |
| M7 | Variation + conductor | Zvara pool, Markov playback, intensity layers, quantised changes | 5 min render with an intensity automation curve, no audible seams |
| M8 | Real time | `rtrb` commands, cpal live player, benchmarks, no-alloc tests | Live CLI with keyboard controls for intensity/pluck/stinger |
| M9 | Engine adapter | kira (or the chosen engine) integration + example scene | Mbira music in the game reacting to gameplay |
| M10 | Polish | Physical buzz (option B), nyunga nyunga/matepe layouts, NPC "player personalities", presets | – |

## Open questions to settle early

1. **Which game engine?** This decides the adapter (doc 06 §6) and whether audio runs in kira's
   graph or a raw callback.
2. **Reference recordings:** pick 3–5 recordings (one dry/contact-mic, one with gourd, one
   ensemble with hosho) and tune parameters against them.
3. **Exact key layout:** get the key-by-key chart of one real instrument (Berliner's *Soul of
   Mbira* or a teacher's chart) into `assets/layouts/dzavadzimu_22.ron`.
4. **Cultural review:** decide how the game will present and credit the music. Contact Zimbabwean
   mbira players for listening feedback.
