//! Scene change detection on the 240p proxy.
//!
//! Two complementary mechanisms (per the V2 plan):
//! 1. **FFmpeg scene filter** — catches hard cuts (`select='gt(scene,T)'`).
//! 2. **Periodic sampling** — catches gradual changes (slide building up,
//!    handwriting, diagrams) that never cross the scene threshold.
//!
//! Results are merged and sorted by timestamp. Downstream dedupe (dHash)
//! removes near-duplicates.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// A detected scene change point.
#[derive(Debug, Clone)]
pub struct Scene {
    /// Timestamp in seconds.
    pub time: f64,
    /// Scene score from ffmpeg (0.0-1.0). `None` for periodic samples.
    pub score: Option<f32>,
    /// How this scene was found.
    pub source: SceneSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneSource {
    /// FFmpeg `select='gt(scene,T)'` filter.
    Filter,
    /// Periodic sampling fallback.
    Sample,
}

/// Detect scenes in `proxy` video.
///
/// - `threshold`: ffmpeg scene threshold (0.30 is a good start; lower =
///   more sensitive). Typical: 0.15 (sensitive) .. 0.45 (only hard cuts).
/// - `sample_interval`: seconds between periodic samples (e.g. 5.0).
///   Set to 0.0 to disable sampling.
pub fn detect_scenes(proxy: &Path, threshold: f32, sample_interval: f64) -> Result<Vec<Scene>> {
    let mut scenes = filter_scenes(proxy, threshold)?;

    if sample_interval > 0.0 {
        let duration = probe_duration(proxy)?;
        let mut t = 0.0;
        while t < duration {
            // Only sample in gaps: skip if within 15s of a filter-detected
            // scene. This keeps sampling as a fallback for gradual changes
            // without flooding the segment planner.
            let near = scenes.iter().any(|s| (s.time - t).abs() < 15.0);
            if !near {
                scenes.push(Scene {
                    time: t,
                    score: None,
                    source: SceneSource::Sample,
                });
            }
            t += sample_interval;
        }
    }

    scenes.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    Ok(scenes)
}

/// Run ffmpeg scene filter and parse timestamps from showinfo output.
fn filter_scenes(proxy: &Path, threshold: f32) -> Result<Vec<Scene>> {
    let out = Command::new("ffmpeg")
        .arg("-v")
        .arg("info")
        .arg("-i")
        .arg(proxy)
        .arg("-vf")
        .arg(format!("select='gt(scene,{threshold})',showinfo"))
        .arg("-an")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .context("failed to run ffmpeg scene detection")?;

    // showinfo goes to stderr.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut scenes = Vec::new();
    for line in stderr.lines() {
        // showinfo line: "[Parsed_showinfo_1 @ 0x...] n: 123 pts: 456 ... pts_time:12.5 ..."
        if let Some(t) = parse_pts_time(line) {
            scenes.push(Scene {
                time: t,
                score: None, // score parsing is fragile; timestamp is what matters
                source: SceneSource::Filter,
            });
        }
    }
    Ok(scenes)
}

/// Extract `pts_time:<float>` from an ffmpeg showinfo line.
fn parse_pts_time(line: &str) -> Option<f64> {
    let idx = line.find("pts_time:")?;
    let rest = &line[idx + 9..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Get video duration in seconds via ffprobe.
fn probe_duration(proxy: &Path) -> Result<f64> {
    let out = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(proxy)
        .output()
        .context("failed to run ffprobe")?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim()
        .parse()
        .with_context(|| format!("parsing duration from {:?}", s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pts_time_extracts_float() {
        let line = "[Parsed_showinfo_1 @ 0x55] n: 42 pts: 1234 pts_time:12.5 pos: 567";
        assert_eq!(parse_pts_time(line), Some(12.5));
    }

    #[test]
    fn parse_pts_time_none_without_marker() {
        assert_eq!(parse_pts_time("no timestamp here"), None);
    }
}
