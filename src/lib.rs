//! mbirasyth — procedural mbira synthesis and composition.
//!
//! The library has no GUI or audio-backend dependency in its core: [`engine::Engine`]
//! fills `f32` buffers. See `docs/` for the research behind every module.

pub mod ambience;
pub mod band;
pub mod compose;
pub mod dsp;
pub mod engine;
pub mod instrument;
pub mod master;
pub mod mood;
pub mod render;
pub mod rng;
pub mod tuning;

#[cfg(feature = "gui")]
pub mod app;
#[cfg(any(feature = "gui", feature = "home"))]
pub mod audio;
#[cfg(feature = "home")]
pub mod home;
