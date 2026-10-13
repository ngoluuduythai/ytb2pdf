//! The end-to-end pipeline: URL -> PDF.
//!
//! [`run_pipeline`] executes the stages in order and reports progress through
//! the `emit` callback, so both the CLI and the egui app can render it.

use crate::frames::{capture_frames, dedupe_frames};
use crate::pdf::{self, PdfBlock, PdfMeta, PdfOptions};
use crate::subtitles::{self, Segment};
use crate::translate::{GeminiTranslator, NoopTranslator, Translator};
use crate::youtube::{self, YtDlp};
use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub url: String,
    /// Caption language to fetch, e.g. `"en"`.
    pub source_lang: String,
    /// Translate captions into this language, e.g. `Some("vi")`. `None` = keep original.
    pub target_lang: Option<String>,
    /// Cookie file (Netscape format) passed to yt-dlp as `--cookies` (anti-403/429).
    pub cookies_file: Option<PathBuf>,
    /// Passed to yt-dlp as `--cookies-from-browser` (anti-403). Used only
    /// when `cookies_file` is not set.
    pub cookies_from_browser: Option<String>,
    /// Scratch dir for video / subtitles / frames.
    pub work_dir: PathBuf,
    /// Final PDF destination.
    pub out_pdf: PathBuf,
    /// dHash hamming-distance threshold for frame dedupe (default 6).
    pub dhash_threshold: u32,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            source_lang: "en".to_string(),
            target_lang: None,
            cookies_file: None,
            cookies_from_browser: None,
            work_dir: PathBuf::from("ytb2pdf-work"),
            out_pdf: PathBuf::from("output.pdf"),
            dhash_threshold: 6,
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

pub fn run_pipeline(cfg: &PipelineConfig, emit: &dyn Fn(PipelineEvent)) -> Result<PathBuf> {
    let log = |s: String| emit(PipelineEvent::Log(s));

    emit(PipelineEvent::Stage("Phân tích URL"));
    let video = youtube::parse_youtube_url(&cfg.url)?;
    log(format!("Video id: {}", video.id));

    if let Some(f) = &cfg.cookies_file {
        if !f.exists() {
            anyhow::bail!("không tìm thấy file cookie: {}", f.display());
        }
    }

    let ytdlp = YtDlp {
        cookies_file: cfg.cookies_file.clone(),
        cookies_from_browser: cfg.cookies_from_browser.clone(),
        retries: 5,
    };

    emit(PipelineEvent::Stage("Lấy thông tin video"));
    let meta = ytdlp.metadata(&video)?;
    log(format!("\"{}\" - {} ({})", meta.title, meta.uploader,
        pdf::format_duration(meta.duration_secs)));

    std::fs::create_dir_all(&cfg.work_dir).context("creating work dir")?;
    let sub_dir = cfg.work_dir.join("subs");
    let frame_dir = cfg.work_dir.join("frames");
    std::fs::create_dir_all(&sub_dir)?;
    std::fs::create_dir_all(&frame_dir)?;

    emit(PipelineEvent::Stage("Tải caption"));
    let sub_file = ytdlp.download_subtitles(&video, &cfg.source_lang, &sub_dir).map_err(|e| {
        anyhow::anyhow!(
            "{e}\n\nGợi ý: YouTube đang giới hạn IP này (429/403). Hãy điền cookie từ browser \
             vào ô \"Cookie từ browser\" (vd: chrome) — request kèm session đăng nhập thường \
             thoát được giới hạn này — rồi bấm Tạo PDF lại. Hoặc đợi vài phút cho hết throttle."
        )
    })?;
    log(format!("Caption: {}", sub_file.display()));

    emit(PipelineEvent::Stage("Chuẩn hoá caption"));
    let segments: Vec<Segment> = subtitles::load_subtitles_file(&sub_file)?;
    log(format!("{} đoạn caption", segments.len()));

    // ---- translation (optional) ----
    let translated: Vec<Option<String>> = match &cfg.target_lang {
        Some(target) if target != &cfg.source_lang => {
            emit(PipelineEvent::Stage("Dịch caption"));
            let translator: Box<dyn Translator> = match GeminiTranslator::from_env() {
                Ok(g) => Box::new(g),
                Err(e) => {
                    log(format!("Cảnh báo: {e} - bỏ qua bước dịch, giữ caption gốc"));
                    Box::new(NoopTranslator)
                }
            };
            let src: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();
            let total = src.len();
            let mut out = Vec::with_capacity(total);
            for (i, chunk) in src.chunks(20).enumerate() {
                let t = translator.translate(chunk, target)?;
                out.extend(t.into_iter().map(Some));
                emit(PipelineEvent::Progress { done: ((i + 1) * 20).min(total), total });
            }
            out
        }
        _ => vec![None; segments.len()],
    };

    emit(PipelineEvent::Stage("Tải video (720p)"));
    let video_path = cfg.work_dir.join(format!("{}.mp4", video.id));
    let video_path = ytdlp.download_video(&video, &video_path)?;
    log(format!("Video: {}", video_path.display()));

    emit(PipelineEvent::Stage("Chụp frame theo caption"));
    let timestamps: Vec<f64> = segments.iter().map(|s| s.midpoint()).collect();
    let total = timestamps.len();
    let frames = capture_frames(&video_path, &timestamps, &frame_dir, &|done, _| {
        emit(PipelineEvent::Progress { done, total });
    })?;

    emit(PipelineEvent::Stage("Loại frame trùng"));
    let kept = dedupe_frames(frames, cfg.dhash_threshold);
    log(format!("Giữ {} / {} frame", kept.len(), total));

    // shrink kept frames so the PDF stays small
    for f in &kept {
        let _ = pdf::shrink_image(&f.path, 1280);
    }

    emit(PipelineEvent::Stage("Dựng PDF"));
    // Each segment shows the latest kept frame at or before its midpoint
    // (slides stay on screen while the speaker keeps talking).
    let mut blocks = Vec::with_capacity(segments.len());
    let mut ki = 0usize;
    for (seg, vi) in segments.iter().zip(translated.iter()) {
        while ki + 1 < kept.len() && kept[ki + 1].at_secs <= seg.midpoint() + 0.5 {
            ki += 1;
        }
        let image = if kept.get(ki).map(|f| f.at_secs <= seg.midpoint() + 0.5).unwrap_or(false) {
            Some(kept[ki].path.clone())
        } else {
            None
        };
        blocks.push(PdfBlock {
            timestamp_secs: seg.start,
            deep_link: format!("https://youtu.be/{}?t={}s", video.id, seg.start as u64),
            caption: seg.text.clone(),
            caption_translated: vi.clone(),
            image,
        });
    }

    let pdf_meta = PdfMeta {
        title: meta.title.clone(),
        uploader: meta.uploader.clone(),
        duration_secs: meta.duration_secs,
        source_url: video.url.clone(),
    };
    pdf::build_pdf(&pdf_meta, &blocks, &PdfOptions::default(), &cfg.out_pdf)?;
    log(format!("PDF: {}", cfg.out_pdf.display()));

    emit(PipelineEvent::Done(cfg.out_pdf.clone()));
    Ok(cfg.out_pdf.clone())
}
