//! cpal microphone capture.
//!
//! The cpal callback downmixes to mono and pushes into a lock-free SPSC ring
//! (`rtrb`); it never blocks or allocates. A dedicated analysis thread drains
//! the ring, discards any backlog older than `MAX_BACKLOG_S` (so analysis
//! tracks "now" after a stall), runs [`Analyzer`], and publishes the result to
//! [`SharedFeatures`]. If the device stops delivering, the thread feeds
//! silence so the published levels decay instead of freezing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample};

use super::analysis::Analyzer;
use super::{AudioFeatures, SharedFeatures};

const RING_S: f32 = 1.0;
const MAX_BACKLOG_S: f32 = 0.1;
const POLL: Duration = Duration::from_millis(5);
const STALL: Duration = Duration::from_millis(100);
const SILENCE_WARNING: Duration = Duration::from_secs(3);
const PERMISSION_HINT: &str = "on macOS, allow microphone access for your terminal in System Settings > Privacy & Security > Microphone";

/// Input devices from the default cpal host, in a stable enumeration order.
pub struct InputDevices {
    devices: Vec<cpal::Device>,
    names: Vec<Option<String>>,
    default: Option<cpal::Device>,
    default_index: Option<usize>,
}

impl InputDevices {
    pub fn enumerate() -> Result<Self, String> {
        let host = cpal::default_host();
        let default = host.default_input_device();
        let default_id = default.as_ref().and_then(|device| device.id().ok());
        let devices: Vec<cpal::Device> = host
            .input_devices()
            .map_err(|error| format!("cannot list audio input devices: {error}"))?
            .collect();
        let names = devices
            .iter()
            .map(|device| {
                device
                    .description()
                    .ok()
                    .map(|description| description.name().to_string())
            })
            .collect();
        let default_index = default_id.and_then(|default_id| {
            devices
                .iter()
                .position(|device| device.id().ok().as_ref() == Some(&default_id))
        });
        Ok(Self {
            devices,
            names,
            default,
            default_index,
        })
    }

    pub fn names(&self) -> &[Option<String>] {
        &self.names
    }

    pub fn print(&self) {
        if self.names.is_empty() {
            println!("no audio input devices detected");
            return;
        }
        for (index, name) in self.names.iter().enumerate() {
            let marker = if Some(index) == self.default_index {
                " (default)"
            } else {
                ""
            };
            println!(
                "{index}: {}{marker}",
                name.as_deref().unwrap_or("<unnamed>")
            );
        }
    }

    /// The device at `index`, or the system default input for `None`.
    fn into_device(mut self, index: Option<usize>) -> Result<cpal::Device, String> {
        match index {
            Some(index) if index < self.devices.len() => Ok(self.devices.swap_remove(index)),
            Some(index) => Err(format!(
                "audio input device index {index} out of range (see --list-audio-devices)"
            )),
            None => self.default.ok_or_else(|| {
                "no audio input device found; connect a microphone or pick one with --audio-device (see --list-audio-devices)".to_string()
            }),
        }
    }
}

/// A running microphone stream plus its analysis thread. Dropping it stops
/// both.
pub struct MicSource {
    shared: Arc<SharedFeatures>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    // Dropped after `Drop::drop` has joined the analysis thread.
    _stream: cpal::Stream,
}

impl MicSource {
    pub fn start(devices: InputDevices, index: Option<usize>) -> Result<Self, String> {
        let device = devices.into_device(index)?;
        let name = device
            .description()
            .map(|description| description.name().to_string())
            .unwrap_or_else(|_| "<unnamed>".to_string());
        let config = device
            .default_input_config()
            .map_err(|error| describe(&name, "has no usable input configuration", &error))?;
        let sample_rate = config.sample_rate();
        let channels = usize::from(config.channels()).max(1);
        let mut analyzer = Analyzer::new(sample_rate)
            .map_err(|error| format!("audio input \"{name}\": {error}"))?;

        let capacity = (sample_rate as f32 * RING_S) as usize;
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(capacity);
        let stream = build_stream(&device, config, channels, producer)
            .map_err(|error| describe(&name, "could not open an input stream", &error))?;
        stream
            .play()
            .map_err(|error| describe(&name, "could not start capturing", &error))?;

        let shared = Arc::new(SharedFeatures::default());
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let shared = shared.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("chipviz-audio".to_string())
                .spawn(move || analysis_loop(&mut analyzer, consumer, &shared, &stop))
                .map_err(|error| format!("could not start the audio analysis thread: {error}"))?
        };
        eprintln!("microphone: {name} ({sample_rate} Hz, {channels} ch)");
        Ok(Self {
            shared,
            stop,
            thread: Some(thread),
            _stream: stream,
        })
    }

    /// Latest analysed features; never blocks.
    pub fn latest(&self) -> AudioFeatures {
        self.shared.load()
    }
}

impl Drop for MicSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn describe(name: &str, what: &str, error: &cpal::Error) -> String {
    let hint = match error.kind() {
        ErrorKind::PermissionDenied => format!("; {PERMISSION_HINT}"),
        _ => String::new(),
    };
    format!("audio input \"{name}\" {what}: {error}{hint}")
}

fn build_stream(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    channels: usize,
    producer: rtrb::Producer<f32>,
) -> Result<cpal::Stream, cpal::Error> {
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    match format {
        SampleFormat::F32 => input_stream::<f32>(device, config, channels, producer),
        SampleFormat::F64 => input_stream::<f64>(device, config, channels, producer),
        SampleFormat::I8 => input_stream::<i8>(device, config, channels, producer),
        SampleFormat::I16 => input_stream::<i16>(device, config, channels, producer),
        SampleFormat::I32 => input_stream::<i32>(device, config, channels, producer),
        SampleFormat::U8 => input_stream::<u8>(device, config, channels, producer),
        SampleFormat::U16 => input_stream::<u16>(device, config, channels, producer),
        SampleFormat::U32 => input_stream::<u32>(device, config, channels, producer),
        _ => Err(cpal::Error::with_message(
            ErrorKind::UnsupportedConfig,
            format!("unsupported sample format {format}"),
        )),
    }
}

fn input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    mut producer: rtrb::Producer<f32>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let frames = data.len() / channels;
            // When the ring is full (analysis stalled) the newest audio is
            // dropped here; the analysis thread drops stale backlog itself.
            let count = frames.min(producer.slots());
            if let Ok(chunk) = producer.write_chunk_uninit(count) {
                let mono = data.chunks_exact(channels).map(|frame| {
                    frame.iter().map(|&s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
                });
                chunk.fill_from_iter(mono);
            }
        },
        |error| eprintln!("audio stream error: {error}"),
        None,
    )
}

fn analysis_loop(
    analyzer: &mut Analyzer,
    mut consumer: rtrb::Consumer<f32>,
    shared: &SharedFeatures,
    stop: &AtomicBool,
) {
    let rate = analyzer.sample_rate();
    let max_backlog = (rate * MAX_BACKLOG_S) as usize;
    let started = Instant::now();
    let mut last_audio = started;
    let mut last_poll = started;
    let mut heard_signal = false;
    let mut warned = false;

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        let available = consumer.slots();
        if available > max_backlog
            && let Ok(stale) = consumer.read_chunk(available - max_backlog)
        {
            stale.commit_all();
        }
        let available = consumer.slots();
        if available > 0 {
            if let Ok(chunk) = consumer.read_chunk(available) {
                let (first, second) = chunk.as_slices();
                heard_signal |= first.iter().chain(second).any(|&s| s != 0.0);
                analyzer.push(first);
                analyzer.push(second);
                chunk.commit_all();
            }
            last_audio = now;
        } else if now.duration_since(last_audio) > STALL {
            let missing = now.duration_since(last_poll).as_secs_f32() * rate;
            analyzer.push_silence(missing as usize);
        }
        last_poll = now;
        shared.store(analyzer.features());

        if !warned && !heard_signal && now.duration_since(started) > SILENCE_WARNING {
            warned = true;
            eprintln!(
                "warning: the microphone has delivered only silence for {} s; {PERMISSION_HINT}",
                SILENCE_WARNING.as_secs()
            );
        }
        std::thread::sleep(POLL);
    }
}
