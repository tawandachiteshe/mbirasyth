//! Dev tool: look up a city and print the current weather and how it would colour the mood.
//! `cargo run --release --example weather_check -- "Harare"`
use mbirasyth::home::{fetch_weather, geocode};
use mbirasyth::mood::time_of_day_mood;

fn main() -> Result<(), String> {
    let city = std::env::args().nth(1).unwrap_or_else(|| "Harare".into());
    let (name, lat, lon) = geocode(&city)?;
    let w = fetch_weather(lat, lon)?;
    let mut m = time_of_day_mood(15.0);
    w.apply(&mut m);
    println!("{name} ({lat:.2}, {lon:.2}): {}  →  rain {:.2}, wind {:.2}", w.describe(), m.rain, m.wind);
    Ok(())
}
