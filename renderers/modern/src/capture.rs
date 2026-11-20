//! `chipviz-capture`: renders selected frames through the real renderer
//! pipeline into an offscreen texture and writes PNGs plus `manifest.json`.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::audio::analysis::Analyzer;
use crate::audio::mic_frame;
use crate::audio::scope::{ScopeSnapshot, ScopeTap};
use crate::control_frame::{ControlFrame, WIRE_SIZE};
use crate::profile::RenderProfile;
use crate::renderer::{OffscreenRenderer, ScenePipeline};
use crate::scenes::{self, CameraMode, SceneConfig, colour_field, mic_layers, starfield};

pub use crate::scenes::TunnelVariant;

/// Capture time base: frame `N` is rendered at `elapsed = N / CAPTURE_FPS`.
pub const CAPTURE_FPS: f32 = 60.0;

const USAGE: &str = "usage: chipviz-capture --out DIR [--profile m1|pi5] [--scene N] [--starfield]
                       [--frames 0,60,120] [--input procedural|synth-mic|PATH.cvz]
                       [--size WxH] [--mic-layers off|scope|bursts|both]
                       [--catalog FILE --location LAT,LON]
                       [--time UNIX_SECONDS] [--follow-time]
                       [--lines-drop-cuts] [--tunnel-variant original|hallway|abstract]
                       [--exclusive-output]

Renders frames offscreen (no window) and writes PNGs plus manifest.json to DIR.
  --profile  render budget (default m1)
  --scene    force the control-frame scene byte: 7 is the colour field;
             scene 6 is legacy SignalField unless paired with catalog and
             location; other bytes keep the original modulo-six mapping
  --frames   comma-separated frame indices (default 0); elapsed = index / 60 s
  --input    procedural frames (default), synth-mic (a deterministic synthetic
             microphone: tones plus a click every 0.5 s, silent from 6 s, run
             through the live analyzer and waveform tap) or a raw .cvz file
  --size     output size in pixels (default: the profile's target size)
  --mic-layers
             overlay the mic layers: off (default), scope, bursts or both;
             only builds with the unmeasured-flash feature draw them
  --lines-drop-cuts
             Lines (scene 3) cuts on a rising drop flag, as the renderer's
             flag of the same name (default: drops are ignored); only
             .cvz inputs carry drop flags, procedural input never does
  --star-catalog PATH
             user-supplied HYG-compatible star CSV for scene 6; not bundled
  --catalog  alias for --star-catalog
  --location observer latitude,longitude in decimal degrees; longitude east
  --starfield explicitly opt into Starfield in recorded scene-6 frames
             (requires --star-catalog and --location; may be used without
             --scene to replay scene transitions)
  --time     observation UTC as Unix seconds (1800..2200); defaults to current UTC
  --follow-time advance from --time (or current UTC), for reproducible moving-sky captures
  --tunnel-variant
             PulseTunnel look: original (default), hallway or abstract
  --exclusive-output
             create PNGs and manifest exclusively; fail on existing leaves or
             duplicate frames. Requires a trusted, non-mutable output directory;
             newly created files (including partial files) may remain on failure.
  With --exclusive-output, prefix a literal path named like an option with ./
  (for example ./--out).";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Procedural,
    SynthMic,
    File(PathBuf),
}

#[derive(Clone, Debug)]
pub struct CaptureArgs {
    pub profile: RenderProfile,
    pub scene: Option<u8>,
    pub frames: Vec<u64>,
    pub input: Input,
    pub size: (u32, u32),
    pub out: PathBuf,
    pub mic_layers: mic_layers::Mode,
    pub lines_drop_cuts: bool,
}

/// Additional capture options, kept separate to preserve `CaptureArgs` literals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureOptions {
    pub tunnel_variant: TunnelVariant,
}

/// Programmatic Starfield setup is separate from the stable public capture
/// argument and option structs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StarfieldOptions {
    pub catalog_path: Option<PathBuf>,
    pub location: Option<(f64, f64)>,
    pub unix_seconds: Option<f64>,
    pub follow_time: bool,
}

/// Combined capture and Starfield settings.
#[derive(Clone, Debug, PartialEq)]
pub struct StarfieldCaptureOptions {
    pub capture: CaptureOptions,
    pub starfield: StarfieldOptions,
    /// Whether this capture opts into Starfield. Public CLI parsing records
    /// the exact opt-in here so parse/runner round trips preserve dispatch.
    pub starfield_enabled: bool,
}

impl Default for StarfieldCaptureOptions {
    fn default() -> Self {
        Self {
            capture: CaptureOptions::default(),
            starfield: StarfieldOptions::default(),
            // Calling the Starfield-specific API directly is itself opt-in.
            starfield_enabled: true,
        }
    }
}

#[derive(Debug)]
pub enum Command {
    Help,
    Capture(CaptureArgs),
}

/// Parsed capture command with the optional extended configuration.
#[derive(Debug)]
pub enum ConfiguredCommand {
    Help,
    Capture(CaptureArgs, CaptureOptions),
}

/// Parsed capture command with explicit Starfield setup.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // Keep the new public command variant stable.
pub enum StarfieldConfiguredCommand {
    Help,
    Capture(CaptureArgs, StarfieldCaptureOptions),
}

/// Parse the original capture arguments. Extended flags are rejected rather
/// than silently dropped; use one of the configured APIs for those flags.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    match parse_args_internal(args, false, false, false, false)?.0 {
        ConfiguredCommand::Help => Ok(Command::Help),
        ConfiguredCommand::Capture(args, _) => Ok(Command::Capture(args)),
    }
}

/// Parse capture arguments including `--tunnel-variant`, preserving the
/// #139 public command enum arity and parser signature.
pub fn parse_args_with_options(
    args: impl IntoIterator<Item = String>,
) -> Result<ConfiguredCommand, String> {
    parse_args_internal(args, true, false, false, false).map(|(command, _, _, _)| command)
}

/// Parse capture arguments with both tunnel and Starfield options.
pub fn parse_args_with_starfield_options(
    args: impl IntoIterator<Item = String>,
) -> Result<StarfieldConfiguredCommand, String> {
    parse_starfield_args(args, false).map(|(command, _)| command)
}

fn parse_starfield_args(
    args: impl IntoIterator<Item = String>,
    allow_exclusive: bool,
) -> Result<(StarfieldConfiguredCommand, bool), String> {
    let (parsed, starfield, explicit_starfield, exclusive) = if allow_exclusive {
        // Inspect the raw CLI tokens so the opt-in cannot be swallowed as a
        // value and also protects missing values appearing before the flag.
        let args: Vec<_> = args.into_iter().collect();
        let strict_values = args.iter().any(|arg| arg == "--exclusive-output");
        parse_args_internal(args, true, true, true, strict_values)?
    } else {
        parse_args_internal(args, true, true, false, false)?
    };
    let command = match parsed {
        ConfiguredCommand::Help => StarfieldConfiguredCommand::Help,
        ConfiguredCommand::Capture(args, capture_options) => {
            if exclusive {
                validate_unique_frames(&args.frames)?;
            }
            let starfield_enabled = cli_starfield_opt_in(&args, &starfield, explicit_starfield);
            StarfieldConfiguredCommand::Capture(
                args,
                StarfieldCaptureOptions {
                    capture: capture_options,
                    starfield,
                    starfield_enabled,
                },
            )
        }
    };
    Ok((command, exclusive))
}

fn parse_args_internal(
    args: impl IntoIterator<Item = String>,
    allow_capture_options: bool,
    allow_starfield_options: bool,
    allow_exclusive: bool,
    strict_values: bool,
) -> Result<(ConfiguredCommand, StarfieldOptions, bool, bool), String> {
    let mut profile = RenderProfile::m1();
    let mut scene = None;
    let mut frames = vec![0];
    let mut input = Input::Procedural;
    let mut size = None;
    let mut out = None;
    let mut layers = mic_layers::Mode::Off;
    let mut lines_drop_cuts = false;
    let mut catalog_path = None;
    let mut location = None;
    let mut unix_seconds = None;
    let mut follow_time = false;
    let mut explicit_starfield = false;
    let mut exclusive = false;
    let mut tunnel_variant = TunnelVariant::Original;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Ok((
                ConfiguredCommand::Help,
                StarfieldOptions::default(),
                false,
                false,
            ));
        }
        let mut value = || {
            let value = args
                .next()
                .ok_or_else(|| format!("{arg} requires a value"))?;
            if strict_values && is_cli_option(&value) {
                return Err(format!(
                    "{arg} requires a value; {value} is an option (use ./{value} for a literal path)"
                ));
            }
            Ok(value)
        };
        // Keep new options in is_cli_option and the exhaustive
        // cli_rejects_all_known_options_as_values_in_any_exclusive_flag_order test.
        match arg.as_str() {
            "--profile" => {
                profile = match value()?.as_str() {
                    "m1" => RenderProfile::m1(),
                    "pi5" => RenderProfile::pi5(),
                    other => return Err(format!("unknown profile: {other} (expected m1|pi5)")),
                }
            }
            "--scene" => {
                let text = value()?;
                let parsed = text
                    .parse()
                    .map_err(|_| format!("--scene expects 0-255, got {text}"))?;
                scene = Some(parsed);
            }
            "--frames" => {
                let text = value()?;
                frames = text
                    .split(',')
                    .map(|item| {
                        item.trim().parse().map_err(|_| {
                            format!("--frames expects indices like 0,60,120, got {text}")
                        })
                    })
                    .collect::<Result<_, _>>()?;
            }
            "--input" => {
                let text = value()?;
                input = match text.as_str() {
                    "procedural" => Input::Procedural,
                    "synth-mic" => Input::SynthMic,
                    _ => Input::File(PathBuf::from(text)),
                };
            }
            "--size" => {
                let text = value()?;
                let parsed = text
                    .split_once('x')
                    .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)))
                    .filter(|&(width, height): &(u32, u32)| width > 0 && height > 0);
                size = Some(parsed.ok_or_else(|| format!("--size expects WxH, got {text}"))?);
            }
            "--out" => out = Some(PathBuf::from(value()?)),
            "--mic-layers" => {
                let text = value()?;
                layers = mic_layers::Mode::parse(&text).ok_or_else(|| {
                    format!("--mic-layers expects off|scope|bursts|both, got {text}")
                })?;
            }
            "--lines-drop-cuts" => lines_drop_cuts = true,
            "--exclusive-output" => {
                if !allow_exclusive {
                    return Err("--exclusive-output is only supported by the capture CLI".into());
                }
                exclusive = true;
            }
            "--tunnel-variant" => {
                if !allow_capture_options {
                    return Err("--tunnel-variant requires parse_args_with_options".into());
                }
                let text = value()?;
                tunnel_variant = TunnelVariant::parse(&text).ok_or_else(|| {
                    format!("--tunnel-variant expects original|hallway|abstract, got {text}")
                })?;
            }
            "--catalog" | "--star-catalog" => {
                if !allow_starfield_options {
                    return Err("--star-catalog requires parse_args_with_starfield_options".into());
                }
                catalog_path = Some(PathBuf::from(value()?));
            }
            "--location" => {
                if !allow_starfield_options {
                    return Err("--location requires parse_args_with_starfield_options".into());
                }
                location = Some(parse_location(&value()?)?);
            }
            "--time" => {
                if !allow_starfield_options {
                    return Err("--time requires parse_args_with_starfield_options".into());
                }
                let text = value()?;
                let seconds = text
                    .parse::<f64>()
                    .map_err(|_| format!("--time must be finite UNIX seconds, got {text}"))?;
                if !seconds.is_finite() || !(-5_364_662_400.0..7_289_654_400.0).contains(&seconds) {
                    return Err(format!(
                        "--time must be finite UNIX seconds in the UTC year 1800..2200 range, got {text}"
                    ));
                }
                unix_seconds = Some(seconds);
            }
            "--follow-time" => {
                if !allow_starfield_options {
                    return Err("--follow-time requires parse_args_with_starfield_options".into());
                }
                follow_time = true;
            }
            "--starfield" => {
                if !allow_starfield_options {
                    return Err("--starfield requires parse_args_with_starfield_options".into());
                }
                explicit_starfield = true;
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }

    let out = out.ok_or("--out DIR is required")?;
    if allow_starfield_options && catalog_path.is_some() != location.is_some() {
        return Err("--star-catalog and --location must be supplied together for scene 6".into());
    }
    if explicit_starfield && catalog_path.is_none() {
        return Err("--starfield requires --star-catalog PATH and --location LAT,LON".into());
    }
    if allow_starfield_options && unix_seconds.is_some() && catalog_path.is_none() {
        return Err("--time requires --star-catalog and --location".into());
    }
    if allow_starfield_options && follow_time && catalog_path.is_none() {
        return Err("--follow-time requires --star-catalog and --location".into());
    }
    let size = size.unwrap_or((profile.target_width, profile.target_height));
    Ok((
        ConfiguredCommand::Capture(
            CaptureArgs {
                profile,
                scene,
                frames,
                input,
                size,
                out,
                mic_layers: layers,
                lines_drop_cuts,
            },
            CaptureOptions { tunnel_variant },
        ),
        StarfieldOptions {
            catalog_path,
            location,
            unix_seconds,
            follow_time,
        },
        explicit_starfield,
        exclusive,
    ))
}

fn is_cli_option(value: &str) -> bool {
    matches!(
        value,
        "--help"
            | "-h"
            | "--profile"
            | "--scene"
            | "--frames"
            | "--input"
            | "--size"
            | "--out"
            | "--mic-layers"
            | "--lines-drop-cuts"
            | "--exclusive-output"
            | "--tunnel-variant"
            | "--catalog"
            | "--star-catalog"
            | "--location"
            | "--time"
            | "--follow-time"
            | "--starfield"
    )
}

fn validate_unique_frames(frames: &[u64]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for &frame in frames {
        if !seen.insert(frame) {
            return Err(format!("duplicate output frame: {frame}"));
        }
    }
    Ok(())
}

fn parse_location(value: &str) -> Result<(f64, f64), String> {
    let (latitude, longitude) = value
        .split_once(',')
        .filter(|(_, longitude)| !longitude.contains(','))
        .ok_or_else(|| {
            format!("--location expects exactly LAT,LON decimal degrees, got {value}")
        })?;
    let latitude = latitude
        .parse::<f64>()
        .map_err(|_| format!("--location latitude must be finite, got {latitude}"))?;
    let longitude = longitude
        .parse::<f64>()
        .map_err(|_| format!("--location longitude must be finite, got {longitude}"))?;
    if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
        return Err(format!(
            "--location latitude must be finite and in [-90, 90], got {latitude}"
        ));
    }
    if !longitude.is_finite() || !(-180.0..=180.0).contains(&longitude) {
        return Err(format!(
            "--location longitude must be finite and in [-180, 180] (east positive), got {longitude}"
        ));
    }
    Ok((latitude, longitude))
}

/// Splits a `.cvz` file (concatenated control-frame-v0 packets) into frames.
pub fn parse_cvz(bytes: &[u8]) -> Result<Vec<ControlFrame>, String> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(WIRE_SIZE) {
        return Err(format!(
            "expected a non-empty multiple of {WIRE_SIZE} bytes, got {}",
            bytes.len()
        ));
    }
    bytes
        .chunks_exact(WIRE_SIZE)
        .enumerate()
        .map(|(index, packet)| {
            ControlFrame::parse_wire(packet).map_err(|error| format!("frame {index}: {error}"))
        })
        .collect()
}

struct FrameSource {
    fixture: Option<Vec<ControlFrame>>,
    scene: Option<u8>,
}

impl FrameSource {
    fn frame(&self, index: u64) -> Result<ControlFrame, String> {
        let frame = match &self.fixture {
            None => ControlFrame::procedural(index),
            Some(frames) => *usize::try_from(index)
                .ok()
                .and_then(|index| frames.get(index))
                .ok_or_else(|| {
                    format!("frame {index} is past the input's {} frames", frames.len())
                })?,
        };
        Ok(ControlFrame {
            scene: self.scene.unwrap_or(frame.scene),
            ..frame
        })
    }
}

fn elapsed(index: u64) -> f64 {
    index as f64 / f64::from(CAPTURE_FPS)
}

pub const SYNTH_RATE: u32 = 48_000;
const SYNTH_SAMPLES_PER_FRAME: u64 = SYNTH_RATE as u64 / CAPTURE_FPS as u64;
const SYNTH_BEAT_S: f64 = 0.5;
const SYNTH_SILENT_FROM_S: f64 = 6.0;

/// Sample `n` of the synthetic microphone: a 110 Hz tone with its third
/// harmonic, plus a 40 ms noise click every 0.5 s (120 BPM); silent from
/// 6 s on, so later frames show the layers decaying.
pub fn synth_sample(n: u64) -> f32 {
    let t = n as f64 / f64::from(SYNTH_RATE);
    if t >= SYNTH_SILENT_FROM_S {
        return 0.0;
    }
    let tau = std::f64::consts::TAU;
    let tone = 0.25 * (tau * 110.0 * t).sin() + 0.12 * (tau * 330.0 * t + 0.5).sin();
    let since = t % SYNTH_BEAT_S;
    let click = if since < 0.04 {
        let noise = f64::from(scenes::hash(n as u32)) * 2.0 - 1.0;
        0.6 * (-since / 0.008).exp() * noise
    } else {
        0.0
    };
    (tone + click) as f32
}

/// `--input synth-mic`: feeds [`synth_sample`] through the live analyzer and
/// waveform tap, one capture frame at a time, as the analysis thread would.
struct SynthMic {
    analyzer: Analyzer,
    tap: ScopeTap,
    next_sample: u64,
    beat_count: Option<u16>,
}

impl SynthMic {
    fn new() -> Result<Self, String> {
        let analyzer = Analyzer::new(SYNTH_RATE)?;
        Ok(Self {
            tap: ScopeTap::new(analyzer.sample_rate()),
            analyzer,
            next_sample: 0,
            beat_count: None,
        })
    }

    /// Feeds the audio up to frame `index`'s time; returns the mic frame
    /// (built on `base`, as live mic frames are) and the scope snapshot.
    fn advance(&mut self, base: ControlFrame, index: u64) -> (ControlFrame, ScopeSnapshot) {
        let end = index * SYNTH_SAMPLES_PER_FRAME;
        let samples: Vec<f32> = (self.next_sample..end).map(synth_sample).collect();
        self.next_sample = self.next_sample.max(end);
        self.analyzer.push(&samples);
        self.tap.push(&samples);
        let features = self.analyzer.features();
        let frame = mic_frame(base, features, self.beat_count);
        self.beat_count = Some(features.beat_count);
        let snapshot = ScopeSnapshot {
            onset_age_s: self.analyzer.onset_age_s(),
            ..self.tap.snapshot(self.analyzer.onsets() as u32)
        };
        (frame, snapshot)
    }
}

/// Hands the pipeline frame `index`'s side-channel snapshot (synthetic mic
/// only) and returns the frame to draw.
fn feed(
    pipeline: &mut ScenePipeline,
    synth: Option<&mut SynthMic>,
    base: ControlFrame,
    index: u64,
) -> ControlFrame {
    match synth {
        Some(synth) => {
            let (frame, scope) = synth.advance(base, index);
            pipeline.set_live_scope(Some(scope));
            frame
        }
        None => {
            pipeline.set_live_scope(None);
            base
        }
    }
}

pub fn run(args: &CaptureArgs) -> Result<(), String> {
    run_with_options(args, CaptureOptions::default())
}

/// Capture using #139's tunnel options. Legacy scene bytes, including scene 6,
/// keep their original native mapping in this API.
pub fn run_with_options(args: &CaptureArgs, options: CaptureOptions) -> Result<(), String> {
    run_internal(args, options, &StarfieldOptions::default(), false, false)
}

/// Capture with an explicit Starfield opt-in and its separate configuration.
/// Unlike the CLI's forced-scene option, this API enables Starfield for every
/// scene-6 frame in a recording, including when `args.scene` is `None`.
pub fn run_with_starfield_options(
    args: &CaptureArgs,
    options: &StarfieldCaptureOptions,
) -> Result<(), String> {
    run_internal(
        args,
        options.capture,
        &options.starfield,
        options.starfield_enabled,
        false,
    )
}

fn cli_starfield_opt_in(
    args: &CaptureArgs,
    options: &StarfieldOptions,
    explicit_starfield: bool,
) -> bool {
    explicit_starfield
        || (args.scene == Some(starfield::SCENE)
            && options.catalog_path.is_some()
            && options.location.is_some())
}

fn run_internal(
    args: &CaptureArgs,
    options: CaptureOptions,
    starfield_options: &StarfieldOptions,
    starfield_enabled: bool,
    exclusive: bool,
) -> Result<(), String> {
    if exclusive {
        validate_unique_frames(&args.frames)?;
    }
    if starfield_options.catalog_path.is_some() != starfield_options.location.is_some() {
        return Err("--star-catalog and --location must be supplied together for scene 6".into());
    }
    if starfield_options.unix_seconds.is_some() && starfield_options.catalog_path.is_none() {
        return Err("--time requires --star-catalog and --location".into());
    }
    if starfield_options.follow_time && starfield_options.catalog_path.is_none() {
        return Err("--follow-time requires --star-catalog and --location".into());
    }
    if let Some((latitude, longitude)) = starfield_options.location
        && (!latitude.is_finite()
            || !(-90.0..=90.0).contains(&latitude)
            || !longitude.is_finite()
            || !(-180.0..=180.0).contains(&longitude))
    {
        return Err("--location must be finite LAT,LON in [-90,90] and [-180,180]".into());
    }
    if let Some(seconds) = starfield_options.unix_seconds
        && (!seconds.is_finite() || !(-5_364_662_400.0..7_289_654_400.0).contains(&seconds))
    {
        return Err("--time must be finite UNIX seconds in the UTC year 1800..2200 range".into());
    }
    if starfield_enabled
        && args.scene == Some(starfield::SCENE)
        && starfield_options.catalog_path.is_none()
    {
        return Err("scene 6 (Starfield) needs --star-catalog PATH and --location LAT,LON".into());
    }
    let source = FrameSource {
        fixture: match &args.input {
            Input::Procedural | Input::SynthMic => None,
            Input::File(path) => {
                let bytes =
                    std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
                Some(parse_cvz(&bytes).map_err(|error| format!("{}: {error}", path.display()))?)
            }
        },
        scene: args.scene,
    };
    // Always retain direct fixture bounds checks. Only scan the full replay
    // when an enabled Starfield lacks a catalog and missing-input validation
    // is actually needed.
    preflight_replay(
        &source,
        args.frames.iter().copied().max(),
        starfield_enabled,
        starfield_options.catalog_path.is_some(),
    )?;
    let starfield = match (&starfield_options.catalog_path, starfield_options.location) {
        (Some(path), Some((latitude_degrees, longitude_degrees))) => {
            let catalog = starfield::StarCatalog::load(path).map_err(|error| error.to_string())?;
            let unix_seconds = starfield_options.unix_seconds.unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs_f64())
                    .unwrap_or_else(|error| -error.duration().as_secs_f64())
            });
            Some(starfield::StarfieldConfig {
                catalog: std::sync::Arc::new(catalog),
                latitude_degrees,
                longitude_degrees,
                unix_seconds,
                follow_elapsed: starfield_options.unix_seconds.is_none()
                    || starfield_options.follow_time,
            })
        }
        _ => None,
    };
    std::fs::create_dir_all(&args.out)
        .map_err(|error| format!("{}: {error}", args.out.display()))?;
    let (width, height) = args.size;
    let mut renderer =
        pollster::block_on(OffscreenRenderer::new(args.profile.clone(), width, height))
            .map_err(|error| error.to_string())?;
    // Recorded frames carry wire palettes; procedural frames use a local
    // palette 0. Default builds never flash. With `unmeasured-flash`, as on a
    // target, only a recording's stable absolute wire 3 or 7 held in scene 7
    // can arm flash/gate; procedural input never arms.
    renderer
        .pipeline_mut()
        .configure_colour_field(colour_field::Config {
            palette: match args.input {
                Input::Procedural | Input::SynthMic => {
                    colour_field::PaletteSource::Local { p: 0, armed: false }
                }
                Input::File(_) => colour_field::PaletteSource::Wire,
            },
            ..colour_field::Config::default()
        });
    // Default builds draw no mic layers (see `mic_layers::LAYERS_BUILD`).
    let layers = mic_layers::allowed(args.mic_layers);
    if layers != args.mic_layers {
        eprintln!("{}", mic_layers::NO_LAYERS_NOTICE);
    }
    renderer.pipeline_mut().set_mic_layers(layers);
    renderer.pipeline_mut().configure_scenes(SceneConfig {
        lines_drop_cuts: args.lines_drop_cuts,
        tunnel_variant: options.tunnel_variant,
    });
    renderer
        .pipeline_mut()
        .configure_starfield(starfield.clone());
    renderer
        .pipeline_mut()
        .enable_starfield_dispatch(starfield_enabled);
    let mut captures = Vec::new();
    let mut outputs = OutputFiles::new(exclusive);

    for &index in &args.frames {
        // Replay camera smoothing and the colour field from frame 0 so stills
        // match a live run.
        let size = renderer.size();
        let mut synth = match args.input {
            Input::SynthMic => Some(SynthMic::new()?),
            _ => None,
        };
        let pipeline = renderer.pipeline_mut();
        pipeline.reset_state();
        pipeline.configure_starfield(starfield.clone());
        pipeline.enable_starfield_dispatch(starfield_enabled);
        for previous in 0..index {
            let frame = feed(pipeline, synth.as_mut(), source.frame(previous)?, previous);
            pipeline.advance_state(frame, elapsed(previous), size);
        }
        let frame = feed(pipeline, synth.as_mut(), source.frame(index)?, index);
        let rgb = renderer
            .render(frame, elapsed(index))
            .map_err(|error| error.to_string())?;
        let displayed_scene = renderer.pipeline_mut().committed_displayed_scene();
        let entry = scenes::dispatch_configured(displayed_scene, starfield_enabled);
        let name = match (entry.name(), options.tunnel_variant) {
            ("PulseTunnel", TunnelVariant::Hallway) => "PulseTunnel-Hallway",
            ("PulseTunnel", TunnelVariant::Abstract) => "PulseTunnel-Abstract",
            _ => entry.name(),
        };
        let camera =
            if entry.name() == "PulseTunnel" && options.tunnel_variant != TunnelVariant::Original {
                CameraMode::ScreenSpace
            } else {
                entry.camera()
            };
        let file = format!("frame{index:06}-{name}.png");
        write_png(&mut outputs, &args.out.join(&file), width, height, &rgb)?;
        println!("wrote {}", args.out.join(&file).display());
        let frame_starfield_epoch = starfield.as_ref().map_or_else(
            || "null".to_string(),
            |config| config.epoch_at(elapsed(index) as f32).to_string(),
        );
        captures.push(manifest_capture_entry(ManifestCaptureEntry {
            file: &file,
            frame_index: index,
            elapsed: elapsed(index),
            starfield_epoch: frame_starfield_epoch,
            requested_scene: frame.scene,
            displayed_scene,
            scene_name: name,
            camera,
        }));
    }

    let input = match &args.input {
        Input::Procedural => "procedural".to_string(),
        Input::SynthMic => "synth-mic".to_string(),
        Input::File(path) => path.display().to_string(),
    };
    let (starfield_epoch, starfield_advances) = starfield_metadata(starfield.as_ref());
    let manifest = format!(
        "{{\n  \"profile\": {},\n  \"input\": {},\n  \"mic_layers\": {},\n  \"width\": {width},\n  \"height\": {height},\n  \"fps\": {CAPTURE_FPS},\n  \"starfield_epoch_unix_seconds\": {starfield_epoch},\n  \"starfield_epoch_advances_with_elapsed\": {starfield_advances},\n  \"captures\": [\n{}\n  ]\n}}\n",
        json_string(args.profile.name),
        json_string(&input),
        json_string(layers.name()),
        captures.join(",\n"),
    );
    let path = args.out.join("manifest.json");
    write_manifest(&mut outputs, &path, manifest.as_bytes())
}

fn starfield_metadata(config: Option<&starfield::StarfieldConfig>) -> (String, String) {
    match config {
        Some(config) => (
            config.unix_seconds.to_string(),
            config.follow_elapsed.to_string(),
        ),
        None => ("null".to_string(), "null".to_string()),
    }
}

struct ManifestCaptureEntry<'a> {
    file: &'a str,
    frame_index: u64,
    elapsed: f64,
    starfield_epoch: String,
    requested_scene: u8,
    displayed_scene: u8,
    scene_name: &'a str,
    camera: CameraMode,
}

fn manifest_capture_entry(entry: ManifestCaptureEntry<'_>) -> String {
    format!(
        "    {{\"file\": {}, \"frame_index\": {}, \"elapsed\": {}, \"starfield_epoch_unix_seconds\": {}, \"scene\": {}, \"displayed_scene\": {}, \"scene_name\": {}, \"camera\": {}}}",
        json_string(entry.file),
        entry.frame_index,
        entry.elapsed,
        entry.starfield_epoch,
        entry.requested_scene,
        entry.displayed_scene,
        json_string(entry.scene_name),
        json_string(match entry.camera {
            CameraMode::Orbit3D => "orbit3d",
            CameraMode::ScreenSpace => "screen_space",
        }),
    )
}

#[cfg(test)]
fn displayed_scene_at(
    source: &FrameSource,
    index: u64,
    starfield_enabled: bool,
) -> Result<u8, String> {
    let mut field = colour_field::ColourField::new(colour_field::Config::default());
    let mut display = colour_field::Display::Native(0);
    for frame_index in 0..=index {
        field.observe(&source.frame(frame_index)?);
        if field.has_new_frame() {
            field.step(colour_field::Controls::default());
        }
        display = field.display();
    }
    Ok(match display {
        colour_field::Display::Native(scene) => {
            scenes::normalize_scene_byte(scene, starfield_enabled)
        }
        colour_field::Display::ColourField(_) => colour_field::SCENE,
    })
}

fn validate_replay_catalog(
    source: &FrameSource,
    last_index: u64,
    catalog_available: bool,
    starfield_enabled: bool,
) -> Result<(), String> {
    let mut field = colour_field::ColourField::new(colour_field::Config::default());
    for frame_index in 0..=last_index {
        field.observe(&source.frame(frame_index)?);
        if field.has_new_frame() {
            field.step(colour_field::Controls::default());
        }
        let displayed = match field.display() {
            colour_field::Display::Native(scene) => {
                scenes::normalize_scene_byte(scene, starfield_enabled)
            }
            colour_field::Display::ColourField(_) => colour_field::SCENE,
        };
        if starfield_enabled && displayed == starfield::SCENE && !catalog_available {
            return Err(format!(
                "replay frame {frame_index}: scene 6 (Starfield) needs --star-catalog PATH and --location LAT,LON"
            ));
        }
    }
    Ok(())
}

fn preflight_replay(
    source: &FrameSource,
    last_index: Option<u64>,
    starfield_enabled: bool,
    catalog_available: bool,
) -> Result<(), String> {
    let Some(last_index) = last_index else {
        return Ok(());
    };
    // Bounds errors take precedence over a later missing-catalog replay error.
    source.frame(last_index)?;
    if starfield_enabled && !catalog_available {
        validate_replay_catalog(source, last_index, false, true)
    } else {
        // Do not replay the ColourField solely for validation when Starfield
        // is unavailable or already configured.
        Ok(())
    }
}

struct OutputFiles {
    exclusive: bool,
    paths: HashSet<PathBuf>,
}

impl OutputFiles {
    fn new(exclusive: bool) -> Self {
        Self {
            exclusive,
            paths: HashSet::new(),
        }
    }

    fn create(&mut self, path: &Path) -> Result<std::fs::File, String> {
        if self.exclusive && !self.paths.insert(path.to_path_buf()) {
            return Err(format!("duplicate output path: {}", path.display()));
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(!self.exclusive)
            .create_new(self.exclusive)
            .open(path)
            .map_err(|error| format!("{}: {error}", path.display()))
    }
}

fn write_manifest(outputs: &mut OutputFiles, path: &Path, manifest: &[u8]) -> Result<(), String> {
    outputs
        .create(path)?
        .write_all(manifest)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn write_png(
    outputs: &mut OutputFiles,
    path: &Path,
    width: u32,
    height: u32,
    rgb: &[u8],
) -> Result<(), String> {
    let error = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    let file = outputs.create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(|e| error(&e))?;
    writer.write_image_data(rgb).map_err(|e| error(&e))?;
    writer.finish().map_err(|e| error(&e))
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            control if u32::from(control) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", u32::from(control)));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

pub fn main() -> ExitCode {
    match parse_starfield_args(std::env::args().skip(1), true) {
        Ok((StarfieldConfiguredCommand::Help, _)) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok((StarfieldConfiguredCommand::Capture(args, options), exclusive)) => {
            match run_internal(
                &args,
                options.capture,
                &options.starfield,
                options.starfield_enabled,
                exclusive,
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("chipviz-capture: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("chipviz-capture: {error}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phosphor;

    fn output_test_dir() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "chipviz-capture-output-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn exclusive_outputs_preserve_existing_png_and_manifest_and_default_overwrites() {
        let root = output_test_dir();
        let png = root.join("frame000000-SignalField.png");
        let manifest = root.join("manifest.json");
        std::fs::write(&png, b"existing png").unwrap();
        std::fs::write(&manifest, b"existing manifest").unwrap();

        let mut exclusive = OutputFiles::new(true);
        assert!(write_png(&mut exclusive, &png, 1, 1, &[1, 2, 3]).is_err());
        assert!(write_manifest(&mut exclusive, &manifest, b"new manifest").is_err());
        assert_eq!(std::fs::read(&png).unwrap(), b"existing png");
        assert_eq!(std::fs::read(&manifest).unwrap(), b"existing manifest");

        let mut legacy = OutputFiles::new(false);
        write_png(&mut legacy, &png, 1, 1, &[1, 2, 3]).unwrap();
        write_manifest(&mut legacy, &manifest, b"new manifest").unwrap();
        assert!(
            std::fs::read(&png)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
        assert_eq!(std::fs::read(&manifest).unwrap(), b"new manifest");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_exclusive_outputs_match_legacy_bytes() {
        let root = output_test_dir();
        let mut exclusive = OutputFiles::new(true);
        let mut legacy = OutputFiles::new(false);
        let exclusive_png = root.join("exclusive.png");
        let legacy_png = root.join("legacy.png");
        let exclusive_manifest = root.join("exclusive.json");
        let legacy_manifest = root.join("legacy.json");
        write_png(&mut exclusive, &exclusive_png, 1, 1, &[1, 2, 3]).unwrap();
        write_png(&mut legacy, &legacy_png, 1, 1, &[1, 2, 3]).unwrap();
        write_manifest(&mut exclusive, &exclusive_manifest, b"manifest").unwrap();
        write_manifest(&mut legacy, &legacy_manifest, b"manifest").unwrap();
        assert_eq!(
            std::fs::read(exclusive_png).unwrap(),
            std::fs::read(legacy_png).unwrap()
        );
        assert_eq!(
            std::fs::read(exclusive_manifest).unwrap(),
            std::fs::read(legacy_manifest).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exclusive_outputs_reject_duplicate_paths_even_after_removal() {
        let root = output_test_dir();
        let path = root.join("manifest.json");
        let mut outputs = OutputFiles::new(true);
        write_manifest(&mut outputs, &path, b"first").unwrap();
        std::fs::remove_file(&path).unwrap();
        let error = write_manifest(&mut outputs, &path, b"second").unwrap_err();
        assert!(error.contains("duplicate output path"), "{error}");
        assert!(!path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_outputs_refuse_existing_and_dangling_symlinks() {
        use std::os::unix::fs::symlink;

        let root = output_test_dir();
        let target = root.join("target");
        let missing = root.join("missing");
        let png = root.join("frame000000-SignalField.png");
        let manifest = root.join("manifest.json");
        std::fs::write(&target, b"untouched").unwrap();
        symlink(&missing, &png).unwrap();
        symlink(&target, &manifest).unwrap();
        let mut outputs = OutputFiles::new(true);
        assert!(write_png(&mut outputs, &png, 1, 1, &[1, 2, 3]).is_err());
        assert!(write_manifest(&mut outputs, &manifest, b"new").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
        assert!(!missing.exists());
        assert!(
            std::fs::symlink_metadata(&png)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            std::fs::symlink_metadata(&manifest)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exclusive_cli_parsing_and_legacy_parsers() {
        let flags = ["--out", "shots", "--exclusive-output"];
        let (command, exclusive) = parse_starfield_args(flags.map(str::to_string), true).unwrap();
        assert!(matches!(command, StarfieldConfiguredCommand::Capture(_, _)));
        assert!(exclusive);
        let (_, exclusive) =
            parse_starfield_args(["--out", "shots"].map(str::to_string), true).unwrap();
        assert!(!exclusive);
        assert!(parse_args(flags.map(str::to_string)).is_err());
        assert!(parse_args_with_options(flags.map(str::to_string)).is_err());
        assert!(parse_args_with_starfield_options(flags.map(str::to_string)).is_err());

        let duplicate = ["--out", "shots", "--frames", "0,60,0", "--exclusive-output"];
        let error = parse_starfield_args(duplicate.map(str::to_string), true).unwrap_err();
        assert!(error.contains("duplicate output frame: 0"), "{error}");
        assert!(parse_args(["--out", "shots", "--frames", "0,0"].map(str::to_string)).is_ok());
    }

    #[test]
    fn exclusive_flag_cannot_be_consumed_as_a_cli_option_value() {
        for option in [
            "--out",
            "--input",
            "--catalog",
            "--star-catalog",
            "--profile",
            "--scene",
            "--frames",
            "--size",
            "--mic-layers",
            "--tunnel-variant",
            "--location",
            "--time",
        ] {
            let error = parse_starfield_args(
                ["--out", "shots", option, "--exclusive-output"].map(str::to_string),
                true,
            )
            .unwrap_err();
            assert!(
                error.contains(&format!(
                    "{option} requires a value; --exclusive-output is an option"
                )),
                "{option}: {error}"
            );
        }

        let (command, exclusive) = parse_starfield_args(
            [
                "--out",
                "./--exclusive-output",
                "--input",
                "./--exclusive-output",
                "--catalog",
                "./--exclusive-output",
                "--location",
                "0,0",
                "--exclusive-output",
            ]
            .map(str::to_string),
            true,
        )
        .unwrap();
        let StarfieldConfiguredCommand::Capture(args, options) = command else {
            panic!("unexpected help");
        };
        assert!(exclusive);
        assert_eq!(args.out, PathBuf::from("./--exclusive-output"));
        assert_eq!(args.input, Input::File("./--exclusive-output".into()));
        assert_eq!(
            options.starfield.catalog_path,
            Some(PathBuf::from("./--exclusive-output"))
        );

        // The public legacy parsers still accept the literal as a path value.
        let Command::Capture(args) =
            parse_args(["--out", "--exclusive-output"].map(str::to_string)).unwrap()
        else {
            panic!("unexpected help");
        };
        assert_eq!(args.out, PathBuf::from("--exclusive-output"));
        let StarfieldConfiguredCommand::Capture(args, _) = parse_args_with_starfield_options(
            ["--out", "shots", "--input", "--exclusive-output"].map(str::to_string),
        )
        .unwrap() else {
            panic!("unexpected help");
        };
        assert_eq!(args.input, Input::File("--exclusive-output".into()));
    }

    #[test]
    fn cli_rejects_all_known_options_as_values_in_any_exclusive_flag_order() {
        let value_options = [
            "--out",
            "--input",
            "--catalog",
            "--star-catalog",
            "--profile",
            "--scene",
            "--frames",
            "--size",
            "--mic-layers",
            "--tunnel-variant",
            "--location",
            "--time",
        ];
        let option_tokens = [
            "--help",
            "-h",
            "--profile",
            "--scene",
            "--frames",
            "--input",
            "--size",
            "--out",
            "--mic-layers",
            "--lines-drop-cuts",
            "--exclusive-output",
            "--tunnel-variant",
            "--catalog",
            "--star-catalog",
            "--location",
            "--time",
            "--follow-time",
            "--starfield",
        ];
        for value_option in value_options {
            for token in option_tokens {
                for position in 0..2 {
                    let mut args = vec!["--out", "shots"];
                    if position == 0 {
                        args.push("--exclusive-output");
                    }
                    args.extend([value_option, token]);
                    if position == 1 {
                        args.push("--exclusive-output");
                    }
                    let error = parse_starfield_args(args.into_iter().map(str::to_string), true)
                        .unwrap_err();
                    assert!(
                        error.contains(&format!(
                            "{value_option} requires a value; {token} is an option"
                        )),
                        "{value_option} {token}, position {position}: {error}"
                    );
                }
            }
        }

        let (command, exclusive) = parse_starfield_args(
            [
                "--exclusive-output",
                "--out",
                "./--lines-drop-cuts",
                "--input",
                "--unknown-path",
                "--catalog",
                "./--out",
                "--location",
                "-45,-73",
                "--time",
                "-1",
            ]
            .map(str::to_string),
            true,
        )
        .unwrap();
        let StarfieldConfiguredCommand::Capture(args, options) = command else {
            panic!("unexpected help");
        };
        assert!(exclusive);
        assert_eq!(args.out, PathBuf::from("./--lines-drop-cuts"));
        assert_eq!(args.input, Input::File("--unknown-path".into()));
        assert_eq!(options.starfield.catalog_path, Some("./--out".into()));
        assert_eq!(options.starfield.location, Some((-45.0, -73.0)));
        assert_eq!(options.starfield.unix_seconds, Some(-1.0));

        let (command, exclusive) = parse_starfield_args(
            ["--out", "--lines-drop-cuts", "--input", "--scene"].map(str::to_string),
            true,
        )
        .unwrap();
        let StarfieldConfiguredCommand::Capture(args, _) = command else {
            panic!("unexpected help");
        };
        assert!(!exclusive);
        assert_eq!(args.out, PathBuf::from("--lines-drop-cuts"));
        assert_eq!(args.input, Input::File("--scene".into()));
        assert!(!args.lines_drop_cuts);

        let Command::Capture(args) =
            parse_args(["--out", "--lines-drop-cuts"].map(str::to_string)).unwrap()
        else {
            panic!("unexpected help");
        };
        assert_eq!(args.out, PathBuf::from("--lines-drop-cuts"));
        assert!(!args.lines_drop_cuts);
    }

    fn parse(args: &[&str]) -> Result<CaptureArgs, String> {
        match parse_args(args.iter().map(|arg| arg.to_string()))? {
            Command::Capture(args) => Ok(args),
            Command::Help => Err("help".into()),
        }
    }

    fn parse_configured(args: &[&str]) -> Result<(CaptureArgs, StarfieldCaptureOptions), String> {
        match parse_args_with_starfield_options(args.iter().map(|arg| arg.to_string()))? {
            StarfieldConfiguredCommand::Capture(args, options) => Ok((args, options)),
            StarfieldConfiguredCommand::Help => Err("help".into()),
        }
    }

    fn encode_test_frame(frame: ControlFrame) -> [u8; WIRE_SIZE] {
        let mut bytes = [0; WIRE_SIZE];
        bytes[0] = crate::control_frame::MAGIC;
        bytes[1] = crate::control_frame::VERSION;
        bytes[2..4].copy_from_slice(&frame.frame.to_le_bytes());
        bytes[4] = frame.bpm;
        bytes[5] = frame.beat_phase;
        bytes[6..8].copy_from_slice(&frame.beat_count.to_le_bytes());
        bytes[8] = frame.energy;
        bytes[9] = frame.bass;
        bytes[10] = frame.mid;
        bytes[11] = frame.treble;
        bytes[12..20].copy_from_slice(&frame.spectrum);
        bytes[20] = frame.scene;
        bytes[21] = frame.palette;
        bytes[22] = frame.flags;
        bytes[23] = frame.note_count;
        for (index, note) in frame.notes.iter().enumerate() {
            bytes[24 + index * 2] = note.note;
            bytes[25 + index * 2] = note.velocity;
        }
        let checksum = bytes[..32].iter().fold(0, |checksum, byte| checksum ^ byte);
        bytes[32] = checksum;
        bytes
    }

    #[test]
    fn parses_full_argument_set() {
        let args = parse(&[
            "--profile",
            "pi5",
            "--scene",
            "4",
            "--frames",
            "0, 60,120",
            "--input",
            "fixture.cvz",
            "--size",
            "1920x1080",
            "--out",
            "shots",
            "--mic-layers",
            "scope",
            "--lines-drop-cuts",
        ])
        .unwrap();
        assert_eq!(args.mic_layers, mic_layers::Mode::Scope);
        assert!(args.lines_drop_cuts);
        assert_eq!(args.profile.name, "pi5");
        assert_eq!(args.scene, Some(4));
        assert_eq!(args.frames, [0, 60, 120]);
        assert_eq!(args.input, Input::File("fixture.cvz".into()));
        assert_eq!(args.size, (1920, 1080));
        assert_eq!(args.out, PathBuf::from("shots"));
    }

    #[test]
    fn capture_cli_preserves_legacy_scene_six_and_supports_explicit_opt_in() {
        let legacy_wrapped = parse(&["--out", "shots", "--scene", "13"]).unwrap();
        assert_eq!(legacy_wrapped.scene, Some(13));
        let (legacy_six, legacy_options) =
            parse_configured(&["--out", "shots", "--scene", "6"]).unwrap();
        assert_eq!(legacy_six.scene, Some(6));
        assert!(!legacy_options.starfield_enabled);
        assert!(!cli_starfield_opt_in(
            &legacy_six,
            &legacy_options.starfield,
            false
        ));
        assert_eq!(scenes::dispatch_configured(6, false).name(), "SignalField");

        let (paired_args, paired) = parse_configured(&[
            "--out",
            "shots",
            "--scene",
            "6",
            "--star-catalog",
            "stars.csv",
            "--location",
            "45,-73",
            "--time",
            "946728000",
        ])
        .unwrap();
        assert!(paired.starfield_enabled);
        assert!(cli_starfield_opt_in(&paired_args, &paired.starfield, false));
        let starfield = paired;
        assert_eq!(
            starfield.starfield.catalog_path.as_deref(),
            Some(Path::new("stars.csv"))
        );
        assert_eq!(starfield.starfield.location, Some((45.0, -73.0)));
        assert_eq!(starfield.starfield.unix_seconds, Some(946_728_000.0));
        assert!(!starfield.starfield.follow_time);

        let (recorded_args, recorded) = parse_configured(&[
            "--out",
            "shots",
            "--starfield",
            "--star-catalog",
            "stars.csv",
            "--location",
            "45,-73",
        ])
        .unwrap();
        assert_eq!(recorded_args.scene, None);
        assert!(recorded.starfield_enabled);
        assert!(cli_starfield_opt_in(
            &recorded_args,
            &recorded.starfield,
            true
        ));
        let error = parse_configured(&["--out", "shots", "--starfield"]).unwrap_err();
        assert!(
            error.contains("--starfield requires --star-catalog PATH and --location LAT,LON"),
            "{error}"
        );

        let (_, catalog_only) = parse_configured(&[
            "--out",
            "shots",
            "--star-catalog",
            "stars.csv",
            "--location",
            "45,-73",
        ])
        .unwrap();
        assert!(!catalog_only.starfield_enabled);

        let (_, advancing) = parse_configured(&[
            "--out",
            "shots",
            "--scene",
            "6",
            "--star-catalog",
            "stars.csv",
            "--location",
            "45,-73",
            "--time",
            "946728000",
            "--follow-time",
        ])
        .unwrap();
        assert!(advancing.starfield.follow_time);
        assert!(
            parse_configured(&["--out", "shots", "--follow-time"]).is_err(),
            "follow-time needs a starfield catalog and location"
        );

        for options in [
            &["--star-catalog", "stars.csv"][..],
            &["--location", "0,0"][..],
            &["--location", "91,0"][..],
            &["--time", "inf"][..],
        ] {
            let mut args = vec!["--out", "shots"];
            args.extend_from_slice(options);
            assert!(parse_configured(&args).is_err(), "{args:?}");
        }

        match parse_args_with_starfield_options(
            [
                "--out",
                "shots",
                "--tunnel-variant",
                "hallway",
                "--scene",
                "6",
                "--star-catalog",
                "stars.csv",
                "--location",
                "0,0",
            ]
            .map(str::to_string),
        )
        .unwrap()
        {
            StarfieldConfiguredCommand::Capture(_, options) => {
                assert_eq!(options.capture.tunnel_variant, TunnelVariant::Hallway);
                assert_eq!(
                    options.starfield.catalog_path,
                    Some(PathBuf::from("stars.csv"))
                );
            }
            StarfieldConfiguredCommand::Help => panic!("unexpected help"),
        }
    }

    #[test]
    fn legacy_capture_args_and_command_preserve_scene_six_as_signalfield() {
        let args = CaptureArgs {
            profile: RenderProfile::pi5(),
            scene: Some(starfield::SCENE),
            frames: vec![0],
            input: Input::Procedural,
            size: (160, 90),
            out: PathBuf::from("unused"),
            mic_layers: mic_layers::Mode::Off,
            lines_drop_cuts: false,
        };
        assert_eq!(
            scenes::dispatch_configured(args.scene.unwrap(), false).name(),
            "SignalField"
        );
        assert!(!cli_starfield_opt_in(
            &args,
            &StarfieldOptions::default(),
            false
        ));
        assert!(matches!(
            parse_args([
                "--out".into(),
                "unused".into(),
                "--scene".into(),
                "6".into()
            ]),
            Ok(Command::Capture(_))
        ));
        let configured = CaptureOptions {
            tunnel_variant: TunnelVariant::Hallway,
        };
        assert_eq!(configured.tunnel_variant, TunnelVariant::Hallway);
    }

    #[test]
    fn capture_starfield_opt_in_requires_exact_scene_override() {
        let mut args = CaptureArgs {
            profile: RenderProfile::pi5(),
            scene: None,
            frames: vec![0],
            input: Input::File(PathBuf::from("fixture.cvz")),
            size: (160, 90),
            out: PathBuf::from("unused"),
            mic_layers: mic_layers::Mode::Off,
            lines_drop_cuts: false,
        };
        let mut options = StarfieldOptions {
            catalog_path: Some(PathBuf::from("stars.csv")),
            location: Some((45.0, -73.0)),
            ..StarfieldOptions::default()
        };
        assert!(
            !cli_starfield_opt_in(&args, &options, false),
            "a catalog alone does not enable scene 6 without --starfield"
        );
        args.scene = Some(13);
        assert!(
            !cli_starfield_opt_in(&args, &options, false),
            "wrapped bytes keep legacy dispatch without explicit opt-in"
        );
        args.scene = Some(6);
        assert!(
            cli_starfield_opt_in(&args, &options, false),
            "the paired setup with exact --scene 6 opts in"
        );
        options = StarfieldOptions::default();
        assert!(
            !cli_starfield_opt_in(&args, &options, false),
            "bare --scene 6 remains legacy"
        );
        assert!(
            cli_starfield_opt_in(&args, &options, true),
            "explicit --starfield opts in"
        );
    }

    #[test]
    fn defaults_to_m1_procedural_frame_zero_at_profile_size() {
        let args = parse(&["--out", "shots", "--input", "procedural"]).unwrap();
        assert_eq!(args.profile.name, "m1");
        assert_eq!(args.scene, None);
        assert_eq!(args.frames, [0]);
        assert_eq!(args.input, Input::Procedural);
        assert_eq!(args.size, (3840, 2160));
        assert_eq!(args.mic_layers, mic_layers::Mode::Off);
        let synth = parse(&["--out", "o", "--input", "synth-mic"]).unwrap();
        assert_eq!(synth.input, Input::SynthMic);
        assert!(!args.lines_drop_cuts);
        assert_eq!(
            CaptureOptions::default().tunnel_variant,
            TunnelVariant::Original
        );
    }

    #[test]
    fn parses_tunnel_variants() {
        for (text, expected) in [
            ("original", TunnelVariant::Original),
            ("hallway", TunnelVariant::Hallway),
            ("abstract", TunnelVariant::Abstract),
        ] {
            let args = ["--out", "o", "--tunnel-variant", text].map(str::to_string);
            match parse_args_with_options(args).unwrap() {
                ConfiguredCommand::Capture(_, options) => {
                    assert_eq!(options.tunnel_variant, expected)
                }
                ConfiguredCommand::Help => panic!("unexpected help"),
            }
        }
        assert!(
            parse_args_with_options(
                ["--out", "o", "--tunnel-variant", "lines"].map(str::to_string)
            )
            .is_err()
        );
        assert!(
            parse_args_with_options(["--out", "o", "--tunnel-variant"].map(str::to_string))
                .is_err()
        );
        assert!(parse(&["--out", "o", "--tunnel-variant", "hallway"]).is_err());
    }

    #[test]
    fn rejects_invalid_arguments() {
        for args in [
            &["--frames", "0"][..],
            &["--out", "o", "--profile", "n64"],
            &["--out", "o", "--scene", "256"],
            &["--out", "o", "--frames", "1,,2"],
            &["--out", "o", "--size", "0x10"],
            &["--out", "o", "--size", "1920"],
            &["--out", "o", "--bogus"],
            &["--out", "o", "--mic-layers", "all"],
            &["--out"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
        assert!(
            parse(&[
                "--out",
                "shots",
                "--star-catalog",
                "stars.csv",
                "--location",
                "0,0",
                "--time",
                "7289654400",
            ])
            .is_err()
        );
        assert!(matches!(parse_args(["-h".to_string()]), Ok(Command::Help)));
        assert!(USAGE.contains("--star-catalog PATH"));
        assert!(USAGE.contains("--tunnel-variant"));
    }

    #[test]
    fn scene_override_dispatches_seven_to_the_colour_field() {
        let source = FrameSource {
            fixture: None,
            scene: Some(7),
        };
        let frame = source.frame(120).unwrap();
        assert_eq!(frame.scene, 7);
        assert_eq!(frame.energy, ControlFrame::procedural(120).energy);
        assert_eq!(scenes::dispatch(frame.scene).name(), colour_field::NAME);
        assert_eq!(scenes::dispatch(13).name(), "SpectrumVolume");
        assert_eq!(scenes::dispatch_configured(6, true).name(), "Starfield");
        let unforced = FrameSource {
            fixture: None,
            scene: None,
        };
        assert_eq!(
            unforced.frame(200).unwrap().scene,
            ControlFrame::procedural(200).scene
        );
    }

    #[test]
    fn capture_preflight_respects_colour_field_debounce_and_legacy_scene_six() {
        let source = FrameSource {
            fixture: Some(vec![
                ControlFrame {
                    scene: 6,
                    ..ControlFrame::procedural(0)
                },
                ControlFrame {
                    scene: 7,
                    ..ControlFrame::procedural(1)
                },
            ]),
            scene: None,
        };
        // The raw requested frame stays scene 7. It is debounced: legacy
        // display remains scene 0, while opt-in display remains scene 6.
        assert_eq!(source.frame(1).unwrap().scene, 7);
        assert_eq!(displayed_scene_at(&source, 1, false).unwrap(), 0);
        assert_eq!(displayed_scene_at(&source, 1, true).unwrap(), 6);
        assert_eq!(scenes::dispatch(6).name(), scenes::REGISTRY[0].name);
        let scene_six = FrameSource {
            fixture: Some(vec![ControlFrame {
                scene: 6,
                ..ControlFrame::procedural(0)
            }]),
            scene: None,
        };
        assert_eq!(displayed_scene_at(&scene_six, 0, false).unwrap(), 0);
        assert_eq!(displayed_scene_at(&scene_six, 0, true).unwrap(), 6);
        assert_eq!(scenes::dispatch_configured(6, false).name(), "SignalField");
        assert_eq!(scenes::dispatch_configured(6, true).name(), "Starfield");
    }

    #[test]
    fn capture_preflight_checks_skipped_intermediate_replay_frames() {
        let source = FrameSource {
            fixture: Some(vec![
                ControlFrame {
                    scene: 0,
                    ..ControlFrame::procedural(0)
                },
                ControlFrame {
                    scene: 6,
                    ..ControlFrame::procedural(1)
                },
                ControlFrame {
                    scene: 0,
                    ..ControlFrame::procedural(2)
                },
            ]),
            scene: None,
        };
        assert_eq!(displayed_scene_at(&source, 2, true).unwrap(), 0);
        assert!(validate_replay_catalog(&source, 2, false, false).is_ok());
        let error = validate_replay_catalog(&source, 2, false, true).unwrap_err();
        assert!(error.contains("replay frame 1"), "{error}");
        assert!(error.contains("--star-catalog PATH"), "{error}");
        assert!(validate_replay_catalog(&source, 2, true, true).is_ok());

        let wrapped = FrameSource {
            fixture: Some(vec![ControlFrame {
                scene: 13,
                ..ControlFrame::procedural(0)
            }]),
            scene: None,
        };
        assert!(validate_replay_catalog(&wrapped, 0, false, true).is_ok());
    }

    #[test]
    fn replay_preflight_scans_only_for_missing_enabled_starfield_and_checks_bounds() {
        let source = FrameSource {
            fixture: Some(vec![ControlFrame {
                scene: 6,
                ..ControlFrame::procedural(0)
            }]),
            scene: None,
        };
        assert!(preflight_replay(&source, Some(0), false, false).is_ok());
        assert!(preflight_replay(&source, Some(0), true, true).is_ok());
        let missing = preflight_replay(&source, Some(0), true, false).unwrap_err();
        assert!(missing.contains("replay frame 0"), "{missing}");
        for (enabled, catalog_available) in [(false, false), (true, true), (true, false)] {
            let error = preflight_replay(&source, Some(1), enabled, catalog_available)
                .expect_err("requested frame must be bounds-checked");
            assert!(error.contains("past the input's 1 frames"), "{error}");
        }
    }

    /// State-only replay with layers enabled must produce the same next image
    /// as rendering every skipped frame, without building unused scene images.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn lazy_undrawn_replay_matches_full_scene_rendering() {
        let (width, height) = (160, 90);
        let mut full =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                .unwrap();
        let mut lazy =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                .unwrap();
        full.pipeline_mut().set_mic_layers(mic_layers::Mode::Both);
        lazy.pipeline_mut().set_mic_layers(mic_layers::Mode::Both);

        for index in 0..60 {
            let frame = ControlFrame {
                scene: 0,
                ..ControlFrame::procedural(index)
            };
            full.render(frame, elapsed(index)).unwrap();
            lazy.pipeline_mut()
                .advance_state(frame, elapsed(index), (width, height));
        }

        let final_frame = ControlFrame {
            scene: 0,
            ..ControlFrame::procedural(60)
        };
        assert_eq!(
            full.render(final_frame, elapsed(60)).unwrap(),
            lazy.render(final_frame, elapsed(60)).unwrap(),
            "lazy state advancement changed the next rendered frame"
        );
    }

    #[test]
    fn capture_manifest_records_resolved_starfield_epoch_and_clock_mode() {
        let catalog =
            starfield::StarCatalog::from_reader("ra,dec,mag\n0,0,1\n".as_bytes()).unwrap();
        let config = starfield::StarfieldConfig {
            catalog: std::sync::Arc::new(catalog),
            latitude_degrees: 45.0,
            longitude_degrees: 80.0,
            unix_seconds: 946_728_000.0,
            follow_elapsed: true,
        };
        assert_eq!(
            starfield_metadata(Some(&config)),
            ("946728000".to_string(), "true".to_string())
        );
        assert_eq!(starfield_metadata(None), ("null".into(), "null".into()));
    }

    #[test]
    fn manifest_separates_requested_wrapped_byte_from_displayed_scene() {
        let wrapped = manifest_capture_entry(ManifestCaptureEntry {
            file: "frame000000-SpectrumVolume.png",
            frame_index: 0,
            elapsed: 0.0,
            starfield_epoch: "null".into(),
            requested_scene: 13,
            displayed_scene: 1,
            scene_name: "SpectrumVolume",
            camera: CameraMode::Orbit3D,
        });
        assert!(wrapped.contains("\"scene\": 13, \"displayed_scene\": 1"));

        let source = FrameSource {
            fixture: Some(vec![
                ControlFrame {
                    scene: 6,
                    ..ControlFrame::procedural(0)
                },
                ControlFrame {
                    scene: 7,
                    ..ControlFrame::procedural(1)
                },
            ]),
            scene: None,
        };
        let displayed = displayed_scene_at(&source, 1, false).unwrap();
        let debounced = manifest_capture_entry(ManifestCaptureEntry {
            file: "frame000001-SignalField.png",
            frame_index: 1,
            elapsed: elapsed(1),
            starfield_epoch: "null".into(),
            requested_scene: 7,
            displayed_scene: displayed,
            scene_name: "SignalField",
            camera: CameraMode::Orbit3D,
        });
        assert!(debounced.contains("\"scene\": 7, \"displayed_scene\": 0"));
    }

    /// Full replay path: a fixed base epoch plus `--follow-time` must produce
    /// the same manifest and PNGs on repeated runs, including a ten-second
    /// Starfield snapshot change. Uses only this synthetic catalog.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn advancing_capture_manifest_and_pngs_are_replay_deterministic() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "chipviz-starfield-replay-{}-{nonce}",
            std::process::id()
        ));
        let catalog = root.join("synthetic-stars.csv");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&catalog, "ra,dec,mag,proper\n0,45,1,\"Synthetic star\"\n").unwrap();

        let args = CaptureArgs {
            profile: RenderProfile::pi5(),
            scene: Some(6),
            frames: vec![0, 600],
            input: Input::Procedural,
            size: (160, 90),
            out: root.join("first"),
            mic_layers: mic_layers::Mode::Off,
            lines_drop_cuts: false,
        };
        let options = StarfieldCaptureOptions {
            capture: CaptureOptions::default(),
            starfield: StarfieldOptions {
                catalog_path: Some(catalog),
                location: Some((45.0, 80.0)),
                unix_seconds: Some(946_728_000.0),
                follow_time: true,
            },
            starfield_enabled: true,
        };
        run_with_starfield_options(&args, &options).unwrap();
        let args = CaptureArgs {
            out: root.join("second"),
            ..args
        };
        run_with_starfield_options(&args, &options).unwrap();

        let first_manifest = std::fs::read(root.join("first/manifest.json")).unwrap();
        let second_manifest = std::fs::read(root.join("second/manifest.json")).unwrap();
        assert_eq!(first_manifest, second_manifest);
        let manifest = String::from_utf8(first_manifest).unwrap();
        assert!(manifest.contains("\"starfield_epoch_unix_seconds\": 946728000"));
        assert!(manifest.contains("\"starfield_epoch_advances_with_elapsed\": true"));
        assert!(manifest.contains("\"frame_index\": 600"));
        assert!(manifest.contains(
            "\"frame_index\": 0, \"elapsed\": 0, \"starfield_epoch_unix_seconds\": 946728000"
        ));
        assert!(manifest.contains(
            "\"frame_index\": 600, \"elapsed\": 10, \"starfield_epoch_unix_seconds\": 946728010"
        ));
        for frame in [0, 600] {
            let file = format!("frame{frame:06}-Starfield.png");
            assert_eq!(
                std::fs::read(root.join("first").join(&file)).unwrap(),
                std::fs::read(root.join("second").join(&file)).unwrap(),
                "replay PNG differs at frame {frame}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The programmatic opt-in applies to recorded scene transitions even when
    /// CaptureArgs.scene is unset; legacy APIs keep raw scene 6 on SignalField.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn recorded_scene_transitions_are_starfield_only_for_explicit_opt_in() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "chipviz-starfield-transition-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("transition.cvz");
        let frames = [0, 6, 0].into_iter().enumerate().map(|(index, scene)| {
            encode_test_frame(ControlFrame {
                scene,
                ..ControlFrame::procedural(index as u64)
            })
        });
        let bytes: Vec<u8> = frames.flatten().collect();
        std::fs::write(&input, bytes).unwrap();
        let catalog_path = root.join("synthetic-stars.csv");
        std::fs::write(&catalog_path, "ra,dec,mag\n18.7,45,-2\n").unwrap();

        let (args, options) = parse_configured(&[
            "--profile",
            "pi5",
            "--frames",
            "0,1,2",
            "--input",
            input.to_str().unwrap(),
            "--size",
            "160x90",
            "--out",
            root.join("opt-in").to_str().unwrap(),
            "--starfield",
            "--star-catalog",
            catalog_path.to_str().unwrap(),
            "--location",
            "45,0",
            "--time",
            "946728000",
        ])
        .unwrap();
        assert!(options.starfield_enabled);
        run_with_starfield_options(&args, &options).unwrap();
        assert!(
            root.join("opt-in/frame000001-Starfield.png").exists(),
            "explicit programmatic opt-in should render the recorded scene-6 frame"
        );
        for (frame, scene_name) in [(0, "SignalField"), (2, "SignalField")] {
            assert!(
                root.join(format!("opt-in/frame{frame:06}-{scene_name}.png"))
                    .exists(),
                "opt-in must preserve recorded scene {scene_name} at frame {frame}"
            );
            assert!(
                !root
                    .join(format!("opt-in/frame{frame:06}-Starfield.png"))
                    .exists(),
                "opt-in must not force Starfield at frame {frame}"
            );
        }
        let opted_manifest = std::fs::read_to_string(root.join("opt-in/manifest.json")).unwrap();
        assert!(opted_manifest.contains("\"scene\": 0, \"displayed_scene\": 0"));
        assert!(opted_manifest.contains("\"scene\": 6, \"displayed_scene\": 6"));

        let legacy_args = CaptureArgs {
            out: root.join("legacy"),
            ..args
        };
        run_with_options(&legacy_args, CaptureOptions::default()).unwrap();
        for frame in 0..=2 {
            assert!(
                root.join(format!("legacy/frame{frame:06}-SignalField.png"))
                    .exists(),
                "legacy capture should keep frame {frame} on SignalField"
            );
            assert!(
                !root
                    .join(format!("legacy/frame{frame:06}-Starfield.png"))
                    .exists(),
                "legacy capture must not display Starfield at frame {frame}"
            );
        }
        let legacy_manifest = std::fs::read_to_string(root.join("legacy/manifest.json")).unwrap();
        assert!(legacy_manifest.contains("\"scene\": 0, \"displayed_scene\": 0"));
        assert!(legacy_manifest.contains("\"scene\": 6, \"displayed_scene\": 0"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_cvz_fixture_frames() {
        let bytes = include_bytes!("../../../host/fixtures/musical/groove.cvz");
        let frames = parse_cvz(bytes).unwrap();
        assert_eq!(frames.len(), bytes.len() / WIRE_SIZE);
        let source = FrameSource {
            fixture: Some(frames),
            scene: None,
        };
        assert!(source.frame(0).is_ok());
        assert!(source.frame(bytes.len() as u64).is_err());
        assert!(parse_cvz(&bytes[..WIRE_SIZE - 1]).is_err());
        assert!(parse_cvz(&[]).is_err());
    }

    #[test]
    fn synthetic_mic_is_deterministic_detects_clicks_and_falls_silent() {
        let run = || {
            let mut synth = SynthMic::new().unwrap();
            (0..=420)
                .map(|index| synth.advance(ControlFrame::procedural(index), index))
                .collect::<Vec<_>>()
        };
        let frames = run();
        let again = run();
        for ((frame, scope), (other, other_scope)) in frames.iter().zip(&again) {
            assert_eq!(scope, other_scope);
            assert_eq!(
                (frame.energy, frame.spectrum),
                (other.energy, other.spectrum)
            );
        }
        // About one onset per click in the 6 s of sound (12 clicks).
        let onsets = frames[400].1.onsets;
        assert!((8..=14).contains(&onsets), "{onsets} onsets");
        let (loud, loud_scope) = &frames[200];
        assert!(loud.energy > 40, "{loud:?}");
        assert!(loud_scope.upper.iter().any(|&point| point > 0.3));
        // From 6 s (frame 360) the input is silent: flat waveform, no onsets.
        let (quiet, quiet_scope) = &frames[420];
        assert_eq!(quiet_scope.upper, [0.0; crate::audio::scope::POINTS]);
        assert_eq!(quiet_scope.lower, quiet_scope.upper);
        assert_eq!(quiet_scope.onsets, frames[380].1.onsets);
        assert!(quiet.energy < loud.energy);
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
    }

    /// Needs a GPU adapter; run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn renders_every_registry_scene_offscreen() {
        let mut renderer =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), 160, 90)).unwrap();
        renderer
            .pipeline_mut()
            .configure_starfield(Some(synthetic_starfield_config()));
        renderer.pipeline_mut().enable_starfield_dispatch(true);
        for scene in 0..scenes::REGISTRY.len() as u8 {
            // A distinct counter per scene: a repeated counter is not a new
            // frame, so the colour field would keep showing the first scene.
            let frame = ControlFrame {
                scene,
                ..ControlFrame::procedural(120 + u64::from(scene))
            };
            let rgb = renderer.render(frame, 2.0).unwrap();
            assert_eq!(
                renderer.pipeline_mut().colour_field_display(),
                colour_field::Display::Native(scene)
            );
            assert_eq!(rgb.len(), 160 * 90 * 3);
            assert!(
                rgb.iter().any(|&value| value > 16),
                "scene {scene} is blank"
            );
        }
    }

    fn synthetic_starfield_config() -> starfield::StarfieldConfig {
        let catalog =
            starfield::StarCatalog::from_reader("ra,dec,mag\n0,0,1\n".as_bytes()).unwrap();
        starfield::StarfieldConfig {
            catalog: std::sync::Arc::new(catalog),
            latitude_degrees: 45.0,
            longitude_degrees: 80.0,
            unix_seconds: 946_728_000.0,
            follow_elapsed: false,
        }
    }

    /// Scene 6 must retain a visible footprint for a centered bright star on
    /// the smallest display used by the registry smoke test.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn centered_magnitude_one_star_is_visible_at_160_by_90() {
        let (width, height) = (160, 90);
        let mut renderer =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                .unwrap();
        renderer
            .pipeline_mut()
            .configure_starfield(Some(synthetic_starfield_config()));
        renderer.pipeline_mut().enable_starfield_dispatch(true);
        let frame = ControlFrame {
            scene: 6,
            ..ControlFrame::procedural(6)
        };
        let rgb = renderer.render(frame, 0.0).unwrap();
        let pixel = ((height / 2 * width + width / 2) * 3) as usize;
        assert!(
            rgb[pixel..pixel + 3].iter().any(|&channel| channel > 20),
            "center pixel was not lit: {:?}",
            &rgb[pixel..pixel + 3]
        );
    }

    /// Reads the actual scene selected by the monotonic crossing gate and
    /// checks the Starfield's bounded linear-light output at 60–480 Hz.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn starfield_scene_crossings_are_limited_at_multiple_rates() {
        for fps in [60_u64, 120, 240, 480] {
            let (width, height) = (160, 90);
            let mut renderer =
                pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                    .unwrap();
            renderer
                .pipeline_mut()
                .configure_starfield(Some(synthetic_starfield_config()));
            renderer.pipeline_mut().enable_starfield_dispatch(true);
            renderer
                .pipeline_mut()
                .set_mic_layers(mic_layers::allowed(mic_layers::Mode::Both));
            let mut crossings = Vec::new();
            let mut previous_starfield = false;
            let mut previous_pixels: Option<Vec<u8>> = None;
            for step in 0..=fps * 2 {
                let phase = step / 6;
                let scene = if phase.is_multiple_of(2) {
                    4
                } else if (phase / 2).is_multiple_of(2) {
                    13
                } else {
                    6
                };
                let mut frame = ControlFrame::procedural(2_048 + step);
                frame.scene = scene;
                frame.flags = 0;
                frame.beat_count = 0;
                frame.beat_phase = 0;
                let pixels = renderer.render(frame, step as f64 / fps as f64).unwrap();
                let shown = renderer.pipeline_mut().committed_displayed_scene();
                let is_starfield = scenes::dispatch_configured(shown, true).name() == "Starfield";
                if is_starfield != previous_starfield {
                    let previous = previous_pixels
                        .as_ref()
                        .expect("first frame establishes the prior output");
                    let changed_pixels = previous
                        .chunks_exact(3)
                        .zip(pixels.chunks_exact(3))
                        .filter(|(left, right)| left != right)
                        .count();
                    assert!(
                        changed_pixels > 0,
                        "fps={fps}, step={step}: committed Starfield crossing changed no rendered pixels"
                    );
                    let now = step as f64 / fps as f64;
                    crossings.push(now);
                    let in_rolling_second = crossings
                        .iter()
                        .filter(|&&crossing| now - crossing <= 1.0)
                        .count();
                    assert!(
                        in_rolling_second <= 2,
                        "fps={fps}, step={step}: {in_rolling_second} rendered Starfield crossings in one second"
                    );
                    previous_starfield = is_starfield;
                }
                if is_starfield {
                    let max_luminance = pixels
                        .chunks_exact(3)
                        .map(luminance_srgb)
                        .fold(0.0_f32, f32::max);
                    assert!(
                        max_luminance <= 0.051,
                        "fps={fps}, step={step}: Starfield luminance {max_luminance} exceeds its 0.05 scene bound"
                    );
                    assert!(
                        pixels.iter().any(|&value| value > 0),
                        "fps={fps}, step={step}: synthetic Starfield output was blank"
                    );
                }
                previous_pixels = Some(pixels);
            }
            assert!(crossings.len() >= 2, "fps={fps}: gate was not exercised");
        }
    }

    /// Simulates acquisition failure by previewing dispatch but deliberately
    /// skipping encode, then retrying the same frame through the real
    /// offscreen encoder. It also checks the colour-field debounce afterward.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn starfield_fallback_retry_keeps_output_and_scene_seven_debounce() {
        let (width, height) = (160, 90);
        let mut renderer =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                .unwrap();
        let mut frame = ControlFrame::procedural(8);
        frame.scene = 6;
        frame.flags = 0x05;
        frame.energy = 120;
        frame.beat_count = 7;
        renderer.pipeline_mut().enable_starfield_dispatch(true);
        renderer.pipeline_mut().observe(&frame, 0.0);
        assert_eq!(renderer.pipeline_mut().displayed_scene(frame, 0.0), 6);
        renderer.pipeline_mut().fallback_next_missing_starfield();
        assert_eq!(renderer.pipeline_mut().displayed_scene(frame, 0.0), 0);

        // No encode here models get_current_texture() failing. The retry
        // must still consume the fallback and produce the selected safe scene.
        let retry = renderer.render(frame, 0.0).unwrap();
        assert_eq!(renderer.pipeline_mut().committed_displayed_scene(), 0);
        assert!(
            retry.iter().any(|&value| value > 16),
            "retry output must not be black"
        );
        let mut baseline =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                .unwrap();
        let mut native = frame;
        native.scene = 0;
        assert_eq!(retry, baseline.render(native, 0.0).unwrap());

        for index in 1..=40 {
            let mut color_field = ControlFrame::procedural(8 + index);
            color_field.scene = colour_field::SCENE;
            renderer.render(color_field, index as f64 / 60.0).unwrap();
        }
        assert_eq!(renderer.pipeline_mut().committed_displayed_scene(), 7);
    }

    /// Repeated pre-acquisition previews must leave ColourField debounce
    /// unchanged; a later successful encode consumes the frame once.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn failed_surface_acquisition_preview_does_not_advance_colour_field_debounce() {
        let (width, height) = (160, 90);
        let mut renderer =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                .unwrap();
        let mut baseline =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                .unwrap();
        let mut last = ControlFrame::procedural(0);
        for index in 0..40 {
            last = ControlFrame {
                scene: colour_field::SCENE,
                ..ControlFrame::procedural(index)
            };
            renderer.pipeline_mut().observe(&last, elapsed(index));
            baseline.pipeline_mut().observe(&last, elapsed(index));
            for _ in 0..3 {
                let _ = renderer.pipeline_mut().requested_scene(last);
            }
        }
        assert_eq!(
            renderer.pipeline_mut().colour_field_display(),
            colour_field::Display::Native(0),
            "preview must not consume any scene-7 debounce steps"
        );
        let retried = renderer.render(last, 39.0 / 60.0).unwrap();
        let expected = baseline.render(last, 39.0 / 60.0).unwrap();
        assert_eq!(retried, expected);
        assert_eq!(
            renderer.pipeline_mut().committed_displayed_scene(),
            baseline.pipeline_mut().committed_displayed_scene()
        );
    }

    fn luminance_srgb(pixel: &[u8]) -> f32 {
        let decode = |byte: u8| {
            let value = f32::from(byte) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * decode(pixel[0]) + 0.7152 * decode(pixel[1]) + 0.0722 * decode(pixel[2])
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn tunnel_variants_render_synthetic_input_at_representative_aspects() {
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            for (width, height) in [(320, 180), (320, 240), (180, 320)] {
                let mut renderer =
                    pollster::block_on(OffscreenRenderer::new(profile.clone(), width, height))
                        .unwrap();
                let mut images = Vec::new();
                for variant in [
                    TunnelVariant::Original,
                    TunnelVariant::Hallway,
                    TunnelVariant::Abstract,
                ] {
                    renderer.pipeline_mut().configure_scenes(SceneConfig {
                        tunnel_variant: variant,
                        ..SceneConfig::default()
                    });
                    renderer.pipeline_mut().reset_state();
                    let mut synth = SynthMic::new().unwrap();
                    for index in 0..120 {
                        let base = ControlFrame {
                            scene: 2,
                            palette: 1,
                            ..ControlFrame::procedural(index)
                        };
                        let frame = feed(renderer.pipeline_mut(), Some(&mut synth), base, index);
                        renderer.pipeline_mut().advance_state(
                            frame,
                            elapsed(index),
                            (width, height),
                        );
                    }
                    let base = ControlFrame {
                        scene: 2,
                        palette: 1,
                        ..ControlFrame::procedural(120)
                    };
                    let frame = feed(renderer.pipeline_mut(), Some(&mut synth), base, 120);
                    let rgb = renderer.render(frame, elapsed(120)).unwrap();
                    assert!(
                        rgb.iter().any(|&value| value > 16),
                        "{} {width}x{height} {variant:?} blank",
                        profile.name
                    );
                    assert!(
                        images.iter().all(|other| other != &rgb),
                        "{} {width}x{height} {variant:?} identical",
                        profile.name
                    );
                    images.push(rgb);
                }
            }
        }
    }

    /// Supporting evidence only: this tests sampled whole-frame changes, not
    /// the WCAG viewing-area calculation or physical display behaviour.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn tunnel_variants_bound_gpu_luminance_changes() {
        fn linear(byte: u8) -> f32 {
            let value = f32::from(byte) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        fn luminance(rgb: &[u8]) -> Vec<f32> {
            rgb.chunks_exact(3)
                .map(|p| 0.2126 * linear(p[0]) + 0.7152 * linear(p[1]) + 0.0722 * linear(p[2]))
                .collect()
        }
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            for (width, height) in [(320, 180), (180, 320)] {
                let mut renderer =
                    pollster::block_on(OffscreenRenderer::new(profile.clone(), width, height))
                        .unwrap();
                for variant in [TunnelVariant::Hallway, TunnelVariant::Abstract] {
                    renderer.pipeline_mut().configure_scenes(SceneConfig {
                        tunnel_variant: variant,
                        ..SceneConfig::default()
                    });
                    renderer.pipeline_mut().reset_state();
                    let mut previous: Option<Vec<f32>> = None;
                    let mut max_changed = 0.0_f32;
                    let mut max_mean_delta = 0.0_f32;
                    for index in 0..120 {
                        // Deliberately hostile input: jump audio and palette every
                        // frame, including a warm palette.
                        let high = index % 2 == 0;
                        let frame = ControlFrame {
                            scene: 2,
                            palette: if high { 0 } else { 2 },
                            energy: if high { 255 } else { 0 },
                            bass: if high { 255 } else { 0 },
                            treble: if high { 255 } else { 0 },
                            ..ControlFrame::procedural(index)
                        };
                        let current = luminance(&renderer.render(frame, elapsed(index)).unwrap());
                        if let Some(before) = &previous {
                            let count = current.len() as f32;
                            let changed = before
                                .iter()
                                .zip(&current)
                                .filter(|(a, b)| (*a - *b).abs() >= 0.1)
                                .count() as f32
                                / count;
                            let mean_delta =
                                (before.iter().sum::<f32>() - current.iter().sum::<f32>()).abs()
                                    / count;
                            max_changed = max_changed.max(changed);
                            max_mean_delta = max_mean_delta.max(mean_delta);
                        }
                        previous = Some(current);
                    }
                    println!(
                        "{} {width}x{height} {variant:?}: max changed={max_changed:.4}, max mean delta={max_mean_delta:.4}",
                        profile.name
                    );
                    assert!(max_changed < 0.12);
                    assert!(max_mean_delta < 0.08);
                }
            }
        }
    }

    /// The mic layers never touch the colour field, and `off` renders exactly
    /// as a pipeline that never steps or encodes them; over a native scene
    /// `both` does draw (the no-mic fallback ribbon and beat bursts).
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn mic_layers_skip_the_colour_field_and_off_matches_the_baseline() {
        // `None` renders through the encode path with no mic layers at all.
        let render = |scene: u8, mode: Option<mic_layers::Mode>| {
            let (width, height) = (160, 90);
            let mut renderer =
                pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                    .unwrap();
            if let Some(mode) = mode {
                renderer.pipeline_mut().set_mic_layers(mode);
            }
            let mut last = Vec::new();
            // Two seconds at 60 fps, so beats change and bursts would fire.
            for index in 0..120 {
                let frame = ControlFrame {
                    scene,
                    ..ControlFrame::procedural(index)
                };
                let elapsed = elapsed(index);
                last = match mode {
                    Some(_) => renderer.render(frame, elapsed).unwrap(),
                    None => {
                        renderer.pipeline_mut().observe(&frame, elapsed);
                        renderer
                            .render_with(|pipeline, encoder, view| {
                                pipeline.encode_without_mic_layers(
                                    encoder,
                                    view,
                                    frame,
                                    elapsed,
                                    (width, height),
                                );
                            })
                            .unwrap()
                    }
                };
            }
            last
        };
        let both = Some(mic_layers::Mode::Both);
        let off = Some(mic_layers::Mode::Off);
        let field = render(colour_field::SCENE, None);
        assert_eq!(render(colour_field::SCENE, both), field, "scene 7 overlaid");
        assert_eq!(render(colour_field::SCENE, off), field);
        let native = render(0, None);
        assert_eq!(render(0, off), native, "off differs from the baseline");
        assert_ne!(render(0, both), native, "both drew nothing over scene 0");
    }

    /// The GPU phosphor behaves as its CPU model: over scene 0, with both
    /// layers drawing and then turned off, the light the layers add to any
    /// unsaturated pixel (the capture minus the layer-free baseline, in
    /// linear light), whether the animated scene there moves or not, falls
    /// by at most `DECAY_PER_S` per second (plus 8-bit rounding), and once
    /// it has faded the capture matches the baseline byte for byte.
    /// This holds on an sRGB target and on one that is not sRGB (shown as
    /// sRGB-coded), where the composite encodes for itself.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn phosphor_falls_no_faster_than_its_decay_and_fades_to_the_baseline() {
        for format in [
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm,
        ] {
            phosphor_falls_on(format);
        }
    }

    fn phosphor_falls_on(format: wgpu::TextureFormat) {
        let (width, height) = (160, 90);
        let new = || {
            pollster::block_on(OffscreenRenderer::with_format(
                RenderProfile::pi5(),
                width,
                height,
                format,
            ))
            .unwrap()
        };
        let (mut layered, mut baseline) = (new(), new());
        layered
            .pipeline_mut()
            .set_mic_layers(mic_layers::Mode::Both);
        let linear = |byte: u8| {
            let value = f32::from(byte) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let fps = CAPTURE_FPS as u64;
        // 3 s drawing, then 7 s off: longer than the 1 / DECAY_PER_S fade.
        let (lit, total) = (3 * fps, 10 * fps);
        let window = fps / 2;
        let mut excess: Vec<Vec<f32>> = Vec::new();
        let mut last = (Vec::new(), Vec::new());
        for index in 0..total {
            if index == lit {
                layered.pipeline_mut().set_mic_layers(mic_layers::Mode::Off);
            }
            let frame = ControlFrame {
                scene: 0,
                ..ControlFrame::procedural(index)
            };
            let elapsed = elapsed(index);
            let drawn = layered.render(frame, elapsed).unwrap();
            baseline.pipeline_mut().observe(&frame, elapsed);
            let base = baseline
                .render_with(|pipeline, encoder, view| {
                    pipeline.encode_without_mic_layers(
                        encoder,
                        view,
                        frame,
                        elapsed,
                        (width, height),
                    );
                })
                .unwrap();
            // Saturated channels hide the added light; mark them unknown.
            excess.push(
                drawn
                    .iter()
                    .zip(&base)
                    .map(|(&drawn, &base)| {
                        if drawn == 255 {
                            f32::NAN
                        } else {
                            linear(drawn) - linear(base)
                        }
                    })
                    .collect(),
            );
            if index == lit {
                assert_ne!(
                    drawn, base,
                    "{format:?}: no afterglow just after turning off"
                );
            }
            last = (drawn, base);
        }
        assert!(
            last.0 == last.1,
            "{format:?}: the afterglow never faded to the baseline"
        );
        // Half-second windows: at most DECAY_PER_S / 2, plus a rounding
        // margin for two 8-bit encodes per frame.
        let bound = phosphor::DECAY_PER_S * 0.5 + 0.02;
        let (mut steepest, mut measured) = (0.0_f32, 0_usize);
        for (earlier, later) in excess.iter().zip(&excess[window as usize..]) {
            for (&before, &after) in earlier.iter().zip(later) {
                if before.is_finite() && after.is_finite() {
                    steepest = steepest.max(before - after);
                    measured += 1;
                }
            }
        }
        assert!(measured > 100_000, "{format:?}: only {measured} samples");
        assert!(
            steepest <= bound,
            "{format:?}: fell {steepest} in 0.5 s (bound {bound})"
        );
        eprintln!(
            "{format:?}: steepest fall {steepest} in 0.5 s over {measured} samples (bound {bound})"
        );
        assert!(
            steepest > 0.02,
            "{format:?}: the afterglow should visibly fade: {steepest}"
        );
    }

    /// Switching from a native scene with glowing layers to the colour
    /// field drops the afterglow on the frame the field appears: every
    /// colour-field frame matches a layer-free baseline byte for byte.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn entering_the_colour_field_drops_the_afterglow() {
        let (width, height) = (160, 90);
        let new = || {
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height)).unwrap()
        };
        let (mut layered, mut baseline) = (new(), new());
        layered
            .pipeline_mut()
            .set_mic_layers(mic_layers::Mode::Both);
        let fps = CAPTURE_FPS as u64;
        let (native, total) = (2 * fps, 4 * fps);
        let (mut glowed, mut field_frames) = (false, 0);
        for index in 0..total {
            let scene = if index < native {
                0
            } else {
                colour_field::SCENE
            };
            let frame = ControlFrame {
                scene,
                ..ControlFrame::procedural(index)
            };
            let elapsed = elapsed(index);
            let drawn = layered.render(frame, elapsed).unwrap();
            baseline.pipeline_mut().observe(&frame, elapsed);
            let base = baseline
                .render_with(|pipeline, encoder, view| {
                    pipeline.encode_without_mic_layers(
                        encoder,
                        view,
                        frame,
                        elapsed,
                        (width, height),
                    );
                })
                .unwrap();
            let display = layered.pipeline_mut().colour_field_display();
            assert_eq!(display, baseline.pipeline_mut().colour_field_display());
            match display {
                colour_field::Display::ColourField(_) => {
                    field_frames += 1;
                    assert!(drawn == base, "frame {index}: glow over the colour field");
                }
                colour_field::Display::Native(_) => glowed |= drawn != base,
            }
        }
        assert!(glowed, "the layers never drew over the native scene");
        assert!(field_frames > fps / 2, "{field_frames} colour-field frames");
    }

    /// A capture still replays earlier frames with `advance_state`, which
    /// runs the phosphor passes too: it matches a live run byte for byte.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn replayed_stills_match_a_live_run_with_the_phosphor() {
        let (width, height) = (160, 90);
        let mut renderer =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                .unwrap();
        renderer
            .pipeline_mut()
            .set_mic_layers(mic_layers::Mode::Both);
        let frame = |index| ControlFrame {
            scene: 0,
            ..ControlFrame::procedural(index)
        };
        let still = 150;
        let mut live = Vec::new();
        for index in 0..=still {
            live = renderer.render(frame(index), elapsed(index)).unwrap();
        }
        renderer.pipeline_mut().reset_state();
        for index in 0..still {
            renderer
                .pipeline_mut()
                .advance_state(frame(index), elapsed(index), (width, height));
        }
        let replayed = renderer.render(frame(still), elapsed(still)).unwrap();
        assert!(replayed == live, "replayed still differs from the live run");
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn tunnel_held_redraw_and_batched_packets_match_replayed_still() {
        let size = (320, 180);
        let mut renderer =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), size.0, size.1))
                .unwrap();
        let quiet = ControlFrame {
            scene: 2,
            palette: 1,
            energy: 0,
            bass: 0,
            treble: 0,
            ..ControlFrame::procedural(0)
        };
        let old = ControlFrame { frame: 1, ..quiet };
        let loud = ControlFrame {
            frame: 2,
            energy: 255,
            bass: 255,
            treble: 255,
            ..quiet
        };
        for variant in [TunnelVariant::Hallway, TunnelVariant::Abstract] {
            renderer.pipeline_mut().configure_scenes(SceneConfig {
                tunnel_variant: variant,
                ..SceneConfig::default()
            });
            renderer.pipeline_mut().reset_state();
            let mut live = Vec::new();
            for index in 0..=120 {
                let time = elapsed(index);
                let frame = if index == 0 { quiet } else { loud };
                if index == 0 {
                    renderer.pipeline_mut().observe(&quiet, time);
                } else if index == 1 {
                    renderer.pipeline_mut().observe(&old, time);
                    renderer.pipeline_mut().observe(&loud, time);
                }
                live = renderer
                    .render_with(|pipeline, encoder, view| {
                        pipeline.encode(encoder, view, frame, time, size)
                    })
                    .unwrap();
            }
            renderer.pipeline_mut().reset_state();
            for index in 0..120 {
                let frame = if index == 0 { quiet } else { loud };
                renderer
                    .pipeline_mut()
                    .advance_state(frame, elapsed(index), size);
            }
            let replayed = renderer.render(loud, elapsed(120)).unwrap();
            assert_eq!(
                live, replayed,
                "{variant:?}: replay differs from held redraws"
            );
        }
    }

    /// Scene 7 enters the colour field at power-on and draws flat regions
    /// whose sRGB pixels and boundaries match the CPU model exactly.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn colour_field_renders_exact_flat_regions() {
        let (width, height) = (161, 91);
        let mut renderer =
            pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), width, height))
                .unwrap();
        for layout in 0..4 {
            renderer
                .pipeline_mut()
                .configure_colour_field(colour_field::Config {
                    palette: colour_field::PaletteSource::Local { p: 1, armed: false },
                    pin: Some(layout),
                    ..colour_field::Config::default()
                });
            let frame = ControlFrame {
                scene: colour_field::SCENE,
                ..ControlFrame::procedural(120)
            };
            let rgb = renderer.render(frame, 2.0).unwrap();
            let colour_field::Display::ColourField(trace) =
                renderer.pipeline_mut().colour_field_display()
            else {
                panic!("scene 7 did not enter the colour field");
            };
            assert_eq!(trace.layout, layout);
            let colours = colour_field::region_colours(&trace);
            let vertical = layout % 2 == 0;
            let n = colour_field::regions(layout);
            let edges = colour_field::boundaries(if vertical { width } else { height }, n);
            for (row, line) in rgb.chunks_exact(width as usize * 3).enumerate() {
                for (column, pixel) in line.chunks_exact(3).enumerate() {
                    // Rows run from the bottom; readback rows from the top.
                    let position = if vertical {
                        column as u32
                    } else {
                        height - 1 - row as u32
                    };
                    let region = edges
                        .iter()
                        .skip(1)
                        .position(|&edge| position < edge)
                        .unwrap();
                    assert_eq!(
                        pixel, colours[region],
                        "layout {layout} pixel ({column}, {row})"
                    );
                }
            }
        }
    }

    /// 100 px rows are 400 bytes, padded to 512 for readback, so a stride or
    /// row-order bug moves the quadrant edges. Also checks that ScreenSpace
    /// output has no quad-edge darkening or beat/energy pulse.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn offscreen_readback_preserves_rows_and_flat_screen_space_colour() {
        use crate::scene::{Camera, Instance, scene_uniforms};

        let (width, height) = (100, 30);
        let profile = RenderProfile::pi5();
        let mut renderer =
            pollster::block_on(OffscreenRenderer::new(profile.clone(), width, height)).unwrap();
        // Top-left quadrant in NDC; linear 0.5 grey encodes to sRGB ~188.
        let quad = Instance {
            center: [-0.5, 0.5, 0.0],
            size: [1.0, 1.0],
            color: [0.5, 0.5, 0.5, 1.0],
            rotation: 0.0,
            scene_mix: 0.0,
            pattern: [0.0; 2],
        };
        let frame = ControlFrame {
            energy: 255,
            flags: 0x01,
            ..ControlFrame::procedural(0)
        };
        let uniforms = scene_uniforms(
            CameraMode::ScreenSpace,
            &mut Camera::default(),
            &profile,
            frame,
            1.0,
            renderer.aspect(),
        );
        assert_eq!(uniforms.beat, 1.0);
        let rgb = renderer
            .render_with(|pipeline, encoder, view| {
                pipeline.encode_instances(encoder, view, &[quad], &uniforms);
            })
            .unwrap();
        assert_eq!(rgb.len(), (width * height * 3) as usize);
        for (row, line) in rgb.chunks_exact(width as usize * 3).enumerate() {
            for (column, pixel) in line.chunks_exact(3).enumerate() {
                let inside = column < width as usize / 2 && row < height as usize / 2;
                let ok = if inside {
                    pixel.iter().all(|value| (186..=190).contains(value))
                } else {
                    pixel == [0, 0, 0]
                };
                assert!(ok, "pixel ({column}, {row}) = {pixel:?}, inside {inside}");
            }
        }
    }

    /// Renders `instances` flat (ScreenSpace) into a `width` x `height` RGB
    /// readback.
    fn render_screen_space(
        width: u32,
        height: u32,
        instances: &[crate::scene::Instance],
    ) -> Vec<u8> {
        use crate::scene::{Camera, scene_uniforms};

        let profile = RenderProfile::pi5();
        let mut renderer =
            pollster::block_on(OffscreenRenderer::new(profile.clone(), width, height)).unwrap();
        let uniforms = scene_uniforms(
            CameraMode::ScreenSpace,
            &mut Camera::default(),
            &profile,
            ControlFrame::procedural(0),
            0.0,
            renderer.aspect(),
        );
        renderer
            .render_with(|pipeline, encoder, view| {
                pipeline.encode_instances(encoder, view, instances, &uniforms);
            })
            .unwrap()
    }

    fn full_screen(color: [f32; 4], pattern: [f32; 2]) -> crate::scene::Instance {
        crate::scene::Instance {
            center: [0.0; 3],
            size: [2.0, 2.0],
            color,
            rotation: 0.0,
            scene_mix: 0.0,
            pattern,
        }
    }

    /// A full-screen Static glyph quad: each of the 5x8 font pixels covers a
    /// 20x20 block, which must be white exactly where the CPU bitmap is set.
    /// 'L' has a lit left column and bottom row and a dark top right, so a
    /// flipped or transposed decode fails.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn static_glyph_pattern_decodes_upright_on_gpu() {
        let (bits, pattern) = scenes::static_gpu_reference::glyph_bits('L');
        let rgb = render_screen_space(100, 160, &[full_screen([1.0; 4], pattern)]);
        let at = |x: usize, y: usize| &rgb[(y * 100 + x) * 3..][..3];
        for row in 0..8 {
            for column in 0..5 {
                let lit = bits >> (row * 5 + column) & 1 == 1;
                // Skip the block edges, where rasterisation may round.
                for y in row * 20 + 2..row * 20 + 18 {
                    for x in column * 20 + 2..column * 20 + 18 {
                        let expected = if lit { [255; 3] } else { [0; 3] };
                        assert_eq!(at(x, y), expected, "({column}, {row}) pixel ({x}, {y})");
                    }
                }
            }
        }
        // Spot checks spelled out: left column, bottom row, top right.
        assert_eq!(at(10, 70), [255; 3]);
        assert_eq!(at(90, 150), [255; 3]);
        assert_eq!(at(90, 30), [0; 3]);
    }

    /// A full-screen Static pixel-noise quad matches the CPU prediction at
    /// every pixel (blank or lit, colour and level), its blank share is near
    /// the requested chance, and rendering is repeatable. Covers monochrome
    /// and rainbow (palette 3) noise on black, and both inverted on a white
    /// section's background, where rainbow hues must be complemented.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn static_pixel_noise_matches_cpu_prediction_on_gpu() {
        use scenes::static_gpu_reference::{noise, pixel};

        let (width, height) = (128, 96);
        let srgb = |linear: f32| {
            let encoded = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            (encoded * 255.0).round() as i32
        };
        let tint = [0.2, 0.6, 0.9, 1.0];
        for (rainbow, inverted, color) in [
            (false, false, [1.0; 4]),
            (true, false, tint),
            (true, true, tint),
            (false, true, tint),
        ] {
            let pattern = noise(0.5, 4, 1234, 3, rainbow, inverted);
            let background = if inverted { 1.0 } else { 0.0 };
            let mut quads = vec![full_screen(color, pattern)];
            if inverted {
                quads.insert(0, full_screen([1.0; 4], [0.0; 2]));
            }
            let rgb = render_screen_space(width, height, &quads);
            assert_eq!(rgb, render_screen_space(width, height, &quads));

            let case = format!("rainbow {rainbow} inverted {inverted}");
            let mut blank = 0;
            for y in 0..height {
                for x in 0..width {
                    let actual = &rgb[((y * width + x) * 3) as usize..][..3];
                    let expected = match pixel(x, y, color, pattern) {
                        None => {
                            blank += 1;
                            [srgb(background); 3]
                        }
                        Some([red, green, blue, alpha]) => [red, green, blue]
                            .map(|channel| srgb(channel * alpha + background * (1.0 - alpha))),
                    };
                    assert!(
                        actual
                            .iter()
                            .zip(expected)
                            .all(|(&value, expected)| (i32::from(value) - expected).abs() <= 2),
                        "{case}: pixel ({x}, {y}) = {actual:?}, expected ~{expected:?}"
                    );
                }
            }
            let share = f64::from(blank) / f64::from(width * height);
            assert!(
                (0.45..=0.55).contains(&share),
                "{case}: blank share {share}"
            );
        }
    }
    /// Object's shape patterns `[0, -(1 + shape)]` on a full-screen white
    /// quad over black: an antialiased disc, a squared radial glow, a
    /// four-pointed sparkle and alternating two-row scanlines.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn object_shape_patterns_render_on_gpu() {
        let size = 100;
        let render = |shape: f32| {
            let quad = full_screen([1.0; 4], [0.0, -1.0 - shape]);
            let rgb = render_screen_space(size, size, &[quad]);
            move |x: u32, y: u32| rgb[((y * size + x) * 3) as usize]
        };
        let disc = render(0.0);
        assert_eq!(disc(50, 50), 255);
        assert_eq!(disc(20, 50), 255);
        assert_eq!(disc(2, 2), 0);
        assert_eq!(disc(98, 97), 0);

        // Linear alpha (1 - 2r)^2 is 0.25 at r = 0.25: sRGB ~137.
        let glow = render(1.0);
        assert!(glow(50, 50) > 250, "{}", glow(50, 50));
        assert!((130..=144).contains(&glow(75, 50)), "{}", glow(75, 50));
        assert_eq!(glow(5, 5), 0);

        let sparkle = render(2.0);
        assert!(sparkle(80, 50) > 150, "arm {}", sparkle(80, 50));
        assert!(sparkle(50, 20) > 150, "arm {}", sparkle(50, 20));
        assert_eq!(sparkle(80, 80), 0, "diagonal");

        let scanlines = render(3.0);
        for y in 0..size {
            let lit = y % 4 < 2;
            assert_eq!(scanlines(30, y), if lit { 255 } else { 0 }, "row {y}");
        }
    }
}
