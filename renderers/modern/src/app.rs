use std::net::UdpSocket;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowBuilder};

use crate::audio::{self, AudioFeatures, InputDevices, MicSource};
use crate::control_frame::{ControlFrame, WIRE_SIZE};
use crate::profile::RenderProfile;
use crate::renderer::{Renderer, RendererError};
use crate::scenes::{self, SceneConfig, colour_field, mic_layers, starfield};

#[derive(Debug)]
pub enum AppError {
    Window(winit::error::OsError),
    EventLoop(winit::error::EventLoopError),
    Io(std::io::Error),
    Renderer(RendererError),
    Monitor(String),
    Audio(String),
    Starfield(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(error) => write!(formatter, "window error: {error}"),
            Self::EventLoop(error) => write!(formatter, "event loop error: {error}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Renderer(error) => write!(formatter, "renderer error: {error}"),
            Self::Monitor(message) => write!(formatter, "monitor error: {message}"),
            Self::Audio(message) => write!(formatter, "audio error: {message}"),
            Self::Starfield(message) => write!(formatter, "starfield error: {message}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<winit::error::OsError> for AppError {
    fn from(error: winit::error::OsError) -> Self {
        Self::Window(error)
    }
}

impl From<winit::error::EventLoopError> for AppError {
    fn from(error: winit::error::EventLoopError) -> Self {
        Self::EventLoop(error)
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<RendererError> for AppError {
    fn from(error: RendererError) -> Self {
        Self::Renderer(error)
    }
}

pub fn run(profile: RenderProfile) -> Result<(), AppError> {
    let program = std::env::args()
        .next()
        .unwrap_or_else(|| "chipviz".to_string());
    let args = match AppArgs::parse_from(std::env::args().skip(1))? {
        ArgsCommand::Help => {
            println!("{}", usage(&program));
            return Ok(());
        }
        ArgsCommand::Run(args) => args,
    };
    let starfield = args.starfield_config()?;

    // Both list flags may be given together; each list gets a heading then.
    let both_lists = args.list_audio_devices && args.list_monitors;
    if args.list_audio_devices {
        let devices = InputDevices::enumerate().map_err(AppError::Audio)?;
        if both_lists {
            println!("Audio input devices:");
        }
        devices.print();
        if !args.list_monitors {
            return Ok(());
        }
    }

    let event_loop = EventLoop::new()?;

    if args.list_monitors {
        let monitors: Vec<MonitorHandle> = event_loop.available_monitors().collect();
        if both_lists {
            println!("\nMonitors:");
        }
        print_monitors(&monitors);
        return Ok(());
    }

    // Only enumerate displays when one was requested; the default path is unchanged.
    let target_monitor = match &args.monitor {
        Some(selector) => {
            let monitors: Vec<MonitorHandle> = event_loop.available_monitors().collect();
            let names: Vec<Option<String>> = monitors.iter().map(MonitorHandle::name).collect();
            let index = select_monitor(&names, selector).map_err(AppError::Monitor)?;
            Some(monitors[index].clone())
        }
        None => None,
    };

    if args.allow_flash {
        eprintln!("{}", colour_field::allow_flash_notice());
    }

    let udp = UdpFrameSource::bind(args.udp_endpoint.as_deref())?;
    let mic = if args.mic {
        Some(start_mic(args.audio_device.as_ref()).map_err(AppError::Audio)?)
    } else {
        None
    };
    // `None` in default builds, which draw no mic layers.
    let scope = mic.as_ref().and_then(MicSource::scope);

    let mut builder = WindowBuilder::new()
        .with_title(profile.window_title)
        .with_inner_size(LogicalSize::new(
            profile.target_width,
            profile.target_height,
        ));
    if let Some(monitor) = &target_monitor {
        builder = builder.with_position(monitor.position());
    }
    if args.fullscreen {
        builder = builder.with_fullscreen(Some(Fullscreen::Borderless(
            target_monitor
                .clone()
                .or_else(|| event_loop.primary_monitor()),
        )));
    }
    let window = Arc::new(builder.build(&event_loop)?);
    window.set_cursor_visible(cursor_visible(args.fullscreen));

    let mut renderer = pollster::block_on(Renderer::new(window.clone(), profile))?;
    renderer
        .pipeline_mut()
        .configure_colour_field(colour_field_config(&args));
    renderer.pipeline_mut().configure_scenes(SceneConfig {
        lines_drop_cuts: args.lines_drop_cuts,
    });
    renderer
        .pipeline_mut()
        .configure_starfield(starfield.clone());
    renderer.pipeline_mut().limit_colour_field_steps();
    if mic_layers::allowed(args.mic_layers) != args.mic_layers {
        eprintln!("{}", mic_layers::NO_LAYERS_NOTICE);
    }
    renderer
        .pipeline_mut()
        .set_mic_layers(mic_layers::allowed(args.mic_layers));
    let mut frame_source = FrameSource::new(udp, mic);
    let mut modifiers = ModifiersState::empty();
    let start = Instant::now();
    let mut missing_starfield_warning = false;

    event_loop.run(move |event, target| match event {
        Event::WindowEvent { event, window_id } if window_id == window.id() => match event {
            WindowEvent::CloseRequested => target.exit(),
            WindowEvent::Resized(size) => {
                // Fullscreen can also change via the OS (e.g. the green window
                // button), so resync the cursor whenever the surface changes.
                window.set_cursor_visible(cursor_visible(window.fullscreen().is_some()));
                renderer.resize(size.width, size.height);
            }
            WindowEvent::ModifiersChanged(new_modifiers) => modifiers = new_modifiers.state(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                let input = KeyInput::from_winit(&event.logical_key, event.physical_key);
                let mods = KeyModifiers {
                    command: modifiers.super_key(),
                    control: modifiers.control_key(),
                    alt: modifiers.alt_key(),
                };
                let fullscreen = window.fullscreen().is_some();
                match key_action(input, mods, fullscreen) {
                    Some(KeyAction::ToggleFullscreen) => set_fullscreen(&window, !fullscreen),
                    Some(KeyAction::ExitFullscreen) => set_fullscreen(&window, false),
                    Some(KeyAction::CycleLayoutPin) => renderer.pipeline_mut().cycle_layout_pin(),
                    Some(KeyAction::CycleMicLayers) if !mic_layers::LAYERS_BUILD => {
                        eprintln!("{}", mic_layers::NO_LAYERS_NOTICE);
                    }
                    Some(KeyAction::CycleMicLayers) => {
                        let mode = renderer.pipeline_mut().cycle_mic_layers();
                        eprintln!("mic layers: {}", mode.name());
                    }
                    None => {}
                }
            }
            WindowEvent::RedrawRequested => {
                let elapsed = start.elapsed().as_secs_f64();
                let mut frame = with_scene_override(frame_source.next_frame(), args.scene);
                // The colour field sees each received or generated frame once;
                // held redraws only repeat its last step.
                observe_fresh_frames(
                    frame_source.fresh(),
                    args.scene,
                    elapsed,
                    |fresh, elapsed| {
                        renderer.pipeline_mut().observe(fresh, elapsed);
                    },
                );
                let displayed_scene = renderer.pipeline_mut().displayed_scene(frame);
                if needs_starfield_catalog(displayed_scene) && starfield.is_none() {
                    if !missing_starfield_warning {
                        eprintln!(
                            "scene 6 (Starfield) needs --star-catalog PATH and --location LAT,LON; \
                             rendering scene 0 until both options are supplied."
                        );
                        missing_starfield_warning = true;
                    }
                    frame = safe_starfield_fallback(frame);
                }
                // The waveform side channel only describes mic frames; UDP
                // and procedural frames make the layers fall back.
                let live = scope
                    .as_ref()
                    .filter(|_| frame_source.showing_mic())
                    .map(|scope| scope.load());
                renderer.pipeline_mut().set_live_scope(live);
                match renderer.render(frame, elapsed) {
                    Ok(()) => {}
                    Err(RendererError::Surface(
                        wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated,
                    )) => {
                        renderer.resize_to_current_window();
                    }
                    Err(RendererError::Surface(wgpu::SurfaceError::OutOfMemory)) => target.exit(),
                    Err(error) => eprintln!("{error}"),
                }
            }
            _ => {}
        },
        Event::AboutToWait => window.request_redraw(),
        _ => {}
    })?;

    Ok(())
}

fn set_fullscreen(window: &Window, fullscreen: bool) {
    if fullscreen {
        window.set_fullscreen(Some(Fullscreen::Borderless(window.current_monitor())));
    } else {
        window.set_fullscreen(None);
    }
    window.set_cursor_visible(cursor_visible(fullscreen));
}

fn print_monitors(monitors: &[MonitorHandle]) {
    if monitors.is_empty() {
        println!("no monitors detected");
        return;
    }
    for (index, monitor) in monitors.iter().enumerate() {
        let size = monitor.size();
        println!(
            "{index}: {} ({}x{}, scale {:.2})",
            monitor.name().unwrap_or_else(|| "<unnamed>".to_string()),
            size.width,
            size.height,
            monitor.scale_factor()
        );
    }
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} [--udp LISTEN_ADDR:PORT] [--mic [--audio-device INDEX|NAME]] [--fullscreen] [--monitor INDEX|NAME]
       [--layout auto|v2|h2|v3|h3] [--allow-flash] [--mic-layers off|scope|bursts|both]
       [--lines-drop-cuts] [--scene N]
       {program} --list-monitors | --list-audio-devices

Options:
  --udp LISTEN_ADDR:PORT  consume control-frame-v0 packets (default: procedural frames)
  --star-catalog PATH     HYG-compatible star CSV for scene 6 (not bundled)
  --catalog FILE          alias for --star-catalog
  --location LAT,LON      observer position in decimal degrees (longitude east)
  --time UNIX_SECONDS     UTC observation time for scene 6 (1800..2200; default: current UTC)
  --scene N               override the displayed scene byte (0-255), including UDP;
                          without this option, incoming and procedural scenes are unchanged
  --mic                   drive frames from live microphone analysis instead of
                          procedural frames; UDP packets, when they arrive, still win
  --audio-device INDEX|NAME
                          microphone to use with --mic, by index or case-insensitive
                          name substring (default: system default input)
  --list-audio-devices    print available audio input devices and exit
  --fullscreen            start in borderless fullscreen (default: windowed)
  --monitor INDEX|NAME    display to use, by index or case-insensitive name substring
  --list-monitors         print available monitors and exit
  --layout LAYOUT         colour-field (scene 7) layout pin (default: auto)
  --allow-flash           print the flash/gate warning and arm operator flash requests;
                          scene 7 wire palettes 3 and 7 from a host started with
                          --allow-flash arm flash/gate without it. Only builds with
                          the unmeasured-flash feature can flash; default builds run
                          3 and 7 as hard cuts and print a notice instead
  --mic-layers MODE       overlay an oscilloscope ribbon and/or onset bursts on the
                          native scenes: off (default), scope, bursts or both. With
                          --mic they follow the microphone; otherwise they are
                          synthesised from the spectrum, energy and beats. Only
                          builds with the unmeasured-flash feature draw them;
                          default builds print a notice and draw nothing
  --lines-drop-cuts       Lines (scene 3) cuts between plane and hallway on a drop
                          flag, at most once per bar (default: drops are ignored)
  Scene 6 requires both --star-catalog and --location; scene 7 remains ColourField
  -h, --help              print this help

Keys:
  F, Cmd+F, Ctrl+Cmd+F    toggle fullscreen
  L                       cycle the colour-field layout pin (auto, V2, H2, V3, H3)
  M                       cycle the mic layers (off, scope, bursts, both) in builds
                          with the unmeasured-flash feature
  Esc                     exit fullscreen
  Cmd+Q                   quit (macOS application menu)"
    )
}

/// Pure cursor policy: the cursor is hidden only while fullscreen.
fn cursor_visible(fullscreen: bool) -> bool {
    !fullscreen
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeviceSelector {
    Index(usize),
    Name(String),
}

impl DeviceSelector {
    fn parse(value: &str) -> Self {
        match value.parse::<usize>() {
            Ok(index) => Self::Index(index),
            Err(_) => Self::Name(value.to_string()),
        }
    }
}

/// Wording for selection errors about one kind of device.
struct DeviceKind {
    noun: &'static str,
    flag: &'static str,
    list_flag: &'static str,
}

const MONITOR: DeviceKind = DeviceKind {
    noun: "monitor",
    flag: "--monitor",
    list_flag: "--list-monitors",
};

const AUDIO_INPUT: DeviceKind = DeviceKind {
    noun: "audio input device",
    flag: "--audio-device",
    list_flag: "--list-audio-devices",
};

fn select_monitor(names: &[Option<String>], selector: &DeviceSelector) -> Result<usize, String> {
    select_device(names, selector, &MONITOR)
}

/// Resolves a selector against device names (in enumeration order).
/// All-digit selectors are indices (as printed by the list flag). Names
/// match as a case-insensitive substring; a *unique* exact (case-insensitive)
/// name wins when several devices match. Duplicate names (e.g. two identical
/// displays) are ambiguous and must be selected by index.
fn select_device(
    names: &[Option<String>],
    selector: &DeviceSelector,
    kind: &DeviceKind,
) -> Result<usize, String> {
    let DeviceKind {
        noun,
        flag,
        list_flag,
    } = kind;
    match selector {
        DeviceSelector::Index(index) if *index < names.len() => Ok(*index),
        DeviceSelector::Index(index) => Err(format!(
            "{noun} index {index} out of range ({} {noun}(s) available; see {list_flag})",
            names.len()
        )),
        DeviceSelector::Name(query) => {
            let needle = query.to_lowercase();
            let lowered: Vec<Option<String>> = names
                .iter()
                .map(|name| name.as_ref().map(|name| name.to_lowercase()))
                .collect();
            let exact: Vec<usize> = lowered
                .iter()
                .enumerate()
                .filter(|(_, name)| name.as_deref() == Some(&*needle))
                .map(|(index, _)| index)
                .collect();
            if let [index] = exact.as_slice() {
                return Ok(*index);
            }
            let matches: Vec<usize> = lowered
                .iter()
                .enumerate()
                .filter(|(_, name)| name.as_deref().is_some_and(|name| name.contains(&needle)))
                .map(|(index, _)| index)
                .collect();
            match matches.as_slice() {
                [index] => Ok(*index),
                [] => Err(format!(
                    "no {noun} name contains \"{query}\" (see {list_flag})"
                )),
                _ => Err(format!(
                    "{noun} name \"{query}\" is ambiguous: matches indices {matches:?}; select one with {flag} INDEX (see {list_flag})"
                )),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyInput {
    Character(char),
    Escape,
    Other,
}

impl KeyInput {
    /// Prefers the layout-aware logical key so Latin layouts (QWERTY, Dvorak,
    /// AZERTY, ...) use the labelled key. Falls back to the physical key when
    /// the logical key is not ASCII (e.g. Cyrillic or Greek layouts).
    fn from_winit(logical: &Key, physical: PhysicalKey) -> Self {
        match logical {
            Key::Named(NamedKey::Escape) => return Self::Escape,
            Key::Character(text) => {
                let mut chars = text.chars();
                if let (Some(character), None) = (chars.next(), chars.next())
                    && character.is_ascii()
                {
                    return Self::Character(character.to_ascii_lowercase());
                }
            }
            _ => {}
        }
        match physical {
            PhysicalKey::Code(KeyCode::KeyF) => Self::Character('f'),
            PhysicalKey::Code(KeyCode::KeyL) => Self::Character('l'),
            PhysicalKey::Code(KeyCode::KeyM) => Self::Character('m'),
            PhysicalKey::Code(KeyCode::Escape) => Self::Escape,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KeyModifiers {
    /// Cmd on macOS (the "super" key elsewhere).
    command: bool,
    control: bool,
    alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    ToggleFullscreen,
    ExitFullscreen,
    CycleLayoutPin,
    CycleMicLayers,
}

fn key_action(input: KeyInput, mods: KeyModifiers, fullscreen: bool) -> Option<KeyAction> {
    if mods.alt {
        return None;
    }
    match (input, mods.command, mods.control) {
        // F, Cmd+F, and the macOS-standard Ctrl+Cmd+F.
        (KeyInput::Character('f'), _, false) | (KeyInput::Character('f'), true, true) => {
            Some(KeyAction::ToggleFullscreen)
        }
        (KeyInput::Escape, false, false) if fullscreen => Some(KeyAction::ExitFullscreen),
        (KeyInput::Character('l'), false, false) => Some(KeyAction::CycleLayoutPin),
        (KeyInput::Character('m'), false, false) => Some(KeyAction::CycleMicLayers),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
enum ArgsCommand {
    Run(AppArgs),
    Help,
}

#[derive(Debug, Default, PartialEq)]
struct AppArgs {
    udp_endpoint: Option<String>,
    fullscreen: bool,
    monitor: Option<DeviceSelector>,
    list_monitors: bool,
    mic: bool,
    audio_device: Option<DeviceSelector>,
    list_audio_devices: bool,
    /// Colour-field layout pin; `None` is `auto`.
    layout: Option<u8>,
    allow_flash: bool,
    mic_layers: mic_layers::Mode,
    lines_drop_cuts: bool,
    scene: Option<u8>,
    catalog_path: Option<std::path::PathBuf>,
    location: Option<(f64, f64)>,
    unix_seconds: Option<f64>,
}

/// Wire palettes go through the stability gate, and a stable absolute 3 or 7
/// arms whatever `--allow-flash` says (the host's flag is the opt-in).
/// Procedural and mic-only frames use a local palette 0, which never arms.
fn colour_field_config(args: &AppArgs) -> colour_field::Config {
    colour_field::Config {
        allow_flash: args.allow_flash,
        palette: if args.udp_endpoint.is_some() {
            colour_field::PaletteSource::Wire
        } else {
            colour_field::PaletteSource::Local { p: 0, armed: false }
        },
        pin: args.layout,
    }
}

fn parse_layout(value: &str) -> Result<Option<u8>, AppError> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(None),
        "v2" => Ok(Some(0)),
        "h2" => Ok(Some(1)),
        "v3" => Ok(Some(2)),
        "h3" => Ok(Some(3)),
        _ => Err(invalid_input(format!(
            "--layout expects auto|v2|h2|v3|h3, got {value}"
        ))),
    }
}

impl AppArgs {
    fn starfield_config(&self) -> Result<Option<starfield::StarfieldConfig>, AppError> {
        let (Some(path), Some((latitude_degrees, longitude_degrees))) =
            (&self.catalog_path, self.location)
        else {
            return Ok(None);
        };
        let catalog = starfield::StarCatalog::load(path)
            .map_err(|error| AppError::Starfield(error.to_string()))?;
        let unix_seconds = match self.unix_seconds {
            Some(seconds) => seconds,
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs_f64())
                .unwrap_or_else(|error| -error.duration().as_secs_f64()),
        };
        Ok(Some(starfield::StarfieldConfig {
            catalog: std::sync::Arc::new(catalog),
            latitude_degrees,
            longitude_degrees,
            unix_seconds,
            follow_elapsed: self.unix_seconds.is_none(),
        }))
    }

    fn parse_from<I: IntoIterator<Item = String>>(args: I) -> Result<ArgsCommand, AppError> {
        let mut parsed = Self::default();
        let mut args = args.into_iter();

        let mut first = true;
        while let Some(arg) = args.next() {
            let is_first = std::mem::replace(&mut first, false);
            match arg.as_str() {
                "--udp" => {
                    parsed.udp_endpoint =
                        Some(required_value(&mut args, "--udp", "LISTEN_ADDR:PORT")?)
                }
                "--fullscreen" => parsed.fullscreen = true,
                "--monitor" => {
                    let value = required_value(&mut args, "--monitor", "INDEX or NAME")?;
                    parsed.monitor = Some(DeviceSelector::parse(&value));
                }
                "--list-monitors" => parsed.list_monitors = true,
                "--mic" => parsed.mic = true,
                "--audio-device" => {
                    let value = required_value(&mut args, "--audio-device", "INDEX or NAME")?;
                    parsed.audio_device = Some(DeviceSelector::parse(&value));
                }
                "--list-audio-devices" => parsed.list_audio_devices = true,
                "--layout" => {
                    let value = required_value(&mut args, "--layout", "auto|v2|h2|v3|h3")?;
                    parsed.layout = parse_layout(&value)?;
                }
                "--allow-flash" => parsed.allow_flash = true,
                "--mic-layers" => {
                    let value = required_value(&mut args, "--mic-layers", "off|scope|bursts|both")?;
                    parsed.mic_layers = mic_layers::Mode::parse(&value).ok_or_else(|| {
                        invalid_input(format!(
                            "--mic-layers expects off|scope|bursts|both, got {value}"
                        ))
                    })?;
                }
                "--lines-drop-cuts" => parsed.lines_drop_cuts = true,
                "--scene" => {
                    let value = required_value(&mut args, "--scene", "0-255")?;
                    parsed.scene = Some(value.parse::<u8>().map_err(|_| {
                        invalid_input(format!("--scene expects 0-255, got {value}"))
                    })?);
                }
                "--catalog" | "--star-catalog" => {
                    parsed.catalog_path = Some(std::path::PathBuf::from(required_value(
                        &mut args,
                        arg.as_str(),
                        "PATH",
                    )?));
                }
                "--location" => {
                    let value = required_value(&mut args, "--location", "LAT,LON")?;
                    parsed.location = Some(parse_location(&value)?);
                }
                "--time" => {
                    let value = required_value(&mut args, "--time", "UNIX_SECONDS")?;
                    let seconds = value.parse::<f64>().map_err(|_| {
                        invalid_input(format!("--time must be finite UNIX seconds, got {value}"))
                    })?;
                    if !seconds.is_finite()
                        || !(-5_364_662_400.0..7_289_654_400.0).contains(&seconds)
                    {
                        return Err(invalid_input(format!(
                            "--time must be finite UNIX seconds in the UTC year 1800..2200 range, got {value}"
                        )));
                    }
                    parsed.unix_seconds = Some(seconds);
                }
                // `cargo run -- ...` / `just recipe -- ...` may forward a separator.
                "--" if is_first => {}
                "--help" | "-h" => return Ok(ArgsCommand::Help),
                _ => return Err(invalid_input(format!("unknown argument: {arg}"))),
            }
        }

        if parsed.audio_device.is_some() && !parsed.mic && !parsed.list_audio_devices {
            return Err(invalid_input("--audio-device requires --mic".to_string()));
        }
        if parsed.catalog_path.is_some() != parsed.location.is_some() {
            return Err(invalid_input(
                "--star-catalog and --location must be supplied together for scene 6".to_string(),
            ));
        }
        if parsed.unix_seconds.is_some() && parsed.catalog_path.is_none() {
            return Err(invalid_input(
                "--time requires --star-catalog and --location".to_string(),
            ));
        }
        if parsed.scene.is_some_and(needs_starfield_catalog) && parsed.catalog_path.is_none() {
            return Err(invalid_input(
                "--scene 6 (Starfield) requires --star-catalog PATH and --location LAT,LON"
                    .to_string(),
            ));
        }

        Ok(ArgsCommand::Run(parsed))
    }
}

fn with_scene_override(mut frame: ControlFrame, scene: Option<u8>) -> ControlFrame {
    if let Some(scene) = scene {
        frame.scene = scene;
    }
    frame
}

fn observe_fresh_frames(
    frames: &[ControlFrame],
    scene_override: Option<u8>,
    elapsed: f64,
    mut observe: impl FnMut(&ControlFrame, f64),
) {
    for frame in frames {
        let frame = with_scene_override(*frame, scene_override);
        observe(&frame, elapsed);
    }
}

fn needs_starfield_catalog(scene: u8) -> bool {
    scenes::dispatch(scene).name() == "Starfield"
}

fn safe_starfield_fallback(mut frame: ControlFrame) -> ControlFrame {
    frame.scene = 0;
    frame
}
fn parse_location(value: &str) -> Result<(f64, f64), AppError> {
    let Some((latitude, longitude)) = value.split_once(',') else {
        return Err(invalid_input(format!(
            "--location expects LAT,LON decimal degrees, got {value}"
        )));
    };
    if longitude.contains(',') {
        return Err(invalid_input(format!(
            "--location expects exactly LAT,LON, got {value}"
        )));
    }
    let latitude = latitude.parse::<f64>().map_err(|_| {
        invalid_input(format!(
            "--location latitude must be finite, got {latitude}"
        ))
    })?;
    let longitude = longitude.parse::<f64>().map_err(|_| {
        invalid_input(format!(
            "--location longitude must be finite, got {longitude}"
        ))
    })?;
    if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
        return Err(invalid_input(format!(
            "--location latitude must be finite and in [-90, 90], got {latitude}"
        )));
    }
    if !longitude.is_finite() || !(-180.0..=180.0).contains(&longitude) {
        return Err(invalid_input(format!(
            "--location longitude must be finite and in [-180, 180] (east positive), got {longitude}"
        )));
    }
    Ok((latitude, longitude))
}

fn required_value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
    expected: &str,
) -> Result<String, AppError> {
    args.next()
        .ok_or_else(|| invalid_input(format!("{flag} requires {expected}")))
}

fn invalid_input(message: String) -> AppError {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into()
}

struct UdpFrameSource {
    socket: Option<UdpSocket>,
}

impl UdpFrameSource {
    fn bind(endpoint: Option<&str>) -> Result<Self, std::io::Error> {
        let Some(endpoint) = endpoint else {
            return Ok(Self { socket: None });
        };

        let socket = UdpSocket::bind(endpoint)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket: Some(socket),
        })
    }

    /// Every valid frame received since the last call, oldest first.
    fn receive_all(&self) -> Vec<ControlFrame> {
        let mut frames = Vec::new();
        let Some(socket) = self.socket.as_ref() else {
            return frames;
        };
        let mut packet = [0; WIRE_SIZE];

        loop {
            match socket.recv(&mut packet) {
                Ok(WIRE_SIZE) => match ControlFrame::parse_wire(&packet) {
                    Ok(frame) => frames.push(frame),
                    Err(error) => eprintln!("ignored invalid control frame: {error}"),
                },
                Ok(size) => eprintln!("ignored short UDP packet: {size} bytes"),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    eprintln!("UDP receive error: {error}");
                    break;
                }
            }
        }

        frames
    }
}

/// Anything that can hand the render thread the latest analysed audio.
trait LatestFeatures {
    fn latest(&self) -> AudioFeatures;
}

impl LatestFeatures for MicSource {
    fn latest(&self) -> AudioFeatures {
        MicSource::latest(self)
    }
}

fn start_mic(selector: Option<&DeviceSelector>) -> Result<MicSource, String> {
    let devices = InputDevices::enumerate()?;
    let index = selector
        .map(|selector| select_device(devices.names(), selector, &AUDIO_INPUT))
        .transpose()?;
    MicSource::start(devices, index)
}

/// After a UDP packet, UDP keeps priority over the mic for this long, so a
/// bridge streaming at about the render rate never flickers with mic frames.
const UDP_HOLD: Duration = Duration::from_millis(500);

/// What to render when no UDP packet arrived this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdleSource {
    /// Keep the last frame (UDP-only mode, or a recent UDP packet).
    Hold,
    Procedural,
    Mic,
}

/// Precedence: fresh UDP packets > mic (`--mic`) > procedural fallback.
/// Without `--mic` this is the pre-mic behaviour: UDP mode holds the last
/// packet and procedural frames run only when `--udp` is absent.
fn idle_source(mic: bool, udp: bool, since_udp: Option<Duration>) -> IdleSource {
    if mic {
        if since_udp.is_some_and(|elapsed| elapsed < UDP_HOLD) {
            IdleSource::Hold
        } else {
            IdleSource::Mic
        }
    } else if udp {
        IdleSource::Hold
    } else {
        IdleSource::Procedural
    }
}

struct FrameSource {
    udp: UdpFrameSource,
    mic: Option<Box<dyn LatestFeatures>>,
    fallback_index: u64,
    latest: ControlFrame,
    last_udp: Option<Instant>,
    mic_beat_count: Option<u16>,
    fresh: Vec<ControlFrame>,
    /// The last `next_frame_at` returned a mic frame.
    showing_mic: bool,
}

impl FrameSource {
    fn new(udp: UdpFrameSource, mic: Option<MicSource>) -> Self {
        Self::with_feed(udp, mic.map(|mic| Box::new(mic) as Box<dyn LatestFeatures>))
    }

    fn with_feed(udp: UdpFrameSource, mic: Option<Box<dyn LatestFeatures>>) -> Self {
        Self {
            udp,
            mic,
            fallback_index: 0,
            latest: ControlFrame::procedural(0),
            last_udp: None,
            mic_beat_count: None,
            fresh: Vec::new(),
            showing_mic: false,
        }
    }

    fn next_frame(&mut self) -> ControlFrame {
        self.next_frame_at(Instant::now())
    }

    /// Frames received or generated by the last `next_frame_at`, oldest
    /// first. Empty on a held redraw: a held frame is a display repeat, not
    /// a new frame, so the colour field neither steps on it nor sees its
    /// cleared one-off flags as falling edges.
    fn fresh(&self) -> &[ControlFrame] {
        &self.fresh
    }

    /// Whether the frame from the last `next_frame_at` came from the mic.
    fn showing_mic(&self) -> bool {
        self.showing_mic
    }

    fn next_frame_at(&mut self, now: Instant) -> ControlFrame {
        self.showing_mic = false;
        self.fresh = self.udp.receive_all();
        if let Some(&frame) = self.fresh.last() {
            self.latest = frame;
            self.last_udp = Some(now);
            self.mic_beat_count = None;
            return frame;
        }

        let since_udp = self.last_udp.map(|last| now.duration_since(last));
        match idle_source(self.mic.is_some(), self.udp.socket.is_some(), since_udp) {
            IdleSource::Hold => {
                // Beat, bar, and drop are one-off events: a held packet (with
                // or without a mic) keeps its levels but must not repeat them
                // every redraw, or a paused sender pulses forever. Notes are
                // sustained voice state (active/gated voices that producers
                // resend until note-off), so they are kept.
                self.latest.flags &= !(audio::BEAT_FLAG | audio::BAR_FLAG | audio::DROP_FLAG);
            }
            // Generated frames number from `fallback_index`, independent of
            // UDP counters. On a handover the first one can equal the last
            // stepped counter; the colour field then skips that one frame
            // (see `ColourField::has_new_frame`).
            IdleSource::Procedural => {
                self.fallback_index = self.fallback_index.wrapping_add(1);
                self.latest = ControlFrame::procedural(self.fallback_index);
                self.fresh.push(self.latest);
            }
            IdleSource::Mic => {
                if let Some(mic) = &self.mic {
                    self.fallback_index = self.fallback_index.wrapping_add(1);
                    let features = mic.latest();
                    let base = ControlFrame::procedural(self.fallback_index);
                    self.latest = audio::mic_frame(base, features, self.mic_beat_count);
                    self.mic_beat_count = Some(features.beat_count);
                    self.fresh.push(self.latest);
                    self.showing_mic = true;
                }
            }
        }

        self.latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ArgsCommand, AppError> {
        AppArgs::parse_from(args.iter().map(|arg| arg.to_string()))
    }

    fn parse_run(args: &[&str]) -> AppArgs {
        match parse(args).expect("args should parse") {
            ArgsCommand::Run(args) => args,
            ArgsCommand::Help => panic!("unexpected help"),
        }
    }

    fn names(names: &[Option<&str>]) -> Vec<Option<String>> {
        names.iter().map(|name| name.map(str::to_string)).collect()
    }

    #[test]
    fn default_args_are_windowed() {
        assert_eq!(parse_run(&[]), AppArgs::default());
        assert!(!AppArgs::default().fullscreen);
        assert_eq!(AppArgs::default().scene, None);
    }

    #[test]
    fn scene_override_changes_only_the_displayed_scene() {
        let frame = ControlFrame {
            scene: 13,
            ..ControlFrame::procedural(12)
        };
        let unchanged = with_scene_override(frame, None);
        assert_eq!(unchanged.scene, 13);
        assert_eq!(unchanged.energy, frame.energy);
        assert_eq!(unchanged.spectrum, frame.spectrum);
        let overridden = with_scene_override(frame, Some(6));
        assert_eq!(overridden.scene, 6);
        assert_eq!(overridden.energy, frame.energy);
        assert_eq!(overridden.spectrum, frame.spectrum);
    }

    #[test]
    fn wrapped_scene_selection_requires_catalog_and_falls_back_without_losing_frame_data() {
        assert!(needs_starfield_catalog(6));
        assert!(needs_starfield_catalog(13));
        assert!(!needs_starfield_catalog(7));

        let frame = ControlFrame {
            scene: 13,
            flags: audio::BEAT_FLAG | audio::DROP_FLAG,
            energy: 177,
            ..ControlFrame::procedural(44)
        };
        let fallback = safe_starfield_fallback(frame);
        assert_eq!(fallback.scene, 0);
        assert_eq!(fallback.flags, frame.flags);
        assert_eq!(fallback.energy, frame.energy);
    }

    #[test]
    fn missing_starfield_fallback_does_not_skip_earlier_udp_batch_events() {
        let frames = [
            ControlFrame {
                flags: audio::BEAT_FLAG | audio::DROP_FLAG,
                ..ControlFrame::procedural(1)
            },
            ControlFrame {
                scene: 13,
                ..ControlFrame::procedural(2)
            },
        ];
        let mut observed = Vec::new();
        observe_fresh_frames(&frames, None, 1.5, |frame, elapsed| {
            observed.push((frame.scene, frame.flags, elapsed));
        });
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0], (0, audio::BEAT_FLAG | audio::DROP_FLAG, 1.5));
        assert_eq!(observed[1], (13, 0, 1.5));
        assert!(needs_starfield_catalog(observed[1].0));
    }

    #[test]
    fn parses_fullscreen_monitor_and_udp() {
        let args = parse_run(&["--fullscreen", "--monitor", "1", "--udp", "127.0.0.1:6464"]);
        assert!(args.fullscreen);
        assert_eq!(args.monitor, Some(DeviceSelector::Index(1)));
        assert_eq!(args.udp_endpoint.as_deref(), Some("127.0.0.1:6464"));
        assert!(!args.list_monitors);
    }

    #[test]
    fn parses_starfield_catalog_location_and_time() {
        let wrapped_error = parse(&["--scene", "13"]).unwrap_err().to_string();
        assert!(wrapped_error.contains("requires --star-catalog PATH"));
        let args = parse_run(&[
            "--scene",
            "6",
            "--star-catalog",
            "stars.csv",
            "--location",
            "45.5,-73.25",
            "--time",
            "946728000",
        ]);
        assert_eq!(
            args.catalog_path.as_deref(),
            Some(std::path::Path::new("stars.csv"))
        );
        assert_eq!(args.location, Some((45.5, -73.25)));
        assert_eq!(args.unix_seconds, Some(946_728_000.0));
        assert_eq!(args.scene, Some(6));
        assert_eq!(
            parse_run(&["--catalog", "stars.csv", "--location", "0,0"]).location,
            Some((0.0, 0.0))
        );
        assert_eq!(
            parse_run(&["--catalog", "stars.csv", "--location", "0,180"]).location,
            Some((0.0, 180.0))
        );
        assert_eq!(
            parse_run(&[
                "--catalog",
                "stars.csv",
                "--location",
                "0,0",
                "--time",
                "7289654399",
            ])
            .unix_seconds,
            Some(7_289_654_399.0)
        );
        assert!(
            parse(&[
                "--catalog",
                "stars.csv",
                "--location",
                "0,0",
                "--time",
                "7289654400",
            ])
            .is_err()
        );
    }

    #[test]
    fn starfield_cli_loads_a_local_csv_path() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chipviz-starfield-{}-{unique}.csv",
            std::process::id()
        ));
        std::fs::write(&path, "ra,dec,mag,ci,proper\n0,90,1,,\n").expect("write fixture");
        let args = parse_run(&[
            "--scene",
            "6",
            "--star-catalog",
            path.to_str().expect("UTF-8 temp path"),
            "--location",
            "45,-73",
            "--time",
            "946728000",
        ]);
        let loaded = args.starfield_config();
        let _ = std::fs::remove_file(&path);
        let config = loaded
            .expect("catalog setup")
            .expect("configured starfield");
        assert_eq!(config.catalog.stars().len(), 1);
        assert_eq!(config.latitude_degrees, 45.0);
        assert_eq!(config.longitude_degrees, -73.0);
    }

    #[test]
    fn rejects_incomplete_and_invalid_starfield_options() {
        for args in [
            &["--catalog", "stars.csv"][..],
            &["--location", "0,0"][..],
            &["--scene", "6"][..],
            &["--scene", "256"][..],
            &["--scene"][..],
            &["--time", "1"][..],
            &["--catalog", "stars.csv", "--location", "91,0"][..],
            &["--catalog", "stars.csv", "--location", "0,181"][..],
            &["--catalog", "stars.csv", "--location", "nan,0"][..],
            &[
                "--catalog",
                "stars.csv",
                "--location",
                "0,0",
                "--time",
                "inf",
            ][..],
        ] {
            let error = parse(args).expect_err("invalid starfield options");
            assert!(
                matches!(error, AppError::Io(ref io) if io.kind() == std::io::ErrorKind::InvalidInput),
                "{args:?}: {error}"
            );
        }
        let error = parse(&["--scene", "6"]).unwrap_err().to_string();
        assert!(
            error.contains("--star-catalog PATH and --location LAT,LON"),
            "{error}"
        );
    }

    #[test]
    fn non_numeric_monitor_is_a_name_selector() {
        let args = parse_run(&["--monitor", "Studio Display"]);
        assert_eq!(
            args.monitor,
            Some(DeviceSelector::Name("Studio Display".to_string()))
        );
    }

    #[test]
    fn parses_list_monitors_and_help() {
        assert!(parse_run(&["--list-monitors"]).list_monitors);
        assert_eq!(parse(&["--help"]).unwrap(), ArgsCommand::Help);
        assert_eq!(parse(&["-h", "--bogus"]).unwrap(), ArgsCommand::Help);
    }

    #[test]
    fn rejects_missing_values_and_unknown_args() {
        for args in [&["--monitor"][..], &["--udp"][..], &["--bogus"][..]] {
            let error = parse(args).expect_err("should fail");
            assert!(
                matches!(error, AppError::Io(ref io) if io.kind() == std::io::ErrorKind::InvalidInput)
            );
        }
        let message = parse(&["--monitor"]).unwrap_err().to_string();
        assert!(message.contains("--monitor requires"), "{message}");
    }

    #[test]
    fn selects_monitor_by_index() {
        let monitors = names(&[Some("Built-in Retina Display"), Some("LG UltraFine")]);
        assert_eq!(select_monitor(&monitors, &DeviceSelector::Index(1)), Ok(1));
        let error = select_monitor(&monitors, &DeviceSelector::Index(2)).unwrap_err();
        assert!(error.contains("out of range"), "{error}");
    }

    #[test]
    fn selects_monitor_by_case_insensitive_substring() {
        let monitors = names(&[Some("Built-in Retina Display"), None, Some("LG UltraFine")]);
        let select = |query: &str| select_monitor(&monitors, &DeviceSelector::Name(query.into()));
        assert_eq!(select("ultrafine"), Ok(2));
        assert_eq!(select("RETINA"), Ok(0));
        assert!(select("Sidecar").unwrap_err().contains("no monitor"));
    }

    #[test]
    fn ambiguous_monitor_name_errors_unless_exact() {
        let monitors = names(&[
            Some("DELL U2720Q"),
            Some("DELL U2720Q (2)"),
            Some("DELL P2419H"),
        ]);
        let select = |query: &str| select_monitor(&monitors, &DeviceSelector::Name(query.into()));
        assert!(select("dell").unwrap_err().contains("ambiguous"));
        assert_eq!(select("dell u2720q"), Ok(0));
        assert_eq!(select("(2)"), Ok(1));
    }

    #[test]
    fn duplicate_exact_names_are_ambiguous_and_need_an_index() {
        let monitors = names(&[
            Some("Built-in Retina Display"),
            Some("DELL U2720Q"),
            Some("dell u2720q"),
        ]);
        let by_name = |query: &str| select_monitor(&monitors, &DeviceSelector::Name(query.into()));
        let error = by_name("DELL U2720Q").unwrap_err();
        assert!(error.contains("ambiguous"), "{error}");
        assert!(error.contains("[1, 2]"), "{error}");
        assert!(error.contains("--monitor INDEX"), "{error}");
        assert_eq!(by_name("retina"), Ok(0));
        let by_index = |index: usize| select_monitor(&monitors, &DeviceSelector::Index(index));
        assert_eq!(by_index(2), Ok(2));
        assert!(by_index(3).unwrap_err().contains("out of range"));
        assert_eq!(DeviceSelector::parse("2"), DeviceSelector::Index(2));
    }

    #[test]
    fn f_toggles_fullscreen_with_supported_modifiers() {
        let plain = KeyModifiers::default();
        let cmd = KeyModifiers {
            command: true,
            ..plain
        };
        let ctrl_cmd = KeyModifiers {
            command: true,
            control: true,
            ..plain
        };
        let ctrl = KeyModifiers {
            control: true,
            ..plain
        };
        let alt = KeyModifiers { alt: true, ..plain };
        let f = KeyInput::Character('f');
        for fullscreen in [false, true] {
            assert_eq!(
                key_action(f, plain, fullscreen),
                Some(KeyAction::ToggleFullscreen)
            );
            assert_eq!(
                key_action(f, cmd, fullscreen),
                Some(KeyAction::ToggleFullscreen)
            );
            assert_eq!(
                key_action(f, ctrl_cmd, fullscreen),
                Some(KeyAction::ToggleFullscreen)
            );
            assert_eq!(key_action(f, ctrl, fullscreen), None);
            assert_eq!(key_action(f, alt, fullscreen), None);
        }
    }

    #[test]
    fn escape_only_exits_fullscreen_and_never_quits() {
        let plain = KeyModifiers::default();
        assert_eq!(
            key_action(KeyInput::Escape, plain, true),
            Some(KeyAction::ExitFullscreen)
        );
        assert_eq!(key_action(KeyInput::Escape, plain, false), None);
    }

    #[test]
    fn q_and_other_keys_do_nothing() {
        let plain = KeyModifiers::default();
        let cmd = KeyModifiers {
            command: true,
            ..plain
        };
        let q = KeyInput::Character('q');
        for fullscreen in [false, true] {
            // Plain Q must not quit; Cmd+Q is left to winit's macOS app menu.
            assert_eq!(key_action(q, plain, fullscreen), None);
            assert_eq!(key_action(q, cmd, fullscreen), None);
            assert_eq!(
                key_action(KeyInput::Character('x'), plain, fullscreen),
                None
            );
            assert_eq!(key_action(KeyInput::Other, plain, fullscreen), None);
        }
    }

    #[test]
    fn winit_keys_map_to_inputs() {
        let unidentified = PhysicalKey::Code(KeyCode::KeyA);
        assert_eq!(
            KeyInput::from_winit(
                &Key::Character("F".into()),
                PhysicalKey::Code(KeyCode::KeyF)
            ),
            KeyInput::Character('f')
        );
        assert_eq!(
            KeyInput::from_winit(&Key::Named(NamedKey::Escape), unidentified),
            KeyInput::Escape
        );
        assert_eq!(
            KeyInput::from_winit(&Key::Named(NamedKey::Enter), unidentified),
            KeyInput::Other
        );
    }

    #[test]
    fn non_latin_layouts_fall_back_to_physical_keys() {
        // Cyrillic layout: the physical F key produces 'а'.
        assert_eq!(
            KeyInput::from_winit(
                &Key::Character("а".into()),
                PhysicalKey::Code(KeyCode::KeyF)
            ),
            KeyInput::Character('f')
        );
        assert_eq!(
            KeyInput::from_winit(
                &Key::Character("ф".into()),
                PhysicalKey::Code(KeyCode::KeyA)
            ),
            KeyInput::Other
        );
        assert_eq!(
            KeyInput::from_winit(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Escape)
            ),
            KeyInput::Escape
        );
    }

    #[test]
    fn latin_layouts_use_logical_key_not_position() {
        // Dvorak: the physical F position types 'u', which must not toggle.
        assert_eq!(
            KeyInput::from_winit(
                &Key::Character("u".into()),
                PhysicalKey::Code(KeyCode::KeyF)
            ),
            KeyInput::Character('u')
        );
    }

    #[test]
    fn leading_separator_is_ignored() {
        let args = parse_run(&["--", "--monitor", "Studio Display"]);
        assert_eq!(
            args.monitor,
            Some(DeviceSelector::Name("Studio Display".to_string()))
        );
        assert!(parse_run(&["--", "--list-monitors"]).list_monitors);
        assert!(parse(&["--fullscreen", "--"]).is_err());
    }

    #[test]
    fn parses_mic_options() {
        let args = parse_run(&["--mic", "--audio-device", "2"]);
        assert!(args.mic);
        assert_eq!(args.audio_device, Some(DeviceSelector::Index(2)));
        let args = parse_run(&["--mic", "--audio-device", "MacBook Pro Microphone"]);
        assert_eq!(
            args.audio_device,
            Some(DeviceSelector::Name("MacBook Pro Microphone".to_string()))
        );
        assert!(parse_run(&["--list-audio-devices"]).list_audio_devices);
        assert!(!parse_run(&[]).mic);
        let message = parse(&["--audio-device", "1"]).unwrap_err().to_string();
        assert!(
            message.contains("--audio-device requires --mic"),
            "{message}"
        );
        let message = parse(&["--mic", "--audio-device"]).unwrap_err().to_string();
        assert!(message.contains("--audio-device requires"), "{message}");
    }

    #[test]
    fn selects_audio_devices_like_monitors() {
        let devices = names(&[
            Some("MacBook Pro Microphone"),
            Some("USB Audio Device"),
            Some("USB Audio Device (2)"),
        ]);
        let select = |selector: DeviceSelector| select_device(&devices, &selector, &AUDIO_INPUT);
        assert_eq!(select(DeviceSelector::parse("macbook")), Ok(0));
        assert_eq!(select(DeviceSelector::parse("usb audio device")), Ok(1));
        assert_eq!(select(DeviceSelector::parse("2")), Ok(2));
        let error = select(DeviceSelector::parse("usb")).unwrap_err();
        assert!(error.contains("audio input device name"), "{error}");
        assert!(error.contains("--audio-device INDEX"), "{error}");
        let error = select(DeviceSelector::parse("5")).unwrap_err();
        assert!(error.contains("--list-audio-devices"), "{error}");
        let error = select(DeviceSelector::parse("webcam")).unwrap_err();
        assert!(error.contains("no audio input device name"), "{error}");
    }

    struct FakeMic(std::rc::Rc<std::cell::Cell<AudioFeatures>>);

    impl LatestFeatures for FakeMic {
        fn latest(&self) -> AudioFeatures {
            self.0.get()
        }
    }

    /// A valid wire packet with optional single `(note, velocity)`.
    fn wire_packet(energy: u8, flags: u8, note: Option<(u8, u8)>) -> [u8; WIRE_SIZE] {
        let mut packet = [0; WIRE_SIZE];
        packet[0] = crate::control_frame::MAGIC;
        packet[8] = energy;
        packet[22] = flags;
        if let Some((note, velocity)) = note {
            packet[23] = 1;
            packet[24] = note;
            packet[25] = velocity;
        }
        packet[WIRE_SIZE - 1] = packet[..WIRE_SIZE - 1].iter().fold(0, |a, b| a ^ b);
        packet
    }

    #[test]
    fn mic_beat_flag_resets_after_a_udp_packet() {
        let udp = UdpFrameSource::bind(Some("127.0.0.1:0")).unwrap();
        let address = udp.socket.as_ref().unwrap().local_addr().unwrap();
        let feed = std::rc::Rc::new(std::cell::Cell::new(AudioFeatures {
            beat_count: 5,
            ..AudioFeatures::default()
        }));
        let mut source = FrameSource::with_feed(udp, Some(Box::new(FakeMic(feed.clone()))));
        let set_count = |count: u16| {
            feed.set(AudioFeatures {
                beat_count: count,
                ..feed.get()
            })
        };
        let start = Instant::now();

        // Mic frames: no beat on the first frame, a beat when the count moves.
        assert!(!source.next_frame_at(start).beat_onset());
        set_count(6);
        let frame = source.next_frame_at(start);
        assert!(frame.beat_onset());
        assert_eq!(frame.beat_count, 6);
        assert!(source.showing_mic());

        // A UDP packet wins and is held while recent, even as the mic moves on.
        let onsets = audio::BEAT_FLAG | audio::BAR_FLAG | audio::DROP_FLAG;
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender
            .send_to(&wire_packet(77, onsets, Some((60, 100))), address)
            .unwrap();
        let mut udp_frame = None;
        for _ in 0..200 {
            let frame = source.next_frame_at(start);
            if frame.energy == 77 {
                udp_frame = Some(frame);
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let udp_frame = udp_frame.expect("UDP packet never arrived");
        assert_eq!(udp_frame.flags, onsets);
        assert_eq!(udp_frame.note_count, 1);
        assert_eq!(
            (udp_frame.notes[0].note, udp_frame.notes[0].velocity),
            (60, 100)
        );
        set_count(9);

        // Held redraws keep the packet's fields and sustained notes but clear
        // its one-off events.
        for elapsed in [
            Duration::ZERO,
            UDP_HOLD / 2,
            UDP_HOLD - Duration::from_millis(1),
        ] {
            let held = source.next_frame_at(start + elapsed);
            assert_eq!(
                (held.energy, held.flags, held.note_count),
                (77, 0, 1),
                "{held:?}"
            );
            assert_eq!((held.notes[0].note, held.notes[0].velocity), (60, 100));
            // The mic layers fall back while UDP frames are shown.
            assert!(!source.showing_mic());
        }

        // Back on the mic: the count jumped while UDP held, but that is not a beat.
        let later = start + UDP_HOLD * 2;
        let frame = source.next_frame_at(later);
        assert_eq!(frame.beat_count, 9);
        assert!(!frame.beat_onset(), "{frame:?}");
        assert!(source.showing_mic());
        set_count(10);
        assert!(source.next_frame_at(later).beat_onset());
    }

    /// Held redraws in mic mode clear one-off flags. The colour field must
    /// not see that as a falling edge (which would turn a sustained beat flag
    /// in the next packet into a second onset), and a held redraw is a
    /// display repeat, so it must not step.
    #[test]
    fn held_udp_redraws_neither_step_nor_retrigger_the_colour_field() {
        use crate::scenes::colour_field::{ColourField, Config, Controls};

        let udp = UdpFrameSource::bind(Some("127.0.0.1:0")).unwrap();
        let address = udp.socket.as_ref().unwrap().local_addr().unwrap();
        let feed = std::rc::Rc::new(std::cell::Cell::new(AudioFeatures::default()));
        let mut source = FrameSource::with_feed(udp, Some(Box::new(FakeMic(feed))));
        let mut field = ColourField::new(Config::default());
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let start = Instant::now();
        // One window redraw, as in `run`: returns (new frames, beat edge
        // pending, stepped).
        fn redraw(
            source: &mut FrameSource,
            field: &mut ColourField,
            now: Instant,
        ) -> (usize, bool, bool) {
            source.next_frame_at(now);
            for fresh in source.fresh() {
                field.observe(fresh);
            }
            let beat = field.pending_beat_bar();
            let stepped = field.has_new_frame();
            if stepped {
                field.step(Controls::default());
            }
            (source.fresh().len(), beat, stepped)
        }
        // Packet counters stay clear of the mic frames' small counters.
        let send = |source: &mut FrameSource, field: &mut ColourField, counter: u16, flags: u8| {
            let mut packet = wire_packet(90, flags, Some((60, 100)));
            packet[2..4].copy_from_slice(&counter.to_le_bytes());
            packet[WIRE_SIZE - 1] = packet[..WIRE_SIZE - 1].iter().fold(0, |a, b| a ^ b);
            sender.send_to(&packet, address).unwrap();
            // Until the first packet lands, redraws in mic mode generate mic
            // frames; wait for this packet itself.
            // About 2 s, for loaded CI hosts.
            for _ in 0..1000 {
                let result = redraw(source, field, start);
                if source.fresh().iter().any(|frame| frame.frame == counter) {
                    return result;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            panic!("UDP packet {counter} never arrived");
        };

        // Mic frames before any packet are new frames too.
        for _ in 0..3 {
            assert_eq!(redraw(&mut source, &mut field, start), (1, false, true));
        }
        assert_eq!(send(&mut source, &mut field, 1001, 0), (1, false, true));
        assert_eq!(
            send(&mut source, &mut field, 1002, audio::BEAT_FLAG),
            (1, true, true)
        );
        // Held redraws (flags cleared for display) add no frames and no steps.
        for _ in 0..5 {
            let held = source.next_frame_at(start);
            assert_eq!(held.flags, 0);
            assert!(source.fresh().is_empty());
            assert_eq!(redraw(&mut source, &mut field, start), (0, false, false));
        }
        // The beat flag is still high in the next packet: no second onset.
        assert_eq!(
            send(&mut source, &mut field, 1003, audio::BEAT_FLAG),
            (1, false, true)
        );
        assert_eq!(send(&mut source, &mut field, 1004, 0), (1, false, true));
        assert_eq!(
            send(&mut source, &mut field, 1005, audio::BEAT_FLAG),
            (1, true, true)
        );
        // After the hold, mic frames are new frames and step again.
        let (fresh, _, stepped) = redraw(&mut source, &mut field, start + UDP_HOLD * 2);
        assert_eq!((fresh, stepped), (1, true));
    }

    /// UDP-only (no mic): a paused sender's last packet is held indefinitely.
    /// Its one-off beat/bar/drop flags show on the first draw only; levels,
    /// notes, scene, palette, and the fill/manual flags stay held.
    #[test]
    fn udp_only_held_redraws_clear_one_off_flags() {
        let udp = UdpFrameSource::bind(Some("127.0.0.1:0")).unwrap();
        let address = udp.socket.as_ref().unwrap().local_addr().unwrap();
        let mut source = FrameSource::with_feed(udp, None);
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let start = Instant::now();
        let onsets = audio::BEAT_FLAG | audio::BAR_FLAG | audio::DROP_FLAG;
        let sustained = 0x08 | 0x10;
        let send = |source: &mut FrameSource, counter: u16| {
            let mut packet = wire_packet(77, onsets | sustained, Some((60, 100)));
            packet[2..4].copy_from_slice(&counter.to_le_bytes());
            packet[20] = 2;
            packet[21] = 5;
            packet[WIRE_SIZE - 1] = packet[..WIRE_SIZE - 1].iter().fold(0, |a, b| a ^ b);
            sender.send_to(&packet, address).unwrap();
            // About 2 s, for loaded CI hosts.
            for _ in 0..1000 {
                let frame = source.next_frame_at(start);
                if frame.frame == counter && !source.fresh().is_empty() {
                    return frame;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            panic!("UDP packet {counter} never arrived");
        };

        for counter in [1001, 1002] {
            // The first draw of each packet carries its events.
            let first = send(&mut source, counter);
            assert_eq!(first.flags, onsets | sustained, "{first:?}");
            // Held redraws of a paused sender, long past any hold window.
            for elapsed in [
                Duration::ZERO,
                UDP_HOLD,
                UDP_HOLD * 2,
                Duration::from_secs(60),
            ] {
                let held = source.next_frame_at(start + elapsed);
                assert!(source.fresh().is_empty());
                assert_eq!(held.frame, counter);
                assert_eq!(held.flags, sustained, "{held:?}");
                assert!(!held.beat_onset());
                assert_eq!(
                    (held.energy, held.scene, held.palette, held.note_count),
                    (77, 2, 5, 1),
                    "{held:?}"
                );
                assert_eq!((held.notes[0].note, held.notes[0].velocity), (60, 100));
            }
        }
    }

    #[test]
    fn udp_packets_win_over_mic_and_procedural_is_unchanged() {
        let recent = Some(Duration::from_millis(100));
        let stale = Some(UDP_HOLD);
        // Without --mic: procedural without --udp, hold the last packet with it.
        assert_eq!(idle_source(false, false, None), IdleSource::Procedural);
        assert_eq!(idle_source(false, true, None), IdleSource::Hold);
        assert_eq!(idle_source(false, true, stale), IdleSource::Hold);
        // With --mic: mic replaces procedural; recent UDP holds priority.
        assert_eq!(idle_source(true, false, None), IdleSource::Mic);
        assert_eq!(idle_source(true, true, None), IdleSource::Mic);
        assert_eq!(idle_source(true, true, recent), IdleSource::Hold);
        assert_eq!(idle_source(true, true, stale), IdleSource::Mic);
    }

    #[test]
    fn parses_colour_field_options() {
        let args = parse_run(&["--layout", "H3", "--allow-flash"]);
        assert_eq!(args.layout, Some(3));
        assert!(args.allow_flash);
        assert_eq!(parse_run(&["--layout", "auto"]).layout, None);
        assert_eq!(parse_run(&["--layout", "v2"]).layout, Some(0));
        assert!(!AppArgs::default().allow_flash);
        assert!(parse(&["--layout", "v4"]).is_err());
        assert!(parse(&["--layout"]).is_err());
    }

    #[test]
    fn lines_drop_cuts_are_opt_in() {
        assert!(!AppArgs::default().lines_drop_cuts);
        assert!(parse_run(&["--lines-drop-cuts"]).lines_drop_cuts);
    }

    #[test]
    fn colour_field_config_arms_locally_only_with_the_flag() {
        let config = colour_field_config(&AppArgs::default());
        assert!(!config.allow_flash);
        assert_eq!(
            config.palette,
            colour_field::PaletteSource::Local { p: 0, armed: false }
        );
        let udp = colour_field_config(&parse_run(&["--udp", "127.0.0.1:6464"]));
        assert_eq!(udp.palette, colour_field::PaletteSource::Wire);
        assert!(!udp.allow_flash);
        assert!(colour_field_config(&parse_run(&["--allow-flash"])).allow_flash);
    }

    #[test]
    fn l_cycles_the_layout_pin() {
        let plain = KeyModifiers::default();
        let cmd = KeyModifiers {
            command: true,
            ..plain
        };
        let l = KeyInput::Character('l');
        assert_eq!(key_action(l, plain, false), Some(KeyAction::CycleLayoutPin));
        assert_eq!(key_action(l, cmd, true), None);
        assert_eq!(
            KeyInput::from_winit(
                &Key::Character("д".into()),
                PhysicalKey::Code(KeyCode::KeyL)
            ),
            l
        );
    }

    #[test]
    fn mic_layers_option_and_m_key() {
        assert_eq!(parse_run(&[]).mic_layers, mic_layers::Mode::Off);
        let args = parse_run(&["--mic-layers", "Both"]);
        assert_eq!(args.mic_layers, mic_layers::Mode::Both);
        assert!(parse(&["--mic-layers", "loud"]).is_err());
        assert!(parse(&["--mic-layers"]).is_err());
        let plain = KeyModifiers::default();
        let m = KeyInput::Character('m');
        assert_eq!(key_action(m, plain, true), Some(KeyAction::CycleMicLayers));
        let cmd = KeyModifiers {
            command: true,
            ..plain
        };
        assert_eq!(key_action(m, cmd, false), None);
        assert_eq!(
            KeyInput::from_winit(
                &Key::Character("ь".into()),
                PhysicalKey::Code(KeyCode::KeyM)
            ),
            m
        );
    }

    #[test]
    fn cursor_hidden_only_in_fullscreen() {
        assert!(!cursor_visible(true));
        assert!(cursor_visible(false));
    }

    #[test]
    fn usage_documents_new_options() {
        let text = usage("chipviz-m1");
        for needle in [
            "--fullscreen",
            "--monitor",
            "--list-monitors",
            "--mic",
            "--audio-device",
            "--list-audio-devices",
            "--layout auto|v2|h2|v3|h3",
            "--allow-flash",
            "--mic-layers off|scope|bursts|both",
            "--lines-drop-cuts",
            "--star-catalog PATH",
            "--catalog FILE",
            "--location LAT,LON",
            "--time UNIX_SECONDS",
            "--scene N",
            "Scene 6 requires both --star-catalog and --location",
            "without this option, incoming and procedural scenes are unchanged",
            "cycle the colour-field layout pin",
            "cycle the mic layers",
            "Esc",
            "Cmd+Q",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
    }
}
