//! Adaptive "home" music: turns what's happening (time of day, weather, smart-home
//! events, the scene you picked) into a mood, and the mood into engine settings.
//!
//! Pure logic, no I/O: the caller supplies the clock, the local hour and the weather,
//! which keeps it testable and usable from a game as well as from the home player.

use crate::band::{BassStyle, DrumStyle};
use crate::compose::{KutsinhiraMode, Standard};
use crate::engine::Settings;
use crate::rng::Rng;

/// Everything the music can express, each roughly 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mood {
    /// Tempo, density, how many parts play.
    pub energy: f32,
    /// Tone: dark/soft ↔ open/bright.
    pub brightness: f32,
    /// Low-end weight and tape warmth.
    pub warmth: f32,
    /// Room size / reverb.
    pub space: f32,
    /// How present the bass and ngoma are.
    pub band: f32,
    /// How much the melody wanders (low = steady, good for focus).
    pub variation: f32,
    pub volume: f32,
    /// Weather ambience.
    pub rain: f32,
    pub wind: f32,
}

impl Mood {
    const fn new(energy: f32, brightness: f32, warmth: f32, space: f32, band: f32, variation: f32, volume: f32) -> Self {
        Mood { energy, brightness, warmth, space, band, variation, volume, rain: 0.0, wind: 0.0 }
    }

    fn fields(&mut self) -> [&mut f32; 9] {
        [
            &mut self.energy,
            &mut self.brightness,
            &mut self.warmth,
            &mut self.space,
            &mut self.band,
            &mut self.variation,
            &mut self.volume,
            &mut self.rain,
            &mut self.wind,
        ]
    }

    pub fn lerp(mut self, mut other: Mood, t: f32) -> Mood {
        for (a, b) in self.fields().into_iter().zip(other.fields()) {
            *a += (*b - *a) * t;
        }
        self
    }

    fn clamp(mut self) -> Mood {
        for f in self.fields() {
            *f = f.clamp(0.0, 1.0);
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// Follow the time of day.
    Auto,
    Morning,
    Focus,
    Dinner,
    Unwind,
    Sleep,
    Gathering,
}

impl Scene {
    pub const ALL: [Scene; 7] =
        [Scene::Auto, Scene::Morning, Scene::Focus, Scene::Dinner, Scene::Unwind, Scene::Sleep, Scene::Gathering];

    pub fn name(self) -> &'static str {
        match self {
            Scene::Auto => "Auto (time of day)",
            Scene::Morning => "Morning",
            Scene::Focus => "Focus",
            Scene::Dinner => "Dinner",
            Scene::Unwind => "Unwind",
            Scene::Sleep => "Sleep",
            Scene::Gathering => "Gathering",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Scene::Auto => "🕒",
            Scene::Morning => "🌅",
            Scene::Focus => "🎧",
            Scene::Dinner => "🍲",
            Scene::Unwind => "🛋",
            Scene::Sleep => "🌙",
            Scene::Gathering => "🎉",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Scene::Auto => "auto",
            Scene::Morning => "morning",
            Scene::Focus => "focus",
            Scene::Dinner => "dinner",
            Scene::Unwind => "unwind",
            Scene::Sleep => "sleep",
            Scene::Gathering => "gathering",
        }
    }

    pub fn parse(s: &str) -> Option<Scene> {
        Scene::ALL.into_iter().find(|sc| sc.id().eq_ignore_ascii_case(s.trim()))
    }

    fn mood(self) -> Option<Mood> {
        Some(match self {
            Scene::Auto => return None,
            Scene::Morning => Mood::new(0.42, 0.7, 0.4, 0.45, 0.25, 0.5, 0.8),
            Scene::Focus => Mood::new(0.3, 0.45, 0.5, 0.4, 0.0, 0.15, 0.6),
            Scene::Dinner => Mood::new(0.45, 0.5, 0.75, 0.4, 0.55, 0.45, 0.75),
            Scene::Unwind => Mood::new(0.25, 0.35, 0.8, 0.6, 0.25, 0.4, 0.7),
            Scene::Sleep => Mood::new(0.06, 0.12, 0.9, 0.85, 0.0, 0.2, 0.4),
            Scene::Gathering => Mood::new(0.72, 0.65, 0.6, 0.35, 0.85, 0.6, 0.9),
        })
    }
}

/// The day's arc. Interpolated between anchors and wrapped around midnight.
const DAY: [(f32, Mood); 8] = [
    (0.0, Mood::new(0.1, 0.15, 0.85, 0.8, 0.0, 0.25, 0.45)),
    (5.0, Mood::new(0.08, 0.15, 0.85, 0.8, 0.0, 0.25, 0.4)),
    (7.0, Mood::new(0.25, 0.45, 0.5, 0.6, 0.0, 0.35, 0.6)),
    (10.0, Mood::new(0.45, 0.7, 0.4, 0.45, 0.3, 0.5, 0.8)),
    (14.0, Mood::new(0.38, 0.6, 0.45, 0.4, 0.2, 0.35, 0.75)),
    (18.0, Mood::new(0.5, 0.5, 0.7, 0.45, 0.6, 0.5, 0.85)),
    (21.0, Mood::new(0.3, 0.35, 0.8, 0.6, 0.3, 0.4, 0.7)),
    (23.0, Mood::new(0.15, 0.2, 0.85, 0.75, 0.0, 0.3, 0.5)),
];

pub fn day_phase_name(hour: f32) -> &'static str {
    match hour as u32 {
        0..=4 => "Night",
        5..=6 => "Dawn",
        7..=11 => "Morning",
        12..=16 => "Afternoon",
        17..=19 => "Evening",
        20..=22 => "Late evening",
        _ => "Night",
    }
}

pub fn time_of_day_mood(hour: f32) -> Mood {
    let h = hour.rem_euclid(24.0);
    for i in 0..DAY.len() {
        let (h0, m0) = DAY[i];
        let (h1, m1) = if i + 1 < DAY.len() { DAY[i + 1] } else { (24.0, DAY[0].1) };
        if h >= h0 && h < h1 {
            // Smoothstep so the music doesn't change pace abruptly at an anchor.
            let t = (h - h0) / (h1 - h0);
            return m0.lerp(m1, t * t * (3.0 - 2.0 * t));
        }
    }
    DAY[0].1
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weather {
    pub temperature_c: f32,
    /// mm per hour.
    pub precipitation: f32,
    /// 0..1
    pub cloud_cover: f32,
    pub wind_kmh: f32,
    /// WMO weather code.
    pub code: u32,
}

impl Weather {
    pub fn describe(&self) -> String {
        let sky = match self.code {
            0 => "Clear",
            1..=2 => "Partly cloudy",
            3 => "Overcast",
            45 | 48 => "Fog",
            51..=57 => "Drizzle",
            61..=67 | 80..=82 => "Rain",
            71..=77 | 85 | 86 => "Snow",
            95..=99 => "Thunderstorm",
            _ => "—",
        };
        format!("{sky}, {:.0}°C, wind {:.0} km/h", self.temperature_c, self.wind_kmh)
    }

    fn is_snow(&self) -> bool {
        matches!(self.code, 71..=77 | 85 | 86)
    }

    /// How the weather colours the mood.
    pub fn apply(&self, m: &mut Mood) {
        m.brightness -= 0.15 * self.cloud_cover;
        let mut rain = 0.0;
        if matches!(self.code, 51..=57) {
            rain = 0.3;
        }
        if self.precipitation > 0.05 && !self.is_snow() {
            rain = f32::max(rain, 0.3 + 0.2 * self.precipitation);
        }
        if matches!(self.code, 95..=99) {
            rain = rain.max(0.85);
        }
        if rain > 0.0 {
            // A rainy day: slower, darker, more room.
            m.energy -= 0.08;
            m.brightness -= 0.1;
            m.space += 0.1;
            m.warmth += 0.05;
        }
        if self.is_snow() {
            m.energy -= 0.1;
            m.space += 0.2;
        }
        if self.code == 0 {
            m.brightness += 0.08;
        }
        if self.temperature_c < 8.0 {
            m.warmth += 0.15;
        } else if self.temperature_c > 28.0 {
            m.energy -= 0.05;
            m.brightness += 0.05;
        }
        m.rain = rain.min(1.0);
        m.wind = ((self.wind_kmh - 12.0) / 40.0).clamp(0.0, 0.8);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeEvent {
    /// Someone came home: a lift in energy that fades over a few minutes; resumes if away.
    Arrive,
    /// Everyone left: fade out and stop.
    Leave,
    /// Movement in the room: a small, decaying lift.
    Motion,
    Dinner,
    /// Pause for a movie / TV, resume after.
    MovieStart,
    MovieEnd,
    /// Switch to Sleep and fade to silence over 30 minutes.
    Bedtime,
    /// Morning scene for an hour.
    Wake,
    /// Guests over: Gathering for three hours.
    Guests,
    /// Duck the music for 20 s.
    Doorbell,
}

impl HomeEvent {
    pub const ALL: [HomeEvent; 10] = [
        HomeEvent::Arrive,
        HomeEvent::Leave,
        HomeEvent::Motion,
        HomeEvent::Dinner,
        HomeEvent::MovieStart,
        HomeEvent::MovieEnd,
        HomeEvent::Bedtime,
        HomeEvent::Wake,
        HomeEvent::Guests,
        HomeEvent::Doorbell,
    ];

    pub fn id(self) -> &'static str {
        match self {
            HomeEvent::Arrive => "arrive",
            HomeEvent::Leave => "leave",
            HomeEvent::Motion => "motion",
            HomeEvent::Dinner => "dinner",
            HomeEvent::MovieStart => "movie_start",
            HomeEvent::MovieEnd => "movie_end",
            HomeEvent::Bedtime => "bedtime",
            HomeEvent::Wake => "wake",
            HomeEvent::Guests => "guests",
            HomeEvent::Doorbell => "doorbell",
        }
    }

    pub fn parse(s: &str) -> Option<HomeEvent> {
        HomeEvent::ALL.into_iter().find(|e| e.id().eq_ignore_ascii_case(s.trim()))
    }
}

/// Seconds for the mood to move ~63 % of the way to a new target.
const SLOW_TAU: f32 = 45.0;
const FAST_TAU: f32 = 8.0;
const SONG_FADE_S: f32 = 6.0;
const SLEEP_FADE_S: f64 = 30.0 * 60.0;

pub struct Director {
    pub scene: Scene,
    scene_until: Option<f64>,
    fast_until: f64,
    pub current: Mood,
    pub target: Mood,
    lift: f32,
    lift_time: f64,
    lift_tau: f32,
    duck_until: f64,
    pub movie: bool,
    pub away: bool,
    sleep_since: Option<f64>,
    pub weather: Option<Weather>,
    pub weather_affects_mood: bool,
    /// Rain/wind sound level, 0..1 (0 = mood only, no sound).
    pub ambience: f32,
    pub user_volume: f32,
    /// Minutes before moving on to a new song (0 = never).
    pub song_minutes: f32,
    song_started: f64,
    fade: f32,
    changing_song: bool,
    pub seed: u64,
    /// Per-song character, re-drawn at every song change so songs don't all share one
    /// chord cycle, key and pace.
    pub song: SongTraits,
    rng: Rng,
    volume: f32,
    pub last_event: Option<(HomeEvent, f64)>,
}

impl Director {
    pub fn new(seed: u64) -> Self {
        let start = time_of_day_mood(12.0);
        Director {
            scene: Scene::Auto,
            scene_until: None,
            fast_until: 0.0,
            current: start,
            target: start,
            lift: 0.0,
            lift_time: 0.0,
            lift_tau: 120.0,
            duck_until: 0.0,
            movie: false,
            away: false,
            sleep_since: None,
            weather: None,
            weather_affects_mood: true,
            ambience: 0.6,
            user_volume: 1.0,
            song_minutes: 12.0,
            song_started: 0.0,
            fade: 1.0,
            changing_song: false,
            seed,
            song: SongTraits::draw(&mut Rng::stream(seed, 0x50E)),
            rng: Rng::stream(seed, 0x40E),
            volume: 1.0,
            last_event: None,
        }
    }

    /// Start immediately at the right mood instead of gliding from noon.
    pub fn snap(&mut self, now: f64, hour: f32) {
        self.current = self.compute_target(now, hour);
        self.target = self.current;
        self.song_started = now;
    }

    pub fn set_scene(&mut self, scene: Scene, now: f64, minutes: Option<f32>) {
        self.scene = scene;
        self.scene_until = minutes.map(|m| now + m as f64 * 60.0);
        self.fast_until = now + 30.0;
        if scene != Scene::Sleep {
            self.sleep_since = None;
        }
    }

    pub fn handle(&mut self, ev: HomeEvent, now: f64) {
        self.last_event = Some((ev, now));
        match ev {
            HomeEvent::Arrive => {
                self.away = false;
                self.bump(0.25, 180.0, now);
            }
            HomeEvent::Leave => self.away = true,
            HomeEvent::Motion => {
                self.away = false;
                let cur = self.current_lift(now);
                if cur < 0.15 {
                    self.bump((cur + 0.08).min(0.2), 120.0, now);
                }
            }
            HomeEvent::Dinner => self.set_scene(Scene::Dinner, now, Some(90.0)),
            HomeEvent::MovieStart => self.movie = true,
            HomeEvent::MovieEnd => self.movie = false,
            HomeEvent::Bedtime => {
                self.set_scene(Scene::Sleep, now, Some(10.0 * 60.0));
                self.sleep_since = Some(now);
            }
            HomeEvent::Wake => {
                self.away = false;
                self.sleep_since = None;
                self.set_scene(Scene::Morning, now, Some(60.0));
            }
            HomeEvent::Guests => self.set_scene(Scene::Gathering, now, Some(180.0)),
            HomeEvent::Doorbell => self.duck_until = now + 20.0,
        }
    }

    fn bump(&mut self, amount: f32, tau: f32, now: f64) {
        self.lift = amount;
        self.lift_tau = tau;
        self.lift_time = now;
        self.fast_until = now + 20.0;
    }

    fn current_lift(&self, now: f64) -> f32 {
        self.lift * (-((now - self.lift_time) as f32) / self.lift_tau).exp()
    }

    fn compute_target(&self, now: f64, hour: f32) -> Mood {
        let mut m = self.scene.mood().unwrap_or_else(|| time_of_day_mood(hour));
        if let Some(w) = self.weather {
            let before = m;
            w.apply(&mut m);
            if !self.weather_affects_mood {
                m = Mood { rain: m.rain, wind: m.wind, ..before };
            }
        }
        let lift = self.current_lift(now);
        m.energy += lift;
        m.brightness += lift * 0.5;
        m.band += lift * 0.6;
        m.rain *= self.ambience;
        m.wind *= self.ambience;
        m.clamp()
    }

    /// Should the music be playing at all right now?
    pub fn should_play(&self, now: f64) -> bool {
        !self.away && !self.movie && self.sleep_gain(now) > 0.0
    }

    fn sleep_gain(&self, now: f64) -> f32 {
        match self.sleep_since {
            Some(t) => (1.0 - ((now - t) / SLEEP_FADE_S) as f32).clamp(0.0, 1.0),
            None => 1.0,
        }
    }

    /// Short human-readable status.
    pub fn summary(&self, now: f64, hour: f32) -> String {
        let mut parts = vec![if self.scene == Scene::Auto {
            format!("{} (auto)", day_phase_name(hour))
        } else {
            self.scene.name().to_string()
        }];
        if let Some(w) = self.weather {
            parts.push(w.describe());
        }
        if self.away {
            parts.push("away".into());
        }
        if self.movie {
            parts.push("paused for movie".into());
        }
        if let Some(t) = self.sleep_since {
            let left = (SLEEP_FADE_S - (now - t)).max(0.0) / 60.0;
            parts.push(format!("sleep fade {left:.0} min left"));
        }
        if self.current_lift(now) > 0.02 {
            parts.push("lifted by activity".into());
        }
        parts.join(" · ")
    }

    /// Advance by `dt` seconds. `now` is a monotonic clock in seconds, `hour` the local
    /// time of day (e.g. 18.5 = 6:30 pm). Returns the settings to play.
    pub fn update(&mut self, now: f64, hour: f32, dt: f32, base: &Settings) -> Settings {
        if let Some(until) = self.scene_until
            && now >= until
        {
            self.scene = Scene::Auto;
            self.scene_until = None;
            self.fast_until = now + 30.0;
        }
        self.target = self.compute_target(now, hour);
        let tau = if now < self.fast_until { FAST_TAU } else { SLOW_TAU };
        self.current = self.current.lerp(self.target, 1.0 - (-dt / tau).exp());

        // Volume: ducks and pauses move faster than the mood.
        let duck = if self.away || self.movie {
            0.0
        } else if now < self.duck_until {
            0.3
        } else {
            1.0
        };
        let vol_target = duck * self.sleep_gain(now);
        self.volume += (vol_target - self.volume) * (1.0 - (-dt / 2.5).exp());

        // Song rotation with a short fade so the change isn't abrupt.
        if self.song_minutes > 0.0 && !self.changing_song && now - self.song_started > self.song_minutes as f64 * 60.0 {
            self.changing_song = true;
        }
        if self.changing_song {
            self.fade -= dt / SONG_FADE_S;
            if self.fade <= 0.0 {
                self.fade = 0.0;
                self.changing_song = false;
                self.seed = self.rng.next_u64() % 1_000_000;
                self.song = SongTraits::draw(&mut self.rng);
                self.song_started = now;
            }
        } else if self.fade < 1.0 {
            self.fade = (self.fade + dt / SONG_FADE_S).min(1.0);
        }

        let mut s = mood_to_settings(&self.current, base);
        s.music.seed = self.seed;
        s.music.standard = self.song.standard;
        s.music.start_degree = self.song.start_degree;
        s.music.kutsinhira = self.song.kutsinhira;
        s.tempo = q(s.tempo * self.song.tempo, 0.01);
        s.volume = base.volume * self.user_volume * self.current.volume * self.volume * self.fade;
        s
    }

    /// Music fully faded out (for pausing playback when away / during a movie / asleep).
    pub fn is_silent(&self) -> bool {
        self.volume < 0.01
    }

    pub fn next_song(&mut self, now: f64) {
        self.song_started = now - self.song_minutes as f64 * 60.0 - 1.0;
        self.changing_song = true;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SongTraits {
    pub standard: Standard,
    /// Which scale degree the chord cycle starts on: a different "key"/mode on the same mbira.
    pub start_degree: i32,
    pub kutsinhira: KutsinhiraMode,
    /// Tempo factor around the mood's tempo.
    pub tempo: f32,
}

impl SongTraits {
    pub fn draw(rng: &mut Rng) -> Self {
        let standard = match rng.f32() {
            x if x < 0.3 => Standard::C,
            x if x < 0.55 => Standard::A,
            x if x < 0.7 => Standard::E,
            x if x < 0.85 => Standard::F,
            _ => Standard::Short,
        };
        SongTraits {
            standard,
            start_degree: rng.below(7) as i32,
            kutsinhira: if rng.chance(0.6) { KutsinhiraMode::Delay } else { KutsinhiraMode::Complement },
            tempo: 1.0 + rng.bipolar() * 0.07,
        }
    }
}

fn q(x: f32, step: f32) -> f32 {
    (x / step).round() * step
}

/// Map a mood onto engine settings, keeping the instrument the user chose in `base`.
pub fn mood_to_settings(m: &Mood, base: &Settings) -> Settings {
    let mut s = *base;
    let e = m.energy;
    s.tempo = q(3.6 + 2.0 * e, 0.01);
    s.intensity = 0.25 + 0.55 * e;
    s.touch = 0.25 + 0.4 * e;
    s.variation = m.variation;
    s.humanize = 0.6;
    s.groove = 0.5;
    s.tone_hz = q(2000.0 + 8000.0 * m.brightness.powf(1.4), 50.0);
    // Timbre changes retune the instruments, so move them in coarse steps only.
    s.timbre.brightness = q(0.25 + 0.45 * m.brightness, 0.05);
    s.timbre.buzz = q(0.15 * (e - 0.5).max(0.0), 0.05);
    s.reverb = 0.2 + 0.4 * m.space;
    s.master.saturation = q(0.1 + 0.35 * m.warmth, 0.01);
    s.master.low_db = q(0.5 + 2.0 * m.warmth, 0.1);
    s.master.air_db = q(-1.5 + 3.0 * m.brightness, 0.1);
    s.auto_layers = false;
    s.play_kushaura = true;
    s.play_kutsinhira = e > 0.35;
    s.play_hosho = e > 0.6;
    s.hosho_level = 0.15 + 0.4 * (e - 0.6).max(0.0);
    s.play_bass = m.band > 0.15;
    s.bass_level = 0.3 + 0.5 * m.band;
    s.bass_style = if e < 0.35 {
        BassStyle::Roots
    } else if e < 0.6 {
        BassStyle::Beat
    } else {
        BassStyle::Chimurenga
    };
    s.play_ngoma = m.band > 0.45;
    s.ngoma_level = ((m.band - 0.45) * 1.4).clamp(0.0, 0.8);
    s.drum_style = if m.band < 0.7 {
        DrumStyle::Sparse
    } else if e > 0.65 {
        DrumStyle::Chimurenga
    } else {
        DrumStyle::Bira
    };
    s.rain = m.rain;
    s.wind = m.wind;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(d: &mut Director, from: f64, secs: f64, hour: f32) -> Settings {
        let base = Settings::default();
        let mut s = base;
        let mut t = from;
        while t < from + secs {
            s = d.update(t, hour, 0.5, &base);
            t += 0.5;
        }
        s
    }

    #[test]
    fn night_is_calmer_than_evening() {
        let base = Settings::default();
        let night = mood_to_settings(&time_of_day_mood(2.0), &base);
        let evening = mood_to_settings(&time_of_day_mood(18.5), &base);
        assert!(night.tempo < evening.tempo);
        assert!(night.tone_hz < evening.tone_hz);
        assert!(!night.play_bass && evening.play_bass);
        assert!(time_of_day_mood(2.0).volume < time_of_day_mood(18.5).volume);
    }

    #[test]
    fn day_curve_is_continuous() {
        let mut h = 0.0;
        let mut prev = time_of_day_mood(0.0);
        while h < 24.0 {
            h += 0.05;
            let m = time_of_day_mood(h);
            assert!((m.energy - prev.energy).abs() < 0.03, "jump at {h}");
            prev = m;
        }
    }

    #[test]
    fn rain_brings_rain_sound_and_calm() {
        let mut d = Director::new(1);
        d.snap(0.0, 15.0);
        let dry = run(&mut d, 0.0, 300.0, 15.0);
        d.weather = Some(Weather { temperature_c: 18.0, precipitation: 2.0, cloud_cover: 1.0, wind_kmh: 30.0, code: 63 });
        let wet = run(&mut d, 300.0, 300.0, 15.0);
        assert!(dry.rain == 0.0 && wet.rain > 0.2, "rain {}", wet.rain);
        assert!(wet.wind > 0.0);
        assert!(wet.tempo < dry.tempo && wet.tone_hz < dry.tone_hz);
    }

    #[test]
    fn mood_glides_instead_of_jumping() {
        let mut d = Director::new(1);
        d.snap(0.0, 3.0);
        let before = run(&mut d, 0.0, 1.0, 3.0);
        d.set_scene(Scene::Gathering, 1.0, None);
        let soon = run(&mut d, 1.0, 2.0, 3.0);
        let later = run(&mut d, 3.0, 120.0, 3.0);
        assert!(soon.tempo > before.tempo && soon.tempo < later.tempo - 0.3, "{} {} {}", before.tempo, soon.tempo, later.tempo);
    }

    #[test]
    fn movie_and_leave_fade_to_silence_then_resume() {
        let mut d = Director::new(1);
        d.snap(0.0, 19.0);
        run(&mut d, 0.0, 10.0, 19.0);
        d.handle(HomeEvent::MovieStart, 10.0);
        let s = run(&mut d, 10.0, 20.0, 19.0);
        assert!(s.volume < 0.01 && d.is_silent() && !d.should_play(30.0));
        d.handle(HomeEvent::MovieEnd, 30.0);
        let s = run(&mut d, 30.0, 20.0, 19.0);
        assert!(s.volume > 0.2 && d.should_play(50.0));
        d.handle(HomeEvent::Leave, 50.0);
        run(&mut d, 50.0, 20.0, 19.0);
        assert!(d.is_silent() && !d.should_play(70.0));
        d.handle(HomeEvent::Arrive, 70.0);
        assert!(d.should_play(70.0));
    }

    #[test]
    fn bedtime_fades_out_over_half_an_hour() {
        let mut d = Director::new(1);
        d.snap(0.0, 22.0);
        d.handle(HomeEvent::Bedtime, 0.0);
        let mid = run(&mut d, 0.0, 15.0 * 60.0, 22.5);
        assert!(mid.volume > 0.02 && mid.tempo < 4.0, "vol {} tempo {}", mid.volume, mid.tempo);
        let end = run(&mut d, 900.0, 16.0 * 60.0, 23.0);
        assert!(end.volume < 0.01 && d.is_silent());
    }

    #[test]
    fn songs_rotate_with_a_fade() {
        let mut d = Director::new(1);
        d.song_minutes = 1.0;
        d.snap(0.0, 12.0);
        let first = d.seed;
        let mut min_vol = f32::MAX;
        let base = Settings::default();
        let mut t = 0.0;
        while t < 75.0 {
            min_vol = min_vol.min(d.update(t, 12.0, 0.1, &base).volume);
            t += 0.1;
        }
        assert_ne!(d.seed, first);
        assert!(min_vol < 0.01, "should fade out before changing song");
    }

    #[test]
    fn events_and_scenes_parse() {
        for e in HomeEvent::ALL {
            assert_eq!(HomeEvent::parse(e.id()), Some(e));
        }
        for s in Scene::ALL {
            assert_eq!(Scene::parse(s.id()), Some(s));
        }
    }
}
