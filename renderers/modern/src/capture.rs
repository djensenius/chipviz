//! `chipviz-capture`: renders selected frames through the real renderer
//! pipeline into an offscreen texture and writes PNGs plus `manifest.json`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::audio::analysis::Analyzer;
use crate::audio::mic_frame;
use crate::audio::scope::{ScopeSnapshot, ScopeTap};
use crate::control_frame::{ControlFrame, WIRE_SIZE};
use crate::profile::RenderProfile;
use crate::renderer::{OffscreenRenderer, ScenePipeline};
use crate::scenes::{self, CameraMode, colour_field, mic_layers};

/// Capture time base: frame `N` is rendered at `elapsed = N / CAPTURE_FPS`.
pub const CAPTURE_FPS: f32 = 60.0;

const USAGE: &str = "usage: chipviz-capture --out DIR [--profile m1|pi5] [--scene N]
                       [--frames 0,60,120] [--input procedural|synth-mic|PATH.cvz]
                       [--size WxH] [--mic-layers off|scope|bursts|both]

Renders frames offscreen (no window) and writes PNGs plus manifest.json to DIR.
  --profile  render budget (default m1)
  --scene    force the control-frame scene byte: 7 is the colour field,
             otherwise registry index = N % scene count
  --frames   comma-separated frame indices (default 0); elapsed = index / 60 s
  --input    procedural frames (default), synth-mic (a deterministic synthetic
             microphone: tones plus a click every 0.5 s, silent from 6 s, run
             through the live analyzer and waveform tap) or a raw .cvz file
  --size     output size in pixels (default: the profile's target size)
  --mic-layers
             overlay the mic layers: off (default), scope, bursts or both";

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
}

#[derive(Debug)]
pub enum Command {
    Help,
    Capture(CaptureArgs),
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut profile = RenderProfile::m1();
    let mut scene = None;
    let mut frames = vec![0];
    let mut input = Input::Procedural;
    let mut size = None;
    let mut out = None;
    let mut layers = mic_layers::Mode::Off;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Ok(Command::Help);
        }
        let mut value = || args.next().ok_or_else(|| format!("{arg} requires a value"));
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
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }

    let out = out.ok_or("--out DIR is required")?;
    let size = size.unwrap_or((profile.target_width, profile.target_height));
    Ok(Command::Capture(CaptureArgs {
        profile,
        scene,
        frames,
        input,
        size,
        out,
        mic_layers: layers,
    }))
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

fn elapsed(index: u64) -> f32 {
    index as f32 / CAPTURE_FPS
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
        (frame, self.tap.snapshot(self.analyzer.onsets() as u32))
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
    // Validate every frame before touching the GPU or the output directory.
    for &index in &args.frames {
        source.frame(index)?;
    }
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
    renderer.pipeline_mut().set_mic_layers(args.mic_layers);
    let mut captures = Vec::new();

    for &index in &args.frames {
        // Replay camera smoothing and the colour field from frame 0 so stills
        // match a live run.
        let aspect = renderer.aspect();
        let mut synth = match args.input {
            Input::SynthMic => Some(SynthMic::new()?),
            _ => None,
        };
        let pipeline = renderer.pipeline_mut();
        pipeline.reset_state();
        for previous in 0..index {
            let frame = feed(pipeline, synth.as_mut(), source.frame(previous)?, previous);
            pipeline.advance_state(frame, elapsed(previous), aspect);
        }
        let frame = feed(pipeline, synth.as_mut(), source.frame(index)?, index);
        let rgb = renderer
            .render(frame, elapsed(index))
            .map_err(|error| error.to_string())?;
        // Named by what is shown: scene 7 enters the colour field after its
        // debounce, and leaving it is debounced too.
        let entry = scenes::dispatch(match renderer.pipeline_mut().colour_field_display() {
            colour_field::Display::ColourField(_) => colour_field::SCENE,
            colour_field::Display::Native(scene) => scene,
        });
        let file = format!("frame{index:06}-{}.png", entry.name());
        write_png(&args.out.join(&file), width, height, &rgb)?;
        println!("wrote {}", args.out.join(&file).display());
        captures.push(format!(
            "    {{\"file\": {}, \"frame_index\": {index}, \"elapsed\": {}, \"scene\": {}, \"scene_name\": {}, \"camera\": {}}}",
            json_string(&file),
            elapsed(index),
            frame.scene,
            json_string(entry.name()),
            json_string(match entry.camera() {
                CameraMode::Orbit3D => "orbit3d",
                CameraMode::ScreenSpace => "screen_space",
            }),
        ));
    }

    let input = match &args.input {
        Input::Procedural => "procedural".to_string(),
        Input::SynthMic => "synth-mic".to_string(),
        Input::File(path) => path.display().to_string(),
    };
    let manifest = format!(
        "{{\n  \"profile\": {},\n  \"input\": {},\n  \"mic_layers\": {},\n  \"width\": {width},\n  \"height\": {height},\n  \"fps\": {CAPTURE_FPS},\n  \"captures\": [\n{}\n  ]\n}}\n",
        json_string(args.profile.name),
        json_string(&input),
        json_string(args.mic_layers.name()),
        captures.join(",\n"),
    );
    let path = args.out.join("manifest.json");
    std::fs::write(&path, manifest).map_err(|error| format!("{}: {error}", path.display()))
}

fn write_png(path: &Path, width: u32, height: u32, rgb: &[u8]) -> Result<(), String> {
    let error = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    let file = std::fs::File::create(path).map_err(|e| error(&e))?;
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
    match parse_args(std::env::args().skip(1)) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Capture(args)) => match run(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("chipviz-capture: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("chipviz-capture: {error}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<CaptureArgs, String> {
        match parse_args(args.iter().map(|arg| arg.to_string()))? {
            Command::Capture(args) => Ok(args),
            Command::Help => Err("help".into()),
        }
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
        ])
        .unwrap();
        assert_eq!(args.mic_layers, mic_layers::Mode::Scope);
        assert_eq!(args.profile.name, "pi5");
        assert_eq!(args.scene, Some(4));
        assert_eq!(args.frames, [0, 60, 120]);
        assert_eq!(args.input, Input::File("fixture.cvz".into()));
        assert_eq!(args.size, (1920, 1080));
        assert_eq!(args.out, PathBuf::from("shots"));
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
        assert!(matches!(parse_args(["-h".to_string()]), Ok(Command::Help)));
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
        assert!(loud_scope.points.iter().any(|point| point.abs() > 0.3));
        // From 6 s (frame 360) the input is silent: flat waveform, no onsets.
        let (quiet, quiet_scope) = &frames[420];
        assert_eq!(quiet_scope.points, [0.0; crate::audio::scope::POINTS]);
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

    /// The mic layers never touch the colour field, and `off` renders exactly
    /// as a pipeline that never heard of them; over a native scene `both`
    /// does draw (the no-mic fallback ribbon and beat bursts).
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn mic_layers_skip_the_colour_field_and_off_matches_the_baseline() {
        let render = |scene: u8, mode: Option<mic_layers::Mode>| {
            let mut renderer =
                pollster::block_on(OffscreenRenderer::new(RenderProfile::pi5(), 160, 90)).unwrap();
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
                last = renderer.render(frame, elapsed(index)).unwrap();
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
