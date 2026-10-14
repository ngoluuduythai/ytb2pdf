//! Map caption segments to their nearest scene frame.
//!
//! The PDF is organized by captions, but V2 captures frames per scene.
//! Each caption segment is assigned the scene frame with the closest
//! timestamp at or before the caption's midpoint.

use crate::v2::scene::Scene;

/// A caption segment assigned to a scene frame time.
#[derive(Debug, Clone)]
pub struct MappedCaption {
    /// Index into the original caption list.
    pub caption_idx: usize,
    /// Timestamp of the assigned scene frame.
    pub scene_time: f64,
}

/// Assign each caption midpoint to the nearest scene at or before it.
/// Scenes must be sorted by time (as returned by `detect_scenes`).
pub fn map_captions_to_scenes(caption_midpoints: &[f64], scenes: &[Scene]) -> Vec<MappedCaption> {
    if scenes.is_empty() {
        return vec![];
    }
    caption_midpoints
        .iter()
        .enumerate()
        .map(|(caption_idx, &mid)| {
            // Binary search for the last scene with time <= mid.
            let scene_time = match scenes.binary_search_by(|s| {
                s.time
                    .partial_cmp(&mid)
                    .unwrap_or(std::cmp::Ordering::Less)
            }) {
                Ok(i) => scenes[i].time,
                Err(0) => scenes[0].time, // before first scene: use first
                Err(i) => scenes[i - 1].time,
            };
            MappedCaption {
                caption_idx,
                scene_time,
            }
        })
        .collect()
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
    fn maps_to_nearest_prior_scene() {
        let scenes = mk(&[10.0, 20.0, 30.0]);
        let mapped = map_captions_to_scenes(&[12.0, 25.0, 35.0], &scenes);
        assert_eq!(mapped[0].scene_time, 10.0);
        assert_eq!(mapped[1].scene_time, 20.0);
        assert_eq!(mapped[2].scene_time, 30.0);
    }

    #[test]
    fn before_first_scene_uses_first() {
        let scenes = mk(&[10.0]);
        let mapped = map_captions_to_scenes(&[5.0], &scenes);
        assert_eq!(mapped[0].scene_time, 10.0);
    }

    #[test]
    fn empty_scenes_gives_empty() {
        assert!(map_captions_to_scenes(&[5.0], &[]).is_empty());
    }
}
