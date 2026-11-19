//! cpal microphone capture.
//!
//! The cpal callback downmixes to mono and pushes into a lock-free SPSC ring
//! (`rtrb`); it never blocks or allocates. A dedicated analysis thread drains
//! the ring, discards any backlog older than `MAX_BACKLOG_S` (so analysis
//! tracks "now" after a stall), runs [`Analyzer`], and publishes the result to
//! [`SharedFeatures`]. Samples dropped on either side are reported to the
//! analyzer with [`Analyzer::skip`] so its beat clock keeps real time. If the
//! device stops delivering, the thread feeds silence so the published levels
//! decay instead of freezing. [`SampleClock`] counts each stretch of time
//! once: the stall threshold adapts to devices that deliver in large batches,
//! and real samples that arrive late for time already filled with silence are
//! discarded rather than analysed a second time.
//!
//! The stream is not reopened: if the device is unplugged or fails, cpal
//! reports an error (printed, rate-limited) and the levels decay to silence
//! until the renderer is restarted.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
const MAX_STALL: Duration = Duration::from_millis(500);
const SILENCE_WARNING: Duration = Duration::from_secs(3);
const ERROR_REPORT_INTERVAL: Duration = Duration::from_secs(5);
const PERMISSION_HINT: &str = if cfg!(target_os = "macos") {
    "allow microphone access for the app that launched chipviz (for example your terminal) in System Settings > Privacy & Security > Microphone"
} else {
    "check that the input is not muted, that this process may use it, and that it is the device you expect (see --list-audio-devices)"
};

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
        let dropped = Arc::new(AtomicUsize::new(0));
        let stream = build_stream(&device, config, channels, producer, dropped.clone())
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
                .spawn(move || analysis_loop(&mut analyzer, consumer, &dropped, &shared, &stop))
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
    dropped: Arc<AtomicUsize>,
) -> Result<cpal::Stream, cpal::Error> {
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let ring = (producer, dropped);
    match format {
        SampleFormat::F32 => input_stream::<f32>(device, config, channels, ring),
        SampleFormat::F64 => input_stream::<f64>(device, config, channels, ring),
        SampleFormat::I8 => input_stream::<i8>(device, config, channels, ring),
        SampleFormat::I16 => input_stream::<i16>(device, config, channels, ring),
        SampleFormat::I32 => input_stream::<i32>(device, config, channels, ring),
        SampleFormat::U8 => input_stream::<u8>(device, config, channels, ring),
        SampleFormat::U16 => input_stream::<u16>(device, config, channels, ring),
        SampleFormat::U32 => input_stream::<u32>(device, config, channels, ring),
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
    (mut producer, dropped): (rtrb::Producer<f32>, Arc<AtomicUsize>),
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
            // dropped here and counted; the analysis thread drops stale
            // backlog itself.
            let count = frames.min(producer.slots());
            if count < frames {
                dropped.fetch_add(frames - count, Ordering::Relaxed);
            }
            if let Ok(chunk) = producer.write_chunk_uninit(count) {
                let mono = data.chunks_exact(channels).map(|frame| {
                    frame.iter().map(|&s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
                });
                chunk.fill_from_iter(mono);
            }
        },
        {
            let mut limiter = ErrorLimiter::default();
            move |error| {
                if let Some(suppressed) = limiter.report(Instant::now()) {
                    let note = if suppressed > 0 {
                        format!(" ({suppressed} similar errors suppressed)")
                    } else {
                        String::new()
                    };
                    eprintln!("audio stream error: {error}{note}");
                }
            }
        },
        None,
    )
}

/// Lets at most one error through per `ERROR_REPORT_INTERVAL`.
#[derive(Default)]
struct ErrorLimiter {
    last_report: Option<Instant>,
    suppressed: u64,
}

impl ErrorLimiter {
    /// `Some(suppressed_since_last_report)` when this error should be printed.
    fn report(&mut self, now: Instant) -> Option<u64> {
        let due = self
            .last_report
            .is_none_or(|last| now.duration_since(last) >= ERROR_REPORT_INTERVAL);
        if due {
            self.last_report = Some(now);
            Some(std::mem::take(&mut self.suppressed))
        } else {
            self.suppressed += 1;
            None
        }
    }
}

/// How one poll's samples are spent. Every sample here is time the analyzer
/// accounts for exactly once.
#[derive(Debug, Default, PartialEq, Eq)]
struct Accounting {
    /// Late real samples covering time already filled with silence. They are
    /// consumed (oldest first) without reaching the analyzer.
    overlap: usize,
    /// Silence to synthesize because the device has stalled.
    silence: usize,
}

/// Single time base for the analysis thread: real samples (analysed or
/// skipped) plus synthesized silence advance the analyzer, and no wall-clock
/// interval is counted twice.
///
/// A delivery of `n` samples seen at `now` is taken to cover the
/// `n / rate` seconds ending at `now`. Silence is synthesized only after a
/// stall, from the end of the covered time up to `now`. When real samples
/// later turn up for time that silence already covered, that overlap is
/// discarded, so it is never counted twice.
#[derive(Debug)]
struct SampleClock {
    rate: f32,
    /// No audio for this long counts as a stall. It grows (up to `MAX_STALL`)
    /// to cover the longest gap seen between deliveries, so batched devices do
    /// not trigger silence between their batches.
    stall: Duration,
    last_audio: Instant,
    /// Wall-clock time up to which the analyzer has been fed.
    covered_until: Instant,
    silence_debt: f64,
    /// Synthesized silence that late real samples may still overlap.
    unmatched_silence: usize,
}

impl SampleClock {
    fn new(rate: f32, now: Instant) -> Self {
        Self {
            rate,
            stall: STALL,
            last_audio: now,
            covered_until: now,
            silence_debt: 0.0,
            unmatched_silence: 0,
        }
    }

    /// `real` is every real sample that became known this poll: dropped by
    /// the callback plus waiting in the ring.
    fn poll(&mut self, now: Instant, real: usize) -> Accounting {
        let mut accounting = Accounting::default();
        let rate = f64::from(self.rate);
        if real > 0 {
            let gap = now.duration_since(self.last_audio);
            self.stall = self.stall.max((gap + gap / 2).min(MAX_STALL));
            let span = Duration::from_secs_f64(real as f64 / rate);
            let overlap = (self.covered_until + span).saturating_duration_since(now);
            let overlap = (overlap.as_secs_f64() * rate).round() as usize;
            if overlap == 0 {
                self.unmatched_silence = 0;
            } else {
                accounting.overlap = overlap.min(self.unmatched_silence).min(real);
                self.unmatched_silence -= accounting.overlap;
            }
            self.last_audio = now;
            self.covered_until = self.covered_until.max(now);
            self.silence_debt = 0.0;
        } else if now.duration_since(self.last_audio) > self.stall {
            // Keep the fractional remainder so the clock does not drift.
            self.silence_debt += now
                .saturating_duration_since(self.covered_until)
                .as_secs_f64()
                * rate;
            let whole = self.silence_debt.floor();
            self.silence_debt -= whole;
            accounting.silence = whole as usize;
            self.unmatched_silence += accounting.silence;
            self.covered_until = self.covered_until.max(now);
        }
        accounting
    }
}

fn analysis_loop(
    analyzer: &mut Analyzer,
    mut consumer: rtrb::Consumer<f32>,
    dropped: &AtomicUsize,
    shared: &SharedFeatures,
    stop: &AtomicBool,
) {
    let rate = analyzer.sample_rate();
    let max_backlog = (rate * MAX_BACKLOG_S) as usize;
    let started = Instant::now();
    let mut clock = SampleClock::new(rate, started);
    let mut heard_signal = false;
    let mut warned = false;

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        let mut skipped = dropped.swap(0, Ordering::Relaxed);
        let accounting = clock.poll(now, skipped + consumer.slots());

        // Late samples for time already filled with silence: drop the
        // callback-dropped ones first, then the oldest samples in the ring.
        let overlap_dropped = accounting.overlap.min(skipped);
        skipped -= overlap_dropped;
        let mut discard = accounting.overlap - overlap_dropped;
        let available = consumer.slots();
        if available > discard + max_backlog {
            skipped += available - discard - max_backlog;
            discard = available - max_backlog;
        }
        if discard > 0
            && let Ok(stale) = consumer.read_chunk(discard)
        {
            stale.commit_all();
        }
        if skipped > 0 {
            analyzer.skip(skipped);
        }
        let available = consumer.slots();
        if available > 0
            && let Ok(chunk) = consumer.read_chunk(available)
        {
            let (first, second) = chunk.as_slices();
            heard_signal |= first.iter().chain(second).any(|&s| s != 0.0);
            analyzer.push(first);
            analyzer.push(second);
            chunk.commit_all();
        }
        analyzer.push_silence(accounting.silence);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_limiter_reports_once_per_interval_with_suppressed_count() {
        let mut limiter = ErrorLimiter::default();
        let start = Instant::now();
        assert_eq!(limiter.report(start), Some(0));
        assert_eq!(limiter.report(start + Duration::from_secs(1)), None);
        assert_eq!(limiter.report(start + Duration::from_secs(2)), None);
        assert_eq!(limiter.report(start + ERROR_REPORT_INTERVAL), Some(2));
        assert_eq!(limiter.report(start + ERROR_REPORT_INTERVAL), None);
    }

    const RATE: f32 = 48_000.0;

    fn samples(duration: Duration) -> usize {
        (duration.as_secs_f64() * RATE as f64).round() as usize
    }

    /// Replays callbacks (delivery time, samples) against 5 ms polls and
    /// returns the samples the analyzer accounted for and the silence fed.
    fn replay(callbacks: &[(Duration, usize)], end: Duration) -> (usize, usize) {
        let start = Instant::now();
        let mut clock = SampleClock::new(RATE, start);
        let (mut pending, mut accounted, mut silence) = (0, 0, 0);
        let mut next = callbacks.iter().peekable();
        let mut elapsed = Duration::ZERO;
        while elapsed <= end {
            while let Some((_, count)) = next.next_if(|(at, _)| *at <= elapsed) {
                pending += count;
            }
            let accounting = clock.poll(start + elapsed, pending);
            accounted += pending - accounting.overlap + accounting.silence;
            silence += accounting.silence;
            pending = 0;
            elapsed += POLL;
        }
        (accounted, silence)
    }

    fn assert_close(accounted: usize, expected: Duration) {
        let error = accounted.abs_diff(samples(expected));
        assert!(
            error <= samples(Duration::from_millis(15)),
            "accounted {accounted} samples, expected {}",
            samples(expected)
        );
    }

    #[test]
    fn steady_small_callbacks_need_no_silence() {
        let callbacks: Vec<_> = (1..=200)
            .map(|i| {
                (
                    Duration::from_millis(10 * i),
                    samples(Duration::from_millis(10)),
                )
            })
            .collect();
        let (accounted, silence) = replay(&callbacks, Duration::from_secs(2));
        assert_close(accounted, Duration::from_secs(2));
        assert_eq!(silence, 0);
    }

    #[test]
    fn batched_callbacks_count_each_interval_once() {
        // 2 s of audio arriving as 200 ms batches, each 200 ms late.
        let batch = Duration::from_millis(200);
        let callbacks: Vec<_> = (1..=10).map(|i| (batch * i, samples(batch))).collect();
        let (accounted, _) = replay(&callbacks, Duration::from_secs(2));
        assert_close(accounted, Duration::from_secs(2));
    }

    #[test]
    fn batches_split_across_polls_count_each_interval_once() {
        // Each 250 ms batch arrives as two callbacks 7 ms apart.
        let mut callbacks = Vec::new();
        for i in 1..=8u32 {
            let at = Duration::from_millis(250) * i;
            callbacks.push((at, samples(Duration::from_millis(125))));
            callbacks.push((
                at + Duration::from_millis(7),
                samples(Duration::from_millis(125)),
            ));
        }
        let (accounted, _) = replay(&callbacks, Duration::from_millis(2010));
        assert_close(accounted, Duration::from_secs(2));
    }

    #[test]
    fn an_outage_is_filled_with_silence_and_live_audio_is_kept_afterwards() {
        // 0.5 s of audio, 1 s with nothing, then live audio again.
        let chunk = samples(Duration::from_millis(10));
        let callbacks: Vec<_> = (1..=50)
            .chain(151..=200)
            .map(|i| (Duration::from_millis(10 * i), chunk))
            .collect();
        let (accounted, silence) = replay(&callbacks, Duration::from_secs(2));
        assert_close(accounted, Duration::from_secs(2));
        assert!(silence >= samples(Duration::from_millis(950)), "{silence}");
    }
}
