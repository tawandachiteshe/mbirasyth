use mbirasyth::app::MbiraApp;
use mbirasyth::engine::Settings;
use mbirasyth::render::{render_wav, LoudnessTarget};
use std::path::PathBuf;

const USAGE: &str = "usage:
  mbirasyth [--play]                          open the GUI (--play starts the music)
  mbirasyth render <out.wav> [seconds] [song-seed] [intensity] [target-lufs]   headless render
  mbirasyth home [--city NAME] [--listen ADDR] [--scene NAME]   headless adaptive home music
      (time of day + weather + HTTP API on 127.0.0.1:7777; see /status)";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("render") => {
            let path = PathBuf::from(args.get(1).ok_or(USAGE)?);
            let seconds: f32 = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(30.0);
            let mut settings = Settings::default();
            if let Some(seed) = args.get(3) {
                settings.music.seed = seed.parse()?;
            }
            if let Some(i) = args.get(4) {
                settings.intensity = i.parse()?;
            }
            let target = match args.get(5) {
                Some(l) => LoudnessTarget::Custom(l.parse()?),
                None => LoudnessTarget::Off,
            };
            let r = render_wav(settings, seconds, 48_000, 3.0, &path, None, target)?;
            println!(
                "wrote {}  ({:.1} LUFS integrated, peak {:.1} dBFS, gain {:+.1} dB)",
                path.display(),
                r.loudness.integrated,
                r.loudness.peak_db,
                r.gain_db
            );
            Ok(())
        }
        Some("home") => home(&args[1..]),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => {
            let autoplay = args.iter().any(|a| a == "--play");
            let options = eframe::NativeOptions {
                viewport: eframe::egui::ViewportBuilder::default()
                    .with_inner_size([1500.0, 860.0])
                    .with_min_inner_size([1000.0, 600.0])
                    .with_title("mbirasyth — procedural mbira"),
                ..Default::default()
            };
            eframe::run_native("mbirasyth", options, Box::new(move |cc| Ok(Box::new(MbiraApp::new(cc, autoplay)))))?;
            Ok(())
        }
    }
}

fn home(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use mbirasyth::home::{self, HomeConfig, HomeMsg};
    use mbirasyth::mood::Scene;

    let mut config = HomeConfig::load();
    let mut scene = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--city" => {
                let city = it.next().ok_or(USAGE)?;
                let (name, lat, lon) = home::geocode(city)?;
                println!("📍 {name} ({lat:.2}, {lon:.2})");
                config.city = name;
                config.latitude = Some(lat);
                config.longitude = Some(lon);
                config.weather = true;
                config.save()?;
            }
            "--listen" => config.listen = it.next().ok_or(USAGE)?.clone(),
            "--scene" => scene = Some(Scene::parse(it.next().ok_or(USAGE)?).ok_or("unknown scene")?),
            _ => return Err(USAGE.into()),
        }
    }
    let settings = Settings::default();
    let (audio, mut link) = mbirasyth::audio::start(settings)?;
    println!("🔊 {} @ {} Hz", audio.device_name, audio.sample_rate);
    let h = home::start(link.tx.clone(), settings, config, true);
    if let Some(sc) = scene {
        h.tx.send(HomeMsg::Scene(sc, None))?;
    }
    println!("🏠 home music running. Ctrl+C to stop.");
    // First status line after the director has settled, then once a minute.
    let mut last_print = std::time::Instant::now() - std::time::Duration::from_secs(57);
    loop {
        while link.rx.pop().is_ok() {}
        if last_print.elapsed().as_secs() >= 60 {
            last_print = std::time::Instant::now();
            let st = h.status.lock().unwrap().clone();
            println!("{} · {} · tempo {:.1} · song #{}", st.api, st.summary, st.settings.tempo, st.settings.music.seed);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
