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

pub mod frames;
pub mod pdf;
pub mod pipeline;
pub mod subtitles;
pub mod translate;
pub mod youtube;

pub use pipeline::{run_pipeline, PipelineConfig, PipelineEvent};
pub use subtitles::Segment;
