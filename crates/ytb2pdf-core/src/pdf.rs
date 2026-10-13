//! PDF assembly with `genpdf` (pure-Rust, built on printpdf).
//!
//! Layout: cover page (title / channel / duration / source link), then one
//! block per caption segment — timestamp, the frame that was on screen,
//! original caption, and the translation underneath when requested.
//!
//! Vietnamese text needs a TTF with Vietnamese glyphs (the built-in PDF
//! fonts only cover WinAnsi). We bundle Be Vietnam Pro (OFL) in
//! `crates/ytb2pdf-core/fonts/` and embed it in the binary; override with
//! [`PdfOptions::font_dir`], with system fonts as the last resort.

use anyhow::{anyhow, Context, Result};
use genpdf::elements::{Break, Image, PageBreak, Paragraph};
use genpdf::style::{Style, StyledString};
use genpdf::{Alignment, Document, Margins, SimplePageDecorator};
use std::path::{Path, PathBuf};

pub struct PdfMeta {
    pub title: String,
    pub uploader: String,
    pub duration_secs: u64,
    pub source_url: String,
}

pub struct PdfBlock {
    pub timestamp_secs: f64,
    pub deep_link: String,
    pub caption: String,
    pub caption_translated: Option<String>,
    pub image: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct PdfOptions {
    /// Directory containing `<font_name>.ttf` (+ optional `-Bold.ttf`).
    pub font_dir: Option<PathBuf>,
    pub font_name: String,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self { font_dir: None, font_name: "DejaVuSans".to_string() }
    }
}

fn candidate_font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".fonts"));
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join("Library/Fonts")); // macOS user fonts
    }
    dirs.push(PathBuf::from("/usr/share/fonts/truetype/dejavu")); // Linux
    dirs.push(PathBuf::from("/usr/share/fonts/truetype")); // Linux
    dirs.push(PathBuf::from("/System/Library/Fonts")); // macOS system fonts
    dirs.push(PathBuf::from("/System/Library/Fonts/Supplemental")); // macOS: Arial.ttf etc.
    dirs
}

/// Font names to try, in order. macOS runners have no DejaVu, but they do
/// ship Arial (with full Vietnamese coverage) in Fonts/Supplemental.
fn font_names(preferred: &str) -> Vec<String> {
    let mut names = vec![preferred.to_string()];
    for n in ["DejaVuSans", "Arial"] {
        if n != preferred {
            names.push(n.to_string());
        }
    }
    names
}

/// Be Vietnam Pro, bundled in `fonts/` (OFL 1.1 — free to embed).
/// Loaded with `include_bytes!` so the binary works on any machine with
/// zero font setup (CI runners, fresh Macs, …).
fn embedded_font_family() -> Result<genpdf::fonts::FontFamily<genpdf::fonts::FontData>> {
    use genpdf::fonts::{FontData, FontFamily};
    let load = |name: &str, bytes: &'static [u8]| {
        FontData::new(bytes.to_vec(), None)
            .map_err(|e| anyhow!("bundled font {name} is unreadable: {e}"))
    };
    Ok(FontFamily {
        regular: load("Regular", include_bytes!("../fonts/BeVietnamPro-Regular.ttf"))?,
        bold: load("Bold", include_bytes!("../fonts/BeVietnamPro-Bold.ttf"))?,
        italic: load("Italic", include_bytes!("../fonts/BeVietnamPro-Italic.ttf"))?,
        bold_italic: load("BoldItalic", include_bytes!("../fonts/BeVietnamPro-BoldItalic.ttf"))?,
    })
}

fn load_font_family(opts: &PdfOptions) -> Result<genpdf::fonts::FontFamily<genpdf::fonts::FontData>> {
    use genpdf::fonts::{FontData, FontFamily};

    // 1. explicit override  2. bundled Be Vietnam Pro  3. system fonts
    let mut dirs: Vec<PathBuf> = opts.font_dir.clone().into_iter().collect();
    let names = font_names(&opts.font_name);
    for dir in &dirs {
        for name in &names {
            if let Ok(f) = genpdf::fonts::from_files(dir, name, None) {
                return Ok(f);
            }
        }
    }
    if let Ok(f) = embedded_font_family() {
        return Ok(f);
    }

    // 3. system fonts: every (dir × name) combo, strict layout first, then
    // a forgiving per-style search (DejaVu/Noto/Arial name files differently).
    let mut dirs: Vec<PathBuf> = opts.font_dir.clone().into_iter().collect();
    dirs.extend(candidate_font_dirs());
    for dir in &dirs {
        for name in &names {
            if let Ok(f) = genpdf::fonts::from_files(dir, name, None) {
                return Ok(f);
            }
        }
    }

    let candidates: &[(&str, &[&str])] = &[
        ("regular", &["{n}.ttf", "{n}-Regular.ttf", "{n}-regular.ttf"]),
        ("bold", &["{n}-Bold.ttf", "{n}Bold.ttf", "{n}-bold.ttf", "{n} Bold.ttf"]),
        ("italic", &["{n}-Oblique.ttf", "{n}-Italic.ttf", "{n}Oblique.ttf", "{n}Italic.ttf"]),
        ("bold_italic", &["{n}-BoldOblique.ttf", "{n}-BoldItalic.ttf", "{n}BoldOblique.ttf"]),
    ];
    let mut tried = Vec::new();
    'names: for name in &names {
        let mut loaded: Vec<FontData> = Vec::new();
        let mut regular_fallback: Option<FontData> = None;
        for (style, patterns) in candidates {
            let mut found = None;
            for dir in &dirs {
                for pat in *patterns {
                    let file = dir.join(pat.replace("{n}", name));
                    tried.push(file.display().to_string());
                    if file.exists() {
                        match FontData::load(&file, None) {
                            Ok(d) => {
                                found = Some(d);
                                break;
                            }
                            Err(_) => continue,
                        }
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            match found {
                Some(d) => {
                    if *style == "regular" {
                        regular_fallback = Some(d.clone());
                    }
                    loaded.push(d);
                }
                None if *style == "regular" => continue 'names, // try next font name
                // missing bold/italic variants fall back to regular — fine for v0.1
                None => loaded.push(regular_fallback.clone().unwrap()),
            }
        }
        return Ok(FontFamily {
            regular: loaded[0].clone(),
            bold: loaded[1].clone(),
            italic: loaded[2].clone(),
            bold_italic: loaded[3].clone(),
        });
    }
    Err(anyhow!(
        "could not find a usable TTF for any of {:?}.\nSearched {} locations.\n\
         Fix: point PdfOptions::font_dir at a folder with a Vietnamese-capable TTF.",
        names,
        tried.len(),
    ))
}

pub fn format_timestamp(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub fn format_duration(secs: u64) -> String {
    format_timestamp(secs as f64)
}

/// Shrink an image in place so its width is at most `max_w` px (keeps the
/// PDF small; 1280px is plenty for reading slides).
pub fn shrink_image(path: &Path, max_w: u32) -> Result<()> {
    let img = image::open(path).with_context(|| format!("opening {}", path.display()))?;
    if img.width() > max_w {
        let h = (img.height() as f64 * max_w as f64 / img.width() as f64) as u32;
        img.resize_exact(max_w, h, image::imageops::FilterType::Triangle)
            .save(path)
            .with_context(|| format!("re-saving {}", path.display()))?;
    }
    Ok(())
}

fn para(text: &str, style: &Style) -> Paragraph {
    Paragraph::new(StyledString::new(text, style.clone()))
}

pub fn build_pdf(
    meta: &PdfMeta,
    blocks: &[PdfBlock],
    opts: &PdfOptions,
    out: &Path,
) -> Result<()> {
    let font_family = load_font_family(opts)?;
    let mut doc = Document::new(font_family);

    doc.set_title(&meta.title);
    let mut decorator = SimplePageDecorator::new();
    decorator.set_margins(Margins::trbl(18, 15, 18, 15));
    doc.set_page_decorator(decorator);

    let title_style = Style::new().bold().with_font_size(20);
    let ts_style = Style::new().bold().with_font_size(11);
    let body_style = Style::new().with_font_size(10);
    let vi_style = Style::new().with_font_size(10);
    let dim_style = Style::new().with_font_size(9);

    // ---- cover ----
    doc.push(para(&meta.title, &title_style));
    doc.push(Break::new(1));
    doc.push(para(&format!("Kênh: {}", meta.uploader), &dim_style));
    doc.push(para(&format!("Thời lượng: {}", format_duration(meta.duration_secs)), &dim_style));
    doc.push(para(&format!("Nguồn: {}", meta.source_url), &dim_style));
    doc.push(Break::new(1));
    doc.push(para(
        &format!(
            "Tài liệu học tập được tạo tự động từ video — {} đoạn caption, {} hình ảnh.",
            blocks.len(),
            blocks.iter().filter(|b| b.image.is_some()).count()
        ),
        &body_style,
    ));
    doc.push(PageBreak::new());

    // ---- blocks ----
    for b in blocks {
        let mut head = para(&format!("[{}]", format_timestamp(b.timestamp_secs)), &ts_style);
        head.set_alignment(Alignment::Left);
        doc.push(head);

        if let Some(img_path) = &b.image {
            match Image::from_path(img_path) {
                Ok(img) => doc.push(img.with_alignment(Alignment::Center)),
                Err(e) => doc.push(para(&format!("[không nhúng được ảnh: {e}]"), &dim_style)),
            }
        }

        doc.push(para(&b.caption, &body_style));
        if let Some(vi) = &b.caption_translated {
            if vi != &b.caption {
                doc.push(para(&format!("VI: {vi}"), &vi_style));
            }
        }
        doc.push(para(&format!("Xem tại: {}", b.deep_link), &dim_style));
        doc.push(Break::new(1));
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).context("creating PDF output dir")?;
    }
    doc.render_to_file(out)
        .with_context(|| format!("writing PDF to {}", out.display()))?;
    Ok(())
}
