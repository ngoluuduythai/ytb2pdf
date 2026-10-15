use std::path::{Path, PathBuf};

pub const LANGUAGES: &[(&str, &str)] = &[
    ("vi", "Vietnamese"),
    ("en", "English"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("zh", "Chinese"),
    ("fr", "French"),
    ("de", "German"),
    ("es", "Spanish"),
];

pub fn validate_url(url: &str) -> Result<(), &'static str> {
    // The core parser extracts IDs; also validate the host before accepting input.
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))
        .ok_or("Paste a full YouTube video URL, starting with https://.")?;
    let host = rest.split('/').next().unwrap_or("");
    if !matches!(
        host,
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be" | "www.youtu.be"
    ) {
        return Err("Use a youtube.com or youtu.be video link.");
    }
    ytb2pdf_core::youtube::parse_youtube_url(url)
        .map(|_| ()).map_err(|_| "This link does not contain a valid video ID. Paste a video link rather than a channel or playlist.")
}

pub fn validate_output(path: &Path) -> Result<(), &'static str> {
    if path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("pdf"))
        != Some(true)
    {
        return Err("Choose a filename ending in .pdf.");
    }
    if path.exists() {
        return Err(
            "This filename already exists. Choose a different name to keep the existing file.",
        );
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err("The output folder does not exist. Choose another folder.");
    }
    Ok(())
}

pub fn default_folder() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    [home.join("Desktop"), home.join("Documents"), home.clone()]
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or(home)
}

pub fn available_output(folder: &Path, stem: &str) -> PathBuf {
    let name: String = stem
        .chars()
        .take(80)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim_matches([' ', '.']);
    let name = if name.is_empty() { "video" } else { name };
    let base = format!("ytb2pdf-{name}");
    let mut path = folder.join(format!("{base}.pdf"));
    let mut index = 2;
    while path.exists() {
        path = folder.join(format!("{base}-{index}.pdf"));
        index += 1;
    }
    path
}

pub fn error_guidance(error: &str) -> (&'static str, &'static str) {
    let lower = error.to_lowercase();
    if lower.contains("pdfunite") || lower.contains("poppler") {
        ("The PDF merging tool is unavailable", "Update to the complete app release, which includes Poppler for long documents. For a standalone CLI installation, install Poppler and try again.")
    } else if lower.contains("429") || lower.contains("too many requests") {
        ("YouTube is limiting requests", "Wait before trying again. You can select a fresh cookies.txt file under Advanced. Repeated retries may extend the limit.")
    } else if lower.contains("403") || lower.contains("sign in") || lower.contains("bot") {
        ("YouTube needs authentication", "Select a cookies.txt file exported while signed in to YouTube under Advanced, then try again.")
    } else if lower.contains("api key")
        || lower.contains("401")
        || lower.contains("gemini")
        || lower.contains("translat")
    {
        ("Translation could not finish", "Check your Gemini API key and quota, or turn off Translate captions to keep the original language.")
    } else if lower.contains("ffmpeg")
        || lower.contains("ffprobe")
        || lower.contains("yt-dlp")
        || lower.contains("whisper-cli")
    {
        ("A media tool could not finish", "Check processing details for the tool involved. Make sure it is installed and the video format is supported. The official macOS package includes media tools.")
    } else if lower.contains("subtitle")
        || lower.contains("caption")
        || lower.contains("transcript")
    {
        ("Captions could not be read", "Select an SRT/VTT subtitle file, or use standard processing for automatic speech recognition when captions are unavailable.")
    } else if lower.contains("save pdf")
        || lower.contains("output folder")
        || lower.contains("permission denied")
        || lower.contains("no space")
    {
        ("The PDF could not be saved", "Choose a writable output folder and a new filename. Check that there is enough free disk space.")
    } else {
        (
            "Conversion could not finish",
            "Check processing details below. Verify your source and settings, then try again.",
        )
    }
}

pub fn duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!(
            "{}h {:02}m {:02}s",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_lookalike_hosts_and_playlist_links() {
        assert!(validate_url("https://youtube.com/watch?v=W25_BbZCZD0").is_ok());
        assert!(validate_url("https://youtu.be/W25_BbZCZD0?t=2").is_ok());
        assert!(validate_url("https://evil.test/youtube.com/watch?v=W25_BbZCZD0").is_err());
        assert!(validate_url("https://youtube.com.evil.test/watch?v=W25_BbZCZD0").is_err());
        assert!(validate_url("https://youtube.com/playlist?list=123").is_err());
    }
    #[test]
    fn output_names_preserve_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let first = available_output(dir.path(), "lesson");
        std::fs::write(&first, b"existing").unwrap();
        assert!(validate_output(&first).is_err());
        assert_eq!(
            available_output(dir.path(), "lesson"),
            dir.path().join("ytb2pdf-lesson-2.pdf")
        );
        assert_eq!(std::fs::read(first).unwrap(), b"existing");
        assert!(validate_output(&dir.path().join("missing/file.pdf")).is_err());
    }
}
