//! Segment planning: merge nearby scene timestamps into download ranges.
//!
//! Instead of calling `yt-dlp --download-sections` once per scene (100+
//! times), nearby scenes are merged into a handful of ranges. Each range
//! is downloaded once, then multiple frames are extracted from it locally.

use crate::v2::scene::Scene;

/// A time range to download at high resolution.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Start time in seconds (with padding applied).
    pub start: f64,
    /// End time in seconds (with padding applied).
    pub end: f64,
    /// Scene timestamps covered by this segment.
    pub scenes: Vec<f64>,
}

/// Merge scene timestamps into download segments.
///
/// - `gap`: max seconds between scenes to merge (e.g. 10.0).
/// - `pad`: seconds of padding on each side (e.g. 2.0) to absorb
///   keyframe misalignment from `--download-sections`.
pub fn plan_segments(scenes: &[Scene], gap: f64, pad: f64) -> Vec<Segment> {
    if scenes.is_empty() {
        return vec![];
    }
    let mut times: Vec<f64> = scenes.iter().map(|s| s.time).collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mut segments: Vec<Segment> = Vec::new();
    for t in times {
        let merge = segments.last_mut().filter(|seg| t - seg.end <= gap + 2.0 * pad);
        match merge {
            Some(seg) => {
                seg.end = t + pad;
                seg.scenes.push(t);
            }
            None => segments.push(Segment {
                start: (t - pad).max(0.0),
                end: t + pad,
                scenes: vec![t],
            }),
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::scene::{Scene, SceneSource};

    fn mk(times: &[f64]) -> Vec<Scene> {
        times
            .iter()
            .map(|&t| Scene {
                time: t,
                score: None,
                source: SceneSource::Filter,
            })
            .collect()
    }

    #[test]
    fn merges_nearby_scenes() {
        let segs = plan_segments(&mk(&[12.5, 14.2, 15.8, 95.8, 97.1, 220.4]), 10.0, 2.0);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].scenes.len(), 3);
        assert_eq!(segs[1].scenes.len(), 2);
        assert_eq!(segs[2].scenes.len(), 1);
    }

    #[test]
    fn empty_gives_empty() {
        assert!(plan_segments(&[], 10.0, 2.0).is_empty());
    }

    #[test]
    fn pad_clamped_at_zero() {
        let segs = plan_segments(&mk(&[1.0]), 10.0, 5.0);
        assert_eq!(segs[0].start, 0.0);
    }
}
