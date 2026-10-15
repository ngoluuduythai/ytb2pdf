//! Whisper fallback: when a video has no YouTube captions, download the
//! audio track and transcribe it locally with a bundled `whisper.cpp`
//! binary (`whisper-cli`). The model file is NOT bundled — it downloads
//! once, on demand, into `~/.ytb2pdf/models/` so the .dmg stays small.
//! No Python needed.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::subtitles::{self, Segment};

/// Transcription models the user can pick from. Downloaded on demand from
/// Hugging Face, cached under `~/.ytb2pdf/models/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhisperModel {
    /// ggml-tiny.bin (~75 MB) — fastest, roughest.
    Fast,
    /// ggml-base.bin (~147 MB) — balanced. Default.
    #[default]
    Medium,
}

impl WhisperModel {
    pub fn file_name(&self) -> &'static str {
        match self {
            WhisperModel::Fast => "ggml-tiny.bin",
            WhisperModel::Medium => "ggml-base.bin",
        }
    }

    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.file_name()
        )
    }

    pub fn size_hint(&self) -> &'static str {
        match self {
            WhisperModel::Fast => "~75 MB",
            WhisperModel::Medium => "~147 MB",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            WhisperModel::Fast => "Nhanh (tiny)",
            WhisperModel::Medium => "Balanced (base)",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "fast" | "tiny" => Some(WhisperModel::Fast),
            "medium" | "base" => Some(WhisperModel::Medium),
            _ => None,
        }
    }
}

fn models_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".ytb2pdf").join("models"))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p).find_map(|dir| {
            let f = dir.join(name);
            f.is_file().then(|| f)
        })
    })
}

/// Locate `whisper-cli`: bundled in the .app (`Contents/Resources/bin`, put
/// on PATH at startup), otherwise expected on PATH (`brew install whisper-cpp`).
pub fn whisper_cli_path() -> Result<PathBuf> {
    find_on_path("whisper-cli")
        .or_else(|| find_on_path("whisper"))
        .ok_or_else(|| {
            anyhow!(
                "`whisper-cli` was not found (included in the official .dmg; \
                 for standalone CLI usage: brew install whisper-cpp)"
            )
        })
}

/// Ensure the model file exists locally, downloading it once if needed.
pub fn ensure_model(model: WhisperModel, log: &dyn Fn(String)) -> Result<PathBuf> {
    ensure_model_in(model, None, log)
}

fn ensure_model_in(
    model: WhisperModel,
    scratch: Option<&Path>,
    log: &dyn Fn(String),
) -> Result<PathBuf> {
    cache_model(model, &models_dir()?, scratch, log, |download| {
        let status = Command::new("curl")
            .arg("-sSL")
            .arg("--fail")
            .arg("-o")
            .arg(download)
            .arg(model.url())
            .status()
            .context("failed to run `curl` to download the whisper model")?;
        anyhow::ensure!(
            status.success(),
            "Failed to download Whisper model from {}",
            model.url()
        );
        Ok(())
    })
}

fn cache_model(
    model: WhisperModel,
    dir: &Path,
    scratch: Option<&Path>,
    log: &dyn Fn(String),
    fetch: impl FnOnce(&Path) -> Result<()>,
) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(model.file_name());
    if path.exists() {
        return Ok(path);
    }
    log(format!(
        "Downloading Whisper model {} ({}) — one-time setup…",
        model.label(),
        model.size_hint()
    ));
    // Download into scratch space so a cancelled desktop worker cannot leave
    // an incomplete model at the cache path that future runs trust.
    let download_dir = scratch.unwrap_or(dir);
    std::fs::create_dir_all(download_dir)?;
    let download = tempfile::Builder::new()
        .prefix("whisper-download-")
        .tempfile_in(download_dir)?;
    fetch(download.path())?;
    if download.as_file().metadata()?.len() == 0 {
        anyhow::bail!("Failed to download Whisper model from {}", model.url());
    }
    // Stage in the cache directory before the atomic rename, even if scratch
    // and the cache live on different filesystems.
    let staged = tempfile::Builder::new()
        .prefix(".whisper-model-")
        .tempfile_in(dir)?;
    std::fs::copy(download.path(), staged.path())?;
    match staged.persist_noclobber(&path) {
        Ok(_) => {}
        // Another conversion may have installed the same complete model first.
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.error).context("Installing the Whisper model"),
    }
    Ok(path)
}

/// Transcribe `audio` with whisper.cpp. Returns caption segments with real
/// (start, end) timestamps, parsed from whisper's SRT output.
pub fn transcribe_audio(
    audio: &Path,
    work_dir: &Path,
    model: WhisperModel,
    language: &str,
    log: &dyn Fn(String),
) -> Result<Vec<Segment>> {
    let cli = whisper_cli_path()?;
    let model_path = ensure_model_in(model, Some(work_dir), log)?;

    // whisper.cpp decodes WAV reliably; convert with the bundled ffmpeg.
    // 16 kHz mono is also exactly what Whisper wants.
    let wav = work_dir.join("whisper-16k.wav");
    let st = Command::new("ffmpeg")
        .arg("-y")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(audio)
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(&wav)
        .status()
        .context("failed to run `ffmpeg` for whisper preprocessing")?;
    if !st.success() || !wav.exists() {
        anyhow::bail!("ffmpeg could not convert audio for whisper");
    }

    let srt_prefix = work_dir.join("whisper");
    let out = Command::new(&cli)
        .arg("-m")
        .arg(&model_path)
        .arg("-f")
        .arg(&wav)
        .arg("-l")
        .arg(language)
        .arg("--output-srt")
        .arg("-of")
        .arg(&srt_prefix)
        .output()
        .with_context(|| format!("failed to run `{}`", cli.display()))?;
    // The temp WAV can be big (16 kHz mono ≈ 115 MB/hour); drop it early.
    let _ = std::fs::remove_file(&wav);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!(
            "whisper-cli failed (exit {}):\n{}",
            out.status,
            err.trim()
        ));
    }
    let srt = srt_prefix.with_extension("srt");
    let segs = subtitles::load_subtitles_file(&srt)?;
    if segs.is_empty() {
        anyhow::bail!("whisper returned no speech segments");
    }
    Ok(segs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_roundtrip() {
        assert_eq!(WhisperModel::from_name("fast"), Some(WhisperModel::Fast));
        assert_eq!(WhisperModel::from_name("tiny"), Some(WhisperModel::Fast));
        assert_eq!(
            WhisperModel::from_name("medium"),
            Some(WhisperModel::Medium)
        );
        assert_eq!(WhisperModel::from_name("base"), Some(WhisperModel::Medium));
        assert_eq!(WhisperModel::from_name("huge"), None);
        assert_eq!(WhisperModel::Fast.file_name(), "ggml-tiny.bin");
        assert_eq!(WhisperModel::Medium.file_name(), "ggml-base.bin");
        assert!(WhisperModel::Medium.url().contains("huggingface.co"));
    }

    #[test]
    fn interrupted_model_download_is_not_cached_and_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let model = WhisperModel::Fast;
        let failed = cache_model(model, dir.path(), Some(scratch.path()), &|_| {}, |path| {
            std::fs::write(path, b"partial")?;
            anyhow::bail!("Download interrupted")
        });
        assert!(failed.is_err());
        assert!(!dir.path().join(model.file_name()).exists());
        assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
        let path = cache_model(model, dir.path(), Some(scratch.path()), &|_| {}, |path| {
            std::fs::write(path, b"complete model")?;
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"complete model");
    }

    #[test]
    fn concurrent_model_install_keeps_the_first_complete_download() {
        let dir = tempfile::tempdir().unwrap();
        let model = WhisperModel::Fast;
        let cached = dir.path().join(model.file_name());
        let path = cache_model(model, dir.path(), None, &|_| {}, |path| {
            std::fs::write(path, b"second complete download")?;
            std::fs::write(&cached, b"first complete download")?;
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"first complete download");
    }

    #[test]
    fn default_is_medium() {
        assert_eq!(WhisperModel::default(), WhisperModel::Medium);
    }
}
