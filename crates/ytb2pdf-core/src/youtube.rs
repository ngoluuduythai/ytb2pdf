//! YouTube access through the `yt-dlp` CLI.
//!
//! We shell out to `yt-dlp` instead of linking a library: it tracks YouTube's
//! constant breakage far faster than any Rust crate can. The [`YtDlp`] builder
//! carries the anti-403 knobs (browser cookies, retries) that home networks
//! behind YouTube's bot checks need.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Accepted YouTube URL shapes.
#[derive(Debug, Clone)]
pub struct VideoRef {
    /// 11-char video id, e.g. `W25_BbZCZD0`
    pub id: String,
    /// The URL as the user pasted it (kept for `t=` deep links in the PDF).
    pub url: String,
}

/// Extract the video id from the common YouTube URL shapes.
pub fn parse_youtube_url(url: &str) -> Result<VideoRef> {
    let url = url.trim();
    let id = if let Some(rest) = url.split("youtu.be/").nth(1) {
        rest.split(['?', '&', '/']).next().unwrap_or("")
    } else if let Some(q) = url.split("youtube.com/watch").nth(1) {
        q.split("v=")
            .nth(1)
            .and_then(|s| s.split(['&', '?']).next())
            .unwrap_or("")
    } else if let Some(rest) = url.split("youtube.com/shorts/").nth(1) {
        rest.split(['?', '&', '/']).next().unwrap_or("")
    } else if let Some(rest) = url.split("youtube.com/embed/").nth(1) {
        rest.split(['?', '&', '/']).next().unwrap_or("")
    } else if let Some(rest) = url.split("youtube.com/live/").nth(1) {
        rest.split(['?', '&', '/']).next().unwrap_or("")
    } else {
        ""
    };
    if id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        Ok(VideoRef {
            id: id.to_string(),
            url: url.to_string(),
        })
    } else {
        Err(anyhow!("not a recognizable YouTube video URL: {url}"))
    }
}

#[derive(Debug, Clone)]
pub struct VideoMeta {
    pub id: String,
    pub title: String,
    pub uploader: String,
    pub duration_secs: u64,
    pub thumbnail_url: Option<String>,
}

/// True when yt-dlp's stderr shows YouTube's player-level bot mitigation.
/// The `web` player client is challenged most aggressively (SABR-only
/// streaming / PO-token); other clients often still work.
fn is_player_challenge(stderr: &str) -> bool {
    stderr.contains("The page needs to be reloaded")
        || stderr.contains("Sign in to confirm you")
}

/// Thin wrapper around the `yt-dlp` binary.
#[derive(Debug, Clone, Default)]
pub struct YtDlp {
    /// Path to a Netscape-format cookies.txt file (e.g. exported from the
    /// "Get cookies.txt LOCALLY" extension while on youtube.com).
    /// Preferred over `cookies_from_browser`: narrower scope, no browser
    /// profile permission needed. Passed as `--cookies <file>`.
    pub cookies_file: Option<PathBuf>,
    /// e.g. `Some("chrome")` — passes `--cookies-from-browser chrome`.
    /// Fallback when no cookie file is given.
    pub cookies_from_browser: Option<String>,
    /// Max video height for downloads (e.g. 720). Lower (480) for very long
    /// videos where 720p would be huge and YouTube throttles the transfer.
    pub max_height: u32,
    /// Extra seconds of backoff is handled by yt-dlp itself (`--retries`).
    pub retries: u32,
}

impl YtDlp {
    pub fn new() -> Self {
        Self {
            cookies_file: None,
            cookies_from_browser: None,
            max_height: 720,
            retries: 5,
        }
    }

    fn base_cmd(&self) -> Command {
        let mut cmd = Command::new("yt-dlp");
        cmd.arg("--no-warnings");
        // YouTube's `n` challenge needs a JS runtime + the EJS solver script.
        // Without this, downloads fail with "n challenge solving failed" /
        // "The page needs to be reloaded" even with valid cookies.
        // (Deno must be on PATH; the app bundles it.)
        cmd.arg("--remote-components").arg("ejs:github");
        if let Some(f) = &self.cookies_file {
            cmd.arg("--cookies").arg(f);
        } else if let Some(browser) = &self.cookies_from_browser {
            cmd.arg("--cookies-from-browser").arg(browser);
        }
        cmd.arg("--retries").arg(self.retries.to_string());
        // Back off on HTTP errors (429/5xx): wait 10s between retries so
        // YouTube's per-IP throttle has time to cool down.
        cmd.arg("--retry-sleep").arg("http:10");
        cmd
    }

    fn run(&self, mut cmd: Command, what: &str) -> Result<std::process::Output> {
        let out = cmd
            .output()
            .with_context(|| "failed to run `yt-dlp` — install it with `brew install yt-dlp` (https://github.com/yt-dlp/yt-dlp)")?;
        if out.status.success() {
            return Ok(out);
        }
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        if is_player_challenge(&err) {
            // YouTube's bot mitigation ("The page needs to be reloaded" /
            // "Sign in to confirm you're not a bot") targets the `web`
            // player client most aggressively. Retry once without it.
            cmd.arg("--extractor-args")
                .arg("youtube:player_client=-web");
            let retry = cmd.output().with_context(|| "failed to re-run `yt-dlp`")?;
            if retry.status.success() {
                return Ok(retry);
            }
            let err2 = String::from_utf8_lossy(&retry.stderr);
            return Err(anyhow!(
                "yt-dlp failed while {what} (exit {}):\n{}\n(thử lại không dùng web player client cũng thất bại (exit {}):\n{})",
                out.status,
                err.trim(),
                retry.status,
                err2.trim()
            ));
        }
        Err(anyhow!(
            "yt-dlp failed while {what} (exit {}):\n{}",
            out.status,
            err.trim()
        ))
    }

    /// Fetch title / duration / uploader without downloading anything.
    pub fn metadata(&self, video: &VideoRef) -> Result<VideoMeta> {
        let mut cmd = self.base_cmd();
        cmd.arg("--skip-download")
            .arg("--dump-single-json")
            .arg(&video.url);
        let out = self.run(cmd, "fetching video metadata")?;
        let v: serde_json::Value =
            serde_json::from_slice(&out.stdout).context("parsing yt-dlp JSON")?;
        Ok(VideoMeta {
            id: v.get("id").and_then(|s| s.as_str()).unwrap_or(&video.id).to_string(),
            title: v
                .get("title")
                .and_then(|s| s.as_str())
                .unwrap_or("(untitled)")
                .to_string(),
            uploader: v
                .get("uploader")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown channel")
                .to_string(),
            duration_secs: v.get("duration").and_then(|d| d.as_u64()).unwrap_or(0),
            thumbnail_url: v
                .get("thumbnail")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
        })
    }

    /// Download the audio track (m4a) — used by the Whisper fallback when
    /// the video has no captions.
    pub fn download_audio(&self, video: &VideoRef, out_path: &Path) -> Result<PathBuf> {
        let mut cmd = self.base_cmd();
        cmd.arg("--no-playlist")
            .arg("-f")
            .arg("bestaudio[ext=m4a]/bestaudio/best")
            .arg("--output")
            .arg(out_path)
            .arg(&video.url);
        self.run(cmd, "downloading audio")?;
        if out_path.exists() {
            Ok(out_path.to_path_buf())
        } else {
            Err(anyhow!("yt-dlp finished but the audio file is missing"))
        }
    }

    /// Download a subtitle file. Tries manual subtitles first, then
    /// auto-generated captions. Returns the path of the downloaded file.
    pub fn download_subtitles(
        &self,
        video: &VideoRef,
        lang: &str,
        out_dir: &Path,
    ) -> Result<PathBuf> {
        for auto in [false, true] {
            let mut cmd = self.base_cmd();
            cmd.arg("--skip-download");
            if auto {
                cmd.arg("--write-auto-sub");
            } else {
                cmd.arg("--write-sub");
            }
            let out_tpl = out_dir.join(format!(
                "{}.{}",
                video.id,
                if auto { "auto" } else { "manual" }
            ));
            cmd.arg("--sub-langs")
                .arg(lang)
                .arg("--sub-format")
                .arg("srt/vtt/best")
                .arg("--convert-subs")
                .arg("srt")
                .arg("--output")
                .arg(format!("{}.%(ext)s", out_tpl.display()))
                .arg(&video.url);
            match self.run(cmd, "downloading subtitles") {
                Ok(_) => {
                    let srt = out_tpl.with_extension("srt");
                    if srt.exists() {
                        return Ok(srt);
                    }
                    let vtt = out_tpl.with_extension("vtt");
                    if vtt.exists() {
                        return Ok(vtt);
                    }
                }
                Err(e) if auto => return Err(e),
                Err(_) => continue, // no manual subs -> fall through to auto
            }
        }
        Err(anyhow!(
            "no subtitles found for video {} (tried manual + auto captions, lang `{lang}`)",
            video.id
        ))
    }

    /// Download the video itself. `max_height` caps the resolution (720p
    /// default is plenty for frame grabs; 480p for very long videos).
    /// Uses patient retries — YouTube throttles datacenter IPs hard.
    pub fn download_video(&self, video: &VideoRef, out_path: &Path) -> Result<PathBuf> {
        let mut cmd = self.base_cmd();
        let h = self.max_height;
        cmd.arg("-f")
            .arg(format!("bv*[height<={h}]+ba/b[height<={h}]/b"))
            .arg("--merge-output-format")
            .arg("mp4")
            // Be patient through YouTube's throttling: many retries with
            // backoff, otherwise long videos die with "read timed out".
            .arg("--retries")
            .arg("20")
            .arg("--fragment-retries")
            .arg("20")
            .arg("--retry-sleep")
            .arg("30")
            .arg("--output")
            .arg(out_path)
            .arg(&video.url);
        self.run(cmd, "downloading video")?;
        if out_path.exists() {
            Ok(out_path.to_path_buf())
        } else {
            // yt-dlp may pick its own extension when merging; find it.
            let stem = out_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let dir = out_path.parent().unwrap_or(Path::new("."));
            for entry in std::fs::read_dir(dir).context("listing download dir")? {
                let p = entry?.path();
                if p.file_stem().and_then(|s| s.to_str()) == Some(stem) {
                    return Ok(p);
                }
            }
            Err(anyhow!("yt-dlp finished but the video file is missing"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_player_challenge_errors() {
        assert!(is_player_challenge(
            "ERROR: [youtube] abc123: The page needs to be reloaded."
        ));
        assert!(is_player_challenge(
            "ERROR: [youtube] abc123: Sign in to confirm you're not a bot"
        ));
        assert!(!is_player_challenge("ERROR: HTTP Error 429: Too Many Requests"));
        assert!(!is_player_challenge(""));
    }

    #[test]
    fn parses_common_url_shapes() {
        for (url, id) in [
            ("https://youtu.be/W25_BbZCZD0?si=mrevVTlqybh9PXgl", "W25_BbZCZD0"),
            ("https://www.youtube.com/watch?v=W25_BbZCZD0", "W25_BbZCZD0"),
            ("https://www.youtube.com/watch?v=W25_BbZCZD0&t=123s", "W25_BbZCZD0"),
            ("https://www.youtube.com/shorts/W25_BbZCZD0", "W25_BbZCZD0"),
            ("https://www.youtube.com/embed/W25_BbZCZD0", "W25_BbZCZD0"),
        ] {
            assert_eq!(parse_youtube_url(url).unwrap().id, id, "{url}");
        }
        assert!(parse_youtube_url("https://example.com/not-youtube").is_err());
    }
}
