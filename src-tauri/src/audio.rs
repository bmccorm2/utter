//! Microphone capture.
//!
//! `cpal::Stream` is `!Send` on macOS, so the stream lives on one dedicated
//! thread that is driven through a channel. The input stream is open only
//! while recording, so the macOS mic indicator shows only then.

use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Sample rate the model expects.
pub const TARGET_RATE: u32 = 16_000;
/// How often a level is sent to the overlay.
const LEVEL_INTERVAL_MS: u32 = 33;

enum Command {
    Start {
        device: Option<String>,
        reply: Sender<Result<(), String>>,
    },
    Stop {
        reply: Sender<Result<Vec<f32>, String>>,
    },
    Cancel,
}

#[derive(Clone, Serialize)]
struct Level {
    rms: f32,
}

/// Handle to the audio thread.
pub struct Recorder {
    tx: Sender<Command>,
}

impl Recorder {
    pub fn spawn(app: AppHandle) -> Self {
        // Levels go through a small bounded channel so the audio callback never blocks.
        let (level_tx, level_rx) = mpsc::sync_channel::<f32>(8);
        thread::spawn(move || {
            for rms in level_rx {
                let _ = app.emit_to("overlay", "level", Level { rms });
            }
        });

        let (tx, rx) = mpsc::channel::<Command>();
        thread::spawn(move || {
            let mut active: Option<Active> = None;
            for command in rx {
                match command {
                    Command::Start { device, reply } => {
                        active = None;
                        let result = Active::open(device.as_deref(), &level_tx).map(|a| {
                            active = Some(a);
                        });
                        let _ = reply.send(result);
                    }
                    Command::Stop { reply } => {
                        let result = match active.take() {
                            Some(a) => a.finish(),
                            None => Err("Not recording".to_string()),
                        };
                        let _ = reply.send(result);
                    }
                    Command::Cancel => active = None,
                }
            }
        });

        Recorder { tx }
    }

    /// Start recording from `device`, or from the system default when it is
    /// `None` or no longer connected.
    pub fn start(&self, device: Option<String>) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.send(Command::Start { device, reply })?;
        rx.recv().map_err(|_| "Audio thread stopped".to_string())?
    }

    /// Stop recording and return the audio as 16 kHz mono.
    pub fn stop(&self) -> Result<Vec<f32>, String> {
        let (reply, rx) = mpsc::channel();
        self.send(Command::Stop { reply })?;
        rx.recv().map_err(|_| "Audio thread stopped".to_string())?
    }

    /// Stop recording and discard the audio.
    pub fn cancel(&self) {
        let _ = self.send(Command::Cancel);
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.tx
            .send(command)
            .map_err(|_| "Audio thread stopped".to_string())
    }
}

/// A recording in progress. Dropping it closes the input stream.
struct Active {
    stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
}

impl Active {
    fn open(name: Option<&str>, level_tx: &SyncSender<f32>) -> Result<Self, String> {
        let host = cpal::default_host();
        if let Some(device) = name.and_then(|n| find_input_device(&host, n)) {
            match Self::open_device(&device, level_tx) {
                Ok(active) => return Ok(active),
                Err(e) => log::warn!("Saved microphone failed ({e}); using the system default"),
            }
        }
        let device = host
            .default_input_device()
            .ok_or_else(|| "No microphone found".to_string())?;
        Self::open_device(&device, level_tx)
    }

    fn open_device(device: &cpal::Device, level_tx: &SyncSender<f32>) -> Result<Self, String> {
        let supported = device
            .default_input_config()
            .map_err(|e| format!("Microphone error: {e}"))?;
        let config = supported.config();
        let sample_rate = config.sample_rate;
        let channels = usize::from(config.channels).max(1);
        let buffer = Arc::new(Mutex::new(Vec::with_capacity(sample_rate as usize * 30)));
        let meter = LevelMeter::new(sample_rate, level_tx.clone());

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(device, config, channels, buffer.clone(), meter),
            SampleFormat::I16 => build::<i16>(device, config, channels, buffer.clone(), meter),
            SampleFormat::U16 => build::<u16>(device, config, channels, buffer.clone(), meter),
            SampleFormat::I32 => build::<i32>(device, config, channels, buffer.clone(), meter),
            other => return Err(format!("Unsupported microphone sample format: {other}")),
        }
        .map_err(|e| format!("Microphone error: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("Microphone error: {e}"))?;

        Ok(Active {
            stream,
            buffer,
            sample_rate,
        })
    }

    fn finish(self) -> Result<Vec<f32>, String> {
        drop(self.stream);
        let samples = std::mem::take(&mut *self.buffer.lock().unwrap_or_else(|e| e.into_inner()));
        resample_to_16k(samples, self.sample_rate)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    buffer: Arc<Mutex<Vec<f32>>>,
    mut meter: LevelMeter,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let mut buffer = buffer.lock().unwrap_or_else(|e| e.into_inner());
            let start = buffer.len();
            downmix(data, channels, &mut buffer);
            meter.push(&buffer[start..]);
        },
        |e| log::warn!("Audio stream error: {e}"),
        None,
    )
}

/// Average interleaved frames down to mono f32 and append them to `out`.
fn downmix<T>(data: &[T], channels: usize, out: &mut Vec<f32>)
where
    T: Copy,
    f32: FromSample<T>,
{
    out.extend(data.chunks_exact(channels).map(|frame| {
        let sum: f32 = frame.iter().map(|&s| f32::from_sample_(s)).sum();
        sum / channels as f32
    }));
}

/// Sends one RMS value for every ~33 ms of audio.
struct LevelMeter {
    window: usize,
    pending: Vec<f32>,
    tx: SyncSender<f32>,
}

impl LevelMeter {
    fn new(sample_rate: u32, tx: SyncSender<f32>) -> Self {
        let window = ((sample_rate * LEVEL_INTERVAL_MS / 1000) as usize).max(1);
        LevelMeter {
            window,
            pending: Vec::with_capacity(window * 4),
            tx,
        }
    }

    fn push(&mut self, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
        while self.pending.len() >= self.window {
            // Non-blocking: drop the level if the emitter thread is behind.
            let _ = self.tx.try_send(rms(&self.pending[..self.window]));
            self.pending.drain(..self.window);
        }
    }
}

/// Root mean square of the samples. 0 for an empty slice.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Resample mono audio to 16 kHz. Returns the input unchanged if it is already 16 kHz.
pub fn resample_to_16k(samples: Vec<f32>, sample_rate: u32) -> Result<Vec<f32>, String> {
    if sample_rate == TARGET_RATE || samples.is_empty() {
        return Ok(samples);
    }
    let frames = samples.len();
    let input = InterleavedSlice::new(&samples, 1, frames).map_err(|e| e.to_string())?;
    let mut resampler = Fft::<f32>::new(
        sample_rate as usize,
        TARGET_RATE as usize,
        1024,
        1,
        FixedSync::Both,
    )
    .map_err(|e| format!("Resampler error: {e}"))?;
    let output = resampler
        .process_all(&input, frames, None)
        .map_err(|e| format!("Resampler error: {e}"))?;
    Ok(output.take_data())
}

fn device_name(device: &cpal::Device) -> Option<String> {
    Some(device.description().ok()?.name().to_string())
}

fn find_input_device(host: &cpal::Host, name: &str) -> Option<cpal::Device> {
    host.input_devices()
        .ok()?
        .find(|d| device_name(d).as_deref() == Some(name))
}

/// Names of all input devices.
pub fn list_input_devices() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    match cpal::default_host().input_devices() {
        Ok(devices) => {
            for name in devices.filter_map(|d| device_name(&d)) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        Err(e) => log::warn!("Could not list microphones: {e}"),
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_silence_and_empty_is_zero() {
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(rms(&[0.0; 480]), 0.0);
    }

    #[test]
    fn rms_of_constant_signal_is_its_magnitude() {
        assert!((rms(&[0.5; 100]) - 0.5).abs() < 1e-6);
        assert!((rms(&[-0.5; 100]) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rms_of_full_scale_sine_is_about_0_707() {
        let sine: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
            .collect();
        assert!((rms(&sine) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    }

    #[test]
    fn resample_48k_one_second_gives_about_16000_samples() {
        let input: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        let output = resample_to_16k(input, 48_000).unwrap();
        assert!(
            (15_900..=16_100).contains(&output.len()),
            "got {} samples",
            output.len()
        );
        // The tone must survive resampling at roughly the same level.
        let level = rms(&output[1_000..15_000]);
        assert!((level - 0.3535).abs() < 0.02, "rms was {level}");
    }

    #[test]
    fn resample_keeps_16k_input_unchanged() {
        let input = vec![0.25_f32; 16_000];
        assert_eq!(resample_to_16k(input.clone(), 16_000).unwrap(), input);
    }

    #[test]
    fn downmix_averages_channels() {
        let mut out = Vec::new();
        downmix(&[1.0_f32, 0.0, 0.5, 0.5, -1.0, 1.0], 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn level_meter_sends_one_value_per_window() {
        let (tx, rx) = mpsc::sync_channel(8);
        let mut meter = LevelMeter::new(48_000, tx);
        assert_eq!(meter.window, 1_584);
        meter.push(&vec![0.5; 1_584 * 2 + 10]);
        assert_eq!(rx.try_iter().count(), 2);
        assert_eq!(meter.pending.len(), 10);
    }
}
