//! High-resolution segment download via `yt-dlp --download-sections`.
//!
//! Each [`Segment`](crate::v2::segments::Segment) is downloaded once at
//! high resolution. Downloads run in parallel (`jobs`) and are resumable:
//! existing segment files are skipped.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::v2::segments::Segment;
use crate::youtube::{VideoRef, YtDlp};

/// Download high-res segments. Returns paths in the same order as `segments`.
pub fn download_segments(
    ytdlp: &YtDlp,
    video: &VideoRef,
    segments: &[Segment],
    out_dir: &Path,
    max_height: u32,
    jobs: usize,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir).context("creating segment dir")?;
    let total = segments.len();
    if total == 0 {
        return Ok(vec![]);
    }
    let jobs = jobs.max(1).min(total);
    let chunk_size = total.div_ceil(jobs);
    let done = std::sync::atomic::AtomicUsize::new(0);

    let chunks: Result<Vec<Vec<PathBuf>>> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for (chunk_idx, chunk) in segments.chunks(chunk_size).enumerate() {
            let start_idx = chunk_idx * chunk_size;
            // YtDlp is not Sync (contains Command builder state), so clone
            // the config values each worker needs instead of sharing.
            let cookies_file = ytdlp.cookies_file.clone();
            let cookies_from_browser = ytdlp.cookies_from_browser.clone();
            let url = video.url.clone();
            let out_dir = out_dir.to_path_buf();
            let done_ref = &done;
            handles.push(s.spawn(move || {
                let worker = YtDlp {
                    cookies_file,
                    cookies_from_browser,
                    max_height,
                    retries: 10,
                };
                let mut paths = Vec::with_capacity(chunk.len());
                for (j, seg) in chunk.iter().enumerate() {
                    let i = start_idx + j;
                    let path = out_dir.join(format!("seg_{i:03}.mp4"));
                    if !crate::resume::artifact_valid(&path) {
                        if path.exists() {
                            std::fs::remove_file(&path)?;
                        }
                        download_one(&worker, &url, seg, &path)?;
                        crate::resume::commit_artifact(&path)?;
                    }
                    paths.push(path);
                    done_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Ok::<_, anyhow::Error>(paths)
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().expect("segment worker panicked"))
            .collect()
    });

    let mut paths: Vec<PathBuf> = chunks?.into_iter().flatten().collect();
    paths.sort();
    Ok(paths)
}

/// Download a single time range at high resolution.
fn download_one(ytdlp: &YtDlp, url: &str, seg: &Segment, out: &Path) -> Result<()> {
    let h = ytdlp.max_height;
    // yt-dlp section syntax: "*START-END" (whole video "*" with range).
    let range = format!("*{}-{}", seg.start as u64, seg.end.ceil() as u64);
    let mut cmd = ytdlp.base_cmd();
    cmd.arg("-f")
        .arg(format!("bv*[height<={h}]+ba/b[height<={h}]/b"))
        .arg("--download-sections")
        .arg(&range)
        .arg("--force-keyframes-at-cuts")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--output")
        .arg(out)
        .arg(url);
    ytdlp.run(cmd, &format!("downloading section {range}"))?;
    anyhow::ensure!(out.exists(), "segment download produced no file");
    Ok(())
}
