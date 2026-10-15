//! The end-to-end pipeline: URL -> PDF.
//!
//! [`run_pipeline`] executes the stages in order and reports progress through
//! the `emit` callback, so both the CLI and the egui app can render it.

use crate::frames::{capture_frames, dedupe_frames};
use crate::local_video;
use crate::pdf::{self, PdfBlock, PdfMeta, PdfOptions};
use crate::subtitles::{self, Segment};
use crate::translate::{GeminiTranslator, Translator};
use crate::whisper;
use crate::youtube::{self, YtDlp};
use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub url: String,
    /// Local file input. When set, the YouTube URL is ignored.
    pub local_file: Option<PathBuf>,
    /// Caption language to fetch, e.g. `"en"`.
    pub source_lang: String,
    /// Translate captions into this language, e.g. `Some("vi")`. `None` = keep original.
    pub target_lang: Option<String>,
    /// Gemini API key for translation. If `None`, falls back to `GEMINI_API_KEY` env var.
    pub gemini_api_key: Option<String>,
    /// Cookie file (Netscape format) passed to yt-dlp as `--cookies` (anti-403/429).
    pub cookies_file: Option<PathBuf>,
    /// Whisper model used when the video has no YouTube captions.
    pub whisper_model: whisper::WhisperModel,
    /// User-supplied transcript (.srt/.vtt). When set, YouTube caption
    /// download AND the Whisper fallback are both skipped — the file is
    /// used directly as the caption segments.
    pub transcript_file: Option<PathBuf>,
    /// Passed to yt-dlp as `--cookies-from-browser` (anti-403). Used only
    /// when `cookies_file` is not set.
    pub cookies_from_browser: Option<String>,
    /// Scratch dir for video / subtitles / frames.
    pub work_dir: PathBuf,
    /// Final PDF destination.
    pub out_pdf: PathBuf,
    /// dHash hamming-distance threshold for frame dedupe (default 6).
    pub dhash_threshold: u32,
    /// Max video height for download (default 720). Use 480 for very long
    /// videos to keep the download smaller and survive YouTube throttling.
    pub max_height: u32,
    /// Parallel ffmpeg workers for frame capture (default: CPU count).
    /// Higher = faster on many-core machines; 2-4 is plenty.
    pub jobs: usize,
    /// Use Architecture V2 (low-res detection, high-res extraction).
    /// Best for long videos where download bandwidth is the bottleneck.
    pub v2: bool,
    /// Skip images, captions only. For lightweight mobile reading.
    pub text_only: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            local_file: None,
            source_lang: "en".to_string(),
            target_lang: None,
            gemini_api_key: None,
            whisper_model: whisper::WhisperModel::default(),
            transcript_file: None,
            cookies_file: None,
            cookies_from_browser: None,
            work_dir: PathBuf::from("ytb2pdf-work"),
            out_pdf: PathBuf::from("output.pdf"),
            dhash_threshold: 6,
            max_height: 720,
            jobs: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(2),
            v2: false,
            text_only: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    Stage(&'static str),
    Progress { done: usize, total: usize },
    Log(String),
    Done(PathBuf),
}

pub fn run_pipeline(
    cfg: &PipelineConfig,
    emit: &(dyn Fn(PipelineEvent) + Sync),
) -> Result<PathBuf> {
    let scoped = crate::resume::scoped_config(cfg)?;
    let cfg = &scoped;
    let _lock = crate::resume::lock_workspace(cfg)?;
    if let Some(path) = crate::resume::try_pdf(cfg, emit)? {
        return Ok(path);
    }

    if let Some(path) = &cfg.local_file {
        return local_video::run_local_pipeline(cfg, path, cfg.v2, emit);
    }
    let log = |s: String| emit(PipelineEvent::Log(s));

    emit(PipelineEvent::Stage("Parsing URL"));
    let video = youtube::parse_youtube_url(&cfg.url)?;
    log(format!("Video id: {}", video.id));

    if let Some(f) = &cfg.cookies_file {
        if !f.exists() {
            anyhow::bail!("Cookie file not found: {}", f.display());
        }
    }

    let ytdlp = YtDlp {
        cookies_file: cfg.cookies_file.clone(),
        cookies_from_browser: cfg.cookies_from_browser.clone(),
        max_height: cfg.max_height,
        retries: 5,
    };

    emit(PipelineEvent::Stage("Fetching video information"));
    let meta = crate::resume::stage(cfg, "metadata", emit, || {
        ytdlp.metadata(&video).map_err(|e| {
        anyhow::anyhow!(
            "{e}\n\nTip: YouTube may be challenging requests from this IP address. Select a cookies.txt file \
             (using the Cookie field in the app; export while signed in to youtube.com) and retry; \
             the app has already retried without the web player client. If it still fails, wait for rate limiting to subside."
        )
    })
    })?;
    log(format!(
        "\"{}\" - {} ({})",
        meta.title,
        meta.uploader,
        pdf::format_duration(meta.duration_secs)
    ));

    std::fs::create_dir_all(&cfg.work_dir).context("creating work dir")?;
    let sub_dir = cfg.work_dir.join("subs");
    let frame_dir = cfg.work_dir.join(format!("frames-{}", video.id));
    std::fs::create_dir_all(&sub_dir)?;
    std::fs::create_dir_all(&frame_dir)?;

    emit(PipelineEvent::Stage("Downloading captions"));
    let segments: Vec<Segment> = crate::resume::stage(cfg, "captions", emit, || {
        let segments = if let Some(srt_path) = &cfg.transcript_file {
            log(format!(
                "Using user-provided transcript: {} — skipping YouTube captions and Whisper",
                srt_path.display()
            ));
            emit(PipelineEvent::Stage("Normalizing captions"));
            subtitles::load_subtitles_file(srt_path)?
        } else {
            match ytdlp.download_subtitles(&video, &cfg.source_lang, &sub_dir) {
            Ok(sub_file) => {
                log(format!("Caption: {}", sub_file.display()));
                emit(PipelineEvent::Stage("Normalizing captions"));
                subtitles::load_subtitles_file(&sub_file)?
            }
            // No captions on YouTube at all -> Whisper fallback:
            // download the audio track and transcribe locally with whisper.cpp.
            Err(e) if e.to_string().contains("no subtitles found") => {
                log("YouTube captions unavailable — falling back to Whisper".to_string());
                emit(PipelineEvent::Stage("Downloading audio"));
                let audio_path = cfg.work_dir.join(format!("{}.m4a", video.id));
                ytdlp.download_audio(&video, &audio_path)?;
                log(format!("Audio: {}", audio_path.display()));
                emit(PipelineEvent::Stage("Speech recognition (Whisper)"));
                log(format!(
                    "Running whisper.cpp (model {}) — this may take several minutes…",
                    cfg.whisper_model.label()
                ));
                whisper::transcribe_audio(
                    &audio_path,
                    &cfg.work_dir,
                    cfg.whisper_model,
                    &cfg.source_lang,
                    &log,
                )?
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "{e}\n\nTip: For HTTP 429/403 or bot challenges, select a cookies.txt file \
                     (exported while signed in to youtube.com), then retry generating the PDF. An authenticated session \
                     may help with IP-based restrictions."
                ))
            }
        }
        };
        Ok(segments)
    })?;
    log(format!("{} caption segments", segments.len()));

    let translated = translate_segments(cfg, &segments, emit)?;

    // Text-only: skip video download entirely (no frames needed).
    let video_path = if cfg.text_only {
        log("Text-only mode: skipping video download".to_string());
        cfg.work_dir.join(format!("{}.mp4", video.id))
    } else {
        emit(PipelineEvent::Stage("Downloading video (720p)"));
        let vp = cfg.work_dir.join(format!("{}.mp4", video.id));
        let vp = ytdlp.download_video(&video, &vp)?;
        log(format!("Video: {}", vp.display()));
        vp
    };

    emit(PipelineEvent::Stage(
        "Extracting frames at caption timestamps",
    ));
    let timestamps: Vec<f64> = segments.iter().map(|s| s.midpoint()).collect();
    let total = timestamps.len();
    // Text-only mode: skip frame capture entirely (no images needed).
    let kept = if cfg.text_only {
        log("Text-only mode: skipping frame capture".to_string());
        Vec::new()
    } else {
        let frames = capture_frames(
            &video_path,
            &timestamps,
            &frame_dir,
            cfg.jobs,
            &|done, _| {
                emit(PipelineEvent::Progress { done, total });
            },
        )?;

        emit(PipelineEvent::Stage("Removing duplicate frames"));
        let kept = dedupe_frames(frames, cfg.dhash_threshold);
        emit(PipelineEvent::Log(format!(
            "Kept {} / {} frames",
            kept.len(),
            total
        )));
        kept
    };
    let pdf_meta = PdfMeta {
        title: meta.title.clone(),
        uploader: meta.uploader.clone(),
        duration_secs: meta.duration_secs,
        source_url: Some(video.url.clone()),
    };
    return render_frames_pdf(cfg, &pdf_meta, &segments, &translated, &kept, emit);
}

pub fn translate_segments(
    cfg: &PipelineConfig,
    segments: &[Segment],
    emit: &(dyn Fn(PipelineEvent) + Sync),
) -> Result<Vec<Option<String>>> {
    let translated: Vec<Option<String>> = match &cfg.target_lang {
        Some(target) if target != &cfg.source_lang => {
            emit(PipelineEvent::Stage("Translating captions"));
            let translator: Box<dyn Translator> = match &cfg.gemini_api_key {
                Some(k) if !k.trim().is_empty() => Box::new(GeminiTranslator::new(k.trim())),
                _ => Box::new(GeminiTranslator::from_env()?),
            };
            let src: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();
            let total = src.len();
            let mut out = Vec::with_capacity(total);
            for (i, chunk) in src.chunks(20).enumerate() {
                let name = format!(
                    "translation-{i}-{}",
                    crate::resume::digest(&serde_json::to_vec(chunk)?)
                );
                let t: Vec<String> =
                    crate::resume::stage(cfg, &name, emit, || translator.translate(chunk, target))?;
                out.extend(t.into_iter().map(Some));
                emit(PipelineEvent::Progress {
                    done: ((i + 1) * 20).min(total),
                    total,
                });
            }
            out
        }
        _ => vec![None; segments.len()],
    };
    Ok(translated)
}

pub fn render_frames_pdf(
    cfg: &PipelineConfig,
    meta: &PdfMeta,
    segments: &[Segment],
    translated: &[Option<String>],
    kept: &[crate::frames::Frame],
    emit: &(dyn Fn(PipelineEvent) + Sync),
) -> Result<PathBuf> {
    // shrink kept frames so the PDF stays reasonable; 1920px stays crisp
    // when rendered large on the landscape pages
    for f in kept {
        let _ = pdf::shrink_image(&f.path, 1920);
    }

    emit(PipelineEvent::Stage("Generating PDF"));
    // Each segment shows the latest kept frame at or before its midpoint
    // (slides stay on screen while the speaker keeps talking).
    let mut seg_frame: Vec<Option<usize>> = Vec::with_capacity(segments.len());
    {
        let mut ki = 0usize;
        for seg in segments.iter() {
            while ki + 1 < kept.len() && kept[ki + 1].at_secs <= seg.midpoint() + 0.5 {
                ki += 1;
            }
            seg_frame.push(
                if kept
                    .get(ki)
                    .map(|f| f.at_secs <= seg.midpoint() + 0.5)
                    .unwrap_or(false)
                {
                    Some(ki)
                } else {
                    None
                },
            );
        }
    }

    // Compact: consecutive segments sharing the same kept frame (i.e. the
    // same on-screen scene, >90% similar by dHash) become ONE block with a
    // single image and the merged narration as a flowing paragraph.
    // Text-only: no frames, so each segment is its own block.
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < segments.len() {
        let mut j = i;
        // Only merge by frame when we have frames (not text-only).
        if !kept.is_empty() {
            while j + 1 < segments.len() && seg_frame[j + 1] == seg_frame[i] {
                j += 1;
            }
        }
        let first = &segments[i];
        let caption = pdf::merge_captions(segments[i..=j].iter().map(|s| s.text.as_str()));
        let vi_merged = pdf::merge_captions(translated[i..=j].iter().filter_map(|o| o.as_deref()));
        let caption_translated = if vi_merged.is_empty() || vi_merged == caption {
            None
        } else {
            Some(vi_merged)
        };
        blocks.push(PdfBlock {
            timestamp_secs: first.start,
            deep_link: meta.source_url.as_ref().map(|url| {
                format!(
                    "{}{}t={}s",
                    url,
                    if url.contains('?') { "&" } else { "?" },
                    first.start as u64
                )
            }),
            caption,
            caption_translated,
            image: seg_frame[i]
                .and_then(|k| kept.get(k))
                .map(|f| f.path.clone()),
            merged: j > i,
        });
        i = j + 1;
    }
    emit(PipelineEvent::Log(format!(
        "Grouped {} caption segments into {} scenes",
        segments.len(),
        blocks.len()
    )));

    let pdf_opts = PdfOptions {
        text_only: cfg.text_only,
        ..PdfOptions::default()
    };
    crate::resume::save_and_render(cfg, meta, &blocks, &pdf_opts, emit)
}
