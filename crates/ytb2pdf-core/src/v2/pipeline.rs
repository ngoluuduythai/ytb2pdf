//! Architecture V2 pipeline: low-res detection, high-res extraction.
//!
//! Flow:
//! 1. Parse URL + metadata (same as V1)
//! 2. Load captions (same as V1)
//! 3. Download 240p proxy
//! 4. Detect scenes on proxy (ffmpeg filter + periodic sampling)
//! 5. Plan download segments (merge nearby scenes)
//! 6. Download segments at high resolution (parallel, resumable)
//! 7. Extract one frame per scene from its segment
//! 8. Map each caption to its nearest scene frame
//! 9. Build PDF blocks and generate PDF (same as V1)

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::frames::{dhash, extract_frame, Frame};
use crate::pdf::{self, PdfBlock, PdfMeta, PdfOptions};
use crate::pipeline::{PipelineConfig, PipelineEvent};
use crate::subtitles;
use crate::v2::{hi_res, mapping, proxy, scene, segments};
use crate::youtube::{self, YtDlp};

/// Run the V2 pipeline. Returns the output PDF path.
pub fn run_v2_pipeline(
    cfg: &PipelineConfig,
    emit: &(dyn Fn(PipelineEvent) + Sync),
) -> Result<PathBuf> {
    let log = |s: String| emit(PipelineEvent::Log(s));

    emit(PipelineEvent::Stage("Phân tích URL (V2)"));
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
        max_height: cfg.max_height,
        retries: 5,
    };

    emit(PipelineEvent::Stage("Lấy thông tin video"));
    let meta = ytdlp.metadata(&video).map_err(|e| {
        anyhow::anyhow!("{e}\n\nGợi ý: đây là bot-check của YouTube với IP này.")
    })?;
    log(format!(
        "\"{}\" - {} ({})",
        meta.title,
        meta.uploader,
        pdf::format_duration(meta.duration_secs)
    ));

    let work_dir = cfg.work_dir.join("v2");
    std::fs::create_dir_all(&work_dir).context("creating v2 work dir")?;
    let seg_dir = work_dir.join("segments");
    let frame_dir = work_dir.join(format!("frames-v2-{}", video.id));
    std::fs::create_dir_all(&seg_dir)?;
    std::fs::create_dir_all(&frame_dir)?;

    // ---- captions (same as V1) ----
    emit(PipelineEvent::Stage("Tải caption"));
    let segments: Vec<subtitles::Segment> = if let Some(srt_path) = &cfg.transcript_file {
        log(format!(
            "Dùng transcript do người dùng cung cấp: {}",
            srt_path.display()
        ));
        subtitles::load_subtitles_file(srt_path)?
    } else {
        match ytdlp.download_subtitles(&video, &cfg.source_lang, &work_dir.join("subs")) {
            Ok(f) => subtitles::load_subtitles_file(&f)?,
            Err(e) => return Err(anyhow::anyhow!("V2 cần caption: {e}")),
        }
    };
    log(format!("{} đoạn caption", segments.len()));

    // ---- 1. proxy download ----
    emit(PipelineEvent::Stage("Tải proxy 240p (V2)"));
    let proxy_path = work_dir.join(format!("{}_proxy.mp4", video.id));
    let proxy_path = proxy::download_proxy(&ytdlp, &video, &proxy_path)?;
    log(format!("Proxy: {}", proxy_path.display()));

    // ---- 2. scene detection ----
    // Sample every 30s as fallback; filter catches the real cuts.
    emit(PipelineEvent::Stage("Phát hiện scene (V2)"));
    let scenes = scene::detect_scenes(&proxy_path, 0.30, 30.0)?;
    log(format!("Phát hiện {} scene", scenes.len()));
    if scenes.is_empty() {
        anyhow::bail!("không phát hiện được scene nào — thử lại với V1");
    }

    // ---- 3. segment planning ----
    emit(PipelineEvent::Stage("Lập kế hoạch download (V2)"));
    let seg_plans = segments::plan_segments(&scenes, 10.0, 2.0);
    log(format!("Gộp thành {} segment", seg_plans.len()));

    // ---- 4. high-res segment download ----
    emit(PipelineEvent::Stage("Tải segment chất lượng cao (V2)"));
    let total_seg = seg_plans.len();
    let seg_paths = hi_res::download_segments(
        &ytdlp,
        &video,
        &seg_plans,
        &seg_dir,
        cfg.max_height,
        cfg.jobs,
    )?;
    // (progress is per-segment inside download_segments; emit summary)
    emit(PipelineEvent::Progress {
        done: total_seg,
        total: total_seg,
    });

    // ---- 5. extract one frame per scene ----
    emit(PipelineEvent::Stage("Trích frame theo scene (V2)"));
    let total_scenes = scenes.len();
    let scene_frames = extract_scene_frames(&seg_plans, &seg_paths, &scenes, &frame_dir, &|done| {
        emit(PipelineEvent::Progress {
            done,
            total: total_scenes,
        });
    })?;
    log(format!("Trích {} frame", scene_frames.len()));

    // ---- 6. map captions to scenes ----
    let midpoints: Vec<f64> = segments.iter().map(|s| s.midpoint()).collect();
    let mapped = mapping::map_captions_to_scenes(&midpoints, &scenes);

    // Build a lookup: scene_time -> frame index
    let mut scene_to_frame: std::collections::HashMap<u64, usize> =
        std::collections::HashMap::new();
    for (idx, sf) in scene_frames.iter().enumerate() {
        scene_to_frame.insert(sf.at_secs.to_bits(), idx);
    }

    // ---- 7. build PDF blocks (same compacting logic as V1) ----
    emit(PipelineEvent::Stage("Dựng PDF (V2)"));
    let mut blocks: Vec<PdfBlock> = Vec::new();
    let mut i = 0;
    while i < mapped.len() {
        // Group consecutive captions assigned to the same scene frame.
        let mut j = i;
        while j + 1 < mapped.len()
            && (mapped[j + 1].scene_time.to_bits() == mapped[i].scene_time.to_bits())
        {
            j += 1;
        }
        let first = &segments[mapped[i].caption_idx];
        let caption =
            pdf::merge_captions(segments[mapped[i].caption_idx..=mapped[j].caption_idx]
                .iter()
                .map(|s| s.text.as_str()));
        let frame_idx = scene_to_frame
            .get(&mapped[i].scene_time.to_bits())
            .copied();
        let image_path = frame_idx
            .and_then(|fi| scene_frames.get(fi))
            .map(|f| f.path.clone());
        blocks.push(PdfBlock {
            timestamp_secs: first.start,
            deep_link: Some(format!("https://youtu.be/{}?t={}s", video.id, first.start as u64)),
            caption,
            caption_translated: None, // V2 skips translation for now
            image: image_path,
            merged: j > i,
        });
        i = j + 1;
    }
    log(format!(
        "Gộp {} đoạn caption thành {} scene (V2)",
        segments.len(),
        blocks.len()
    ));

    // Shrink frames for reasonable PDF size.
    for b in &blocks {
        if let Some(p) = &b.image {
            let _ = pdf::shrink_image(p, 1920);
        }
    }

    let pdf_meta = PdfMeta {
        title: meta.title.clone(),
        uploader: meta.uploader.clone(),
        duration_secs: meta.duration_secs,
        source_url: Some(video.url.clone()),
    };
    let pdf_opts = PdfOptions { text_only: cfg.text_only, ..PdfOptions::default() };
    pdf::build_pdf(&pdf_meta, &blocks, &pdf_opts, &cfg.out_pdf)?;
    log(format!("PDF: {}", cfg.out_pdf.display()));
    let out = cfg.out_pdf.clone();
    emit(PipelineEvent::Done(out.clone()));
    Ok(out)
}

/// Extract one frame per scene from its containing segment.
/// Scenes whose frames can't be extracted (e.g. beyond the segment's
/// actual content) are skipped with a warning instead of failing.
fn extract_scene_frames(
    seg_plans: &[segments::Segment],
    seg_paths: &[PathBuf],
    scenes: &[scene::Scene],
    frame_dir: &PathBuf,
    on_one: &dyn Fn(usize),
) -> Result<Vec<Frame>> {
    std::fs::create_dir_all(frame_dir)?;
    let mut frames = Vec::with_capacity(scenes.len());
    let mut skipped = 0;
    for (idx, sc) in scenes.iter().enumerate() {
        // Find the segment containing this scene time.
        let seg_idx = match seg_plans
            .iter()
            .position(|s| sc.time >= s.start && sc.time <= s.end)
        {
            Some(i) => i,
            None => {
                skipped += 1;
                continue;
            }
        };
        let seg_path = &seg_paths[seg_idx];
        let offset = (sc.time - seg_plans[seg_idx].start).max(0.0);
        let out = frame_dir.join(format!("v2_scene_{idx:05}.jpg"));
        if !out.exists() {
            // Don't fail the whole pipeline on one bad scene.
            if extract_frame(seg_path, offset, &out).is_err() {
                skipped += 1;
                let _ = std::fs::remove_file(&out);
                continue;
            }
        }
        let img = match image::open(&out) {
            Ok(img) => img,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        frames.push(Frame {
            at_secs: sc.time,
            path: out,
            hash: dhash(&img),
        });
        on_one(idx + 1 - skipped);
    }
    if skipped > 0 {
        eprintln!("warning: skipped {skipped} scenes (frame extraction failed)");
    }
    Ok(frames)
}
