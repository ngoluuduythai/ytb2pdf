//! Frame capture + perceptual dedupe.
//!
//! For each caption segment we grab one frame at its midpoint with `ffmpeg`,
//! then drop near-identical consecutive frames using a 64-bit difference
//! hash (dHash). Lecture videos are mostly static slides / talking heads, so
//! this typically cuts the image count by 5-20x.

use anyhow::{Context, Result};
use image::DynamicImage;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Frame {
    /// caption-segment midpoint, seconds
    pub at_secs: f64,
    pub path: PathBuf,
    pub hash: u64,
}

/// Grab a single JPEG frame at `at_secs` (fast seek: `-ss` before `-i`).
pub fn extract_frame(video: &Path, at_secs: f64, out: &Path) -> Result<()> {
    let st = Command::new("ffmpeg")
        .arg("-y")
        .arg("-v")
        .arg("error")
        .arg("-ss")
        .arg(format!("{at_secs:.3}"))
        .arg("-i")
        .arg(video)
        .arg("-frames:v")
        .arg("1")
        .arg("-q:v")
        .arg("5")
        .arg(out)
        .status()
        .context("failed to run `ffmpeg` — install it with `brew install ffmpeg`")?;
    if !st.success() || !out.exists() {
        anyhow::bail!("ffmpeg could not extract a frame at {at_secs:.1}s");
    }
    Ok(())
}

/// 64-bit difference hash: grayscale -> 9x8 -> compare horizontal neighbours.
pub fn dhash(img: &DynamicImage) -> u64 {
    let small = img.grayscale().resize_exact(9, 8, image::imageops::FilterType::Triangle);
    let gray = small.to_luma8();
    let mut hash: u64 = 0;
    for y in 0..8 {
        for x in 0..8 {
            let left = gray.get_pixel(x, y)[0];
            let right = gray.get_pixel(x + 1, y)[0];
            hash = (hash << 1) | u64::from(left < right);
        }
    }
    hash
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Drop consecutive frames whose hashes differ by at most `max_distance`
/// bits (default 6). Keeps the *first* frame of each run.
pub fn dedupe_frames(frames: Vec<Frame>, max_distance: u32) -> Vec<Frame> {
    let mut out = Vec::with_capacity(frames.len());
    for f in frames {
        let dup = out
            .last()
            .map(|prev: &Frame| hamming(prev.hash, f.hash) <= max_distance)
            .unwrap_or(false);
        if !dup {
            out.push(f);
        }
    }
    out
}

/// Capture one frame per timestamp, hashing each for the dedupe pass.
///
/// Runs `jobs` ffmpeg workers in parallel. Resumable: if `frame_{i:05}.jpg`
/// already exists in `out_dir`, ffmpeg is skipped and the file is hashed
/// directly — so a restarted run continues where it left off.
pub fn capture_frames(
    video: &Path,
    timestamps: &[f64],
    out_dir: &Path,
    jobs: usize,
    on_one: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<Frame>> {
    std::fs::create_dir_all(out_dir).context("creating frame dir")?;
    let total = timestamps.len();
    if total == 0 {
        return Ok(vec![]);
    }
    let jobs = jobs.max(1).min(total);
    let chunk_size = total.div_ceil(jobs);
    let done = std::sync::atomic::AtomicUsize::new(0);

    let chunks: Result<Vec<Vec<Frame>>> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for (chunk_idx, chunk) in timestamps.chunks(chunk_size).enumerate() {
            let start_idx = chunk_idx * chunk_size;
            let video = video.to_path_buf();
            let out_dir = out_dir.to_path_buf();
            let done_ref = &done;
            handles.push(s.spawn(move || {
                let mut frames = Vec::with_capacity(chunk.len());
                for (j, &t) in chunk.iter().enumerate() {
                    let i = start_idx + j;
                    let path = out_dir.join(format!("frame_{i:05}.jpg"));
                    if !path.exists() {
                        extract_frame(&video, t, &path)?;
                    }
                    let img = image::open(&path)
                        .with_context(|| format!("reading back {}", path.display()))?;
                    frames.push(Frame {
                        at_secs: t,
                        path,
                        hash: dhash(&img),
                    });
                    let n = done_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    on_one(n, total);
                }
                Ok::<_, anyhow::Error>(frames)
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().expect("frame worker panicked"))
            .collect()
    });

    let mut frames: Vec<Frame> = chunks?.into_iter().flatten().collect();
    // Workers finish out of order — restore timestamp order for dedupe.
    frames.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dhash_is_stable_and_hamming_works() {
        let img = DynamicImage::new_rgb8(32, 32);
        let h1 = dhash(&img);
        let h2 = dhash(&img);
        assert_eq!(h1, h2);
        assert_eq!(hamming(h1, h2), 0);
        assert_eq!(hamming(0, u64::MAX), 64);
    }

    #[test]
    fn dedupe_keeps_first_of_run() {
        let mk = |h| Frame { at_secs: 0.0, path: PathBuf::from("x"), hash: h };
        let frames = vec![mk(0), mk(1), mk(u64::MAX), mk(u64::MAX ^ 3)];
        let kept = dedupe_frames(frames, 6);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].hash, 0);
        assert_eq!(kept[1].hash, u64::MAX);
    }
}
