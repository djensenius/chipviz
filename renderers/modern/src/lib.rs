mod app;
mod audio;
pub mod capture;
pub mod catalog_v1;
mod control_frame;
#[cfg(feature = "dmg-preview")]
mod dmg_preview;
mod phosphor;
mod profile;
mod renderer;
mod scene;
mod scenes;

pub use app::{AppError, run};
pub use profile::RenderProfile;
