//! Procedural weather ambience: rain (hiss + individual drops) and wind (slowly
//! moving band of noise with gusts). Stereo, allocation-free, no samples.

use crate::dsp::{decay_coef, pan_gains, Biquad, Mode};
use crate::rng::Rng;

const DROPS: usize = 12;

#[derive(Clone, Copy, Default)]
struct Drop {
    mode: Mode,
    /// Remaining frequency sweep: drops chirp upwards as the bubble resonates.
    sweep: f32,
    base: f32,
    pan: (f32, f32),
}

pub struct Ambience {
    sr: f32,
    rng: Rng,
    // Rain
    rain_hp: [Biquad; 2],
    rain_lp: [Biquad; 2],
    rumble: [Biquad; 2],
    drops: [Drop; DROPS],
    next_drop: usize,
    sweep_coef: f32,
    // Wind
    wind_bp: [Biquad; 2],
    wind_phase: f32,
    gust: f32,
    gust_target: f32,
    counter: u32,
    // Levels, smoothed toward the targets so weather changes fade in and out.
    rain: f32,
    wind: f32,
    rain_target: f32,
    wind_target: f32,
    smooth: f32,
}

impl Ambience {
    pub fn new(sr: f32, seed: u64) -> Self {
        Ambience {
            sr,
            rng: Rng::stream(seed, 0xA4B),
            rain_hp: [Biquad::highpass(500.0, 0.7, sr); 2],
            rain_lp: [Biquad::lowpass(7000.0, 0.7, sr); 2],
            rumble: [Biquad::lowpass(180.0, 0.7, sr); 2],
            drops: [Drop::default(); DROPS],
            next_drop: 0,
            sweep_coef: decay_coef(0.02, sr),
            wind_bp: [Biquad::bandpass(400.0, 1.2, sr); 2],
            wind_phase: 0.0,
            gust: 0.0,
            gust_target: 0.0,
            counter: 0,
            rain: 0.0,
            wind: 0.0,
            rain_target: 0.0,
            wind_target: 0.0,
            smooth: 1.0 - decay_coef(4.0, sr),
        }
    }

    /// Target levels, 0..1. Changes fade in over a few seconds.
    pub fn set_levels(&mut self, rain: f32, wind: f32) {
        self.rain_target = rain.clamp(0.0, 1.0);
        self.wind_target = wind.clamp(0.0, 1.0);
    }

    fn spawn_drop(&mut self) {
        let i = self.next_drop;
        self.next_drop = (i + 1) % DROPS;
        let f = self.rng.range(1800.0, 4800.0);
        let d = &mut self.drops[i];
        d.base = f;
        d.sweep = self.rng.range(0.1, 0.35);
        d.mode.set(f * (1.0 - d.sweep), self.rng.range(0.01, 0.03), self.sr);
        d.mode.strike(self.rng.range(0.2, 1.0), 0.0);
        d.pan = pan_gains(self.rng.bipolar() * 0.9);
    }

    #[inline]
    pub fn tick(&mut self) -> (f32, f32) {
        self.rain += (self.rain_target - self.rain) * self.smooth;
        self.wind += (self.wind_target - self.wind) * self.smooth;
        if self.rain < 1e-4 && self.wind < 1e-4 {
            return (0.0, 0.0);
        }
        let mut out = [0.0f32; 2];

        if self.rain > 1e-4 {
            // Steady hiss: decorrelated noise per channel, shaped like distant rain on a roof.
            let r = self.rain;
            for ch in 0..2 {
                let n = self.rng.bipolar();
                let hiss = self.rain_lp[ch].tick(self.rain_hp[ch].tick(n));
                let low = self.rumble[ch].tick(self.rng.bipolar());
                out[ch] += hiss * 0.25 * r + low * 0.3 * r * r;
            }
            // Individual drops: a Poisson stream, denser in heavier rain.
            let rate = 6.0 + 140.0 * r * r;
            if self.rng.f32() < rate / self.sr {
                self.spawn_drop();
            }
            for d in self.drops.iter_mut() {
                if d.mode.enabled && d.mode.energy() > 1e-9 {
                    if d.sweep > 1e-3 {
                        d.sweep *= self.sweep_coef;
                        d.mode.set_freq(d.base * (1.0 - d.sweep), self.sr);
                    }
                    let y = d.mode.tick(0.0) * 0.1 * r.sqrt();
                    out[0] += y * d.pan.0;
                    out[1] += y * d.pan.1;
                }
            }
        }

        if self.wind > 1e-4 {
            // Gusts: a slowly wandering target, re-chosen every ~1.5 s.
            self.counter += 1;
            if self.counter as f32 > self.sr * 1.5 {
                self.counter = 0;
                self.gust_target = self.rng.range(0.2, 1.0);
            }
            self.gust += (self.gust_target - self.gust) * (1.0 / (self.sr * 1.2));
            if self.counter % 64 == 0 {
                self.wind_phase += 64.0 / self.sr * 0.07;
                let centre = 250.0 + 500.0 * self.gust + 120.0 * (self.wind_phase * std::f32::consts::TAU).sin();
                for (ch, bp) in self.wind_bp.iter_mut().enumerate() {
                    bp.retune(Biquad::bandpass(centre * (1.0 + 0.08 * ch as f32), 1.4, self.sr));
                }
            }
            let w = self.wind * (0.4 + 0.6 * self.gust);
            for ch in 0..2 {
                out[ch] += self.wind_bp[ch].tick(self.rng.bipolar()) * 0.7 * w;
            }
        }
        (out[0], out[1])
    }
}
