//! Linux live audio capture via CPAL.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Error, SampleFormat, Stream, StreamConfig};
use std::sync::mpsc;

/// An audio input device.
pub struct AudioDevice {
    pub id: usize,
    pub name: String,
    pub input_channels: u16,
}

fn input_devices() -> Vec<(Device, AudioDevice)> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };

    devices
        .enumerate()
        .filter_map(|(id, device)| {
            let config = device.default_input_config().ok()?;
            let name = device
                .description()
                .map(|description| description.name().to_owned())
                .unwrap_or_else(|_| format!("Input device {id}"));
            let info = AudioDevice {
                id,
                name,
                input_channels: config.channels(),
            };
            Some((device, info))
        })
        .collect()
}

/// Get the list of audio input devices.
pub fn list_input_devices() -> Vec<AudioDevice> {
    input_devices().into_iter().map(|(_, info)| info).collect()
}

/// Find an input device by name (case-insensitive substring match).
pub fn find_device_by_name(name: &str) -> Option<AudioDevice> {
    let name_lower = name.to_lowercase();
    list_input_devices()
        .into_iter()
        .find(|device| device.name.to_lowercase().contains(&name_lower))
}

/// Get the default input device's enumerated ID.
pub fn default_input_device() -> Option<usize> {
    let default = cpal::default_host().default_input_device()?;
    let default_name = default.description().ok()?.name().to_owned();
    list_input_devices()
        .into_iter()
        .find(|device| device.name == default_name)
        .map(|device| device.id)
}

/// Print all input devices to stderr.
pub fn print_devices() {
    let devices = list_input_devices();
    if devices.is_empty() {
        eprintln!("No audio input devices found.");
        return;
    }

    let default_id = default_input_device();
    eprintln!("Audio input devices:\n");
    for device in &devices {
        let marker = if Some(device.id) == default_id {
            " (default)"
        } else {
            ""
        };
        eprintln!(
            "  {:30} {} ch{}",
            device.name, device.input_channels, marker
        );
    }
    eprintln!();
}

/// Capture handle; dropping it stops the CPAL stream.
pub struct CaptureHandle {
    _stream: Stream,
}

fn build_stream<T>(
    device: &Device,
    config: StreamConfig,
    channels: usize,
    tx: mpsc::Sender<Vec<f32>>,
    convert: fn(T) -> f32,
) -> Result<Stream, Error>
where
    T: cpal::SizedSample + Copy + Send + 'static,
{
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            let samples = data
                .chunks(channels)
                .map(|frame| {
                    frame.iter().map(|&sample| convert(sample)).sum::<f32>() / channels as f32
                })
                .collect();
            let _ = tx.send(samples);
        },
        |error| eprintln!("Audio input stream error: {error}"),
        None,
    )
}

/// Start capture and return mono f32 chunks at the device's sample rate.
pub fn start_capture(
    device_id: usize,
) -> Result<(mpsc::Receiver<Vec<f32>>, CaptureHandle, f64), String> {
    let (device, _) = input_devices()
        .into_iter()
        .find(|(_, info)| info.id == device_id)
        .ok_or_else(|| format!("Input device {device_id} is no longer available"))?;
    let supported = device
        .default_input_config()
        .map_err(|error| format!("Cannot get default input configuration: {error}"))?;
    let sample_rate = f64::from(supported.sample_rate());
    let channels = usize::from(supported.channels());
    let config: StreamConfig = supported.clone().into();
    let sample_format = supported.sample_format();
    let (tx, rx) = mpsc::channel();

    let stream = match sample_format {
        SampleFormat::F32 => build_stream(&device, config, channels, tx, |sample: f32| sample),
        SampleFormat::I16 => build_stream(&device, config, channels, tx, |sample: i16| {
            f32::from(sample) / 32768.0
        }),
        SampleFormat::U16 => build_stream(&device, config, channels, tx, |sample: u16| {
            f32::from(sample) / 32768.0 - 1.0
        }),
        format => return Err(format!("Unsupported input sample format: {format:?}")),
    }
    .map_err(|error| format!("Cannot build audio input stream: {error}"))?;

    stream
        .play()
        .map_err(|error| format!("Cannot start audio input stream: {error}"))?;

    Ok((rx, CaptureHandle { _stream: stream }, sample_rate))
}
