//! Local media adapter for the video-to-PDF pipelines.
//!
//! Local video files stay on disk. FFprobe reads metadata, FFmpeg extracts
//! frames, and Whisper runs locally if there is no usable subtitle file.

use crate::frames::{capture_frames, dedupe_frames};
use crate::pdf::{self, PdfMeta};
use crate::pipeline::{self, PipelineConfig, PipelineEvent};
use crate::subtitles::{self, Segment};
use crate::v2::scene;
use crate::whisper;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoInput {
    YouTubeUrl(String),
    LocalFile(PathBuf),
}

impl VideoInput {
    /// Keep existing positional YouTube URLs valid; all other inputs are
    /// interpreted as local paths, except unsupported network URL schemes.
    pub fn parse_cli(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            bail!("A YouTube URL or local video path is required");
        }
        if raw.starts_with("https://") || raw.starts_with("http://") {
            crate::youtube::parse_youtube_url(raw)?;
            Ok(Self::YouTubeUrl(raw.to_owned()))
        } else if raw.contains("://") {
            bail!("Unsupported URL scheme. Use a YouTube URL or a local file path");
        } else {
            Ok(Self::LocalFile(PathBuf::from(raw)))
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::YouTubeUrl(url) => { crate::youtube::parse_youtube_url(url)?; }
            Self::LocalFile(path) => {
                if !path.is_file() {
                    bail!("Video file not found or not a regular file: {}", path.display());
                }
            }
        }
        Ok(())
    }

    pub fn filename_stem(&self) -> String {
        match self {
            Self::YouTubeUrl(url) => crate::youtube::parse_youtube_url(url)
                .map(|v| v.id).unwrap_or_else(|_| "video".to_owned()),
            Self::LocalFile(path) => path.file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("video")
                .to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalVideoMetadata {
    pub title: String,
    pub duration_secs: u64,
    pub has_audio: bool,
}

/// Probe the actual streams, not just the extension, before attempting work.
pub fn probe_metadata(path: &Path) -> Result<LocalVideoMetadata> {
    if !path.is_file() {
        bail!("Video file not found or not a regular file: {}", path.display());
    }
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,duration",
               "-of", "json"])
        .arg(path)
        .output()
        .context("ffprobe is required to read local video metadata")?;
    if !output.status.success() {
        bail!("Cannot read local video {}: {}",
              path.display(), String::from_utf8_lossy(&output.stderr).trim());
    }
    let info: Value = serde_json::from_slice(&output.stdout)
        .context("Invalid ffprobe JSON for local video")?;
    let streams = info["streams"].as_array()
        .ok_or_else(|| anyhow!("No decodable media streams found in {}", path.display()))?;
    if !streams.iter().any(|s| s["codec_type"] == "video") {
        bail!("The selected file has no video stream: {}", path.display());
    }
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
    let duration_text = info["format"]["duration"].as_str()
        .and_then(|v| v.parse::<f64>().ok());
    let stream_duration = streams.iter()
        .filter_map(|s| s["duration"].as_str().and_then(|v| v.parse::<f64>().ok()))
        .filter(|d| d.is_finite() && *d > 0.0)
        .fold(0.0f64, f64::max);
    let duration = duration_text.unwrap_or(stream_duration);
    if !duration.is_finite() || duration <= 0.0 {
        bail!("Cannot determine a positive video duration: {}", path.display());
    }
    let title = path.file_stem().and_then(|s| s.to_str())
        .filter(|s| !s.is_empty()).unwrap_or("Local Video").to_owned();
    Ok(LocalVideoMetadata {
        title,
        duration_secs: duration.ceil() as u64,
        has_audio,
    })
}

/// Sidecar subtitles are optional. An explicit transcript always wins.
pub fn find_sidecar(path: &Path) -> Option<PathBuf> {
    ["srt", "vtt"].into_iter()
        .map(|ext| path.with_extension(ext))
        .find(|candidate| candidate.is_file())
}

fn get_captions(
    path: &Path, meta: &LocalVideoMetadata, cfg: &PipelineConfig,
    emit: &(dyn Fn(PipelineEvent) + Sync),
) -> Result<Vec<Segment>> {
    if let Some(transcript) = cfg.transcript_file.clone().or_else(|| find_sidecar(path))
    {
        emit(PipelineEvent::Stage("Reading local subtitles"));
        emit(PipelineEvent::Log(format!("Using subtitle file: {}", transcript.display())));
        return subtitles::load_subtitles_file(&transcript);
    }
    if !meta.has_audio {
        bail!("No transcript or audio stream was found. Supply --transcript <file.srt> or choose a video with audio");
    }
    emit(PipelineEvent::Stage("Transcribing local audio (Whisper)"));
    emit(PipelineEvent::Log(
        "No subtitle file found. Running local Whisper transcription".to_owned()
    ));
    // Whisper's ffmpeg preprocessing can decode video input directly;
    // no intermediate full audio download/copy is required.
    whisper::transcribe_audio(
        path, &cfg.work_dir, cfg.whisper_model, &cfg.source_lang,
        &|message| emit(PipelineEvent::Log(message)),
    )
}

/// Generate a low-resolution analysis proxy for local V2; the original
/// file remains untouched and supplies the full-quality frames.
fn make_local_proxy(source: &Path, out: &Path) -> Result<()> {
    if out.is_file() { return Ok(()); }
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(source)
        .args(["-map", "0:v:0", "-vf", "scale=-2:240",
               "-an", "-c:v", "libx264", "-preset", "ultrafast",
               "-crf", "33", "-pix_fmt", "yuv420p"])
        .arg(out)
        .status()
        .context("Failed to run FFmpeg for local V2 proxy")?;
    if !status.success() || !out.is_file() {
        bail!("FFmpeg could not generate a low-resolution proxy for {}", source.display());
    }
    Ok(())
}

/// The local adapter uses the same caption translation, frame compaction
/// and PDF renderer as YouTube V1; it never calls yt-dlp.
pub fn run_local_pipeline(
    cfg: &PipelineConfig, path: &Path, v2: bool,
    emit: &(dyn Fn(PipelineEvent) + Sync),
) -> Result<PathBuf> {
    emit(PipelineEvent::Stage("Opening local video"));
    let meta = probe_metadata(path)?;
    emit(PipelineEvent::Log(format!(
        "Local video: {} ({})", meta.title, pdf::format_duration(meta.duration_secs)
    )));
    std::fs::create_dir_all(&cfg.work_dir).context("Creating local work directory")?;
    let frame_dir = cfg.work_dir.join(if v2 { "local-v2-frames" } else { "local-frames" });
    std::fs::create_dir_all(&frame_dir)?;
    let captions = get_captions(path, &meta, cfg, emit)?;
    emit(PipelineEvent::Log(format!("{} caption segments", captions.len())));
    let translations = pipeline::translate_segments(cfg, &captions, emit)?;

    let timestamps: Vec<f64> = if v2 {
        emit(PipelineEvent::Stage("Preparing local scene-analysis proxy"));
        let proxy = cfg.work_dir.join("local-v2-proxy.mp4");
        make_local_proxy(path, &proxy)?;
        emit(PipelineEvent::Stage("Detecting scenes in local video"));
        let scenes = scene::detect_scenes(&proxy, 0.30, 30.0)?;
        if scenes.is_empty() {
            bail!("No scenes detected in local V2. Retry without --v2");
        }
        scenes.into_iter().map(|s| s.time).collect()
    } else {
        emit(PipelineEvent::Stage("Extracting local video frames"));
        captions.iter().map(|s| s.midpoint()).collect()
    };
    let total = timestamps.len();
    let frames = capture_frames(path, &timestamps, &frame_dir, cfg.jobs, &|done, _| {
        emit(PipelineEvent::Progress { done, total });
    })?;
    emit(PipelineEvent::Stage("Removing duplicate frames"));
    let frames = dedupe_frames(frames, cfg.dhash_threshold);
    emit(PipelineEvent::Log(format!("Kept {} / {} frames", frames.len(), total)));
    let doc_meta = PdfMeta {
        title: meta.title,
        uploader: "Local Video".to_owned(),
        duration_secs: meta.duration_secs,
        source_url: None,
    };
    pipeline::render_frames_pdf(cfg, &doc_meta, &captions, &translations, &frames, emit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_paths_and_youtube() {
        assert!(matches!(VideoInput::parse_cli("./lecture.mp4").unwrap(), VideoInput::LocalFile(_)));
        assert!(matches!(VideoInput::parse_cli("/tmp/lesson video.mov").unwrap(), VideoInput::LocalFile(_)));
        assert!(matches!(VideoInput::parse_cli("https://youtu.be/W25_BbZCZD0").unwrap(), VideoInput::YouTubeUrl(_)));
        assert!(VideoInput::parse_cli("https://example.com/video.mp4").is_err());
        assert!(VideoInput::parse_cli("ftp://example.com/video.mp4").is_err());
    }

    #[test]
    fn sidecar_is_optional_for_missing_files() {
        assert!(find_sidecar(Path::new("/nonexistent/lecture.mp4")).is_none());
    }
}
