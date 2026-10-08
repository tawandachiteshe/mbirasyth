//! egui front-end: shape the instrument, generate music, play it live, export WAV.

use crate::audio::{self, AudioOut, Link};
use crate::band::{BassStyle, DrumStyle};
use crate::compose::{cycle_roots, KutsinhiraMode, Standard, DYADS, PULSES};
use crate::engine::{Command, Section, Settings, UiEvent};
use crate::instrument::BodyPreset;
use crate::home::{self, HomeConfig, HomeHandle, HomeMsg};
use crate::mood::{HomeEvent, Scene};
use crate::master::{LoudnessReading, MasterParams, MasterPreset};
use crate::render::{render_wav, LoudnessTarget};
use crate::rng::Rng;
use crate::tuning::{degree_letter, hz_to_note_name, Manual, TuningPreset, KEYS, NUM_KEYS};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const ORANGE: Color32 = Color32::from_rgb(240, 150, 50);
const TEAL: Color32 = Color32::from_rgb(60, 200, 190);
const STEEL: Color32 = Color32::from_rgb(150, 155, 165);
const WOOD: Color32 = Color32::from_rgb(92, 60, 38);
const WOOD_DARK: Color32 = Color32::from_rgb(60, 38, 24);

struct Export {
    seconds: f32,
    fade: f32,
    path: String,
    progress: Arc<AtomicU32>,
    result: Arc<Mutex<Option<Result<String, String>>>>,
    running: bool,
    target: LoudnessTarget,
}

/// The "home music" side of the app: the director thread and its settings.
struct HomeUi {
    handle: Option<HomeHandle>,
    on: bool,
    show: bool,
    config: HomeConfig,
    city_input: String,
    geocode: Arc<Mutex<Option<Result<(String, f64, f64), String>>>>,
    geocoding: bool,
    /// The user's own instrument/mix settings; the director steers the rest.
    base: Settings,
    mirrored: Option<Settings>,
    volume: f32,
    weather_affects_mood: bool,
    status: Option<home::HomeStatus>,
}

pub struct MbiraApp {
    settings: Settings,
    sent: Option<Settings>,
    audio: Option<AudioOut>,
    link: Option<Link>,
    audio_error: Option<String>,
    playing: bool,
    start: Instant,
    /// Last hit time (seconds since start) per instrument and key.
    flash: [[f64; NUM_KEYS]; 2],
    pulse: usize,
    variation: [u8; 2],
    section: Section,
    /// How many new variations the player has developed in this song.
    generation: u32,
    level: f32,
    loudness: LoudnessReading,
    /// Gain reduction (dB) of the mastering chain, peak-held for display.
    gr_db: f32,
    manual_vel: f32,
    export: Export,
    home: HomeUi,
}

fn random_seed() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    Rng::new(t).next_u64() % 1_000_000
}

impl MbiraApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, autoplay: bool) -> Self {
        let settings = Settings::default();
        let (audio, link, audio_error) = match audio::start(settings) {
            Ok((a, l)) => (Some(a), Some(l), None),
            Err(e) => (None, None, Some(e)),
        };
        let mut app = MbiraApp {
            settings,
            sent: None,
            audio,
            link,
            audio_error,
            playing: false,
            start: Instant::now(),
            flash: [[-10.0; NUM_KEYS]; 2],
            pulse: 0,
            variation: [0, 0],
            section: Section::Full,
            generation: 0,
            level: 0.0,
            loudness: LoudnessReading::default(),
            gr_db: 0.0,
            manual_vel: 0.8,
            export: Export {
                seconds: 60.0,
                fade: 4.0,
                path: "mbira.wav".into(),
                progress: Arc::new(AtomicU32::new(0)),
                result: Arc::new(Mutex::new(None)),
                running: false,
                target: LoudnessTarget::Off,
            },
            home: {
                let config = HomeConfig::load();
                HomeUi {
                    handle: None,
                    on: false,
                    show: false,
                    city_input: config.city.clone(),
                    config,
                    geocode: Arc::new(Mutex::new(None)),
                    geocoding: false,
                    base: settings,
                    mirrored: None,
                    volume: 1.0,
                    weather_affects_mood: true,
                    status: None,
                }
            },
        };
        if autoplay {
            app.toggle_play();
        }
        app
    }

    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn send(&mut self, cmd: Command) {
        if let Some(link) = self.link.as_mut() {
            if let Ok(mut tx) = link.tx.lock() {
                let _ = tx.push(cmd);
            }
        }
    }

    fn sync(&mut self) {
        self.sync_home();
        if self.sent != Some(self.settings) {
            self.sent = Some(self.settings);
            let s = self.settings;
            self.send(Command::Settings(s));
        }
        let now = self.now();
        if let Some(link) = self.link.as_mut() {
            while let Ok(ev) = link.rx.pop() {
                match ev {
                    UiEvent::Key { inst, key, .. } => self.flash[inst as usize][key as usize] = now,
                    UiEvent::Pulse { pulse, variation, section, generation } => {
                        self.section = section;
                        self.generation = generation;
                        self.pulse = pulse as usize;
                        self.variation = variation;
                    }
                    UiEvent::Level(l) => self.level = self.level.max(l),
                    UiEvent::Loudness(r, gr) => {
                        self.loudness = r;
                        self.gr_db = self.gr_db.max(gr);
                    }
                }
            }
        }
        self.level *= 0.9;
        self.gr_db *= 0.93;
    }

    fn pluck(&mut self, key: usize, vel: f32) {
        self.send(Command::Pluck { key: key as u8, vel });
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            if let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = ev {
                if key == Key::Space {
                    self.toggle_play();
                    continue;
                }
                for (k, def) in KEYS.iter().enumerate() {
                    if Key::from_name(&def.shortcut.to_string()) == Some(key) {
                        let vel = if modifiers.shift { 1.0 } else { self.manual_vel };
                        self.pluck(k, vel);
                    }
                }
            }
        }
    }

    fn toggle_play(&mut self) {
        if self.home.on {
            let paused = self.home.status.as_ref().map(|s| s.paused).unwrap_or(false);
            self.home_send(HomeMsg::Pause(!paused));
            return;
        }
        self.playing = !self.playing;
        let cmd = if self.playing { Command::Play } else { Command::Stop };
        self.send(cmd);
    }

    // ------------------------------------------------------------------ panels

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("🎶 mbirasyth");
            ui.separator();
            let label = if self.playing { "⏹ Stop" } else { "▶ Play" };
            if ui.add(egui::Button::new(egui::RichText::new(label).size(18.0)).min_size(Vec2::new(90.0, 30.0))).clicked() {
                self.toggle_play();
            }
            if ui.button("🎲 New song").on_hover_text("New composition seed").clicked() {
                self.settings.music.seed = random_seed();
            }
            if ui.button("🎲 New instrument").on_hover_text("New per-key detune/overtone jitter").clicked() {
                self.settings.instrument_seed = random_seed();
            }
            if ui.button("🎲 Surprise me").on_hover_text("Randomise song, instrument, tuning and feel").clicked() {
                self.surprise();
            }
            ui.separator();
            let label = if self.home.on { "🏠 Home: on" } else { "🏠 Home music" };
            let b = egui::Button::new(label).selected(self.home.on || self.home.show);
            if ui.add(b).on_hover_text("Chill music that follows the time of day, weather and your smart home").clicked() {
                self.home.show = !self.home.show;
            }
            ui.separator();
            ui.label("Level");
            let lvl = self.level.min(1.0);
            let color = if lvl > 0.85 { Color32::RED } else { Color32::from_rgb(90, 200, 90) };
            ui.add(egui::ProgressBar::new(lvl).desired_width(120.0).fill(color));
            ui.separator();
            match (&self.audio, &self.audio_error) {
                (Some(a), _) => ui.weak(format!("🔊 {} @ {} Hz", a.device_name, a.sample_rate)),
                (None, Some(e)) => ui.colored_label(Color32::RED, format!("No audio: {e} (export still works)")),
                _ => ui.label(""),
            };
        });
    }

    fn surprise(&mut self) {
        let mut r = Rng::new(random_seed());
        let s = &mut self.settings;
        s.music.seed = r.next_u64() % 1_000_000;
        s.instrument_seed = r.next_u64() % 1_000_000;
        s.music.standard = Standard::ALL[r.below(Standard::ALL.len())];
        s.music.start_degree = r.below(7) as i32;
        s.music.kutsinhira = KutsinhiraMode::ALL[r.below(2)];
        s.tuning.preset = TuningPreset::ALL[r.below(TuningPreset::ALL.len())];
        s.tuning.equalness = (s.tuning.preset.default_equalness() + r.range(-0.2, 0.2)).clamp(0.0, 1.0);
        s.tuning.tonic_hz = r.range(95.0, 135.0);
        s.timbre.body = BodyPreset::ALL[r.below(3)];
        s.timbre.buzz = r.range(0.0, 0.4);
        s.timbre.ratio2 = r.range(4.9, 6.0);
        s.tempo = r.range(4.5, 6.5);
        s.variation = r.range(0.3, 0.8);
    }

    fn instrument_panel(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        ui.heading("Instrument");
        ui.add_space(4.0);
        ui.label("Sound preset");
        ui.horizontal_wrapped(|ui| {
            for preset in SoundPreset::ALL {
                if ui.button(preset.name()).on_hover_text(preset.hint()).clicked() {
                    preset.apply(s);
                }
            }
        });
        ui.add_space(4.0);

        egui::CollapsingHeader::new("Tuning").default_open(true).show(ui, |ui| {
            let before = s.tuning.preset;
            egui::ComboBox::from_label("Tuning")
                .selected_text(s.tuning.preset.name())
                .show_ui(ui, |ui| {
                    for p in TuningPreset::ALL {
                        ui.selectable_value(&mut s.tuning.preset, p, p.name());
                    }
                });
            if before != s.tuning.preset {
                s.tuning.equalness = s.tuning.preset.default_equalness();
            }
            ui.add(egui::Slider::new(&mut s.tuning.tonic_hz, 70.0..=180.0).text("Lowest key Hz").logarithmic(true));
            ui.weak(format!("   = {}", hz_to_note_name(s.tuning.tonic_hz)));
            ui.add(egui::Slider::new(&mut s.tuning.equalness, 0.0..=1.0).text("Equal spacing"))
                .on_hover_text("0 = Western mode, 1 = 7-tone equal temperament (Shona tunings lean towards equal spacing)");
            ui.add(egui::Slider::new(&mut s.tuning.stretch, 0.99..=1.02).text("Octave stretch"));
            ui.add(egui::Slider::new(&mut s.tuning.detune_cents, 0.0..=25.0).text("Key detune (cents)"))
                .on_hover_text("Hand-made imperfection; octave pairs beat against each other");
            ui.horizontal(|ui| {
                ui.label("Instrument seed");
                ui.add(egui::DragValue::new(&mut s.instrument_seed));
            });
        });

        egui::CollapsingHeader::new("Tines").default_open(true).show(ui, |ui| {
            ui.add(egui::Slider::new(&mut s.timbre.brightness, 0.0..=2.0).text("Brightness"))
                .on_hover_text("Level of the inharmonic overtones in the attack");
            ui.add(egui::Slider::new(&mut s.timbre.decay, 0.2..=3.0).text("Sustain").logarithmic(true));
            ui.add(egui::Slider::new(&mut s.timbre.ratio2, 3.0..=7.5).text("Overtone 1 ratio"))
                .on_hover_text("Measured ≈5 on mbira, 5.3–5.9 on kalimba, 6.27 for an ideal cantilever");
            ui.add(egui::Slider::new(&mut s.timbre.ratio3, 9.0..=20.0).text("Overtone 2 ratio"));
            ui.add(egui::Slider::new(&mut s.timbre.ratio_jitter, 0.0..=0.15).text("Ratio jitter"));
            ui.add(egui::Slider::new(&mut s.timbre.click, 0.0..=1.5).text("Nail click"));
            ui.add(egui::Slider::new(&mut s.timbre.repluck_damp, 0.0..=1.0).text("Re-pluck damping"));
            ui.add(egui::Slider::new(&mut s.timbre.sympathetic, 0.0..=1.0).text("Sympathetic ring"));
            ui.add(egui::Slider::new(&mut s.timbre.shimmer, 0.0..=1.0).text("Shimmer"))
                .on_hover_text("Slow beating inside each note, like a real metal key");
            ui.add(egui::Slider::new(&mut s.timbre.pluck_variation, 0.0..=1.0).text("Pluck variation"))
                .on_hover_text("Every pluck sounds slightly different (attack colour, pitch settle)");
        });

        egui::CollapsingHeader::new("Buzz (machachara)").default_open(true).show(ui, |ui| {
            ui.add(egui::Slider::new(&mut s.timbre.buzz, 0.0..=1.5).text("Buzz amount"));
            ui.add(egui::Slider::new(&mut s.timbre.buzz_threshold, 0.0..=1.0).text("Threshold"))
                .on_hover_text("Higher = only loud notes make the caps rattle");
            ui.add(egui::Slider::new(&mut s.timbre.buzz_phase_lock, 0.0..=1.0).text("Rattle lock"))
                .on_hover_text("0 = noisy sizzle, 1 = hits locked to the note's vibration");
        });

        egui::CollapsingHeader::new("Gourd & room").default_open(true).show(ui, |ui| {
            egui::ComboBox::from_label("Resonator")
                .selected_text(s.timbre.body.name())
                .show_ui(ui, |ui| {
                    for b in BodyPreset::ALL {
                        ui.selectable_value(&mut s.timbre.body, b, b.name());
                    }
                });
            ui.add(egui::Slider::new(&mut s.timbre.body_mix, 0.0..=1.0).text("Gourd amount"));
            ui.add(egui::Slider::new(&mut s.timbre.wood, 0.0..=1.0).text("Wooden board"))
                .on_hover_text("Dense soundboard resonances — the woody colour of the instrument");
            ui.add(egui::Slider::new(&mut s.tone_hz, 1500.0..=18000.0).text("Tone (Hz)").logarithmic(true))
                .on_hover_text("Master low-pass: lower = warmer, softer");
            ui.add(egui::Slider::new(&mut s.reverb, 0.0..=1.0).text("Room reverb"));
            ui.add(egui::Slider::new(&mut s.volume, 0.0..=1.5).text("Volume"));
        });

        ui.add_space(6.0);
        if ui.button("↺ Reset instrument").clicked() {
            let d = Settings::default();
            s.tuning = d.tuning;
            s.timbre = d.timbre;
            s.reverb = d.reverb;
            s.volume = d.volume;
            s.tone_hz = d.tone_hz;
            s.touch = d.touch;
            s.groove = d.groove;
        }
    }

    fn music_panel(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        ui.heading("Music");
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Composition").default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Song seed");
                ui.add(egui::DragValue::new(&mut s.music.seed));
                if ui.small_button("🎲").clicked() {
                    s.music.seed = random_seed();
                }
            });
            egui::ComboBox::from_label("Chord cycle")
                .selected_text(s.music.standard.name())
                .show_ui(ui, |ui| {
                    for st in Standard::ALL {
                        ui.selectable_value(&mut s.music.standard, st, st.name());
                    }
                });
            ui.add(egui::Slider::new(&mut s.music.start_degree, 0..=6).text("Start degree"))
                .on_hover_text("Which scale degree the cycle starts on (the song's 'key position')");
            egui::ComboBox::from_label("Kutsinhira")
                .selected_text(s.music.kutsinhira.name())
                .show_ui(ui, |ui| {
                    for k in KutsinhiraMode::ALL {
                        ui.selectable_value(&mut s.music.kutsinhira, k, k.name());
                    }
                });
        });

        egui::CollapsingHeader::new("Performance").default_open(true).show(ui, |ui| {
            ui.add(egui::Slider::new(&mut s.tempo, 3.0..=11.0).text("Tempo (pulses/s)"));
            ui.weak(format!("   ≈ {:.0} BPM (3 pulses per beat)", s.tempo * 20.0));
            ui.add(egui::Slider::new(&mut s.intensity, 0.0..=1.0).text("Intensity"))
                .on_hover_text("Density of notes; with auto layers it also adds bass (≥0.45), kutsinhira (≥0.55), ngoma and hosho (≥0.65)");
            ui.add(egui::Slider::new(&mut s.touch, 0.0..=1.0).text("Touch"))
                .on_hover_text("How hard the music plucks: soft = mellow, quiet, no buzz; hard = bright and buzzy");
            ui.add(egui::Slider::new(&mut s.arrangement, 0.0..=1.0).text("Arrangement"))
                .on_hover_text("Parts drop out and come back in sections (drums out, breakdown, solo mbira…). 0 = everything always plays");
            ui.add(egui::Slider::new(&mut s.variation, 0.0..=1.0).text("Variation"))
                .on_hover_text("How much the melody develops: new variations (zvara) keep growing out of the song, plus phrase-level register changes and ornaments");
            ui.add(egui::Slider::new(&mut s.groove, 0.0..=1.0).text("Groove"))
                .on_hover_text("Player feel: consistent micro-timing and phrase dynamics");
            ui.add(egui::Slider::new(&mut s.humanize, 0.0..=1.0).text("Humanize"));
            ui.checkbox(&mut s.auto_layers, "Auto layers from intensity");
            ui.add_enabled_ui(!s.auto_layers, |ui| {
                ui.checkbox(&mut s.play_kushaura, "Kushaura (lead) — orange");
                ui.checkbox(&mut s.play_kutsinhira, "Kutsinhira (follow) — teal");
                ui.checkbox(&mut s.play_hosho, "Hosho (shakers)");
                ui.checkbox(&mut s.play_ngoma, "Ngoma (drum)");
                ui.checkbox(&mut s.play_bass, "Bass guitar");
            });
        });

        egui::CollapsingHeader::new("Band: ngoma, hosho & bass").default_open(true).show(ui, |ui| {
            if ui.button("🥁 Full band").on_hover_text("Turn on every part: both mbiras, ngoma, hosho and bass").clicked() {
                s.auto_layers = false;
                s.play_kushaura = true;
                s.play_kutsinhira = true;
                s.play_hosho = true;
                s.play_ngoma = true;
                s.play_bass = true;
            }
            egui::ComboBox::from_label("Ngoma style")
                .selected_text(s.drum_style.name())
                .show_ui(ui, |ui| {
                    for d in DrumStyle::ALL {
                        ui.selectable_value(&mut s.drum_style, d, d.name());
                    }
                });
            ui.add(egui::Slider::new(&mut s.ngoma_level, 0.0..=1.5).text("Ngoma level"));
            ui.add(egui::Slider::new(&mut s.hosho_level, 0.0..=1.5).text("Hosho level"));
            egui::ComboBox::from_label("Bass style")
                .selected_text(s.bass_style.name())
                .show_ui(ui, |ui| {
                    for b in BassStyle::ALL {
                        ui.selectable_value(&mut s.bass_style, b, b.name());
                    }
                });
            ui.add(egui::Slider::new(&mut s.bass_level, 0.0..=1.5).text("Bass level"));
            ui.weak("The drum plays the 12/8 beat (3 pulses); the bass follows the chord cycle (4 pulses per chord).");
        });

        ui.add_space(6.0);
        ui.separator();
        ui.heading("Export WAV");
        let ex = &mut self.export;
        ui.add(egui::Slider::new(&mut ex.seconds, 5.0..=600.0).text("Length (s)").logarithmic(true));
        ui.add(egui::Slider::new(&mut ex.fade, 0.0..=15.0).text("Fade out (s)"));
        ui.horizontal(|ui| {
            ui.label("File");
            ui.text_edit_singleline(&mut ex.path);
        });
        let is_custom = matches!(ex.target, LoudnessTarget::Custom(_));
        egui::ComboBox::from_label("Loudness")
            .selected_text(ex.target.name())
            .show_ui(ui, |ui| {
                for t in LoudnessTarget::PRESETS {
                    ui.selectable_value(&mut ex.target, t, t.name());
                }
                if ui.selectable_label(is_custom, "Custom…").clicked() && !is_custom {
                    ex.target = LoudnessTarget::Custom(-16.0);
                }
            })
            .response
            .on_hover_text("Normalise the file to a loudness standard (measured with ITU-R BS.1770, two-pass)");
        if let LoudnessTarget::Custom(l) = &mut ex.target {
            ui.add(egui::Slider::new(l, -36.0..=-6.0).text("Target LUFS"));
        }
        ui.weak("Renders the current settings & seed (deterministic), through the master chain. 48 kHz, 16-bit stereo.");
        if ex.running {
            let p = ex.progress.load(Ordering::Relaxed) as f32 / 1000.0;
            ui.add(egui::ProgressBar::new(p).show_percentage());
            if let Some(res) = ex.result.lock().unwrap().take() {
                ex.running = false;
                *ex.result.lock().unwrap() = Some(res);
            }
        } else if ui.add(egui::Button::new("💾 Render to WAV").min_size(Vec2::new(150.0, 26.0))).clicked() {
            ex.running = true;
            ex.progress.store(0, Ordering::Relaxed);
            *ex.result.lock().unwrap() = None;
            let mut settings = *s;
            settings.volume = settings.volume.max(0.1);
            let (secs, fade, target) = (ex.seconds, ex.fade, ex.target);
            let mut path = PathBuf::from(ex.path.trim());
            if path.extension().is_none() {
                path.set_extension("wav");
            }
            let progress = ex.progress.clone();
            let result = ex.result.clone();
            std::thread::spawn(move || {
                let r = render_wav(settings, secs, 48_000, fade, &path, Some(&progress), target);
                let msg = r.map(|rep| {
                    let p = std::fs::canonicalize(&path).unwrap_or(path).display().to_string();
                    let gain = if target.lufs().is_some() { format!(", gain {:+.1} dB", rep.gain_db) } else { String::new() };
                    format!(
                        "{p}\n   {:.1} LUFS integrated · peak {:.1} dBFS{gain}",
                        rep.loudness.integrated, rep.loudness.peak_db
                    )
                });
                *result.lock().unwrap() = Some(msg);
            });
        }
        if !ex.running {
            match &*ex.result.lock().unwrap() {
                Some(Ok(p)) => {
                    ui.colored_label(Color32::from_rgb(90, 200, 90), format!("✔ Saved {p}"));
                }
                Some(Err(e)) => {
                    ui.colored_label(Color32::RED, format!("✘ {e}"));
                }
                None => {}
            }
        }
    }

    // ------------------------------------------------------------------ home

    fn home_send(&mut self, msg: HomeMsg) {
        if let Some(h) = &self.home.handle {
            let _ = h.tx.send(msg);
        }
    }

    fn save_home_config(&mut self) {
        let _ = self.home.config.save();
        let c = self.home.config.clone();
        self.home_send(HomeMsg::Config(c));
    }

    fn set_home_on(&mut self, on: bool) {
        if on && self.home.handle.is_none() {
            let Some(link) = &self.link else { return };
            self.home.base = self.settings;
            let h = home::start(link.tx.clone(), self.settings, self.home.config.clone(), false);
            self.home.handle = Some(h);
        }
        if on {
            self.home.base = self.settings;
            let b = self.home.base;
            self.home_send(HomeMsg::Base(b));
            self.home_send(HomeMsg::Pause(false));
        } else {
            // Hand control back to the sliders, starting from the user's own settings.
            self.settings = self.home.base;
            self.sent = None;
            self.playing = self.home.status.as_ref().map(|s| s.playing).unwrap_or(false);
        }
        self.home.on = on;
        self.home.mirrored = None;
        self.home_send(HomeMsg::Active(on));
    }

    /// While home mode steers, mirror its settings in the GUI; forward instrument edits.
    fn sync_home(&mut self) {
        let Some(h) = &self.home.handle else { return };
        let st = h.status.lock().unwrap().clone();
        if self.home.on {
            if let Some(m) = self.home.mirrored
                && m != self.settings
            {
                // The user changed something: keep their instrument choices in the base.
                let e = self.settings;
                let b = &mut self.home.base;
                b.tuning = e.tuning;
                b.instrument_seed = e.instrument_seed;
                b.timbre = e.timbre;
                b.music.standard = e.music.standard;
                b.music.start_degree = e.music.start_degree;
                b.music.kutsinhira = e.music.kutsinhira;
                b.master = e.master;
                let b = *b;
                self.home_send(HomeMsg::Base(b));
            }
            self.settings = st.settings;
            self.sent = Some(st.settings);
            self.home.mirrored = Some(st.settings);
            self.playing = st.playing;
        }
        self.home.status = Some(st);
    }

    fn home_window(&mut self, ctx: &egui::Context) {
        let mut open = self.home.show;
        if !open {
            return;
        }
        let found = if self.home.geocoding { self.home.geocode.lock().unwrap().take() } else { None };
        if let Some(res) = found {
            self.home.geocoding = false;
            match res {
                Ok((name, lat, lon)) => {
                    self.home.config.city = name.clone();
                    self.home.city_input = name;
                    self.home.config.latitude = Some(lat);
                    self.home.config.longitude = Some(lon);
                    self.home.config.weather = true;
                    self.save_home_config();
                }
                Err(e) => {
                    if let Some(st) = &mut self.home.status {
                        st.weather_note = e;
                    }
                }
            }
        }
        let mut actions: Vec<HomeMsg> = Vec::new();
        let mut toggle_on: Option<bool> = None;
        let mut save = false;
        egui::Window::new("🏠 Home music").open(&mut open).default_width(420.0).resizable(true).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(620.0).show(ui, |ui| {
                let mut on = self.home.on;
                if ui
                    .checkbox(&mut on, egui::RichText::new("Let the house steer the music").strong())
                    .on_hover_text("Time of day, weather, events and scenes choose tempo, mood, layers and volume")
                    .changed()
                {
                    toggle_on = Some(on);
                }
                if self.link.is_none() {
                    ui.colored_label(Color32::RED, "No audio device, so home music can't play.");
                }
                let st = self.home.status.clone();
                if let Some(st) = &st
                    && self.home.on
                {
                    ui.label(egui::RichText::new(&st.summary).size(15.0));
                    let m = st.mood;
                    egui::Grid::new("mood").num_columns(2).show(ui, |ui| {
                        for (name, v) in [
                            ("Energy", m.energy),
                            ("Brightness", m.brightness),
                            ("Warmth", m.warmth),
                            ("Space", m.space),
                            ("Band", m.band),
                            ("Rain", m.rain),
                            ("Wind", m.wind),
                        ] {
                            ui.label(name);
                            ui.add(egui::ProgressBar::new(v).desired_width(220.0));
                            ui.end_row();
                        }
                    });
                    ui.weak(format!(
                        "Tempo {:.1} pulses/s · song #{} {}",
                        st.settings.tempo,
                        st.settings.music.seed,
                        if st.paused { "· paused" } else if st.playing { "" } else { "· silent" }
                    ));
                }

                ui.separator();
                ui.strong("Scene");
                let current = st.as_ref().map(|s| s.scene).unwrap_or(Scene::Auto);
                ui.horizontal_wrapped(|ui| {
                    for sc in Scene::ALL {
                        let b = egui::Button::new(format!("{} {}", sc.icon(), sc.name())).selected(current == sc);
                        if ui.add_enabled(self.home.on, b).clicked() {
                            actions.push(HomeMsg::Scene(sc, None));
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().slider_width = 160.0;
                    if ui.add(egui::Slider::new(&mut self.home.volume, 0.0..=1.5).text("Volume")).changed() {
                        actions.push(HomeMsg::Volume(self.home.volume));
                    }
                    if ui.add_enabled(self.home.on, egui::Button::new("⏭ Next song")).clicked() {
                        actions.push(HomeMsg::NextSong);
                    }
                });
                if ui
                    .add(egui::Slider::new(&mut self.home.config.song_minutes, 0.0..=60.0).text("New song every (min, 0 = never)"))
                    .changed()
                {
                    actions.push(HomeMsg::SongMinutes(self.home.config.song_minutes));
                    save = true;
                }

                ui.separator();
                ui.strong("🌦 Weather");
                ui.horizontal(|ui| {
                    ui.label("City");
                    ui.text_edit_singleline(&mut self.home.city_input);
                    let busy = self.home.geocoding;
                    if ui.add_enabled(!busy, egui::Button::new(if busy { "Finding…" } else { "Find" })).clicked() {
                        self.home.geocoding = true;
                        let (city, out) = (self.home.city_input.clone(), self.home.geocode.clone());
                        std::thread::spawn(move || *out.lock().unwrap() = Some(home::geocode(&city)));
                    }
                });
                ui.weak("Looks the place up and fetches the weather every 15 min from Open-Meteo (free, no account).");
                if ui.checkbox(&mut self.home.config.weather, "Use the weather").changed() {
                    save = true;
                }
                if ui.checkbox(&mut self.home.weather_affects_mood, "Weather changes the mood (rainy = slower, darker)").changed() {
                    actions.push(HomeMsg::WeatherAffectsMood(self.home.weather_affects_mood));
                }
                if ui.add(egui::Slider::new(&mut self.home.config.ambience, 0.0..=1.0).text("Rain & wind sounds")).changed() {
                    actions.push(HomeMsg::Ambience(self.home.config.ambience));
                    save = true;
                }
                if let Some(st) = &st {
                    ui.weak(&st.weather_note);
                }

                ui.separator();
                egui::CollapsingHeader::new("🏡 Smart home (Home Assistant, scripts, phone)").show(ui, |ui| {
                    let api = st.as_ref().map(|s| s.api.clone()).unwrap_or_default();
                    ui.label(if api.is_empty() { "API starts when home music is first switched on.".into() } else { format!("API: {api}") });
                    let mut lan = self.home.config.listen.starts_with("0.0.0.0");
                    if ui
                        .checkbox(&mut lan, "Allow other devices on my network")
                        .on_hover_text("Listen on all interfaces (0.0.0.0:7777) so Home Assistant on another machine can reach it")
                        .changed()
                    {
                        self.home.config.listen = if lan { "0.0.0.0:7777".into() } else { home::DEFAULT_LISTEN.into() };
                        save = true;
                    }
                    let host = self.home.config.listen.replace("0.0.0.0", "<this-computer-ip>");
                    ui.weak("Examples:");
                    ui.code(format!(
                        "curl http://{host}/scene/dinner\ncurl http://{host}/event/arrive\ncurl http://{host}/event/movie_start\ncurl http://{host}/volume/40\ncurl http://{host}/status"
                    ));
                    ui.weak("Home Assistant configuration.yaml:");
                    ui.code(format!(
                        "rest_command:\n  mbira_event:\n    url: \"http://{host}/event/{{{{ event }}}}\"\n  mbira_scene:\n    url: \"http://{host}/scene/{{{{ scene }}}}\""
                    ));
                    ui.weak("Try the events here:");
                    ui.horizontal_wrapped(|ui| {
                        for ev in HomeEvent::ALL {
                            if ui.add_enabled(self.home.on, egui::Button::new(ev.id()).small()).clicked() {
                                actions.push(HomeMsg::Event(ev));
                            }
                        }
                    });
                });
                if let Some(st) = &st
                    && !st.log.is_empty()
                {
                    egui::CollapsingHeader::new("Activity").show(ui, |ui| {
                        for l in st.log.iter().rev() {
                            ui.weak(l);
                        }
                    });
                }
            });
        });
        self.home.show = open;
        if let Some(on) = toggle_on {
            self.set_home_on(on);
        }
        if save {
            self.save_home_config();
        }
        for a in actions {
            self.home_send(a);
        }
    }

    // ------------------------------------------------------------- mastering

    fn master_panel(&mut self, ui: &mut egui::Ui) {
        let mut reset = false;
        let (loudness, gr) = (self.loudness, self.gr_db);
        let m = &mut self.settings.master;
        let on = m.enabled;
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 100.0;
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.strong("🎚 Mastering");
                    ui.checkbox(&mut m.enabled, "On");
                    let edited = m.enabled && *m != MasterPreset::params(m.preset);
                    egui::ComboBox::from_id_salt("master_preset")
                        .selected_text(format!("{}{}", m.preset.name(), if edited { " (edited)" } else { "" }))
                        .show_ui(ui, |ui| {
                            for p in MasterPreset::ALL {
                                if ui.selectable_label(m.preset == p, p.name()).on_hover_text(p.hint()).clicked() {
                                    *m = p.params();
                                }
                            }
                        });
                    if ui.button("Reset").clicked() {
                        *m = MasterParams::default();
                    }
                });
            });
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.strong("Loudness (EBU R128)");
                    let bar = |ui: &mut egui::Ui, name: &str, v: f32| {
                        ui.horizontal(|ui| {
                            ui.add_sized([72.0, 16.0], egui::Label::new(name));
                            let frac = ((v + 40.0) / 40.0).clamp(0.0, 1.0);
                            let color = if v > -9.0 { Color32::from_rgb(220, 80, 60) } else { Color32::from_rgb(90, 170, 220) };
                            let text = if v > -90.0 { format!("{v:.1} LUFS") } else { "—".into() };
                            ui.add(egui::ProgressBar::new(frac).desired_width(140.0).fill(color).text(text));
                        });
                    };
                    bar(ui, "Momentary", loudness.momentary);
                    bar(ui, "Short-term", loudness.short_term);
                    bar(ui, "Integrated", loudness.integrated);
                    ui.horizontal(|ui| {
                        ui.add_sized([72.0, 16.0], egui::Label::new("Reduction"));
                        ui.add(
                            egui::ProgressBar::new((gr / 12.0).clamp(0.0, 1.0))
                                .desired_width(140.0)
                                .fill(Color32::from_rgb(230, 170, 60))
                                .text(format!("{gr:.1} dB")),
                        );
                    });
                    ui.horizontal(|ui| {
                        let peak = loudness.peak_db;
                        ui.weak(if peak > -90.0 { format!("Peak {peak:.1} dBFS") } else { "Peak —".into() });
                        if ui.small_button("Reset meter").clicked() {
                            reset = true;
                        }
                    });
                })
                .response
                .on_hover_text("Targets: streaming -14 LUFS · game mobile -18 · game console/PC -24 · broadcast -23");
            });
            ui.add_enabled_ui(on, |ui| {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.strong("EQ");
                        ui.add(egui::Slider::new(&mut m.low_cut_hz, 10.0..=150.0).logarithmic(true).suffix(" Hz").text("Low cut"))
                            .on_hover_text("Removes sub rumble you can't hear but that eats headroom");
                        ui.add(egui::Slider::new(&mut m.low_db, -6.0..=6.0).suffix(" dB").text("Low 110 Hz"));
                        ui.add(egui::Slider::new(&mut m.mid_db, -6.0..=6.0).suffix(" dB").text("Mid"));
                        ui.add(egui::Slider::new(&mut m.mid_hz, 200.0..=5000.0).logarithmic(true).suffix(" Hz").text("Mid freq"));
                        ui.add(egui::Slider::new(&mut m.air_db, -6.0..=6.0).suffix(" dB").text("Air 9 kHz"));
                    });
                });
            });
            ui.add_enabled_ui(on, |ui| {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.strong("Compressor (glue)");
                        ui.add(egui::Slider::new(&mut m.threshold_db, -40.0..=0.0).suffix(" dB").text("Threshold"));
                        ui.add(egui::Slider::new(&mut m.ratio, 1.0..=10.0).logarithmic(true).text("Ratio"));
                        ui.add(egui::Slider::new(&mut m.attack_ms, 0.5..=100.0).logarithmic(true).suffix(" ms").text("Attack"))
                            .on_hover_text("Slower attack lets the pluck transients through (punchier)");
                        ui.add(egui::Slider::new(&mut m.release_ms, 20.0..=1000.0).logarithmic(true).suffix(" ms").text("Release"));
                        ui.add(egui::Slider::new(&mut m.makeup_db, 0.0..=12.0).suffix(" dB").text("Makeup"));
                    });
                });
            });
            ui.add_enabled_ui(on, |ui| {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(250.0);
                        ui.strong("Colour, space & output");
                        ui.add(egui::Slider::new(&mut m.saturation, 0.0..=1.0).text("Tape warmth"));
                        ui.add(egui::Slider::new(&mut m.width, 0.0..=2.0).text("Stereo width"));
                        ui.checkbox(&mut m.mono_bass, "Mono bass (<120 Hz)");
                        ui.add(egui::Slider::new(&mut m.output_db, -12.0..=12.0).suffix(" dB").text("Output gain"));
                        ui.add(egui::Slider::new(&mut m.ceiling_db, -6.0..=-0.1).suffix(" dBFS").text("Ceiling"))
                            .on_hover_text("Limiter ceiling. -1 dBFS leaves room for MP3/OGG encoding");
                    });
                });
            });
        });
        if reset {
            self.send(Command::ResetLoudness);
            self.loudness = LoudnessReading::default();
        }
    }

    // ----------------------------------------------------------- central view

    fn cycle_view(&mut self, ui: &mut egui::Ui) {
        let s = &self.settings;
        let roots = cycle_roots(s.music.standard, s.music.start_degree);
        ui.strong("Chord cycle");
        ui.weak("12 dyads (root + fifth) × 4 pulses = 48 pulses; moves only up a 3rd or a 4th (A. Tracey)");
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 74.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let box_w = rect.width() / DYADS as f32;
        let current = if self.playing { Some(self.pulse / 4) } else { None };
        for (i, &root) in roots.iter().enumerate() {
            let r = Rect::from_min_size(
                Pos2::new(rect.left() + i as f32 * box_w + 2.0, rect.top()),
                Vec2::new(box_w - 4.0, 44.0),
            );
            let active = current == Some(i);
            let fill = if active { ORANGE } else { ui.visuals().faint_bg_color };
            painter.rect(r, CornerRadius::same(6), fill, Stroke::new(1.0, Color32::GRAY), StrokeKind::Inside);
            let text_color = if active { Color32::BLACK } else { ui.visuals().text_color() };
            painter.text(
                r.center() - Vec2::new(0.0, 6.0),
                Align2::CENTER_CENTER,
                degree_letter(root).to_string(),
                FontId::proportional(20.0),
                text_color,
            );
            painter.text(
                r.center() + Vec2::new(0.0, 13.0),
                Align2::CENTER_CENTER,
                format!("+{}", degree_letter(root + 4)),
                FontId::proportional(11.0),
                text_color,
            );
            if i % 3 == 0 {
                painter.line_segment(
                    [Pos2::new(r.left() - 2.0, rect.top()), Pos2::new(r.left() - 2.0, rect.top() + 70.0)],
                    Stroke::new(2.0, Color32::from_gray(120)),
                );
            }
        }
        // Pulse dots.
        let dot_w = rect.width() / PULSES as f32;
        for p in 0..PULSES {
            let c = Pos2::new(rect.left() + (p as f32 + 0.5) * dot_w, rect.top() + 60.0);
            let on = self.playing && p == self.pulse;
            let rad = if p % 3 == 0 { 4.0 } else { 2.5 };
            let col = if on { ORANGE } else if p % 12 == 0 { Color32::from_gray(200) } else { Color32::from_gray(110) };
            painter.circle_filled(c, if on { 6.0 } else { rad }, col);
        }
        if self.playing {
            ui.weak(format!(
                "Quarter {} · pulse {} · variations: kushaura #{}, kutsinhira #{} · {} new ideas · section: {}",
                self.pulse / 12 + 1,
                self.pulse + 1,
                self.variation[0],
                self.variation[1],
                self.generation,
                self.section.name()
            ));
        } else {
            ui.weak("Press ▶ Play (or Space) to start the generated music.");
        }
    }

    fn mbira_view(&mut self, ui: &mut egui::Ui) {
        let s = self.settings;
        let freqs = s.tuning.key_freqs(s.instrument_seed);
        let roots = cycle_roots(s.music.standard, s.music.start_degree);
        let current_root = roots[(self.pulse / 4) % DYADS];
        let now = self.now();

        ui.add_space(8.0);
        ui.strong("Mbira");
        ui.weak("Click a key or use the keyboard (Shift = loud, Space = play/stop). Lowest keys sit in the centre.");
        ui.add(egui::Slider::new(&mut self.manual_vel, 0.1..=1.0).text("Pluck velocity"));

        let width = ui.available_width().clamp(320.0, 1100.0);
        let height = 380.0;
        let (board, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let painter = ui.painter_at(board);
        painter.rect_filled(board, CornerRadius::same(18), WOOD);
        // Bridge bar.
        let bridge_y = board.top() + 34.0;
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(board.left() + 20.0, bridge_y - 8.0), Pos2::new(board.right() - 20.0, bridge_y + 2.0)),
            CornerRadius::same(3),
            Color32::from_gray(70),
        );
        painter.text(board.left_top() + Vec2::new(26.0, 8.0), Align2::LEFT_TOP, "gwariva", FontId::proportional(11.0), WOOD_DARK);

        let gap = 18.0;
        let left_w = board.width() * 0.55 - gap;
        let right_x0 = board.left() + board.width() * 0.55 + gap / 2.0;
        let right_w = board.right() - 20.0 - right_x0;

        // (manual, x0, width, y offset, max length)
        let groups: [(&[Manual], f32, f32, f32, f32); 3] = [
            (&[Manual::Upper], board.left() + 20.0, left_w, 0.0, 120.0),
            (&[Manual::Bass], board.left() + 20.0, left_w, 132.0, 150.0),
            (&[Manual::RightThumb, Manual::RightIndex], right_x0, right_w, 0.0, 260.0),
        ];

        let min_f = freqs.iter().cloned().fold(f32::MAX, f32::min);
        let max_f = freqs.iter().cloned().fold(0.0, f32::max);
        let mut pluck: Option<(usize, f32)> = None;

        for (gi, (manuals, x0, w, yoff, max_len)) in groups.iter().enumerate() {
            let mut keys: Vec<usize> = (0..NUM_KEYS).filter(|&k| manuals.contains(&KEYS[k].manual)).collect();
            keys.sort_by_key(|&k| KEYS[k].degree);
            if gi < 2 {
                keys.reverse(); // left side: lowest key nearest the centre
            }
            let n = keys.len() as f32;
            let slot = w / n;
            let tine_w = (slot * 0.62).min(34.0);
            for (i, &k) in keys.iter().enumerate() {
                let f = freqs[k];
                let t = ((f / min_f).ln() / (max_f / min_f).ln()).clamp(0.0, 1.0);
                let len = max_len * (1.0 - 0.55 * t);
                let cx = x0 + (i as f32 + 0.5) * slot;
                let top = bridge_y + yoff;
                let rect = Rect::from_min_size(Pos2::new(cx - tine_w / 2.0, top), Vec2::new(tine_w, len));
                let resp = ui.interact(rect, ui.id().with(("tine", k)), Sense::click());
                if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
                    // Pluck nearer the tip = louder.
                    let y = ui.input(|i| i.pointer.interact_pos()).map(|p| p.y).unwrap_or(rect.bottom());
                    let along = ((y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                    pluck = Some((k, (self.manual_vel * (0.6 + 0.5 * along)).min(1.0)));
                }
                let fa = (-(now - self.flash[0][k]) / 0.35).exp() as f32;
                let fb = (-(now - self.flash[1][k]) / 0.35).exp() as f32;
                let mut col = STEEL;
                col = lerp_color(col, ORANGE, fa);
                col = lerp_color(col, TEAL, fb * (1.0 - fa * 0.5));
                if resp.hovered() {
                    col = lerp_color(col, Color32::WHITE, 0.25);
                }
                painter.rect(rect, CornerRadius { nw: 2, ne: 2, sw: 10, se: 10 }, col, Stroke::new(1.0, Color32::from_gray(60)), StrokeKind::Inside);
                // Highlight keys that belong to the current dyad while playing.
                if self.playing && crate::compose::in_dyad(KEYS[k].degree, current_root) {
                    painter.rect_stroke(rect.expand(2.0), CornerRadius::same(6), Stroke::new(2.0, Color32::from_rgb(255, 220, 120)), StrokeKind::Outside);
                }
                painter.text(
                    Pos2::new(cx, rect.bottom() - 12.0),
                    Align2::CENTER_CENTER,
                    KEYS[k].shortcut.to_string(),
                    FontId::monospace(12.0),
                    Color32::from_gray(30),
                );
                let note = hz_to_note_name(f);
                let note_short = note.split(' ').next().unwrap_or("").to_string();
                let caption = if slot >= 46.0 { format!("{} {}", KEYS[k].label, note_short) } else { KEYS[k].label.to_string() };
                painter.text(
                    Pos2::new(cx, rect.bottom() + 9.0),
                    Align2::CENTER_CENTER,
                    caption,
                    FontId::proportional(if slot >= 30.0 { 10.0 } else { 8.5 }),
                    Color32::from_gray(225),
                );
                resp.on_hover_text(format!(
                    "{} — degree {} ({})\n{:.1} Hz  ≈ {}\nkey: {}",
                    KEYS[k].label,
                    KEYS[k].degree,
                    degree_letter(KEYS[k].degree),
                    f,
                    note,
                    KEYS[k].shortcut
                ));
            }
        }
        // Machachara (bottle caps) decoration — jiggles with the level.
        let jig = self.level * 4.0;
        let caps = 9;
        let cap_x0 = board.center().x - (caps as f32 - 1.0) * 11.0;
        for i in 0..caps {
            let x = cap_x0 + i as f32 * 22.0;
            let y = board.bottom() - 18.0 + if i % 2 == 0 { -jig } else { jig };
            painter.circle(Pos2::new(x, y), 8.0, Color32::from_rgb(185, 160, 90), Stroke::new(1.5, Color32::from_rgb(120, 100, 50)));
        }
        painter.text(
            board.right_top() + Vec2::new(-26.0, 8.0),
            Align2::RIGHT_TOP,
            "provisional 24-key layout · machachara below",
            FontId::proportional(10.0),
            Color32::from_rgb(200, 170, 140),
        );

        if let Some((k, v)) = pluck {
            self.pluck(k, v);
        }
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

impl eframe::App for MbiraApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_keyboard(&ctx);
        self.sync();

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("master").resizable(false).show(ui, |ui| {
            ui.add_space(4.0);
            egui::ScrollArea::horizontal().show(ui, |ui| self.master_panel(ui));
            ui.add_space(2.0);
        });
        egui::Panel::left("instrument").resizable(true).default_size(270.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.instrument_panel(ui));
        });
        egui::Panel::right("music").resizable(true).default_size(270.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.music_panel(ui));
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.cycle_view(ui);
                self.mbira_view(ui);
            });
        });

        self.home_window(&ctx);

        // Keep animating while sound may be ringing.
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

#[derive(Clone, Copy)]
enum SoundPreset {
    Soft,
    Warm,
    Traditional,
    Kalimba,
}

impl SoundPreset {
    const ALL: [SoundPreset; 4] = [SoundPreset::Soft, SoundPreset::Warm, SoundPreset::Traditional, SoundPreset::Kalimba];

    fn name(self) -> &'static str {
        match self {
            SoundPreset::Soft => "🌙 Soft",
            SoundPreset::Warm => "🔥 Warm gourd",
            SoundPreset::Traditional => "🪘 Traditional (buzzy)",
            SoundPreset::Kalimba => "✨ Bright kalimba",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            SoundPreset::Soft => "Gentle and quiet: no buzz, soft touch, warm tone, more room",
            SoundPreset::Warm => "Default-ish: a little buzz, big gourd, mellow",
            SoundPreset::Traditional => "Ceremonial mbira: strong machachara sizzle, hosho, harder touch",
            SoundPreset::Kalimba => "Clean, bright, dry kalimba-like tone",
        }
    }

    fn apply(self, s: &mut Settings) {
        let t = &mut s.timbre;
        match self {
            SoundPreset::Soft => {
                t.brightness = 0.3;
                t.click = 0.05;
                t.buzz = 0.0;
                t.body = BodyPreset::BigGourd;
                t.body_mix = 0.6;
                t.sympathetic = 0.4;
                s.touch = 0.35;
                s.tone_hz = 3500.0;
                s.reverb = 0.4;
                s.intensity = 0.35;
                s.hosho_level = 0.15;
                s.tempo = 4.5;
            }
            SoundPreset::Warm => {
                t.brightness = 0.55;
                t.click = 0.2;
                t.buzz = 0.15;
                t.buzz_threshold = 0.6;
                t.body = BodyPreset::BigGourd;
                t.body_mix = 0.5;
                s.touch = 0.55;
                s.tone_hz = 7500.0;
                s.reverb = 0.3;
                s.hosho_level = 0.3;
            }
            SoundPreset::Traditional => {
                t.brightness = 0.8;
                t.click = 0.4;
                t.buzz = 0.9;
                t.buzz_threshold = 0.3;
                t.body = BodyPreset::BigGourd;
                t.body_mix = 0.5;
                s.touch = 0.8;
                s.tone_hz = 12000.0;
                s.reverb = 0.2;
                s.intensity = 0.85;
                s.hosho_level = 0.6;
                s.tempo = 6.0;
            }
            SoundPreset::Kalimba => {
                t.brightness = 0.9;
                t.click = 0.3;
                t.buzz = 0.0;
                t.ratio2 = 6.4; // measured on real kalimba recordings
                t.body = BodyPreset::None;
                t.body_mix = 0.0;
                s.touch = 0.6;
                s.tone_hz = 14000.0;
                s.reverb = 0.25;
            }
        }
    }
}
