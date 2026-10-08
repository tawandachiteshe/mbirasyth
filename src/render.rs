//! Offline rendering to WAV (deterministic: same settings → same file).

use crate::engine::{Engine, Settings};
use crate::master::LoudnessReading;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// Common loudness targets (integrated LUFS).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoudnessTarget {
    /// As mixed: no normalisation.
    Off,
    /// Spotify / YouTube / Apple Music normalise to about -14 LUFS.
    Streaming,
    /// ASWG-R001 game audio guideline for mobile / handheld.
    GameMobile,
    /// ASWG-R001 game audio guideline for home console / PC (-24 LUFS ±2).
    GameConsole,
    /// EBU R128 broadcast.
    Broadcast,
    Custom(f32),
}

impl LoudnessTarget {
    pub const PRESETS: [LoudnessTarget; 5] = [
        LoudnessTarget::Off,
        LoudnessTarget::Streaming,
        LoudnessTarget::GameMobile,
        LoudnessTarget::GameConsole,
        LoudnessTarget::Broadcast,
    ];

    pub fn lufs(self) -> Option<f32> {
        match self {
            LoudnessTarget::Off => None,
            LoudnessTarget::Streaming => Some(-14.0),
            LoudnessTarget::GameMobile => Some(-18.0),
            LoudnessTarget::GameConsole => Some(-24.0),
            LoudnessTarget::Broadcast => Some(-23.0),
            LoudnessTarget::Custom(l) => Some(l),
        }
    }

    pub fn name(self) -> String {
        match self {
            LoudnessTarget::Off => "Off (as mixed)".into(),
            LoudnessTarget::Streaming => "Streaming  -14 LUFS".into(),
            LoudnessTarget::GameMobile => "Game, mobile  -18 LUFS".into(),
            LoudnessTarget::GameConsole => "Game, console/PC  -24 LUFS".into(),
            LoudnessTarget::Broadcast => "Broadcast (EBU R128)  -23 LUFS".into(),
            LoudnessTarget::Custom(l) => format!("Custom  {l:.0} LUFS"),
        }
    }
}

/// What the exported file measures.
#[derive(Clone, Copy, Debug)]
pub struct RenderReport {
    pub loudness: LoudnessReading,
    /// Gain applied to reach the target, dB.
    pub gain_db: f32,
}

fn run(
    settings: Settings,
    total: usize,
    sample_rate: u32,
    post_gain_db: f32,
    mut sink: impl FnMut(usize, &[f32]) -> Result<(), String>,
    mut on_progress: impl FnMut(usize),
) -> Result<Engine, String> {
    let mut engine = Engine::new(sample_rate as f32, settings);
    engine.set_post_gain_db(post_gain_db);
    engine.play();
    let mut buf = vec![0.0f32; 2048];
    let mut done = 0usize;
    while done < total {
        engine.process(&mut buf, 2);
        for frame in buf.chunks(2) {
            if done >= total {
                break;
            }
            sink(done, frame)?;
            done += 1;
        }
        on_progress(done);
    }
    Ok(engine)
}

/// Render `seconds` of generated music to a 16-bit stereo WAV.
/// `progress` receives 0..=1000 (per-mille) as rendering advances.
/// With a loudness `target`, a first pass measures the integrated loudness (the engine is
/// deterministic, so the second pass is the same music) and the second pass is rendered
/// with the gain that reaches the target, still through the limiter.
pub fn render_wav(
    settings: Settings,
    seconds: f32,
    sample_rate: u32,
    fade_out: f32,
    path: &Path,
    progress: Option<&AtomicU32>,
    target: LoudnessTarget,
) -> Result<RenderReport, String> {
    let total = (seconds.max(0.1) * sample_rate as f32) as usize;
    let passes = if target.lufs().is_some() { 2 } else { 1 };
    let report_progress = |pass: usize, done: usize| {
        if let Some(p) = progress {
            let x = (pass * total + done) as u64 * 1000 / (passes * total) as u64;
            p.store(x as u32, Ordering::Relaxed);
        }
    };

    let mut gain_db = 0.0;
    if let Some(lufs) = target.lufs() {
        let engine = run(settings, total, sample_rate, 0.0, |_, _| Ok(()), |d| report_progress(0, d))?;
        let measured = engine.loudness().integrated;
        if measured > -70.0 {
            gain_db = (lufs - measured).clamp(-40.0, 30.0);
        }
    }

    let spec = hound::WavSpec { channels: 2, sample_rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    let fade_start = total.saturating_sub((fade_out.max(0.0) * sample_rate as f32) as usize);
    let engine = run(
        settings,
        total,
        sample_rate,
        gain_db,
        |i, frame| {
            let g = if i >= fade_start && total > fade_start {
                1.0 - (i - fade_start) as f32 / (total - fade_start) as f32
            } else {
                1.0
            };
            for &x in frame {
                let v = (x * g * i16::MAX as f32).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                writer.write_sample(v).map_err(|e| e.to_string())?;
            }
            Ok(())
        },
        |d| report_progress(passes - 1, d),
    )?;
    writer.finalize().map_err(|e| e.to_string())?;
    Ok(RenderReport { loudness: engine.loudness(), gain_db })
}
