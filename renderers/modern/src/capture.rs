//! `chipviz-capture`: renders selected frames through the real renderer
//! pipeline into an offscreen texture and writes PNGs plus `manifest.json`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::control_frame::{ControlFrame, WIRE_SIZE};
use crate::profile::RenderProfile;
use crate::renderer::OffscreenRenderer;
use crate::scenes::{self, CameraMode};

/// Capture time base: frame `N` is rendered at `elapsed = N / CAPTURE_FPS`.
pub const CAPTURE_FPS: f32 = 60.0;

const USAGE: &str = "usage: chipviz-capture --out DIR [--profile m1|pi5] [--scene N]
                       [--frames 0,60,120] [--input procedural|PATH.cvz] [--size WxH]

Renders frames offscreen (no window) and writes PNGs plus manifest.json to DIR.
  --profile  render budget (default m1)
  --scene    force the control-frame scene byte; registry index = N % scene count
  --frames   comma-separated frame indices (default 0); elapsed = index / 60 s
  --input    procedural frames (default) or a raw .cvz control-frame file
  --size     output size in pixels (default: the profile's target size)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Procedural,
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
                input = if text == "procedural" {
                    Input::Procedural
                } else {
                    Input::File(PathBuf::from(text))
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

pub fn run(args: &CaptureArgs) -> Result<(), String> {
    let source = FrameSource {
        fixture: match &args.input {
            Input::Procedural => None,
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
    let mut captures = Vec::new();

    for &index in &args.frames {
        // Replay camera smoothing from frame 0 so stills match a live run.
        let aspect = renderer.aspect();
        let pipeline = renderer.pipeline_mut();
        pipeline.reset_camera();
        for previous in 0..index {
            pipeline.advance_camera(source.frame(previous)?, elapsed(previous), aspect);
        }
        let frame = source.frame(index)?;
        let rgb = renderer
            .render(frame, elapsed(index))
            .map_err(|error| error.to_string())?;
        let entry = scenes::select(frame.scene);
        let file = format!("frame{index:06}-{}.png", entry.name);
        write_png(&args.out.join(&file), width, height, &rgb)?;
        println!("wrote {}", args.out.join(&file).display());
        captures.push(format!(
            "    {{\"file\": {}, \"frame_index\": {index}, \"elapsed\": {}, \"scene\": {}, \"scene_name\": {}, \"camera\": {}}}",
            json_string(&file),
            elapsed(index),
            frame.scene,
            json_string(entry.name),
            json_string(match entry.camera {
                CameraMode::Orbit3D => "orbit3d",
                CameraMode::ScreenSpace => "screen_space",
            }),
        ));
    }

    let input = match &args.input {
        Input::Procedural => "procedural".to_string(),
        Input::File(path) => path.display().to_string(),
    };
    let manifest = format!(
        "{{\n  \"profile\": {},\n  \"input\": {},\n  \"width\": {width},\n  \"height\": {height},\n  \"fps\": {CAPTURE_FPS},\n  \"captures\": [\n{}\n  ]\n}}\n",
        json_string(args.profile.name),
        json_string(&input),
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
        ])
        .unwrap();
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
            &["--out"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
        assert!(matches!(parse_args(["-h".to_string()]), Ok(Command::Help)));
    }

    #[test]
    fn scene_override_selects_registry_entry() {
        let source = FrameSource {
            fixture: None,
            scene: Some(5),
        };
        let frame = source.frame(120).unwrap();
        assert_eq!(frame.scene, 5);
        assert_eq!(frame.energy, ControlFrame::procedural(120).energy);
        assert_eq!(scenes::select(frame.scene).name, "PulseTunnel");
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
            let frame = ControlFrame {
                scene,
                ..ControlFrame::procedural(120)
            };
            let rgb = renderer.render(frame, 2.0).unwrap();
            assert_eq!(rgb.len(), 160 * 90 * 3);
            assert!(
                rgb.iter().any(|&value| value > 16),
                "scene {scene} is blank"
            );
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
            _padding: [0.0; 2],
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
}
