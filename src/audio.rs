//! Live audio output via cpal. The engine lives inside the audio callback and is
//! controlled through lock-free queues.

use crate::engine::{Command, Engine, Settings, UiEvent};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

pub struct AudioOut {
    _stream: cpal::Stream,
    pub sample_rate: u32,
    pub device_name: String,
}

pub struct Link {
    /// Shared so the home player thread can steer the engine too.
    pub tx: std::sync::Arc<std::sync::Mutex<rtrb::Producer<Command>>>,
    pub rx: rtrb::Consumer<UiEvent>,
}

pub fn start(settings: Settings) -> Result<(AudioOut, Link), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device found")?;
    let device_name = device.id().map(|id| id.to_string()).unwrap_or_else(|_| "default".into());
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let sample_rate = config.sample_rate as u32;

    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::<Command>::new(256);
    let (ui_tx, ui_rx) = rtrb::RingBuffer::<UiEvent>::new(4096);
    let engine = Engine::new(sample_rate as f32, settings).with_queues(cmd_rx, ui_tx);

    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, engine),
        SampleFormat::I16 => build::<i16>(&device, config, engine),
        SampleFormat::I32 => build::<i32>(&device, config, engine),
        SampleFormat::U16 => build::<u16>(&device, config, engine),
        SampleFormat::F64 => build::<f64>(&device, config, engine),
        other => return Err(format!("unsupported sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((AudioOut { _stream: stream, sample_rate, device_name }, Link { tx: std::sync::Arc::new(std::sync::Mutex::new(cmd_tx)), rx: ui_rx }))
}

fn build<T>(device: &cpal::Device, config: StreamConfig, mut engine: Engine) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    // Pre-allocated scratch; only grows if the backend asks for an unusually large block.
    let mut scratch = vec![0.0f32; 16_384];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                if scratch.len() < data.len() {
                    scratch.resize(data.len(), 0.0);
                }
                let buf = &mut scratch[..data.len()];
                engine.process(buf, channels);
                for (o, &x) in data.iter_mut().zip(buf.iter()) {
                    *o = T::from_sample(x);
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}
