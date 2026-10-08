//! Dev tool: RMS / peak of each layer alone, to balance the mix.
//! `cargo run --release --example stems [dir-to-write-wavs]`
use mbirasyth::engine::{Engine, Settings};

fn measure(s: Settings) -> (f32, f32) {
    let sr = 48_000.0;
    let mut e = Engine::new(sr, s);
    e.play();
    let mut buf = vec![0.0f32; 2048];
    let (mut sum, mut n, mut peak) = (0.0f64, 0usize, 0.0f32);
    for _ in 0..(sr as usize * 30 / 1024) {
        e.process(&mut buf, 2);
        for &x in &buf {
            sum += (x * x) as f64;
            n += 1;
            peak = peak.max(x.abs());
        }
    }
    (10.0 * (sum / n as f64).log10() as f32, peak)
}

fn main() {
    let mut base = Settings::default();
    base.auto_layers = false;
    base.reverb = 0.0;
    let off = |mut s: Settings| {
        s.play_kushaura = false;
        s.play_kutsinhira = false;
        s.play_hosho = false;
        s.play_ngoma = false;
        s.play_bass = false;
        s
    };
    let layers: [(&str, fn(&mut Settings)); 8] = [
        ("rain 0.6", |s| s.rain = 0.6),
        ("wind 0.6", |s| s.wind = 0.6),
        ("kushaura", |s| s.play_kushaura = true),
        ("hosho", |s| s.play_hosho = true),
        ("ngoma", |s| s.play_ngoma = true),
        ("bass", |s| s.play_bass = true),
        ("all", |s| {
            s.play_kushaura = true;
            s.play_kutsinhira = true;
            s.play_hosho = true;
            s.play_ngoma = true;
            s.play_bass = true;
        }),
        ("default", |_| {}),
    ];
    for (name, f) in layers {
        let mut s = off(base);
        f(&mut s);
        if name == "default" {
            s = Settings::default();
        }
        if let Some(dir) = std::env::args().nth(1) {
            let path = std::path::Path::new(&dir).join(format!("{name}.wav"));
            mbirasyth::render::render_wav(s, 30.0, 48_000, 0.0, &path, None, mbirasyth::render::LoudnessTarget::Off).unwrap();
        }
        let (rms, peak) = measure(s);
        println!("{name:10} rms {rms:6.1} dBFS  peak {peak:.2}");
    }
}
