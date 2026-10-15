//! ytb2pdf-core: the engine behind the YouTube -> PDF desktop app.
//!
//! The pipeline is split into small, UI-independent stages (FilmCraft-style
//! layering: nothing here knows about egui or the CLI):
//!
//! ```text
//! YouTube URL -> metadata -> subtitles -> (translate) -> video download
//!           -> frame capture -> perceptual dedupe -> PDF assembly
//! ```
//!
//! External tools are shelled out to, never linked:
//! - `yt-dlp` for metadata / subtitles / video download
//! - `ffmpeg` for frame extraction

/// Display version supplied by release builds; local builds use Cargo's version.
pub const VERSION: &str = match option_env!("YTB2PDF_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// Source commit embedded by the release workflow.
pub const BUILD_COMMIT: &str = match option_env!("YTB2PDF_COMMIT") {
    Some(commit) => commit,
    None => "local",
};

pub mod local_video;
pub mod frames;
pub mod pdf;
pub mod pipeline;
pub mod resume;
pub mod subtitles;
pub mod translate;
pub mod v2;
pub mod whisper;
pub mod youtube;

pub use pipeline::{run_pipeline, PipelineConfig, PipelineEvent};
pub use local_video::VideoInput;
pub use subtitles::Segment;
