//! PDF assembly with `genpdf` (pure-Rust, built on printpdf).
//!
//! Layout ("video frame" style): landscape A4, each scene starts on a fresh page —
//! title + kicker pinned to the top, the frame full-width right below them,
//! the subtitle under the frame, and a fixed footer with a clickable
//! "YouTube reference" link. Geometry, typography, and palette follow the
//! reference layout `EDA_2_Page_Layout_Test.pdf`: deep navy page, white bold
//! captions, steel-blue accents.
//!
//! Vietnamese text needs a TTF with Vietnamese glyphs (the built-in PDF
//! fonts only cover WinAnsi). We bundle Be Vietnam Pro (OFL) in
//! `crates/ytb2pdf-core/fonts/` and embed it in the binary; override with
//! [`PdfOptions::font_dir`], with system fonts as the last resort.

use anyhow::{anyhow, Context, Result};
use genpdf::elements::{Break, Image, LinearLayout, Paragraph};
use genpdf::render::Area;
use genpdf::style::{Color, Style, StyledString};
use genpdf::{
    Alignment, Context as GenContext, Document, Margins, Mm, PageDecorator, Position, Rotation,
    Scale, Size,
};
use lopdf::dictionary;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

// ---- landscape A4 "video frame" layout (all mm) ----
// Fixed geometry measured from the reference `EDA_2_Page_Layout_Test.pdf`
// (841.89 x 595.28 pt pages; mm = pt * 25.4 / 72).
const PAGE_W_MM: f64 = 297.0;
const PAGE_H_MM: f64 = 210.0;
/// Left edge of the pinned title and the frame (38 pt).
const FRAME_X_MM: f64 = 13.4;
/// Left edge of the caption and the footer texts (40 pt).
const TEXT_X_MM: f64 = 14.1;
/// Baselines of the pinned header lines, from the top of the page.
const TITLE_BASE_MM: f64 = 13.5;
const KICKER_BASE_MM: f64 = 19.0;
/// The frame band: full width, top right below the kicker (68 pt).
const FRAME_TOP_MM: f64 = 24.0;
const FRAME_W_MM: f64 = 270.2; // 766 pt
/// Full-width 16:9 height (431 pt); the frame only shrinks to make room for
/// long captions.
const FRAME_MAX_H_MM: f64 = 152.0;
const FRAME_MIN_H_MM: f64 = 60.0;
/// Gap between the frame's bottom edge and the caption paragraph.
const CAPTION_GAP_MM: f64 = 4.0;
/// The flow area (frame + captions) ends a hair above the footer divider.
const FLOW_BOTTOM_MM: f64 = 188.5;
// ---- footer geometry, measured from the top of the page ----
const DIVIDER_Y_MM: f64 = 190.8; // 541 pt
const DIVIDER_X1_MM: f64 = 282.9; // 802 pt
const LABEL_BASE_MM: f64 = 197.4;
const URL_BASE_MM: f64 = 202.8;

// ---- palette: deep navy page, white ink, steel-blue accents ----
const BG: (u8, u8, u8) = (0x0E, 0x13, 0x1F);
const TITLE_C: (u8, u8, u8) = (0xF5, 0xF7, 0xFF);
const CAPTION_C: (u8, u8, u8) = (0xF2, 0xF5, 0xFF);
const STEEL: (u8, u8, u8) = (0x8F, 0xAD, 0xD1);
const LABEL_C: (u8, u8, u8) = (0x8A, 0xB5, 0xE8);
const URL_C: (u8, u8, u8) = (0xBF, 0xD4, 0xF0);
const DIVIDER_C: (u8, u8, u8) = (0x40, 0x57, 0x75);

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// `print_str` places the baseline one glyph-height below the given y, and
/// the glyph height is 1.265 em for Be Vietnam Pro (hhea ascent 1000,
/// descent -265, upm 1000). Convert a target baseline (mm from the top of
/// the page) into the position genpdf expects.
fn baseline_y(baseline_mm: f64, size_pt: f64) -> Mm {
    Mm::from(baseline_mm - 1.265 * size_pt * 25.4 / 72.0)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PdfMeta {
    pub title: String,
    pub uploader: String,
    pub duration_secs: u64,
    pub source_url: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PdfBlock {
    pub timestamp_secs: f64,
    pub deep_link: Option<String>,
    pub caption: String,
    pub caption_translated: Option<String>,
    pub image: Option<PathBuf>,
    /// True when this block merges several consecutive segments that share
    /// the same visual scene. The caption is then a flowing paragraph
    /// (rendered without per-segment timestamps).
    pub merged: bool,
}

/// Merge several caption texts into one flowing paragraph.
pub fn merge_captions<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for p in parts {
        // collapse internal whitespace/newlines from the subtitle source
        let t: String = p.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&t);
    }
    out
}

#[derive(Debug, Clone)]
pub struct PdfOptions {
    /// Directory containing `<font_name>.ttf` (+ optional `-Bold.ttf`).
    pub font_dir: Option<PathBuf>,
    pub font_name: String,
    /// Skip images, captions only. For lightweight mobile reading.
    pub text_only: bool,
    /// Persist completed PDF parts for retries. Namespaced by prepared input digest.
    pub cache_dir: Option<PathBuf>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            font_dir: None,
            font_name: "DejaVuSans".to_string(),
            text_only: false,
            cache_dir: None,
        }
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

/// Raw bytes of the bundled Be Vietnam Pro faces, for UIs that want the
/// same typeface on screen (the PDF renderer embeds these too).
/// Returns `(regular, bold)`.
pub fn bundled_ui_font() -> (&'static [u8], &'static [u8]) {
    (
        include_bytes!("../fonts/BeVietnamPro-Regular.ttf"),
        include_bytes!("../fonts/BeVietnamPro-Bold.ttf"),
    )
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
        regular: load(
            "Regular",
            include_bytes!("../fonts/BeVietnamPro-Regular.ttf"),
        )?,
        bold: load("Bold", include_bytes!("../fonts/BeVietnamPro-Bold.ttf"))?,
        italic: load("Italic", include_bytes!("../fonts/BeVietnamPro-Italic.ttf"))?,
        bold_italic: load(
            "BoldItalic",
            include_bytes!("../fonts/BeVietnamPro-BoldItalic.ttf"),
        )?,
    })
}

fn load_font_family(
    opts: &PdfOptions,
) -> Result<genpdf::fonts::FontFamily<genpdf::fonts::FontData>> {
    use genpdf::fonts::{FontData, FontFamily};

    // 1. explicit override  2. bundled Be Vietnam Pro  3. system fonts
    let dirs: Vec<PathBuf> = opts.font_dir.clone().into_iter().collect();
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
        (
            "regular",
            &["{n}.ttf", "{n}-Regular.ttf", "{n}-regular.ttf"],
        ),
        (
            "bold",
            &[
                "{n}-Bold.ttf",
                "{n}Bold.ttf",
                "{n}-bold.ttf",
                "{n} Bold.ttf",
            ],
        ),
        (
            "italic",
            &[
                "{n}-Oblique.ttf",
                "{n}-Italic.ttf",
                "{n}Oblique.ttf",
                "{n}Italic.ttf",
            ],
        ),
        (
            "bold_italic",
            &[
                "{n}-BoldOblique.ttf",
                "{n}-BoldItalic.ttf",
                "{n}BoldOblique.ttf",
            ],
        ),
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

/// Page decorator: paints the navy background, the pinned header (title +
/// kicker) and the fixed footer (divider + "YouTube reference" + per-page
/// URL), then hands the frame band between them to the flow layout.
///
/// The footer URL sits at a fixed rect on every page, which is what
/// [`add_link_annotations`] targets for the clickable links.
struct FrameDecorator {
    bg: image023::DynamicImage,
    title: String,
    kicker: String,
    urls: Vec<Option<String>>,
    timestamps: Vec<f64>,
    scene_idx: Rc<Cell<usize>>,
    page_urls: Rc<RefCell<Vec<Option<String>>>>,
}

impl FrameDecorator {
    fn new(
        meta: &PdfMeta,
        urls: Vec<Option<String>>,
        timestamps: Vec<f64>,
        scene_idx: Rc<Cell<usize>>,
        page_urls: Rc<RefCell<Vec<Option<String>>>>,
    ) -> Self {
        let mut px = image023::RgbImage::new(1, 1);
        px.put_pixel(0, 0, image023::Rgb([BG.0, BG.1, BG.2]));
        Self {
            bg: image023::DynamicImage::ImageRgb8(px),
            title: meta.title.to_uppercase(),
            kicker: format!("ORIGINAL VIDEO FRAME  /  {}", meta.uploader.to_uppercase()),
            urls,
            timestamps,
            scene_idx,
            page_urls,
        }
    }
}

impl PageDecorator for FrameDecorator {
    fn decorate_page<'a>(
        &mut self,
        context: &GenContext,
        area: Area<'a>,
        _style: Style,
    ) -> Result<Area<'a>, genpdf::error::Error> {
        // 1. full-page dark background: a 1x1 px image scaled to the page.
        //    Drawn first, so the content lands on top of it.
        //    NOTE: add_image expects the position of the image's BOTTOM edge
        //    (measured from the area's top) — the image extends upwards.
        area.add_image(
            &self.bg,
            Position::new(Mm::from(0.0), Mm::from(PAGE_H_MM)),
            Scale::new(PAGE_W_MM * 300.0 / 25.4, PAGE_H_MM * 300.0 / 25.4),
            Rotation::default(),
            Some(300.0),
        );
        // 2. pinned header: title + kicker.
        let title_style = Style::new()
            .bold()
            .with_font_size(16)
            .with_color(rgb(TITLE_C));
        let _ = area.print_str(
            &context.font_cache,
            Position::new(Mm::from(FRAME_X_MM), baseline_y(TITLE_BASE_MM, 16.0)),
            title_style,
            &self.title,
        );
        let kicker_style = Style::new().with_font_size(8).with_color(rgb(STEEL));
        let _ = area.print_str(
            &context.font_cache,
            Position::new(Mm::from(FRAME_X_MM), baseline_y(KICKER_BASE_MM, 8.0)),
            kicker_style,
            &self.kicker,
        );
        let scene_idx = self.scene_idx.get();
        self.page_urls
            .borrow_mut()
            .push(self.urls.get(scene_idx).cloned().flatten());
        // 3. footer at fixed positions.
        area.draw_line(
            vec![
                Position::new(Mm::from(TEXT_X_MM), Mm::from(DIVIDER_Y_MM)),
                Position::new(Mm::from(DIVIDER_X1_MM), Mm::from(DIVIDER_Y_MM)),
            ],
            Style::new().with_color(rgb(DIVIDER_C)),
        );
        let label_style = Style::new()
            .bold()
            .with_font_size(8)
            .with_color(rgb(LABEL_C));
        let _ = area.print_str(
            &context.font_cache,
            Position::new(Mm::from(TEXT_X_MM), baseline_y(LABEL_BASE_MM, 8.0)),
            label_style,
            if self
                .urls
                .get(scene_idx)
                .and_then(|link| link.as_ref())
                .is_some()
            {
                "YouTube reference"
            } else {
                "VIDEO TIMESTAMP"
            },
        );
        if let Some(url) = self.urls.get(scene_idx).and_then(|url| url.as_ref()) {
            let url_style = Style::new().with_font_size(9).with_color(rgb(URL_C));
            let _ = area.print_str(
                &context.font_cache,
                Position::new(Mm::from(TEXT_X_MM), baseline_y(URL_BASE_MM, 9.0)),
                url_style,
                url,
            );
        } else {
            let timestamp = self.timestamps.get(scene_idx).copied().unwrap_or(0.0);
            let text = format_timestamp(timestamp);
            let style = Style::new().with_font_size(9).with_color(rgb(URL_C));
            let _ = area.print_str(
                &context.font_cache,
                Position::new(Mm::from(TEXT_X_MM), baseline_y(URL_BASE_MM, 9.0)),
                style,
                &text,
            );
        }
        // 4. flow area for frame + captions: the band between header and
        //    footer.
        let mut content = area;
        content.add_margins(Margins::trbl(
            Mm::from(FRAME_TOP_MM),
            Mm::from(PAGE_W_MM - FRAME_X_MM - FRAME_W_MM),
            Mm::from(0.0),
            Mm::from(FRAME_X_MM),
        ));
        content.set_height(Mm::from(FLOW_BOTTOM_MM - FRAME_TOP_MM));
        Ok(content)
    }
}

/// Uniform scale fitting a `pw` x `ph` px frame into
/// (`FRAME_W_MM` x `box_h_mm`), keeping aspect ratio. genpdf draws images
/// at 300 dpi by default, so mm = px / 300 * 25.4.
fn frame_scale(pw: u32, ph: u32, box_h_mm: f64) -> f64 {
    let w_mm = pw as f64 / 300.0 * 25.4;
    let h_mm = ph as f64 / 300.0 * 25.4;
    (FRAME_W_MM / w_mm).min(box_h_mm / h_mm)
}

/// Frame height budget: the full 16:9 height, shrunk only when a long
/// caption (+ translation) needs the room above the footer.
fn frame_box_h(caption: &str, translated: Option<&str>) -> f64 {
    let per_line = 125; // conservative estimate for 11 pt Be Vietnam Pro over 270 mm
    let lines = |t: &str| (t.chars().count() + per_line - 1) / per_line;
    let mut n = lines(caption).max(1);
    if let Some(vi) = translated {
        if vi != caption {
            n += lines(vi).max(1);
        }
    }
    let text_h_mm = n as f64 * 5.6 + CAPTION_GAP_MM;
    (FLOW_BOTTOM_MM - FRAME_TOP_MM - text_h_mm).clamp(FRAME_MIN_H_MM, FRAME_MAX_H_MM)
}

/// Add clickable link annotations over the footer's "YouTube reference" URL
/// on every page. genpdf/printpdf 0.3 cannot emit link annotations, so they
/// are added afterwards with lopdf. The footer URL sits at a fixed rect on
/// every page (see [`FrameDecorator`]), so one constant rect works for all.
fn add_link_annotations(pdf_path: &Path, urls: &[Option<String>]) -> Result<()> {
    let mut doc = lopdf::Document::load(pdf_path).context("re-opening rendered PDF")?;
    let pages = doc.get_pages();
    if pages.len() != urls.len() {
        // A wrong link is worse than no link: skip rather than mis-link.
        anyhow::bail!(
            "page count {} != {} rendered page references; skipping link annotations",
            pages.len(),
            urls.len()
        );
    }
    // Rect of the footer URL in PDF points (origin: bottom-left), generous.
    // The URL baseline sits at URL_BASE_MM (202.8 mm from the top), i.e.
    // ~20.5 pt from the bottom of the 595.28 pt page.
    let rect = lopdf::Object::Array(vec![
        lopdf::Object::Real(38.0),
        lopdf::Object::Real(12.0),
        lopdf::Object::Real(806.0),
        lopdf::Object::Real(30.0),
    ]);
    for ((_n, page_id), url) in pages.iter().zip(urls.iter()) {
        let Some(url) = url else {
            continue;
        };
        let annot_id = doc.new_object_id();
        let annot = lopdf::dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => rect.clone(),
            "Border" => lopdf::Object::Array(vec![
                lopdf::Object::Integer(0),
                lopdf::Object::Integer(0),
                lopdf::Object::Integer(0),
            ]),
            "A" => lopdf::dictionary! {
                "S" => "URI",
                "URI" => lopdf::Object::string_literal(url.clone()),
            },
        };
        doc.objects
            .insert(annot_id, lopdf::Object::Dictionary(annot));
        // Fresh genpdf pages never have /Annots, so a straight set is enough.
        let page = doc
            .get_object_mut(*page_id)
            .and_then(|o| o.as_dict_mut())
            .map_err(|e| anyhow!("page dict: {e}"))?;
        page.set(
            "Annots",
            lopdf::Object::Array(vec![lopdf::Object::Reference(annot_id)]),
        );
    }
    // Save via a temp file so a failure can't corrupt the rendered PDF.
    let tmp = pdf_path.with_extension("pdf.tmp");
    doc.save(&tmp)
        .map_err(|e| anyhow!("saving annotated PDF: {e}"))?;
    std::fs::rename(&tmp, pdf_path).context("replacing PDF with annotated copy")?;
    Ok(())
}

/// Build large documents in bounded batches of scenes, then merge with Poppler.
pub fn build_pdf(meta: &PdfMeta, blocks: &[PdfBlock], opts: &PdfOptions, out: &Path) -> Result<()> {
    build_pdf_chunked(meta, blocks, opts, out, 1500)
}

fn build_pdf_chunked(
    meta: &PdfMeta,
    blocks: &[PdfBlock],
    opts: &PdfOptions,
    out: &Path,
    chunk_size: usize,
) -> Result<()> {
    if blocks.len() <= chunk_size {
        return build_pdf_single(meta, blocks, opts, out);
    }
    // Fail before rendering thousands of pages if the runtime is incomplete.
    let version = std::process::Command::new("pdfunite").arg("-v").output()
        .context("PDF merging needs pdfunite. Update to the complete app release, or install Poppler (macOS: brew install poppler; Linux: apt install poppler-utils)")?;
    anyhow::ensure!(
        version.status.success(),
        "pdfunite could not start: {}",
        String::from_utf8_lossy(&version.stderr)
    );
    let scratch = if opts.cache_dir.is_none() {
        Some(
            tempfile::Builder::new()
                .prefix("ytb2pdf-chunks-")
                .tempdir()?,
        )
    } else {
        None
    };
    let parts_dir = opts
        .cache_dir
        .as_deref()
        .unwrap_or_else(|| scratch.as_ref().unwrap().path());
    std::fs::create_dir_all(parts_dir)?;
    let mut chunk_files = Vec::new();
    for (ci, chunk) in blocks.chunks(chunk_size).enumerate() {
        let path = parts_dir.join(format!("chunk-{ci:03}.pdf"));
        let checksum = path.with_extension("sha256");
        if let (Ok(expected), Ok(actual)) = (
            std::fs::read_to_string(&checksum),
            crate::resume::file_digest(&path),
        ) {
            if expected == actual {
                eprintln!("Resuming completed PDF part {}", ci + 1);
                chunk_files.push(path);
                continue;
            }
        }
        let _ = std::fs::remove_file(&checksum);
        // Each chunk annotates its actual rendered pages, including caption
        // continuations. pdfunite preserves these annotations during merging.
        build_pdf_single(meta, chunk, opts, &path)?;
        crate::resume::atomic_write(&checksum, crate::resume::file_digest(&path)?.as_bytes())?;
        chunk_files.push(path);
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    merge_pdfs(&chunk_files, out)?;
    // Do not reopen the full merged PDF with lopdf: that defeats chunking's
    // memory bound, and per-scene URL counts do not include overflow pages.
    Ok(())
}

/// Concatenate several PDFs into one using `pdfunite` (poppler-utils).
fn merge_pdfs(ins: &[PathBuf], out: &Path) -> Result<()> {
    use std::process::Command;
    let mut cmd = Command::new("pdfunite");
    for p in ins {
        cmd.arg(p);
    }
    cmd.arg(out);
    let st = cmd
        .output()
        .context("running pdfunite (install poppler-utils)")?;
    if !st.status.success() {
        anyhow::bail!(
            "pdfunite failed: {}",
            String::from_utf8_lossy(&st.stderr).trim()
        );
    }
    Ok(())
}

/// Render exactly one scene at a time on fresh pages. A scene may span several
/// pages; its shared index changes only after all of its content is rendered.
struct ScenePages {
    scenes: Vec<LinearLayout>,
    current: Rc<Cell<usize>>,
}

impl genpdf::Element for ScenePages {
    fn render(
        &mut self,
        context: &GenContext,
        area: Area<'_>,
        style: Style,
    ) -> Result<genpdf::RenderResult, genpdf::error::Error> {
        let index = self.current.get();
        let Some(scene) = self.scenes.get_mut(index) else {
            return Ok(genpdf::RenderResult::default());
        };
        let mut result = genpdf::Element::render(scene, context, area, style)?;
        if !result.has_more {
            self.current.set(index + 1);
            result.has_more = index + 1 < self.scenes.len();
        }
        Ok(result)
    }
}

fn build_pdf_single(
    meta: &PdfMeta,
    blocks: &[PdfBlock],
    opts: &PdfOptions,
    out: &Path,
) -> Result<()> {
    let font_family = load_font_family(opts)?;
    let mut doc = Document::new(font_family);

    doc.set_title(&meta.title);
    doc.set_paper_size(Size::new(PAGE_W_MM, PAGE_H_MM));
    let urls: Vec<Option<String>> = blocks.iter().map(|b| b.deep_link.clone()).collect();
    let timestamps: Vec<f64> = blocks.iter().map(|b| b.timestamp_secs).collect();
    let current = Rc::new(Cell::new(0));
    let page_urls = Rc::new(RefCell::new(Vec::new()));
    doc.set_page_decorator(FrameDecorator::new(
        meta,
        urls,
        timestamps,
        Rc::clone(&current),
        Rc::clone(&page_urls),
    ));
    let mut scenes = Vec::new();

    let caption_style = Style::new()
        .bold()
        .with_font_size(11)
        .with_color(rgb(CAPTION_C));
    let vi_style = Style::new().with_font_size(10).with_color(rgb(STEEL));

    /// Height of one `Break` line with genpdf's default 12 pt style (mm).
    const BREAK_LINE_MM: f64 = 1.265 * 12.0 * 25.4 / 72.0;

    // Each scene starts on a landscape page: full-width frame, then captions.
    // Continuation pages keep that scene's footer reference and annotation.
    for b in blocks {
        let mut scene = LinearLayout::vertical();
        if !opts.text_only {
            if let Some(img_path) = &b.image {
                match Image::from_path(img_path) {
                    Ok(img) => {
                        let (pw, ph) = image::image_dimensions(img_path).unwrap_or((1280, 720));
                        let translated = b.caption_translated.as_deref();
                        let s = frame_scale(pw, ph, frame_box_h(&b.caption, translated));
                        scene.push(
                            img.with_scale(Scale::new(s, s))
                                .with_alignment(Alignment::Center),
                        );
                    }
                    Err(e) => scene.push(para(&format!("[Unable to embed image: {e}]"), &vi_style)),
                }
            }
        }
        scene.push(Break::new(CAPTION_GAP_MM / BREAK_LINE_MM));

        // Caption as a clean flowing paragraph — no timestamps in the text
        // (the footer's YouTube link already carries the time reference).
        let mut sub = Paragraph::new("");
        sub.push_styled(b.caption.clone(), caption_style.clone());
        scene.push(sub);
        if let Some(vi) = &b.caption_translated {
            if vi != &b.caption {
                scene.push(para(&format!("VI: {vi}"), &vi_style));
            }
        }
        scenes.push(scene);
    }

    doc.push(ScenePages { scenes, current });
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).context("creating PDF output dir")?;
    }
    doc.render_to_file(out)
        .with_context(|| format!("writing PDF to {}", out.display()))?;

    // The decorator recorded one URL per real page, including continuations.
    let urls = page_urls.borrow();
    if urls.iter().any(Option::is_some) {
        if let Err(e) = add_link_annotations(out, &urls) {
            eprintln!("warning: could not add PDF link annotations: {e:#}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overflow_fixture() -> (PdfMeta, Vec<PdfBlock>) {
        let meta = PdfMeta {
            title: "Overflow regression".into(),
            uploader: "ytb2pdf".into(),
            duration_secs: 120,
            source_url: Some("https://youtu.be/TEST".into()),
        };
        let blocks = vec![
            PdfBlock {
                timestamp_secs: 1.0,
                deep_link: Some("https://youtu.be/TEST?t=1s".into()),
                caption: "A long caption must continue without changing its video reference. "
                    .repeat(700),
                caption_translated: Some("Ban dich tiep tuc tren nhieu trang. ".repeat(100)),
                image: None,
                merged: true,
            },
            PdfBlock {
                timestamp_secs: 60.0,
                deep_link: None,
                caption: "Local timestamp only.".into(),
                caption_translated: None,
                image: None,
                merged: false,
            },
            PdfBlock {
                timestamp_secs: 90.0,
                deep_link: Some("https://youtu.be/TEST?t=90s".into()),
                caption: "Last scene.".into(),
                caption_translated: None,
                image: None,
                merged: false,
            },
        ];
        (meta, blocks)
    }

    fn page_links(path: &Path) -> Vec<Option<String>> {
        let doc = lopdf::Document::load(path).unwrap();
        doc.get_pages()
            .values()
            .map(|id| {
                let page = doc.get_object(*id).unwrap().as_dict().unwrap();
                let Ok(annotations) = page.get(b"Annots") else {
                    return None;
                };
                let annotations = doc.dereference(annotations).unwrap().1.as_array().unwrap();
                if annotations.is_empty() {
                    return None;
                }
                let annotation = doc
                    .dereference(&annotations[0])
                    .unwrap()
                    .1
                    .as_dict()
                    .unwrap();
                let action = doc
                    .dereference(annotation.get(b"A").unwrap())
                    .unwrap()
                    .1
                    .as_dict()
                    .unwrap();
                Some(
                    String::from_utf8(action.get(b"URI").unwrap().as_str().unwrap().to_vec())
                        .unwrap(),
                )
            })
            .collect()
    }

    fn assert_overflow_links(links: &[Option<String>]) {
        assert!(
            links.len() > 3,
            "The long caption must actually spill onto extra pages"
        );
        for link in &links[..links.len() - 2] {
            assert_eq!(link.as_deref(), Some("https://youtu.be/TEST?t=1s"));
        }
        assert_eq!(links[links.len() - 2], None);
        assert_eq!(
            links.last().unwrap().as_deref(),
            Some("https://youtu.be/TEST?t=90s")
        );
    }

    #[test]
    fn scene_boundaries_do_not_add_blank_separator_pages() {
        let dir = tempfile::tempdir().unwrap();
        let (meta, mut blocks) = overflow_fixture();
        blocks[0].caption = "First scene.".into();
        blocks[0].caption_translated = None;
        let out = dir.path().join("short.pdf");
        build_pdf_single(&meta, &blocks, &PdfOptions::default(), &out).unwrap();
        let links = page_links(&out);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].as_deref(), Some("https://youtu.be/TEST?t=1s"));
        assert_eq!(links[1], None);
        assert_eq!(links[2].as_deref(), Some("https://youtu.be/TEST?t=90s"));
    }

    #[test]
    fn overflow_pages_keep_their_scene_links_and_following_scenes_stay_aligned() {
        let dir = tempfile::tempdir().unwrap();
        let (meta, blocks) = overflow_fixture();
        let out = dir.path().join("overflow.pdf");
        build_pdf_single(&meta, &blocks, &PdfOptions::default(), &out).unwrap();
        assert_overflow_links(&page_links(&out));
    }

    #[test]
    fn chunk_merge_preserves_links_without_reopening_the_entire_document() {
        let dir = tempfile::tempdir().unwrap();
        let (meta, blocks) = overflow_fixture();
        let out = dir.path().join("merged.pdf");
        build_pdf_chunked(&meta, &blocks, &PdfOptions::default(), &out, 2).unwrap();
        assert_overflow_links(&page_links(&out));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn merge_captions_joins_and_cleans() {
        let parts = vec!["Hello  world.", "  This is\na test. ", "", "Bye."];
        assert_eq!(
            merge_captions(parts.into_iter()),
            "Hello world. This is a test. Bye."
        );
        let empty: Vec<&str> = vec![];
        assert_eq!(merge_captions(empty.into_iter()), "");
    }
}
