//! `ytb2pdf` — turn a YouTube lecture into a captioned, illustrated PDF.
//!
//! ```sh
//! ytb2pdf "https://youtu.be/W25_BbZCZD0" --to vi --out bai-giang.pdf
//! ytb2pdf "https://www.youtube.com/watch?v=..." --cookies-from-browser chrome
//! ```
//!
//! Needs `yt-dlp` and `ffmpeg` on PATH. Translation needs `GEMINI_API_KEY`.

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use ytb2pdf_core::{run_pipeline, v2, whisper::WhisperModel, PipelineConfig, PipelineEvent};

fn usage() -> &'static str {
    "usage: ytb2pdf <youtube-url|local-video-path> [--to <lang>] [--from <lang>] [--cookies <cookies.txt>] [--cookies-from-browser <name>] [--gemini-key <key>] [--whisper-model fast|medium] [--transcript <file.srt>]\n       [--work-dir <dir>] [--out <file.pdf>] [--json] [--quality <480|720|1080>] [--jobs <n>] [--v2]\n\n--json: emit one JSON object per line (stage/progress/log/done) for agent parsing.\n\nexamples:\n  ytb2pdf \"https://youtu.be/W25_BbZCZD0\" --to vi --cookies ~/Downloads/cookies.txt\n  ytb2pdf \"https://www.youtube.com/watch?v=abc\" --cookies-from-browser chrome --out out.pdf"
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("{}", usage());
        std::process::exit(2);
    }
    let mut cfg = PipelineConfig::default();
    let input = ytb2pdf_core::VideoInput::parse_cli(&args[1])?;
    input.validate()?;
    match input {
        ytb2pdf_core::VideoInput::YouTubeUrl(url) => cfg.url = url,
        ytb2pdf_core::VideoInput::LocalFile(file) => cfg.local_file = Some(file),
    }
    let mut json_out = false;
    let mut i = 2;
    while i < args.len() {
        let flag = |n: usize| -> Result<String> {
            args.get(n)
                .cloned()
                .ok_or_else(|| anyhow!("missing value after {}", args[n - 1]))
        };
        match args[i].as_str() {
            "--to" => {
                cfg.target_lang = Some(flag(i + 1)?);
                i += 2;
            }
            "--from" => {
                cfg.source_lang = flag(i + 1)?;
                i += 2;
            }
            "--cookies" => {
                cfg.cookies_file = Some(PathBuf::from(flag(i + 1)?));
                i += 2;
            }
            "--cookies-from-browser" => {
                cfg.cookies_from_browser = Some(flag(i + 1)?);
                i += 2;
            }
            "--gemini-key" => {
                cfg.gemini_api_key = Some(flag(i + 1)?);
                i += 2;
            }
            "--transcript" => {
                let p = std::path::PathBuf::from(flag(i + 1)?);
                if !p.is_file() {
                    anyhow::bail!("--transcript: file not found: {}", p.display());
                }
                cfg.transcript_file = Some(p);
                i += 2;
            }
            "--whisper-model" => {
                cfg.whisper_model = WhisperModel::from_name(&flag(i + 1)?)
                    .ok_or_else(|| anyhow!("--whisper-model must be fast|medium"))?;
                i += 2;
            }
            "--work-dir" => {
                cfg.work_dir = PathBuf::from(flag(i + 1)?);
                i += 2;
            }
            "--out" => {
                cfg.out_pdf = PathBuf::from(flag(i + 1)?);
                i += 2;
            }
            "--json" => {
                json_out = true;
                i += 1;
            }
            "--quality" => {
                cfg.max_height = flag(i + 1)?
                    .parse()
                    .map_err(|_| anyhow!("--quality must be a height like 480, 720 or 1080"))?;
                i += 2;
            }
            "--jobs" => {
                cfg.jobs = flag(i + 1)?
                    .parse()
                    .map_err(|_| anyhow!("--jobs must be a positive integer"))?;
                i += 2;
            }
            "--v2" => {
                cfg.v2 = true;
                i += 1;
            }
            other => {
                eprintln!("unknown flag: {other}\n{}", usage());
                std::process::exit(2);
            }
        }
    }
    if cfg.out_pdf.as_os_str() == "output.pdf" {
        // default the filename from the video id when the user didn't choose
        if let Some(file) = &cfg.local_file {
            cfg.out_pdf = PathBuf::from(format!("{}.pdf", file.file_stem().unwrap_or_default().to_string_lossy()));
        } else if let Ok(v) = ytb2pdf_core::youtube::parse_youtube_url(&cfg.url) {
            cfg.out_pdf = PathBuf::from(format!("{}.pdf", v.id));
        }
    }

    if cfg.v2 && cfg.local_file.is_none() {
        v2::pipeline::run_v2_pipeline(&cfg, &|ev| emit_event(ev, json_out))?;
    } else {
        run_pipeline(&cfg, &|ev| emit_event(ev, json_out))?;
    }
    Ok(())
}

/// Emit a pipeline event as JSON (one object per line) or human-readable text.
fn emit_event(ev: PipelineEvent, json_out: bool) {
    if json_out {
        let v = match ev {
            PipelineEvent::Stage(s) => {
                serde_json::json!({"type": "stage", "stage": s})
            }
            PipelineEvent::Progress { done, total } => {
                serde_json::json!({"type": "progress", "done": done, "total": total})
            }
            PipelineEvent::Log(m) => {
                serde_json::json!({"type": "log", "message": m})
            }
            PipelineEvent::Done(p) => {
                serde_json::json!({"type": "done", "pdf": p.to_string_lossy()})
            }
        };
        println!("{}", v);
        return;
    }
    match ev {
        PipelineEvent::Stage(s) => println!("\n▶ {s}"),
        PipelineEvent::Progress { done, total } => {
            print!("\r  {done}/{total}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            if done == total {
                println!();
            }
        }
        PipelineEvent::Log(m) => println!("  {m}"),
        PipelineEvent::Done(p) => println!("\nPDF created successfully: {}", p.display()),
    }
}
