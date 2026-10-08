//! Dev tool: how repetitive is the music?
//! Renders long stretches and records every 48-pulse cycle (which notes, drum strokes and
//! bass notes were played on which pulse). Exact repeats are rare because of live ornaments,
//! so the main measure is perceptual: how much of each cycle (Jaccard overlap of its events)
//! was already heard in the most similar earlier cycle — "déjà vu".
//! `cargo run --release --example repetition [minutes]`
use mbirasyth::engine::{Engine, Settings};
use mbirasyth::mood::Director;
use std::collections::{HashMap, HashSet};

fn jaccard(a: &HashSet<u32>, b: &HashSet<u32>) -> f32 {
    let i = a.intersection(b).count();
    let u = a.len() + b.len() - i;
    if u == 0 { 1.0 } else { i as f32 / u as f32 }
}

/// (mean déjà vu vs anything earlier, % cycles ≥ 0.8 like an earlier one,
///  mean similarity to the previous cycle, mean déjà vu within the last 5 minutes' worth)
fn deja_vu(sets: &[HashSet<u32>], recent: usize) -> (f32, f32, f32, f32) {
    let (mut sum, mut high, mut prev, mut rec) = (0.0, 0, 0.0, 0.0);
    for i in 1..sets.len() {
        let best = (0..i).map(|j| jaccard(&sets[i], &sets[j])).fold(0.0f32, f32::max);
        let best_recent = (i.saturating_sub(recent)..i).map(|j| jaccard(&sets[i], &sets[j])).fold(0.0f32, f32::max);
        sum += best;
        rec += best_recent;
        if best >= 0.8 {
            high += 1;
        }
        prev += jaccard(&sets[i], &sets[i - 1]);
    }
    let n = (sets.len() - 1).max(1) as f32;
    (sum / n, 100.0 * high as f32 / n, prev / n, rec / n)
}

struct Stats {
    cycles: usize,
    melody_repeats: usize,
    full_repeats: usize,
    distinct_melody: usize,
    longest_same_run: usize,
}

fn analyse(fps: &[(u64, u64)]) -> Stats {
    let mut seen_m: HashMap<u64, usize> = HashMap::new();
    let mut seen_f: HashMap<(u64, u64), usize> = HashMap::new();
    let (mut mr, mut fr, mut run, mut longest) = (0, 0, 1, 1);
    for (i, &(m, b)) in fps.iter().enumerate() {
        if seen_m.insert(m, i).is_some() {
            mr += 1;
        }
        if seen_f.insert((m, b), i).is_some() {
            fr += 1;
        }
        if i > 0 && fps[i - 1] == (m, b) {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 1;
        }
    }
    Stats { cycles: fps.len(), melody_repeats: mr, full_repeats: fr, distinct_melody: seen_m.len(), longest_same_run: longest }
}

fn run(name: &str, settings: Settings, minutes: f32, director: bool) {
    let sr = 48_000.0;
    let mut e = Engine::new(sr, settings);
    e.play();
    let mut d = Director::new(42);
    d.snap(0.0, 19.0);
    let mut buf = vec![0.0f32; 1024];
    let mut fps = Vec::new();
    let mut sets: Vec<HashSet<u32>> = Vec::new();
    let mut seen = 0;
    let blocks = (minutes * 60.0 * sr / 512.0) as usize;
    for i in 0..blocks {
        if director && i % 47 == 0 {
            let t = i as f64 * 512.0 / sr as f64;
            let s = d.update(t, 19.0 + (t / 3600.0) as f32, 0.5, &settings);
            e.apply_settings(s);
        }
        e.process(&mut buf, 2);
        if e.cycles != seen {
            seen = e.cycles;
            fps.push(e.last_cycle);
            sets.push(e.last_cycle_events.as_slice().iter().copied().collect());
        }
    }
    let s = analyse(&fps);
    let recent = (5.0 * 60.0 * settings.tempo / 48.0) as usize;
    let (dv, high, prev, rec) = deja_vu(&sets, recent);
    println!(
        "{name:26} {:4} cycles | exact repeats {:4.1}% | déjà vu {:.2} (5-min {:.2}) | ≥80% like earlier {:5.1}% | vs previous {:.2}",
        s.cycles,
        100.0 * s.full_repeats as f32 / s.cycles as f32,
        dv,
        rec,
        high,
        prev
    );
    let _ = (s.melody_repeats, s.distinct_melody, s.longest_same_run);
}

fn main() {
    let minutes: f32 = std::env::args().nth(1).and_then(|m| m.parse().ok()).unwrap_or(20.0);
    run("default (GUI)", Settings::default(), minutes, false);
    let mut full = Settings::default();
    full.intensity = 0.85;
    run("full band", full, minutes, false);
    run("home director (evening)", Settings::default(), minutes * 3.0, true);
}
