//! Offline end-to-end test: synthetic video -> frames -> dedupe -> PDF.
//!
//! Needs `ffmpeg` on PATH. No network. Verifies the whole local half of the
//! pipeline (the YouTube half is covered by unit tests + manual runs).

use std::path::{Path, PathBuf};
use std::process::Command;
use ytb2pdf_core::frames::{capture_frames, dedupe_frames};
use ytb2pdf_core::pdf::{build_pdf, PdfBlock, PdfMeta, PdfOptions};
use ytb2pdf_core::subtitles::parse_srt;

fn workdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ytb2pdf-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 6s video: 3s red with a white box on the left, 3s blue with a white
/// box on the right. (Flat solid colors would dHash identically — the boxes
/// give each scene internal structure, like real slides.)
fn make_test_video(out: &Path) {
    let st = Command::new("ffmpeg")
        .args(["-y", "-v", "error"])
        .args(["-f", "lavfi", "-i", "color=c=red:size=640x360:rate=10:duration=3,drawbox=x=40:y=40:w=120:h=120:c=white:t=fill"])
        .args(["-f", "lavfi", "-i", "color=c=blue:size=640x360:rate=10:duration=3,drawbox=x=480:y=200:w=120:h=120:c=white:t=fill"])
        .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0"])
        .args(["-pix_fmt", "yuv420p"])
        .arg(out)
        .status()
        .expect("ffmpeg not on PATH?");
    assert!(st.success() && out.exists(), "ffmpeg failed to make test video");
}

const SRT: &str = "1\n00:00:00,500 --> 00:00:01,500\nDoan mo dau\n\n\
                   2\n00:00:01,500 --> 00:00:02,500\nVan noi ve mau do\n\n\
                   3\n00:00:03,500 --> 00:00:04,500\nChuyen sang mau xanh\n\n\
                   4\n00:00:04,500 --> 00:00:05,500\nKet thuc mau xanh\n\n";

#[test]
fn offline_video_to_pdf() {
    let dir = workdir();
    let video = dir.join("test.mp4");
    make_test_video(&video);

    // subtitles
    let mut segs = parse_srt(SRT);
    assert_eq!(segs.len(), 4);

    // frames at segment midpoints: t=1,2 (red) and t=4,5 (blue)
    let stamps: Vec<f64> = segs.iter().map(|s| s.midpoint()).collect();
    let frame_dir = dir.join("frames");
    let frames = capture_frames(&video, &stamps, &frame_dir, 2, &|_, _| {}).unwrap();
    assert_eq!(frames.len(), 4);

    // dedupe: red+red -> 1, blue+blue -> 1
    let kept = dedupe_frames(frames, 6);
    assert_eq!(kept.len(), 2, "expected 2 distinct scenes");

    // pdf with one block per segment, images only on kept frames
    let mut ki = 0usize;
    let mut blocks = Vec::new();
    for seg in &segs {
        while ki + 1 < kept.len() && kept[ki + 1].at_secs <= seg.midpoint() + 0.5 {
            ki += 1;
        }
        blocks.push(PdfBlock {
            timestamp_secs: seg.start,
            deep_link: Some(format!("https://youtu.be/TEST?t={}s", seg.start as u64)),
            caption: seg.text.clone(),
            caption_translated: Some(format!("(dịch) {}", seg.text)),
            image: Some(kept[ki].path.clone()),
            merged: false,
        });
    }
    let meta = PdfMeta {
        title: "Video test offline".to_string(),
        uploader: "ytb2pdf test".to_string(),
        duration_secs: 6,
        source_url: Some("https://youtu.be/TEST".to_string()),
    };
    let pdf_path = dir.join("out.pdf");
    build_pdf(&meta, &blocks, &PdfOptions::default(), &pdf_path).unwrap();

    let size = std::fs::metadata(&pdf_path).unwrap().len();
    assert!(size > 10_000, "PDF suspiciously small: {size} bytes");
    println!("offline PDF ok: {} ({} bytes)", pdf_path.display(), size);

    // keep the workspace tidy
    let _ = std::fs::remove_dir_all(&dir);
    // silence unused-mut warning path
    let _ = &mut segs;
}


/// Exercise the public source-aware pipeline without yt-dlp or network access.
#[test]
fn local_video_pipeline_with_srt_produces_pdf() {
    use ytb2pdf_core::{run_pipeline, PipelineConfig};

    let dir = workdir().join("local-video-adapter");
    std::fs::create_dir_all(&dir).unwrap();
    let video = dir.join("lecture.mp4");
    let subtitles = dir.join("lecture.srt");
    let pdf = dir.join("lecture.pdf");
    make_test_video(&video);
    std::fs::write(&subtitles, SRT).unwrap();

    let mut cfg = PipelineConfig::default();
    cfg.local_file = Some(video);
    cfg.work_dir = dir.join("scratch");
    cfg.out_pdf = pdf.clone();
    // No explicit transcript: verify same-basename sidecar discovery.
    run_pipeline(&cfg, &|_| {}).expect("local file pipeline should succeed offline");

    assert!(pdf.is_file());
    assert!(std::fs::metadata(&pdf).unwrap().len() > 10_000);
    // Local PDFs should not inject fake YouTube link annotations.
    let document = lopdf::Document::load(&pdf).unwrap();
    for (_, id) in document.get_pages() {
        let page = document.get_object(id).unwrap().as_dict().unwrap();
        assert!(page.get(b"Annots").is_err(), "local PDF must have no URL annotations");
    }
    let _ = std::fs::remove_dir_all(dir);
}
