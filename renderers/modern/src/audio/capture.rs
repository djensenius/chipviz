//! cpal microphone capture.
//!
//! The cpal callback downmixes to mono and pushes into a lock-free SPSC ring
//! (`rtrb`); it never blocks or allocates. A dedicated analysis thread drains
//! the ring, discards any backlog older than `MAX_BACKLOG_S` (so analysis
//! tracks "now" after a stall), runs [`Analyzer`], and publishes the result to
//! [`SharedFeatures`]. If the ring overflows (the callback had to drop audio),
//! the callback commits the samples it accepted before reporting the drop, so
//! the next ring snapshot holds all audio accepted before the drop (plus
//! possibly a few ms committed after it); that whole snapshot is discarded
//! and skipped rather than analysed out of order.
//! Samples dropped on either side are reported to the
//! analyzer with [`Analyzer::skip`] so its beat clock keeps real time. If the
//! device stops delivering, the thread feeds silence so the published levels
//! decay instead of freezing, and a shorter gap where the device lost audio is
//! filled with silence too. [`SampleClock`] counts each stretch of time
//! once: the stall threshold adapts to devices that deliver in large batches
//! (and returns to its base after a second of normal deliveries), and real
//! samples that arrive late for time already filled with silence are discarded
//! rather than analysed a second time.
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
use super::scope::{ScopeTap, SharedScope};
use super::{AudioFeatures, SharedFeatures};

const RING_S: f32 = 1.0;
const MAX_BACKLOG_S: f32 = 0.1;
const POLL: Duration = Duration::from_millis(5);
const STALL: Duration = Duration::from_millis(100);
const MAX_STALL: Duration = Duration::from_millis(500);
/// Lag above the delivery latency smaller than this (or 1.5 times the recent
/// delivery jitter, up to `STALL`) is jitter, not lost audio.
const LOSS_MARGIN: Duration = Duration::from_millis(20);
/// Lag must stay above the margin this long before it counts as lost audio.
const LOSS_CONFIRM: Duration = Duration::from_millis(50);
/// Ring snapshots retaken at most this often when audio keeps arriving while
/// the clock is read; the last one is used regardless.
const SNAPSHOT_RETRIES: usize = 3;
/// Deliveries this long with only normal gaps reset the stall threshold.
const STALL_RESET: Duration = Duration::from_secs(1);
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
    scope: Arc<SharedScope>,
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
        let scope = Arc::new(SharedScope::default());
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let shared = shared.clone();
            let scope = scope.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("chipviz-audio".to_string())
                .spawn(move || {
                    let outputs = Outputs {
                        features: &shared,
                        scope: &scope,
                    };
                    analysis_loop(&mut analyzer, consumer, &dropped, outputs, &stop)
                })
                .map_err(|error| format!("could not start the audio analysis thread: {error}"))?
        };
        eprintln!("microphone: {name} ({sample_rate} Hz, {channels} ch)");
        Ok(Self {
            shared,
            scope,
            stop,
            thread: Some(thread),
            _stream: stream,
        })
    }

    /// Latest analysed features; never blocks.
    pub fn latest(&self) -> AudioFeatures {
        self.shared.load()
    }

    /// The waveform and onset side channel for the mic layers.
    pub fn scope(&self) -> Arc<SharedScope> {
        self.scope.clone()
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
            // Release pairs with the analysis thread's Acquire swap: once it
            // sees this drop, it also sees the samples committed before it.
            push_frames(&mut producer, data, channels, |count| {
                dropped.fetch_add(count, Ordering::Release);
            });
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

/// Downmixes interleaved `data` to mono and pushes it into the ring. When the
/// ring is full (analysis stalled) the newest frames are dropped and passed to
/// `report_drop`; the analysis thread drops stale backlog itself.
///
/// The accepted frames are committed *before* the drop is reported. A poll
/// that sees the drop discards its whole ring snapshot; if the drop were
/// reported first, a poll could discard the snapshot and then analyse the
/// late-committed accepted frames on the next poll, out of order with the
/// time already skipped.
fn push_frames<T>(
    producer: &mut rtrb::Producer<f32>,
    data: &[T],
    channels: usize,
    report_drop: impl FnOnce(usize),
) where
    T: SizedSample,
    f32: FromSample<T>,
{
    let frames = data.len() / channels;
    let count = frames.min(producer.slots());
    if let Ok(chunk) = producer.write_chunk_uninit(count) {
        let mono = data.chunks_exact(channels).map(|frame| {
            frame.iter().map(|&s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
        });
        chunk.fill_from_iter(mono);
    }
    if count < frames {
        report_drop(frames - count);
    }
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
    /// Silence to synthesize because the device has stalled or lost audio.
    /// It precedes this poll's real samples.
    silence: usize,
}

/// Single time base for the analysis thread: real samples (analysed or
/// skipped) plus synthesized silence advance the analyzer, and no wall-clock
/// interval is counted twice.
///
/// A delivery of `n` samples seen at `now` is taken to cover the
/// `n / rate` seconds ending at `now`. Silence is synthesized after a
/// stall, from the end of the covered time up to `now`, and when
/// [`LossDetector`] finds that the device lost audio without stalling. When real samples
/// later turn up for time that silence already covered, that overlap is
/// discarded, so it is never counted twice.
#[derive(Debug)]
struct SampleClock {
    rate: f32,
    /// No audio for this long counts as a stall. It grows (up to `MAX_STALL`)
    /// to cover the longest gap seen between deliveries, so batched devices do
    /// not trigger silence between their batches, and returns to `STALL` once
    /// deliveries have had only normal gaps for `STALL_RESET`, so one
    /// analysis-thread hiccup does not slow outage detection for good.
    stall: Duration,
    /// Start of the current run of deliveries whose gaps fit within `STALL`.
    normal_since: Option<Instant>,
    last_audio: Instant,
    /// Wall-clock time up to which the analyzer has been fed.
    covered_until: Instant,
    silence_debt: f64,
    /// Synthesized silence that late real samples may still overlap.
    unmatched_silence: usize,
    start: Instant,
    /// Samples the analyzer has accounted for since `start`.
    fed: u64,
    loss: LossDetector,
}

impl SampleClock {
    fn new(rate: f32, now: Instant) -> Self {
        Self {
            rate,
            stall: STALL,
            normal_since: None,
            last_audio: now,
            covered_until: now,
            silence_debt: 0.0,
            unmatched_silence: 0,
            start: now,
            fed: 0,
            loss: LossDetector::new(now),
        }
    }

    /// `real` is every real sample that became known this poll: dropped by
    /// the callback plus waiting in the ring.
    fn poll(&mut self, now: Instant, real: usize) -> Accounting {
        let mut accounting = Accounting::default();
        let rate = f64::from(self.rate);
        if real > 0 {
            let gap = now.duration_since(self.last_audio);
            let needed = gap + gap / 2;
            if needed > STALL {
                self.normal_since = None;
                self.stall = self.stall.max(needed.min(MAX_STALL));
            } else if now.duration_since(*self.normal_since.get_or_insert(now)) >= STALL_RESET {
                self.stall = STALL;
            }
            let span = Duration::from_secs_f64(real as f64 / rate);
            let overlap = (self.covered_until + span).saturating_duration_since(now);
            let overlap = (overlap.as_secs_f64() * rate).round() as usize;
            if overlap == 0 {
                self.unmatched_silence = 0;
            } else {
                accounting.overlap = overlap.min(self.unmatched_silence).min(real);
                self.unmatched_silence -= accounting.overlap;
            }
            self.fed += (real - accounting.overlap) as u64;
            self.loss.overlapped(accounting.overlap as f64 / rate);
            // Ring overflow drops are already in `real`, so they never look
            // like lost audio.
            let lag = now.duration_since(self.start).as_secs_f64() - self.fed as f64 / rate;
            let spare = (real - accounting.overlap) as f64 / rate;
            let adjust = self.loss.check(now, lag, spare) * rate;
            if adjust >= 0.0 {
                accounting.silence = adjust.round() as usize;
                self.fed += accounting.silence as u64;
            } else {
                // Taken-back fill: the oldest real samples are discarded.
                let back = (-adjust).round() as usize;
                let back = back.min(real - accounting.overlap);
                accounting.overlap += back;
                self.fed -= back as u64;
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
            self.fed += accounting.silence as u64;
            self.loss.stalled(accounting.silence as f64 / rate);
            self.covered_until = self.covered_until.max(now);
        }
        accounting
    }
}

/// Finds audio the device lost without stalling for long enough to trigger
/// stall silence.
///
/// At each delivery, the lag is how far the analysed timeline (real samples
/// plus silence since the start) trails the wall clock. Its minimum is the
/// delivery latency. A late delivery raises the lag only until it catches up,
/// but lost audio raises it for good. So lag above the latency is filled with
/// silence only after it has stayed above the margin for `LOSS_CONFIRM`, and
/// only by its smallest value in that run. Excess lag that goes away again is
/// jitter, and it widens the margin. The fill comes `LOSS_CONFIRM` after the
/// loss, so the beat phase is off by the loss length only for that long.
/// Fill that later proves too large (a delivery lags less than the latency)
/// is taken back by discarding real samples, so neither a jittery estimate
/// nor a false alarm moves the timeline ahead of the wall clock for good.
///
/// Losses within the margin (`LOSS_MARGIN`, or 1.5 times the recent jitter up
/// to `STALL`) are not detected, so jittery devices hide short losses, and
/// a run that ends up within the margin after a partial fill keeps its
/// residual. A large burst (for example after the analysis thread was
/// starved) hides nothing: the lag counts every sample it delivers.
#[derive(Debug)]
struct LossDetector {
    latency: Option<f64>,
    /// Start, smallest and largest excess lag of the run above the margin.
    excess: Option<(Instant, f64, f64)>,
    /// Largest jitter seen in the current and the previous `STALL_RESET`.
    jitter: f64,
    previous_jitter: f64,
    jitter_since: Instant,
    /// Seconds filled that have not been taken back.
    filled: f64,
    /// Start of the current take-back episode (since the last fill, at most
    /// `STALL_RESET` long) and the seconds taken back in it.
    reversed: Option<(Instant, f64)>,
}

impl LossDetector {
    fn new(now: Instant) -> Self {
        Self {
            latency: None,
            excess: None,
            jitter: 0.0,
            previous_jitter: 0.0,
            jitter_since: now,
            filled: 0.0,
            reversed: None,
        }
    }

    /// Stall silence fills up to now, as if deliveries had no latency; any
    /// of it that proves too much is taken back like a loss fill.
    fn stalled(&mut self, silence: f64) {
        self.excess = None;
        self.filled += silence;
        self.reversed = None;
    }

    /// Late real samples already cancelled this much stall silence.
    fn overlapped(&mut self, overlap: f64) {
        self.filled = (self.filled - overlap).max(0.0);
    }

    fn margin(&self) -> f64 {
        (1.5 * self.jitter.max(self.previous_jitter))
            .clamp(LOSS_MARGIN.as_secs_f64(), STALL.as_secs_f64())
    }

    fn note_jitter(&mut self, now: Instant, excess: f64) {
        if now.duration_since(self.jitter_since) >= STALL_RESET {
            self.previous_jitter = std::mem::take(&mut self.jitter);
            self.jitter_since = now;
        }
        self.jitter = self.jitter.max(excess);
    }

    /// Seconds to adjust the timeline by, given the lag (in seconds) after a
    /// delivery: positive is lost audio to fill with silence, negative is
    /// earlier fill to take back, at most `spare` seconds of this delivery.
    fn check(&mut self, now: Instant, lag: f64, spare: f64) -> f64 {
        let latency = *self.latency.get_or_insert(lag);
        if lag < latency && self.filled > 0.0 {
            let back = (latency - lag).min(self.filled).min(spare);
            self.filled -= back;
            // Fill taken back was a delay, not a loss: learn the whole
            // reversal as jitter so the same delay is not filled again. It
            // accumulates over an episode, so a delay that recovers in steps
            // (or a take-back that `spare` splits) counts in full. Episodes
            // last at most `STALL_RESET`: a device clock running fast keeps
            // taking back stall silence for hours, and that slow trend must
            // not pile up into a margin that hides real losses.
            let (since, total) = self.reversed.get_or_insert((now, 0.0));
            if now.duration_since(*since) >= STALL_RESET {
                *since = now;
                *total = 0.0;
            }
            *total += back;
            let total = *total;
            self.note_jitter(now, total);
            self.excess = None;
            return -back;
        }
        let latency = latency.min(lag);
        self.latency = Some(latency);
        let excess = lag - latency;
        if excess <= self.margin() {
            if let Some((_, _, largest)) = self.excess.take() {
                self.note_jitter(now, largest);
            }
            self.note_jitter(now, excess);
            return 0.0;
        }
        let (since, smallest, largest) = self.excess.get_or_insert((now, excess, excess));
        *smallest = smallest.min(excess);
        *largest = largest.max(excess);
        if now.duration_since(*since) < LOSS_CONFIRM {
            return 0.0;
        }
        let lost = *smallest;
        self.excess = None;
        self.filled += lost;
        self.reversed = None;
        lost
    }
}

/// What to do with one poll's real samples. `discard + analyse` never
/// exceeds the ring length snapshotted for the poll.
#[derive(Debug, PartialEq, Eq)]
struct Drain {
    /// Samples the analyzer should skip (time passes, nothing analysed).
    skip: usize,
    /// Oldest ring samples to consume without analysis.
    discard: usize,
    /// Ring samples to analyse after the discarded ones.
    analyse: usize,
}

impl Drain {
    /// `dropped` were lost in the callback and `available` is the ring length
    /// snapshot that the clock was given. Late overlap is taken from the
    /// dropped samples first, then from the oldest ring samples; anything
    /// beyond `max_backlog` is skipped as stale. If the callback dropped
    /// samples, the ring overflowed and holds only audio older than them, so
    /// the whole snapshot is discarded and skipped.
    fn plan(dropped: usize, available: usize, overlap: usize, max_backlog: usize) -> Self {
        if dropped > 0 {
            let real = dropped + available;
            return Self {
                skip: real - overlap.min(real),
                discard: available,
                analyse: 0,
            };
        }
        let overlap_dropped = overlap.min(dropped);
        let mut skip = dropped - overlap_dropped;
        let mut discard = (overlap - overlap_dropped).min(available);
        if available > discard + max_backlog {
            skip += available - discard - max_backlog;
            discard = available - max_backlog;
        }
        Self {
            skip,
            discard,
            analyse: available - discard,
        }
    }

    /// Consumes exactly `discard + analyse` samples, passing the analysed ones
    /// to `push`. Samples that arrived after the snapshot stay in the ring.
    fn apply(&self, consumer: &mut rtrb::Consumer<f32>, mut push: impl FnMut(&[f32])) {
        if self.discard > 0
            && let Ok(stale) = consumer.read_chunk(self.discard)
        {
            stale.commit_all();
        }
        if self.analyse > 0
            && let Ok(chunk) = consumer.read_chunk(self.analyse)
        {
            let (first, second) = chunk.as_slices();
            push(first);
            push(second);
            chunk.commit_all();
        }
    }
}

/// One poll's view of the ring: `(now, dropped, available)`.
///
/// The clock is read after the ring, so the snapshot never holds samples
/// published after `now`. Read first, a preemption before `slots()` would
/// count newer samples against an older time and permanently lower
/// [`LossDetector`]'s latency, making later normal deliveries look lost.
/// If audio arrived while the clock was being read (the thread paused in
/// between), the snapshot is stale for `now` and is taken again; otherwise
/// a stale empty snapshot paired with a late time would look like a stall.
fn snapshot(
    dropped: &AtomicUsize,
    consumer: &rtrb::Consumer<f32>,
    mut clock: impl FnMut() -> Instant,
) -> (Instant, usize, usize) {
    let mut lost = 0;
    let mut attempt = 0;
    loop {
        // The drop count must be read before `slots()`: Acquire pairs with
        // the callback's Release, so the snapshot below includes every
        // sample accepted before the drops counted here (plus possibly a few
        // committed just after), and all of it is discarded on overflow.
        lost += dropped.swap(0, Ordering::Acquire);
        // One snapshot bounds the whole poll; the callback may publish more
        // meanwhile, and those samples wait for the next poll's clock.
        let available = consumer.slots();
        let now = clock();
        let fresh = consumer.slots() == available && dropped.load(Ordering::Relaxed) == 0;
        if fresh || attempt == SNAPSHOT_RETRIES {
            return (now, lost, available);
        }
        attempt += 1;
    }
}

/// Where the analysis thread publishes each poll's results.
struct Outputs<'a> {
    features: &'a SharedFeatures,
    scope: &'a SharedScope,
}

fn analysis_loop(
    analyzer: &mut Analyzer,
    mut consumer: rtrb::Consumer<f32>,
    dropped: &AtomicUsize,
    outputs: Outputs<'_>,
    stop: &AtomicBool,
) {
    let rate = analyzer.sample_rate();
    let mut tap = ScopeTap::new(rate);
    let max_backlog = (rate * MAX_BACKLOG_S) as usize;
    let started = Instant::now();
    let mut clock = SampleClock::new(rate, started);
    let mut heard_signal = false;
    let mut warned = false;

    while !stop.load(Ordering::Relaxed) {
        let (now, dropped, available) = snapshot(dropped, &consumer, Instant::now);
        let accounting = clock.poll(now, dropped + available);
        let drain = Drain::plan(dropped, available, accounting.overlap, max_backlog);
        // Silence covers time before this poll's real samples.
        analyzer.push_silence(accounting.silence);
        tap.push_silence(accounting.silence);
        if drain.skip > 0 {
            analyzer.skip(drain.skip);
        }
        drain.apply(&mut consumer, |samples| {
            heard_signal |= samples.iter().any(|&s| s != 0.0);
            analyzer.push(samples);
            tap.push(samples);
        });
        outputs.features.store(analyzer.features());
        // The onset count wraps; the mic layers only look for changes.
        outputs.scope.store(&tap.snapshot(analyzer.onsets() as u32));

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

    #[test]
    fn drain_plan_takes_overlap_from_dropped_then_oldest_and_skips_stale() {
        let plan = |dropped, available, overlap| Drain::plan(dropped, available, overlap, 100);
        let drain = |skip, discard, analyse| Drain {
            skip,
            discard,
            analyse,
        };
        assert_eq!(plan(0, 50, 0), drain(0, 0, 50));
        assert_eq!(plan(0, 50, 20), drain(0, 20, 30));
        assert_eq!(plan(0, 50, 80), drain(0, 50, 0));
        assert_eq!(plan(0, 300, 40), drain(160, 200, 100));
        // Any drop means the ring overflowed: nothing in it is analysed.
        assert_eq!(plan(30, 50, 20), drain(60, 50, 0));
        assert_eq!(plan(10, 50, 30), drain(30, 50, 0));
        assert_eq!(plan(5, 300, 40), drain(265, 300, 0));
        for (dropped, available, overlap) in [
            (0, 50, 0),
            (0, 50, 20),
            (0, 300, 40),
            (30, 50, 20),
            (10, 50, 30),
            (5, 300, 40),
        ] {
            // The ring gives up exactly the snapshot; time advances by the
            // real samples minus the overlap.
            let d = plan(dropped, available, overlap);
            assert_eq!(d.discard + d.analyse, available);
            assert_eq!(d.skip + d.analyse, dropped + available - overlap);
        }
    }

    #[test]
    fn overflowed_ring_is_skipped_not_analysed() {
        let ring = 48_000;
        for (dropped, overlap) in [(1, 0), (2_000, 0), (2_000, 500), (100, 5_000)] {
            let d = Drain::plan(dropped, ring, overlap, 4_800);
            assert_eq!(d.analyse, 0, "{d:?}");
            assert_eq!(d.discard + d.analyse, ring);
            assert_eq!(d.skip + d.analyse, dropped + ring - overlap);
        }

        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(8);
        for i in 0..8 {
            producer.push(i as f32).unwrap();
        }
        let drain = Drain::plan(3, consumer.slots(), 0, 4);
        let mut analysed = Vec::new();
        drain.apply(&mut consumer, |samples| analysed.extend_from_slice(samples));
        assert!(analysed.is_empty());
        assert_eq!(consumer.slots(), 0);
        assert_eq!(drain.skip, 11);
    }

    #[test]
    fn push_frames_commits_accepted_frames_before_reporting_a_drop() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(8);
        // Stereo: frames (1, 3) downmix to 2.0, and so on.
        let stereo: Vec<f32> = (0..6).flat_map(|i| [i as f32, i as f32 + 2.0]).collect();
        push_frames(&mut producer, &stereo, 2, |_| panic!("ring had room"));
        assert_eq!(consumer.slots(), 6);

        let mut reported = None;
        push_frames(&mut producer, &stereo, 2, |count| {
            // Whenever a drop is visible, the accepted frames already are.
            assert_eq!(consumer.slots(), 8);
            reported = Some(count);
        });
        assert_eq!(reported, Some(4));
        let ring: Vec<f32> = std::iter::from_fn(|| consumer.pop().ok()).collect();
        assert_eq!(ring, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 1.0, 2.0]);
    }

    #[test]
    fn drain_leaves_samples_published_after_the_snapshot_in_the_ring() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(64);
        for i in 0..10 {
            producer.push(i as f32).unwrap();
        }
        let drain = Drain::plan(0, consumer.slots(), 4, 100);
        // A callback publishes between the snapshot and the reads.
        for i in 10..15 {
            producer.push(i as f32).unwrap();
        }
        let mut analysed = Vec::new();
        drain.apply(&mut consumer, |samples| analysed.extend_from_slice(samples));
        assert_eq!(analysed, (4..10).map(|i| i as f32).collect::<Vec<_>>());
        assert_eq!(consumer.slots(), 5);
        assert_eq!(consumer.pop(), Ok(10.0));
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

    #[test]
    fn a_short_device_loss_is_filled_with_silence() {
        // 10 ms callbacks with `lost` ms of them never delivered.
        let chunk = samples(Duration::from_millis(10));
        for lost in [30, 50, 80, 90] {
            let callbacks: Vec<_> = (1..=100)
                .chain(101 + lost / 10..=200)
                .map(|i| (Duration::from_millis(10 * i), chunk))
                .collect();
            let (accounted, silence) = replay(&callbacks, Duration::from_secs(2));
            assert_close(accounted, Duration::from_secs(2));
            assert_close(silence, Duration::from_millis(lost));
        }
    }

    #[test]
    fn late_deliveries_without_loss_add_no_silence() {
        let chunk = samples(Duration::from_millis(10));
        let callbacks: Vec<_> = (1..=200u64)
            .map(|i| {
                let at = match i {
                    // Held back 60 ms, then delivered together.
                    100..=106 => 1060,
                    // Up to 18 ms late in pairs.
                    150..=151 => 1518,
                    160..=161 => 1615,
                    _ => 10 * i,
                };
                (Duration::from_millis(at), chunk)
            })
            .collect();
        let (accounted, silence) = replay(&callbacks, Duration::from_secs(2));
        assert_close(accounted, Duration::from_secs(2));
        assert_eq!(silence, 0);
    }

    #[test]
    fn jittered_batches_add_no_silence_beyond_start_up() {
        // 200 ms batches arriving up to 45 ms late; only the start-up stall
        // before the first batch (as without jitter) is filled.
        let batch = Duration::from_millis(200);
        let late = [0, 40, 0, 45, 10, 0, 40, 0, 30, 0];
        let steady: Vec<_> = (1..=10).map(|i| (batch * i, samples(batch))).collect();
        let jittered: Vec<_> = (1..=10u32)
            .zip(late)
            .map(|(i, late)| (batch * i + Duration::from_millis(late), samples(batch)))
            .collect();
        let (_, baseline) = replay(&steady, Duration::from_secs(2));
        let (accounted, silence) = replay(&jittered, Duration::from_secs(2));
        assert_close(accounted, Duration::from_secs(2));
        assert_eq!(silence, baseline);
    }

    /// `count` callbacks of `chunk` samples, the `k`th captured by
    /// `k * chunk` and delivered `late(k)` after that, in delivery order.
    fn jittered(
        chunk: usize,
        count: usize,
        mut late: impl FnMut(usize) -> Duration,
    ) -> Vec<(Duration, usize)> {
        let mut callbacks: Vec<_> = (1..=count)
            .map(|k| {
                let captured = Duration::from_secs_f64((k * chunk) as f64 / f64::from(RATE));
                (captured + late(k), chunk)
            })
            .collect();
        callbacks.sort_by_key(|&(at, _)| at);
        callbacks
    }

    /// Deterministic lateness in `0..=max_ms` ms.
    fn random_lateness(seed: u64, max_ms: u64) -> impl FnMut(usize) -> Duration {
        let mut state = seed;
        move |_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            Duration::from_millis(state % (max_ms + 1))
        }
    }

    /// Replays `callbacks` that hold `audio` of device time and checks that no
    /// silence was added and the analysed timeline matches the device's.
    fn assert_no_drift(callbacks: &[(Duration, usize)], audio: Duration) {
        let end = callbacks.last().unwrap().0 + POLL;
        let (accounted, silence) = replay(callbacks, end);
        assert!(silence <= samples(Duration::from_millis(5)), "{silence}");
        assert_close(accounted, audio);
    }

    #[test]
    fn sustained_jitter_without_loss_does_not_drift() {
        // 20 ms bursts every 20 ms, repeatedly up to 30 ms late, for 10 s.
        let pattern = [15, 30, 5, 27, 15, 0, 30];
        let burst = samples(Duration::from_millis(20));
        let fixed = jittered(burst, 500, |k| {
            Duration::from_millis(pattern[k % pattern.len()])
        });
        assert_no_drift(&fixed, Duration::from_secs(10));
        // Random lateness up to 30 ms on 20 ms callbacks for 40 s.
        let random = jittered(burst, 2000, random_lateness(0x9E37_79B9, 30));
        assert_no_drift(&random, Duration::from_secs(40));
        // 2048-frame callbacks with up to 30 ms lateness for about 40 s.
        let frames = jittered(2048, 940, random_lateness(0xC0FF_EE11, 30));
        assert_no_drift(&frames, Duration::from_secs_f64(940.0 * 2048.0 / 48_000.0));
    }

    #[test]
    fn a_short_loss_amid_jitter_is_filled() {
        // 20 ms callbacks up to 30 ms late for 10 s, with 80 ms lost at 5 s.
        let burst = samples(Duration::from_millis(20));
        let mut callbacks = jittered(burst, 500, random_lateness(0x5EED_1234, 30));
        let lost = Duration::from_secs(5)..Duration::from_millis(5080);
        callbacks.retain(|&(at, _)| !lost.contains(&at));
        let delivered = callbacks.len();
        assert!(delivered < 500, "{delivered}");
        let end = callbacks.last().unwrap().0 + POLL;
        let (accounted, silence) = replay(&callbacks, end);
        assert_close(accounted, Duration::from_secs(10));
        assert_close(
            silence,
            Duration::from_millis(20) * (500 - delivered) as u32,
        );
    }

    /// Publishes 10 ms callbacks until `delivered` reaches `until`.
    fn publish(producer: &mut rtrb::Producer<f32>, delivered: &mut Duration, until: Duration) {
        let chunk = vec![0.0f32; samples(Duration::from_millis(10))];
        while *delivered + Duration::from_millis(10) <= until {
            push_frames(producer, &chunk, 1, |_| panic!("ring overflowed"));
            *delivered += Duration::from_millis(10);
        }
    }

    /// 10 ms callbacks into a real ring, polled every 5 ms for 3 s. Twice the
    /// analysis thread pauses for `pause` while taking its snapshot (before
    /// the clock reads the time when `before_clock`, else just after), while
    /// the callback keeps publishing. Returns the silence and overlap.
    fn paused_snapshots(pause: Duration, before_clock: bool) -> (usize, usize) {
        let start = Instant::now();
        let (mut producer, mut consumer) =
            rtrb::RingBuffer::<f32>::new(samples(Duration::from_secs(1)));
        let dropped = AtomicUsize::new(0);
        let mut clock = SampleClock::new(RATE, start);
        let (mut elapsed, mut delivered) = (Duration::ZERO, Duration::ZERO);
        let (mut silence, mut overlap) = (0, 0);
        for poll in 0..600 {
            publish(&mut producer, &mut delivered, elapsed);
            // At 505 ms the previous poll has drained the ring, so the
            // snapshot taken before the pause is empty.
            let mut pending = poll % 200 == 101;
            let (now, lost, available) = snapshot(&dropped, &consumer, || {
                let read = start + elapsed;
                if std::mem::take(&mut pending) {
                    elapsed += pause;
                    publish(&mut producer, &mut delivered, elapsed);
                }
                if before_clock { start + elapsed } else { read }
            });
            let accounting = clock.poll(now, lost + available);
            silence += accounting.silence;
            overlap += accounting.overlap;
            Drain::plan(
                lost,
                available,
                accounting.overlap,
                samples(Duration::from_secs(1)),
            )
            .apply(&mut consumer, |_| {});
            elapsed += POLL;
        }
        (silence, overlap)
    }

    #[test]
    fn preemption_after_reading_the_clock_adds_no_silence() {
        let pause = Duration::from_millis(30);
        assert_eq!(paused_snapshots(pause, false), (0, 0));
    }

    #[test]
    fn a_pause_before_reading_the_clock_retakes_the_snapshot() {
        // Longer than the stall threshold: a stale empty snapshot paired
        // with the late time would be filled with silence.
        let pause = Duration::from_millis(150);
        assert_eq!(paused_snapshots(pause, true), (0, 0));
    }

    /// Loss-free 10 ms callbacks for 10 s; each second, those captured in
    /// `delays` (`from..to` ms into the second, ms late) arrive late.
    fn recurring_delays(delays: &[(u64, u64, u64)]) -> (usize, usize) {
        let chunk = samples(Duration::from_millis(10));
        let mut callbacks: Vec<_> = (1..=1000u64)
            .map(|i| {
                let at = 10 * i % 1000;
                let late = delays
                    .iter()
                    .find(|(from, to, _)| (*from..*to).contains(&at))
                    .map_or(0, |&(_, _, late)| late);
                (Duration::from_millis(10 * i + late), chunk)
            })
            .collect();
        callbacks.sort_by_key(|&(at, _)| at);
        replay(&callbacks, Duration::from_secs(10) + POLL)
    }

    #[test]
    fn recurring_delays_are_learned_as_jitter() {
        // Only the first delay may pass for a loss.
        let abrupt = [(500, 700, 30)];
        let stepped = [(500, 700, 30), (700, 740, 20), (740, 780, 10)];
        for delays in [&abrupt[..], &stepped[..]] {
            let (accounted, silence) = recurring_delays(delays);
            assert_close(accounted, Duration::from_secs(10));
            assert!(
                silence <= samples(Duration::from_millis(35)),
                "{delays:?}: {silence}"
            );
        }
    }

    #[test]
    fn fast_clock_after_an_outage_still_fills_a_later_loss() {
        // 10 ms callbacks from a device clock 500 ppm fast for 330 s, polled
        // every 5-8 ms, with a 1 s outage at 10 s, after which the timeline
        // keeps taking back the outage's silence as the clock gains. A 40 ms
        // loss at 300 s must still be filled.
        let chunk = samples(Duration::from_millis(10));
        let lost = |i: u64| (30_001..=30_004).contains(&i);
        let run = |lose: bool| {
            let start = Instant::now();
            let mut clock = SampleClock::new(RATE, start);
            let mut gap = random_lateness(0xFA57, 3);
            let (mut next, mut elapsed, mut silence) = (1u64, Duration::ZERO, 0);
            while next <= 33_000 {
                elapsed += POLL + gap(0);
                let mut real = 0;
                while next <= 33_000
                    && Duration::from_secs_f64(next as f64 * 0.01 / 1.0005) <= elapsed
                {
                    let outage = (1001..=1100).contains(&next);
                    if !(outage || lose && lost(next)) {
                        real += chunk;
                    }
                    next += 1;
                }
                silence += clock.poll(start + elapsed, real).silence;
            }
            silence
        };
        let filled = run(true).abs_diff(run(false));
        assert_close(filled, Duration::from_millis(40));
    }

    #[test]
    fn a_take_back_restarts_loss_confirmation() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut loss = LossDetector::new(start);
        assert_eq!(loss.check(at(0), 0.0, 1.0), 0.0);
        // 30 ms of excess lag for 50 ms is filled.
        assert_eq!(loss.check(at(10), 0.03, 1.0), 0.0);
        assert_eq!(loss.check(at(60), 0.03, 1.0), 0.03);
        // An excess run starts, a take-back interrupts it, and the excess
        // returns: it must be confirmed for a fresh 50 ms.
        assert_eq!(loss.check(at(100), 0.09, 1.0), 0.0);
        assert!(loss.check(at(150), -0.01, 1.0) < 0.0);
        assert_eq!(loss.check(at(160), 0.09, 1.0), 0.0);
        assert_eq!(loss.check(at(210), 0.09, 1.0), 0.09);
    }

    /// Polls every 10 ms over `from..to`, delivering 10 ms of audio per poll
    /// when `deliver`, and returns the silence synthesized.
    fn poll_span(
        clock: &mut SampleClock,
        start: Instant,
        span: (u64, u64),
        deliver: bool,
    ) -> usize {
        let chunk = samples(Duration::from_millis(10));
        (span.0 / 10 + 1..=span.1 / 10)
            .map(|step| {
                let at = start + Duration::from_millis(step * 10);
                clock.poll(at, if deliver { chunk } else { 0 }).silence
            })
            .sum()
    }

    #[test]
    fn stall_threshold_recovers_after_one_starvation() {
        let start = Instant::now();
        let mut clock = SampleClock::new(RATE, start);
        // Steady audio, then the analysis thread is starved for 400 ms and
        // finds all of it waiting at once, which widens the threshold.
        poll_span(&mut clock, start, (0, 500), true);
        let late = clock.poll(
            start + Duration::from_millis(900),
            samples(Duration::from_millis(400)),
        );
        assert_eq!(late.silence, 0);
        assert!(clock.stall > STALL);
        // After well over STALL_RESET of normal deliveries, a real outage is
        // detected at the base threshold again.
        poll_span(&mut clock, start, (900, 2500), true);
        assert_eq!(clock.stall, STALL);
        let silence = poll_span(&mut clock, start, (2500, 2750), false);
        assert!(silence >= samples(Duration::from_millis(100)), "{silence}");
    }
}
