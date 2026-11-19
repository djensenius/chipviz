use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Instant;

use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowBuilder};

use crate::control_frame::{ControlFrame, WIRE_SIZE};
use crate::profile::RenderProfile;
use crate::renderer::{Renderer, RendererError};

#[derive(Debug)]
pub enum AppError {
    Window(winit::error::OsError),
    EventLoop(winit::error::EventLoopError),
    Io(std::io::Error),
    Renderer(RendererError),
    Monitor(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(error) => write!(formatter, "window error: {error}"),
            Self::EventLoop(error) => write!(formatter, "event loop error: {error}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Renderer(error) => write!(formatter, "renderer error: {error}"),
            Self::Monitor(message) => write!(formatter, "monitor error: {message}"),
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

    let event_loop = EventLoop::new()?;

    if args.list_monitors {
        let monitors: Vec<MonitorHandle> = event_loop.available_monitors().collect();
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

    let udp = UdpFrameSource::bind(args.udp_endpoint.as_deref())?;

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
    let mut frame_source = FrameSource::new(udp);
    let mut modifiers = ModifiersState::empty();
    let start = Instant::now();

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
                    None => {}
                }
            }
            WindowEvent::RedrawRequested => {
                let elapsed = start.elapsed().as_secs_f32();
                let frame = frame_source.next_frame();
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
        "usage: {program} [--udp LISTEN_ADDR:PORT] [--fullscreen] [--monitor INDEX|NAME] [--list-monitors]

Options:
  --udp LISTEN_ADDR:PORT  consume control-frame-v0 packets (default: procedural frames)
  --fullscreen            start in borderless fullscreen (default: windowed)
  --monitor INDEX|NAME    display to use, by index or case-insensitive name substring
  --list-monitors         print available monitors and exit
  -h, --help              print this help

Keys:
  F, Cmd+F, Ctrl+Cmd+F    toggle fullscreen
  Esc                     exit fullscreen
  Cmd+Q                   quit (macOS application menu)"
    )
}

/// Pure cursor policy: the cursor is hidden only while fullscreen.
fn cursor_visible(fullscreen: bool) -> bool {
    !fullscreen
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MonitorSelector {
    Index(usize),
    Name(String),
}

impl MonitorSelector {
    fn parse(value: &str) -> Self {
        match value.parse::<usize>() {
            Ok(index) => Self::Index(index),
            Err(_) => Self::Name(value.to_string()),
        }
    }
}

/// Resolves a selector against monitor names (in enumeration order).
/// All-digit selectors are indices (as printed by `--list-monitors`). Names
/// match as a case-insensitive substring; a *unique* exact (case-insensitive)
/// name wins when several monitors match. Duplicate names (e.g. two identical
/// displays) are ambiguous and must be selected by index.
fn select_monitor(names: &[Option<String>], selector: &MonitorSelector) -> Result<usize, String> {
    match selector {
        MonitorSelector::Index(index) if *index < names.len() => Ok(*index),
        MonitorSelector::Index(index) => Err(format!(
            "monitor index {index} out of range ({} monitor(s) available; see --list-monitors)",
            names.len()
        )),
        MonitorSelector::Name(query) => {
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
                    "no monitor name contains \"{query}\" (see --list-monitors)"
                )),
                _ => Err(format!(
                    "monitor name \"{query}\" is ambiguous: matches indices {matches:?}; select one with --monitor INDEX (see --list-monitors)"
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
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ArgsCommand {
    Run(AppArgs),
    Help,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct AppArgs {
    udp_endpoint: Option<String>,
    fullscreen: bool,
    monitor: Option<MonitorSelector>,
    list_monitors: bool,
}

impl AppArgs {
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
                    parsed.monitor = Some(MonitorSelector::parse(&value));
                }
                "--list-monitors" => parsed.list_monitors = true,
                // `cargo run -- ...` / `just recipe -- ...` may forward a separator.
                "--" if is_first => {}
                "--help" | "-h" => return Ok(ArgsCommand::Help),
                _ => return Err(invalid_input(format!("unknown argument: {arg}"))),
            }
        }

        Ok(ArgsCommand::Run(parsed))
    }
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

    fn receive_latest(&self) -> Option<ControlFrame> {
        let socket = self.socket.as_ref()?;
        let mut latest = None;
        let mut packet = [0; WIRE_SIZE];

        loop {
            match socket.recv(&mut packet) {
                Ok(WIRE_SIZE) => match ControlFrame::parse_wire(&packet) {
                    Ok(frame) => latest = Some(frame),
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

        latest
    }
}

struct FrameSource {
    udp: UdpFrameSource,
    fallback_index: u64,
    latest: ControlFrame,
}

impl FrameSource {
    fn new(udp: UdpFrameSource) -> Self {
        Self {
            udp,
            fallback_index: 0,
            latest: ControlFrame::procedural(0),
        }
    }

    fn next_frame(&mut self) -> ControlFrame {
        if let Some(frame) = self.udp.receive_latest() {
            self.latest = frame;
            return frame;
        }

        if self.udp.socket.is_none() {
            self.fallback_index = self.fallback_index.wrapping_add(1);
            self.latest = ControlFrame::procedural(self.fallback_index);
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
    }

    #[test]
    fn parses_fullscreen_monitor_and_udp() {
        let args = parse_run(&["--fullscreen", "--monitor", "1", "--udp", "127.0.0.1:6464"]);
        assert!(args.fullscreen);
        assert_eq!(args.monitor, Some(MonitorSelector::Index(1)));
        assert_eq!(args.udp_endpoint.as_deref(), Some("127.0.0.1:6464"));
        assert!(!args.list_monitors);
    }

    #[test]
    fn non_numeric_monitor_is_a_name_selector() {
        let args = parse_run(&["--monitor", "Studio Display"]);
        assert_eq!(
            args.monitor,
            Some(MonitorSelector::Name("Studio Display".to_string()))
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
        assert_eq!(select_monitor(&monitors, &MonitorSelector::Index(1)), Ok(1));
        let error = select_monitor(&monitors, &MonitorSelector::Index(2)).unwrap_err();
        assert!(error.contains("out of range"), "{error}");
    }

    #[test]
    fn selects_monitor_by_case_insensitive_substring() {
        let monitors = names(&[Some("Built-in Retina Display"), None, Some("LG UltraFine")]);
        let select = |query: &str| select_monitor(&monitors, &MonitorSelector::Name(query.into()));
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
        let select = |query: &str| select_monitor(&monitors, &MonitorSelector::Name(query.into()));
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
        let by_name = |query: &str| select_monitor(&monitors, &MonitorSelector::Name(query.into()));
        let error = by_name("DELL U2720Q").unwrap_err();
        assert!(error.contains("ambiguous"), "{error}");
        assert!(error.contains("[1, 2]"), "{error}");
        assert!(error.contains("--monitor INDEX"), "{error}");
        assert_eq!(by_name("retina"), Ok(0));
        let by_index = |index: usize| select_monitor(&monitors, &MonitorSelector::Index(index));
        assert_eq!(by_index(2), Ok(2));
        assert!(by_index(3).unwrap_err().contains("out of range"));
        assert_eq!(MonitorSelector::parse("2"), MonitorSelector::Index(2));
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
            Some(MonitorSelector::Name("Studio Display".to_string()))
        );
        assert!(parse_run(&["--", "--list-monitors"]).list_monitors);
        assert!(parse(&["--fullscreen", "--"]).is_err());
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
            "Esc",
            "Cmd+Q",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
    }
}
