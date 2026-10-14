//! Architecture V2: low-res detection, high-res extraction.
//!
//! Instead of downloading the full video at 480p/720p, V2:
//! 1. Downloads a small 240p proxy (`proxy`)
//! 2. Detects scene changes on the proxy (`scene`)
//! 3. Plans minimal high-res download segments (`segments`)
//! 4. Downloads only those segments at 1080p (`hi_res`)
//! 5. Extracts frames and maps them to captions (`mapping`)

pub mod hi_res;
pub mod mapping;
pub mod pipeline;
pub mod proxy;
pub mod scene;
pub mod segments;
