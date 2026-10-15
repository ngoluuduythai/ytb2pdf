//! Subtitle parsing and cleanup.
//!
//! Supports SRT (what we ask yt-dlp to convert to), WebVTT, and YouTube's
//! `json3` format. Segments are normalized: whitespace collapsed, exact
//! duplicates dropped, and fragments shorter than ~0.4s merged forward.

use anyhow::{Context, Result};
use std::path::Path;

/// One caption cue with absolute timestamps in seconds.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

impl Segment {
    pub fn midpoint(&self) -> f64 {
        (self.start + self.end) / 2.0
    }
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

fn parse_timestamp(ts: &str) -> Option<f64> {
    // "00:01:23,456" (srt) or "00:01:23.456" (vtt)
    let ts = ts.trim().replace(',', ".");
    let mut parts = ts.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    Some(h.parse::<f64>().ok()? * 3600.0 + m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?)
}

fn clean_text(raw: &str) -> String {
    // strip <i>/<b>/font tags, {\\an8} overrides, collapse whitespace
    let mut s = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '{' => in_tag = true,
            '}' => in_tag = false,
            _ if !in_tag => s.push(ch),
            _ => {}
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn parse_srt(text: &str) -> Vec<Segment> {
    let mut segs = Vec::new();
    for block in text.split("\r\n\r\n").flat_map(|b| b.split("\n\n")) {
        let mut lines = block.lines().map(str::trim).filter(|l| !l.is_empty());
        let first = lines.next().unwrap_or("");
        // first line may be a numeric counter or the timestamp itself
        let ts_line = if first.contains("-->") {
            first
        } else {
            lines.next().unwrap_or("")
        };
        let mut ts = ts_line.split("-->");
        let (start, end) = (ts.next(), ts.next());
        if let (Some(a), Some(b)) = (start, end) {
            if let (Some(s), Some(e)) = (parse_timestamp(a), parse_timestamp(b)) {
                let text = clean_text(&lines.collect::<Vec<_>>().join(" "));
                if !text.is_empty() && e > s {
                    segs.push(Segment {
                        start: s,
                        end: e,
                        text,
                    });
                }
            }
        }
    }
    segs
}

pub fn parse_vtt(text: &str) -> Vec<Segment> {
    let body = text
        .lines()
        .skip_while(|l| !l.contains("-->"))
        .collect::<Vec<_>>()
        .join("\n");
    // vtt cues look like srt minus the counter; reuse the srt parser on the cue blocks
    parse_srt(&body.replace("--> ", "-->"))
}

/// YouTube's json3 caption format: {"events":[{"tStartMs":..,"dDurationMs":..,"segs":[{"utf8":".."}]}]}
pub fn parse_json3(text: &str) -> Result<Vec<Segment>> {
    let v: serde_json::Value = serde_json::from_str(text).context("parsing json3 captions")?;
    let mut segs = Vec::new();
    if let Some(events) = v.get("events").and_then(|e| e.as_array()) {
        for ev in events {
            let start = ev.get("tStartMs").and_then(|n| n.as_f64()).unwrap_or(0.0) / 1000.0;
            let dur = ev
                .get("dDurationMs")
                .and_then(|n| n.as_f64())
                .unwrap_or(0.0)
                / 1000.0;
            let text = ev
                .get("segs")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.get("utf8").and_then(|u| u.as_str()))
                        .collect::<String>()
                })
                .unwrap_or_default();
            let text = clean_text(&text);
            if !text.is_empty() && dur > 0.0 {
                segs.push(Segment {
                    start,
                    end: start + dur,
                    text,
                });
            }
        }
    }
    Ok(segs)
}

/// Load + normalize a subtitle file (dispatches on extension).
pub fn load_subtitles_file(path: &Path) -> Result<Vec<Segment>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading subtitle file {}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mut segs = match ext.as_str() {
        "json3" | "json" => parse_json3(&text)?,
        "vtt" => parse_vtt(&text),
        _ => parse_srt(&text),
    };
    normalize(&mut segs);
    if segs.is_empty() {
        anyhow::bail!("no usable caption cues in {}", path.display());
    }
    Ok(segs)
}

/// Collapse whitespace (done in clean_text), drop exact consecutive
/// duplicates, merge fragments < 0.4s into the next segment.
fn normalize(segs: &mut Vec<Segment>) {
    segs.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    let mut out: Vec<Segment> = Vec::with_capacity(segs.len());
    for seg in segs.drain(..) {
        if let Some(last) = out.last_mut() {
            if last.text == seg.text {
                last.end = last.end.max(seg.end);
                continue;
            }
            if last.duration() < 0.4 {
                // merge a too-short fragment forward
                last.text = format!("{} {}", last.text, seg.text);
                last.end = seg.end;
                continue;
            }
        }
        out.push(seg);
    }
    *segs = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "1\r\n00:00:01,000 --> 00:00:03,500\r\nHello <i>world</i>\r\n\r\n2\r\n00:00:03,500 --> 00:00:05,000\r\nHello world\r\n\r\n";

    #[test]
    fn srt_parses_and_dedupes() {
        let mut segs = parse_srt(SRT);
        normalize(&mut segs);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "Hello world");
        assert!((segs[0].start - 1.0).abs() < 1e-6);
    }

    #[test]
    fn vtt_parses() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\nA caption here\n";
        let segs = parse_vtt(vtt);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "A caption here");
    }
}
