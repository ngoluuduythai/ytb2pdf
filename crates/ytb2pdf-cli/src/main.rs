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
use ytb2pdf_core::{run_pipeline, PipelineConfig, PipelineEvent};

fn usage() -> &'static str {
    "usage: ytb2pdf <youtube-url> [--to <lang>] [--from <lang>] [--cookies <cookies.txt>] [--cookies-from-browser <name>]\n       [--work-dir <dir>] [--out <file.pdf>]\n\nexamples:\n  ytb2pdf \"https://youtu.be/W25_BbZCZD0\" --to vi --cookies ~/Downloads/cookies.txt\n  ytb2pdf \"https://www.youtube.com/watch?v=abc\" --cookies-from-browser chrome --out out.pdf"
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("{}", usage());
        std::process::exit(2);
    }
    let mut cfg = PipelineConfig::default();
    cfg.url = args[1].clone();
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
            "--work-dir" => {
                cfg.work_dir = PathBuf::from(flag(i + 1)?);
                i += 2;
            }
            "--out" => {
                cfg.out_pdf = PathBuf::from(flag(i + 1)?);
                i += 2;
            }
            other => {
                eprintln!("unknown flag: {other}\n{}", usage());
                std::process::exit(2);
            }
        }
    }
    if cfg.out_pdf.as_os_str() == "output.pdf" {
        // default the filename from the video id when the user didn't choose
        if let Ok(v) = ytb2pdf_core::youtube::parse_youtube_url(&cfg.url) {
            cfg.out_pdf = PathBuf::from(format!("{}.pdf", v.id));
        }
    }

    run_pipeline(&cfg, &|ev| match ev {
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
        PipelineEvent::Done(p) => println!("\n🎉 Xong! PDF ở: {}", p.display()),
    })?;
    Ok(())
}
