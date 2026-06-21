use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Instant;

use winit::dpi::LogicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::window::WindowBuilder;

use crate::control_frame::{ControlFrame, WIRE_SIZE};
use crate::profile::RenderProfile;
use crate::renderer::{Renderer, RendererError};

#[derive(Debug)]
pub enum AppError {
    Window(winit::error::OsError),
    EventLoop(winit::error::EventLoopError),
    Io(std::io::Error),
    Renderer(RendererError),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(error) => write!(formatter, "window error: {error}"),
            Self::EventLoop(error) => write!(formatter, "event loop error: {error}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Renderer(error) => write!(formatter, "renderer error: {error}"),
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
    let args = AppArgs::parse()?;
    let udp = UdpFrameSource::bind(args.udp_endpoint.as_deref())?;

    let event_loop = EventLoop::new()?;
    let window = Arc::new(
        WindowBuilder::new()
            .with_title(profile.window_title)
            .with_inner_size(LogicalSize::new(
                profile.target_width,
                profile.target_height,
            ))
            .build(&event_loop)?,
    );

    let mut renderer = pollster::block_on(Renderer::new(window.clone(), profile))?;
    let mut frame_source = FrameSource::new(udp);
    let start = Instant::now();

    event_loop.run(move |event, target| match event {
        Event::WindowEvent { event, window_id } if window_id == window.id() => match event {
            WindowEvent::CloseRequested => target.exit(),
            WindowEvent::Resized(size) => renderer.resize(size.width, size.height),
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

struct AppArgs {
    udp_endpoint: Option<String>,
}

impl AppArgs {
    fn parse() -> Result<Self, AppError> {
        let mut udp_endpoint = None;
        let mut args = std::env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--udp" => {
                    let Some(endpoint) = args.next() else {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "--udp requires LISTEN_ADDR:PORT",
                        )
                        .into());
                    };
                    udp_endpoint = Some(endpoint);
                }
                "--help" | "-h" => {
                    println!(
                        "usage: {} [--udp LISTEN_ADDR:PORT]\n\nWithout --udp, deterministic procedural frames drive the renderer.",
                        std::env::args()
                            .next()
                            .unwrap_or_else(|| "chipviz".to_string())
                    );
                    std::process::exit(0);
                }
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("unknown argument: {arg}"),
                    )
                    .into());
                }
            }
        }

        Ok(Self { udp_endpoint })
    }
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
