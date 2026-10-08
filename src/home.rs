//! The home player: runs the [`Director`](crate::mood::Director) on its own thread, fetches
//! the weather (Open-Meteo, free, no key), and serves a small local HTTP API so
//! Home Assistant, scripts or a phone can tell the music what's happening.
//!
//! ```text
//! GET /status                      JSON: scene, mood, weather, volume…
//! GET /scene/<auto|morning|focus|dinner|unwind|sleep|gathering>[?minutes=90]
//! GET /event/<arrive|leave|motion|dinner|movie_start|movie_end|bedtime|wake|guests|doorbell>
//! GET /volume/<0-100>   GET /next   GET /pause   GET /resume
//! ```
//! POST works too. By default it only listens on this computer (127.0.0.1:7777).

use crate::engine::{Command, Settings};
use crate::mood::{Director, HomeEvent, Mood, Scene, Weather};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type CommandTx = Arc<Mutex<rtrb::Producer<Command>>>;

pub const DEFAULT_LISTEN: &str = "127.0.0.1:7777";
const WEATHER_EVERY: Duration = Duration::from_secs(15 * 60);

// ---------------------------------------------------------------------------
// Config (~/.config/mbirasyth/home.cfg, simple key = value lines)

#[derive(Clone, Debug, PartialEq)]
pub struct HomeConfig {
    pub city: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub weather: bool,
    pub listen: String,
    pub ambience: f32,
    pub song_minutes: f32,
}

impl Default for HomeConfig {
    fn default() -> Self {
        HomeConfig {
            city: String::new(),
            latitude: None,
            longitude: None,
            weather: false,
            listen: DEFAULT_LISTEN.into(),
            ambience: 0.6,
            song_minutes: 12.0,
        }
    }
}

impl HomeConfig {
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("mbirasyth").join("home.cfg"))
    }

    pub fn load() -> HomeConfig {
        let mut c = HomeConfig::default();
        let Some(text) = HomeConfig::path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return c;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "city" => c.city = v.to_string(),
                "latitude" => c.latitude = v.parse().ok(),
                "longitude" => c.longitude = v.parse().ok(),
                "weather" => c.weather = v == "true",
                "listen" => c.listen = v.to_string(),
                "ambience" => c.ambience = v.parse().unwrap_or(c.ambience),
                "song_minutes" => c.song_minutes = v.parse().unwrap_or(c.song_minutes),
                _ => {}
            }
        }
        c
    }

    pub fn save(&self) -> Result<(), String> {
        let path = HomeConfig::path().ok_or("no home directory")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let opt = |x: Option<f64>| x.map(|v| format!("{v:.4}")).unwrap_or_default();
        let text = format!(
            "city = {}\nlatitude = {}\nlongitude = {}\nweather = {}\nlisten = {}\nambience = {}\nsong_minutes = {}\n",
            self.city,
            opt(self.latitude),
            opt(self.longitude),
            self.weather,
            self.listen,
            self.ambience,
            self.song_minutes
        );
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Weather (Open-Meteo)

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let body = ureq::get(url)
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

/// Look up a place name. Returns ("Harare, Zimbabwe", lat, lon).
pub fn geocode(city: &str) -> Result<(String, f64, f64), String> {
    let url = format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&format=json", url_encode(city.trim()));
    let v = get_json(&url)?;
    let r = v["results"].get(0).ok_or_else(|| format!("couldn't find \"{city}\""))?;
    let name = format!("{}, {}", r["name"].as_str().unwrap_or(city), r["country"].as_str().unwrap_or(""));
    let lat = r["latitude"].as_f64().ok_or("bad latitude")?;
    let lon = r["longitude"].as_f64().ok_or("bad longitude")?;
    Ok((name, lat, lon))
}

pub fn fetch_weather(lat: f64, lon: f64) -> Result<Weather, String> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
         &current=temperature_2m,precipitation,cloud_cover,wind_speed_10m,weather_code"
    );
    let v = get_json(&url)?;
    let c = &v["current"];
    let f = |k: &str| c[k].as_f64().map(|x| x as f32).ok_or_else(|| format!("missing {k}"));
    Ok(Weather {
        temperature_c: f("temperature_2m")?,
        precipitation: f("precipitation")?,
        cloud_cover: f("cloud_cover")? / 100.0,
        wind_kmh: f("wind_speed_10m")?,
        code: f("weather_code")? as u32,
    })
}

// ---------------------------------------------------------------------------
// Controller

pub enum HomeMsg {
    Active(bool),
    Scene(Scene, Option<f32>),
    Event(HomeEvent),
    Volume(f32),
    Ambience(f32),
    WeatherAffectsMood(bool),
    SongMinutes(f32),
    NextSong,
    Pause(bool),
    /// The instrument / mix the user chose; the director steers everything else.
    Base(Settings),
    Config(HomeConfig),
}

#[derive(Clone)]
pub struct HomeStatus {
    pub active: bool,
    pub summary: String,
    pub scene: Scene,
    pub mood: Mood,
    pub target: Mood,
    pub weather: Option<Weather>,
    pub weather_note: String,
    pub location: String,
    pub api: String,
    pub settings: Settings,
    pub playing: bool,
    pub paused: bool,
    pub volume: f32,
    pub log: Vec<String>,
}

pub struct HomeHandle {
    pub tx: Sender<HomeMsg>,
    pub status: Arc<Mutex<HomeStatus>>,
}

fn local_hour() -> f32 {
    use chrono::Timelike;
    let t = chrono::Local::now();
    t.hour() as f32 + t.minute() as f32 / 60.0 + t.second() as f32 / 3600.0
}

fn clock() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

struct Shared {
    status: Arc<Mutex<HomeStatus>>,
    config: Arc<Mutex<HomeConfig>>,
}

/// Start the home player threads (director, weather, HTTP API).
pub fn start(cmd: CommandTx, base: Settings, config: HomeConfig, active: bool) -> HomeHandle {
    let (tx, rx) = mpsc::channel();
    let status = Arc::new(Mutex::new(HomeStatus {
        active,
        summary: String::new(),
        scene: Scene::Auto,
        mood: crate::mood::time_of_day_mood(local_hour()),
        target: crate::mood::time_of_day_mood(local_hour()),
        weather: None,
        weather_note: String::new(),
        location: config.city.clone(),
        api: String::new(),
        settings: base,
        playing: false,
        paused: false,
        volume: 1.0,
        log: Vec::new(),
    }));
    let shared = Shared { status: status.clone(), config: Arc::new(Mutex::new(config)) };
    let weather: Arc<Mutex<Option<Weather>>> = Arc::new(Mutex::new(None));

    {
        let (st, cfg, w) = (shared.status.clone(), shared.config.clone(), weather.clone());
        std::thread::Builder::new().name("home-weather".into()).spawn(move || weather_loop(st, cfg, w)).ok();
    }
    {
        let (st, cfg, tx) = (shared.status.clone(), shared.config.clone(), tx.clone());
        std::thread::Builder::new().name("home-api".into()).spawn(move || api_loop(st, cfg, tx)).ok();
    }
    std::thread::Builder::new()
        .name("home-director".into())
        .spawn(move || director_loop(cmd, base, active, rx, shared, weather))
        .ok();
    HomeHandle { tx, status }
}

fn log(status: &Arc<Mutex<HomeStatus>>, msg: String) {
    let mut s = status.lock().unwrap();
    s.log.push(format!("{} {msg}", clock()));
    let n = s.log.len();
    if n > 12 {
        s.log.drain(..n - 12);
    }
}

fn director_loop(
    cmd: CommandTx,
    mut base: Settings,
    mut active: bool,
    rx: Receiver<HomeMsg>,
    shared: Shared,
    weather: Arc<Mutex<Option<Weather>>>,
) {
    let t0 = Instant::now();
    let now = || t0.elapsed().as_secs_f64();
    let mut director = Director::new(crate::rng::Rng::new(t0.elapsed().as_nanos() as u64 ^ 0x5EED).next_u64() % 1_000_000);
    {
        let c = shared.config.lock().unwrap();
        director.ambience = c.ambience;
        director.song_minutes = c.song_minutes;
    }
    director.snap(now(), local_hour());
    let send = |c: Command| {
        if let Ok(mut tx) = cmd.lock() {
            let _ = tx.push(c);
        }
    };
    let mut last_sent: Option<Settings> = None;
    let mut playing = false;
    let mut paused = false;
    let mut last = now();
    loop {
        while let Ok(msg) = rx.try_recv() {
            let t = now();
            match msg {
                HomeMsg::Active(a) => {
                    if a && !active {
                        director.snap(t, local_hour());
                        last_sent = None;
                    }
                    active = a;
                }
                HomeMsg::Scene(s, minutes) => {
                    director.set_scene(s, t, minutes);
                    log(&shared.status, format!("scene → {}", s.name()));
                }
                HomeMsg::Event(e) => {
                    director.handle(e, t);
                    log(&shared.status, format!("event: {}", e.id()));
                }
                HomeMsg::Volume(v) => director.user_volume = v.clamp(0.0, 1.5),
                HomeMsg::Ambience(a) => {
                    director.ambience = a.clamp(0.0, 1.0);
                    shared.config.lock().unwrap().ambience = director.ambience;
                }
                HomeMsg::WeatherAffectsMood(b) => director.weather_affects_mood = b,
                HomeMsg::SongMinutes(m) => {
                    director.song_minutes = m.max(0.0);
                    shared.config.lock().unwrap().song_minutes = director.song_minutes;
                }
                HomeMsg::NextSong => {
                    director.next_song(t);
                    log(&shared.status, "next song".into());
                }
                HomeMsg::Pause(p) => {
                    paused = p;
                    log(&shared.status, if p { "paused".into() } else { "resumed".into() });
                }
                HomeMsg::Base(s) => base = s,
                HomeMsg::Config(c) => {
                    director.ambience = c.ambience;
                    director.song_minutes = c.song_minutes;
                    *shared.config.lock().unwrap() = c;
                }
            }
        }

        let t = now();
        let dt = (t - last).clamp(0.0, 1.0) as f32;
        last = t;
        let hour = local_hour();
        director.weather = *weather.lock().unwrap();
        let settings = director.update(t, hour, dt, &base);
        if active {
            if last_sent != Some(settings) {
                send(Command::Settings(settings));
                last_sent = Some(settings);
            }
            let want = director.should_play(t) && !paused;
            if want && !playing {
                send(Command::Play);
                playing = true;
            } else if !want && playing && (director.is_silent() || paused) {
                send(Command::Stop);
                playing = false;
            }
        }
        {
            let mut st = shared.status.lock().unwrap();
            st.active = active;
            st.summary = director.summary(t, hour);
            st.scene = director.scene;
            st.mood = director.current;
            st.target = director.target;
            st.weather = director.weather;
            st.settings = settings;
            st.playing = playing;
            st.paused = paused;
            st.volume = director.user_volume;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn weather_loop(status: Arc<Mutex<HomeStatus>>, config: Arc<Mutex<HomeConfig>>, out: Arc<Mutex<Option<Weather>>>) {
    let mut last_fetch: Option<(Instant, Option<(f64, f64)>)> = None;
    loop {
        let c = config.lock().unwrap().clone();
        let coords = c.latitude.zip(c.longitude);
        if !c.weather || coords.is_none() {
            *out.lock().unwrap() = None;
            status.lock().unwrap().weather_note =
                if c.weather { "Set your city to get the weather".into() } else { "Weather off".into() };
            last_fetch = None;
        } else {
            let due = match last_fetch {
                None => true,
                Some((t, at)) => at != coords || t.elapsed() >= WEATHER_EVERY,
            };
            if due {
                let (lat, lon) = coords.unwrap();
                let note = match fetch_weather(lat, lon) {
                    Ok(w) => {
                        *out.lock().unwrap() = Some(w);
                        format!("Updated {} · {}", clock(), w.describe())
                    }
                    Err(e) => format!("Weather unavailable ({e}); retrying in 15 min"),
                };
                status.lock().unwrap().weather_note = note;
                last_fetch = Some((Instant::now(), coords));
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

// ---------------------------------------------------------------------------
// HTTP API

fn status_json(st: &HomeStatus) -> serde_json::Value {
    let m = st.mood;
    serde_json::json!({
        "active": st.active,
        "playing": st.playing,
        "paused": st.paused,
        "scene": st.scene.id(),
        "summary": st.summary,
        "volume": (st.volume * 100.0).round(),
        "mood": {
            "energy": m.energy, "brightness": m.brightness, "warmth": m.warmth, "space": m.space,
            "band": m.band, "rain": m.rain, "wind": m.wind,
        },
        "tempo": st.settings.tempo,
        "song": st.settings.music.seed,
        "weather": st.weather.map(|w| w.describe()),
        "scenes": Scene::ALL.iter().map(|s| s.id()).collect::<Vec<_>>(),
        "events": HomeEvent::ALL.iter().map(|e| e.id()).collect::<Vec<_>>(),
    })
}

/// Route one request. Returns (HTTP status, JSON body).
fn route(method: &str, target: &str, status: &Arc<Mutex<HomeStatus>>, tx: &Sender<HomeMsg>) -> (u16, serde_json::Value) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let param = |name: &str| {
        query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v.to_string()))
    };
    let parts: Vec<&str> = path.trim_matches('/').split('/').filter(|p| !p.is_empty()).collect();
    if method != "GET" && method != "POST" {
        return (405, serde_json::json!({"ok": false, "error": "use GET or POST"}));
    }
    let msg = match parts.as_slice() {
        [] | ["status"] => None,
        ["scene", s] => match Scene::parse(s) {
            Some(sc) => Some(HomeMsg::Scene(sc, param("minutes").and_then(|m| m.parse().ok()))),
            None => return (404, serde_json::json!({"ok": false, "error": format!("unknown scene {s}")})),
        },
        ["event", e] => match HomeEvent::parse(e) {
            Some(ev) => Some(HomeMsg::Event(ev)),
            None => return (404, serde_json::json!({"ok": false, "error": format!("unknown event {e}")})),
        },
        ["volume", v] => match v.parse::<f32>() {
            Ok(v) => Some(HomeMsg::Volume(v / 100.0)),
            Err(_) => return (400, serde_json::json!({"ok": false, "error": "volume is 0-100"})),
        },
        ["next"] => Some(HomeMsg::NextSong),
        ["pause"] => Some(HomeMsg::Pause(true)),
        ["resume"] | ["play"] => Some(HomeMsg::Pause(false)),
        _ => return (404, serde_json::json!({"ok": false, "error": "not found; try /status"})),
    };
    if let Some(m) = msg {
        let _ = tx.send(m);
        // Give the director a moment so the reply reflects the change.
        std::thread::sleep(Duration::from_millis(250));
    }
    let st = status.lock().unwrap();
    let mut body = status_json(&st);
    body["ok"] = serde_json::Value::Bool(true);
    (200, body)
}

fn handle(stream: TcpStream, status: &Arc<Mutex<HomeStatus>>, tx: &Sender<HomeMsg>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    // Drain headers (we don't need them or the body).
    let mut h = String::new();
    while reader.read_line(&mut h).map(|n| n > 2).unwrap_or(false) {
        h.clear();
    }
    let mut it = line.split_whitespace();
    let (method, target) = (it.next().unwrap_or(""), it.next().unwrap_or("/"));
    let (code, body) = route(method, target, status, tx);
    let body = serde_json::to_string_pretty(&body).unwrap_or_default();
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    let resp = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut s = &stream;
    let _ = s.write_all(resp.as_bytes());
}

fn api_loop(status: Arc<Mutex<HomeStatus>>, config: Arc<Mutex<HomeConfig>>, tx: Sender<HomeMsg>) {
    let mut bound: Option<(String, TcpListener)> = None;
    loop {
        let want = config.lock().unwrap().listen.clone();
        if bound.as_ref().map(|(a, _)| a != &want).unwrap_or(true) {
            bound = None;
            match TcpListener::bind(&want) {
                Ok(l) => {
                    let _ = l.set_nonblocking(true);
                    status.lock().unwrap().api = format!("http://{want}");
                    bound = Some((want.clone(), l));
                }
                Err(e) => {
                    status.lock().unwrap().api = format!("can't listen on {want}: {e}");
                    std::thread::sleep(Duration::from_secs(3));
                    continue;
                }
            }
        }
        let (_, listener) = bound.as_ref().unwrap();
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                handle(stream, &status, &tx);
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_parse_requests() {
        let (tx, rx) = mpsc::channel();
        let st = Arc::new(Mutex::new(HomeStatus {
            active: true,
            summary: String::new(),
            scene: Scene::Auto,
            mood: crate::mood::time_of_day_mood(12.0),
            target: crate::mood::time_of_day_mood(12.0),
            weather: None,
            weather_note: String::new(),
            location: String::new(),
            api: String::new(),
            settings: Settings::default(),
            playing: false,
            paused: false,
            volume: 1.0,
            log: vec![],
        }));
        assert_eq!(route("GET", "/status", &st, &tx).0, 200);
        assert_eq!(route("POST", "/scene/dinner?minutes=30", &st, &tx).0, 200);
        assert!(matches!(rx.try_recv(), Ok(HomeMsg::Scene(Scene::Dinner, Some(m))) if m == 30.0));
        assert_eq!(route("GET", "/event/movie_start", &st, &tx).0, 200);
        assert!(matches!(rx.try_recv(), Ok(HomeMsg::Event(HomeEvent::MovieStart))));
        assert_eq!(route("GET", "/volume/40", &st, &tx).0, 200);
        assert!(matches!(rx.try_recv(), Ok(HomeMsg::Volume(v)) if (v - 0.4).abs() < 1e-6));
        assert_eq!(route("GET", "/scene/disco", &st, &tx).0, 404);
        assert_eq!(route("DELETE", "/next", &st, &tx).0, 405);
    }

    #[test]
    fn url_encoding() {
        assert_eq!(url_encode("São Paulo"), "S%C3%A3o%20Paulo");
    }
}
