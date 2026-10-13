//! Whisper fallback: when a video has no YouTube captions, download the
//! audio track and transcribe it locally with `mlx-whisper` (Apple Silicon),
//! the same approach as the mini-youglish crawler's whisper provider.
//!
//! The tiny driver script is embedded in the binary (`include_str!`) and
//! written to the work dir at runtime, so the .app needs no extra files.

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

use crate::subtitles::Segment;

/// Default model: same as mini-youglish (`mlx-community/whisper-base-mlx`) —
/// good balance of quality, speed and download size (~150 MB, cached in
/// `~/.cache/huggingface` after the first run).
pub const DEFAULT_MODEL: &str = "mlx-community/whisper-base-mlx";

const DRIVER: &str = include_str!("../scripts/transcribe.py");

fn check_mlx_whisper() -> Result<()> {
    let st = Command::new("python3")
        .arg("-c")
        .arg("import mlx_whisper")
        .output()
        .context("failed to run `python3`")?;
    if st.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "mlx-whisper chưa được cài (cần cho video không có caption).\n\
         Cài một lần trên Mac Apple Silicon:\n  \
         pip3 install --break-system-packages mlx-whisper"
    ))
}

/// Parse the driver's JSON output into caption segments. Pure function,
/// unit-tested below.
pub fn parse_whisper_json(json: &str) -> Result<Vec<Segment>> {
    let v: serde_json::Value = serde_json::from_str(json).context("parsing whisper JSON")?;
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow!("whisper output is not a JSON array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for s in arr {
        let text = s
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            continue;
        }
        out.push(Segment {
            start: s.get("start").and_then(|n| n.as_f64()).unwrap_or(0.0),
            end: s.get("end").and_then(|n| n.as_f64()).unwrap_or(0.0),
            text,
        });
    }
    Ok(out)
}

/// Transcribe `audio` with mlx-whisper. Returns caption segments with
/// real (start, end) timestamps. Writes the driver script into `work_dir`.
pub fn transcribe_audio(
    audio: &Path,
    work_dir: &Path,
    model: &str,
    language: &str,
) -> Result<Vec<Segment>> {
    check_mlx_whisper()?;

    let driver_path = work_dir.join("transcribe_mlx.py");
    std::fs::write(&driver_path, DRIVER).context("writing whisper driver script")?;

    let out = Command::new("python3")
        .arg(&driver_path)
        .arg(audio)
        .arg(model)
        .arg(language)
        .output()
        .context("failed to run the whisper driver (python3)")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!(
            "whisper transcription failed (exit {}):\n{}",
            out.status,
            err.trim()
        ));
    }
    let json = String::from_utf8(out.stdout).context("whisper output is not UTF-8")?;
    let segs = parse_whisper_json(&json)?;
    if segs.is_empty() {
        anyhow::bail!("whisper returned no speech segments");
    }
    Ok(segs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_driver_output() {
        let json = r#"[{"start": 0.0, "end": 2.5, "text": "hello   world"},
                        {"start": 2.5, "end": 4.0, "text": "  "},
                        {"start": 4.0, "end": 6.0, "text": "foo bar"}]"#;
        let segs = parse_whisper_json(json).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "hello world");
        assert!((segs[0].end - 2.5).abs() < 1e-9);
        assert_eq!(segs[1].start, 4.0);
    }

    #[test]
    fn rejects_non_array() {
        assert!(parse_whisper_json(r#"{"a": 1}"#).is_err());
    }
}
