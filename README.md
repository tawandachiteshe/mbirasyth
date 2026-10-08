# mbirasyth

Procedural Shona mbira synthesizer and composer for games. No samples: the tines are synthesised as
modes, there's a threshold-driven bottle-cap buzz, a gourd resonator and hosho shakers. The
interlocking kushaura/kutsinhira parts are generated from Andrew Tracey's 48-pulse chord system.
Research and design are in [`docs/`](docs/README.md).

## Run

```sh
cargo run --release                 # GUI
cargo run --release -- --play       # GUI, start playing immediately
cargo run --release -- render out.wav 60 1234 0.8 -14   # headless: seconds, song seed, intensity, [target LUFS]
cargo run --release -- home --city Harare          # headless adaptive home music (see below)
cargo test --release --lib
```

Linux needs ALSA dev headers (`alsa-lib`) for audio output.

## GUI

- **Left panel, Instrument:** tuning preset (Nyamaropa, Dambatsoko, Mavembe, Katsanzaira, Saungweme),
  equal-spacing blend, detune, overtone ratios, brightness and sustain, buzz amount and threshold, gourd, reverb.
- **Right panel, Music:** song seed, chord cycle (C/E/F/A standard, short), start degree, kutsinhira
  mode, tempo, intensity (density, plus auto-added layers), humanize. **Band:** ngoma (hand drum: bass,
  tone, slap and mute strokes on the 12/8 beat, with fills before variation changes), hosho, and a
  synthesised bass guitar following the chord cycle (roots / on the beat / chimurenga / walking).
  **Export WAV** renders the current settings deterministically.
- **Bottom, Mastering:** presets (Gentle glue, Warm tape, Loud & punchy, Wide & airy, Game headroom),
  EQ (low cut, low shelf, mid bell, air), glue compressor, tape warmth, stereo width with mono bass,
  output gain and a look-ahead limiter ceiling. Live EBU R128 meters (momentary, short-term,
  integrated LUFS) and gain reduction. Export can normalise to a loudness target (streaming -14,
  game mobile -18, game console/PC -24, broadcast -23, or custom) in two passes.
- **Centre:** the live chord cycle and the mbira. Click keys, or play from the computer keyboard:
  `QWERTYUIOP` = right manual, `ASDFGHJ` = left upper, `ZXCVBNM` = bass. Shift = loud, Space = play/stop.
  Orange = kushaura, teal = kutsinhira. Keys in the current dyad are outlined.

## Home music

Chill music for the house that follows what's happening. Open **🏠 Home music** in the top bar and
tick *Let the house steer the music*, or run it headless on an always-on machine with
`mbirasyth home [--city NAME] [--listen ADDR] [--scene NAME]`.

- **Time of day:** slow, dark and quiet at night; bright at mid-morning; warm with bass and a
  soft ngoma in the evening. The mood glides; it never jumps.
- **Weather** (Open-Meteo, free, no account; fetched every 15 min): rain and thunder add
  synthesised rain and make the music slower and darker; wind adds wind; snow adds space.
- **Scenes:** Auto, Morning, Focus (steady, no band), Dinner, Unwind, Sleep, Gathering.
- **Events:** `arrive`, `leave` (fade out and stop), `motion`, `dinner`, `movie_start`/`movie_end`
  (pause/resume), `bedtime` (Sleep, then fade to silence over 30 min), `wake`, `guests`, `doorbell` (duck).
- A new song every 12 min by default, with a short fade.

HTTP API (default `127.0.0.1:7777`; tick *Allow other devices on my network* for `0.0.0.0:7777`):

```sh
curl http://localhost:7777/scene/dinner?minutes=90
curl http://localhost:7777/event/arrive
curl http://localhost:7777/volume/40
curl http://localhost:7777/status     # JSON
```

Home Assistant `configuration.yaml`:

```yaml
rest_command:
  mbira_event:
    url: "http://<mbira-host>:7777/event/{{ event }}"
  mbira_scene:
    url: "http://<mbira-host>:7777/scene/{{ scene }}"
```

Then call e.g. `rest_command.mbira_event` with `event: arrive` from an automation (door opens,
presence detected, TV starts playing). Settings are saved in `~/.config/mbirasyth/home.cfg`.

## Library use

```rust
use mbirasyth::engine::{Engine, Settings};
let mut engine = Engine::new(48_000.0, Settings::default());
engine.play();
engine.process(&mut stereo_buffer, 2); // real-time safe, allocation-free
```
