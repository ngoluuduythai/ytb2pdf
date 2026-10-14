//! 240p proxy download for fast scene detection.
//!
//! The proxy is ~10% the size of a 720p download, so even throttled
//! connections finish quickly. Scene detection runs on the proxy;
//! high-res frames are fetched later only for kept scenes.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::youtube::{VideoRef, YtDlp};

/// Download the lowest practical video stream for scene analysis.
/// Falls back to whatever the lowest available stream is.
pub fn download_proxy(ytdlp: &YtDlp, video: &VideoRef, out_path: &Path) -> Result<PathBuf> {
    let mut cmd = ytdlp.base_cmd();
    cmd.arg("-f")
        .arg("bv*[height<=240]/b[height<=240]/bv*/b*")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--output")
        .arg(out_path)
        .arg(&video.url);
    ytdlp.run(cmd, "downloading 240p proxy")?;
    if out_path.exists() {
        return Ok(out_path.to_path_buf());
    }
    // yt-dlp may pick its own extension when merging; find it.
    let stem = out_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir = out_path.parent().unwrap_or(Path::new("."));
    for entry in std::fs::read_dir(dir).context("listing proxy dir")? {
        let p = entry?.path();
        if p.file_stem().and_then(|s| s.to_str()) == Some(stem) {
            return Ok(p);
        }
    }
    anyhow::bail!("proxy download produced no file at {}", out_path.display())
}
