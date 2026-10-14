//! PDF assembly with `genpdf` (pure-Rust, built on printpdf).
//!
//! Layout ("video frame" style): landscape A4, one caption segment per page —
//! video title header, the frame big and centered, the subtitle right below
//! it, and a fixed footer with a clickable "YouTube reference" link.
//! Dark navy palette, in the spirit of the app's FilmCraft dark theme.
//!
//! Vietnamese text needs a TTF with Vietnamese glyphs (the built-in PDF
//! fonts only cover WinAnsi). We bundle Be Vietnam Pro (OFL) in
//! `crates/ytb2pdf-core/fonts/` and embed it in the binary; override with
//! [`PdfOptions::font_dir`], with system fonts as the last resort.

use anyhow::{anyhow, Context, Result};
use lopdf::dictionary;
use genpdf::elements::{Break, Image, PageBreak, Paragraph};
use genpdf::render::Area;
use genpdf::style::{Color, Style, StyledString};
use genpdf::{
    Alignment, Context as GenContext, Document, Margins, Mm, PageDecorator, Position, Rotation,
    Scale, Size,
};
use std::path::{Path, PathBuf};

// ---- landscape A4 "video frame" layout (all mm) ----
const PAGE_W_MM: f64 = 297.0;
const PAGE_H_MM: f64 = 210.0;
const MARGIN_MM: f64 = 15.0;
/// Footer strip (divider + "YouTube reference" + URL) reserved at the bottom.
const FOOTER_H_MM: f64 = 18.0;
const CONTENT_W_MM: f64 = PAGE_W_MM - 2.0 * MARGIN_MM; // 267
const CONTENT_H_MM: f64 = PAGE_H_MM - 2.0 * MARGIN_MM - FOOTER_H_MM; // 162
/// Header (title + kicker + gap) budget inside the content area.
const HEADER_H_MM: f64 = 16.0;
/// Max frame height; shrinks when the subtitle needs more lines.
const FRAME_MAX_H_MM: f64 = 120.0;
const FRAME_MIN_H_MM: f64 = 40.0;

// ---- palette: deep navy page, steel-blue accents ----
const BG: (u8, u8, u8) = (0x0B, 0x12, 0x20);
const INK: (u8, u8, u8) = (0xF2, 0xF5, 0xF9);
const STEEL: (u8, u8, u8) = (0x7A, 0x8B, 0xA8);
const URL_INK: (u8, u8, u8) = (0xA0, 0xAF, 0xC3);
const DIVIDER_C: (u8, u8, u8) = (0x2D, 0x3C, 0x5A);

// ---- footer geometry, measured from the top of the page ----
const DIVIDER_Y_MM: f64 = PAGE_H_MM - MARGIN_MM - FOOTER_H_MM + 3.0; // 180
const REFLABEL_Y_MM: f64 = DIVIDER_Y_MM + 5.0; // 185
const URL_Y_MM: f64 = DIVIDER_Y_MM + 10.5; // 190.5

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

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

/// Page decorator: paints the dark navy background, draws the fixed footer
/// (divider + "YouTube reference" + per-page URL), then hands the remaining
/// content area (margins minus the footer strip) to the flow layout.
///
/// The footer URL sits at a fixed rect on every page, which is what
/// [`add_link_annotations`] targets for the clickable links.
struct FrameDecorator {
    bg: image023::DynamicImage,
    urls: Vec<String>,
    page_idx: usize,
}

impl FrameDecorator {
    fn new(urls: Vec<String>) -> Self {
        let mut px = image023::RgbImage::new(1, 1);
        px.put_pixel(0, 0, image023::Rgb([BG.0, BG.1, BG.2]));
        Self {
            bg: image023::DynamicImage::ImageRgb8(px),
            urls,
            page_idx: 0,
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
        // 2. footer at fixed positions.
        area.draw_line(
            vec![
                Position::new(Mm::from(MARGIN_MM), Mm::from(DIVIDER_Y_MM)),
                Position::new(Mm::from(PAGE_W_MM - MARGIN_MM), Mm::from(DIVIDER_Y_MM)),
            ],
            Style::new().with_color(rgb(DIVIDER_C)),
        );
        let label_style = Style::new().with_font_size(8).with_color(rgb(STEEL));
        let _ = area.print_str(
            &context.font_cache,
            Position::new(Mm::from(MARGIN_MM), Mm::from(REFLABEL_Y_MM)),
            label_style,
            "YouTube reference",
        );
        if let Some(url) = self.urls.get(self.page_idx) {
            let url_style = Style::new().with_font_size(9).with_color(rgb(URL_INK));
            let _ = area.print_str(
                &context.font_cache,
                Position::new(Mm::from(MARGIN_MM), Mm::from(URL_Y_MM)),
                url_style,
                url,
            );
        }
        self.page_idx += 1;
        // 3. content area for the flow: page margins minus the footer strip.
        let mut content = area;
        content.add_margins(Margins::trbl(MARGIN_MM, MARGIN_MM, MARGIN_MM, MARGIN_MM));
        content.set_height(Mm::from(CONTENT_H_MM));
        Ok(content)
    }
}

/// Uniform scale fitting a `pw` x `ph` px image into
/// (`CONTENT_W_MM` x `box_h_mm`), keeping aspect ratio. genpdf draws images
/// at 300 dpi by default, so mm = px / 300 * 25.4.
fn frame_scale(pw: u32, ph: u32, box_h_mm: f64) -> f64 {
    let w_mm = pw as f64 / 300.0 * 25.4;
    let h_mm = ph as f64 / 300.0 * 25.4;
    (CONTENT_W_MM / w_mm).min(box_h_mm / h_mm)
}

/// Frame height budget for a block: shrinks when the subtitle needs more
/// lines, so header + frame + subtitle always fit the 162 mm content area.
fn frame_box_h(caption: &str, translated: Option<&str>) -> f64 {
    let per_line = 95; // ~chars per 11pt line on 267 mm
    let lines = |t: &str| (t.chars().count() + per_line - 1) / per_line;
    let mut n = lines(caption).max(1);
    if let Some(vi) = translated {
        if vi != caption {
            n += lines(vi).max(1);
        }
    }
    let text_h_mm = n as f64 * 4.6;
    (CONTENT_H_MM - HEADER_H_MM - text_h_mm - 10.0).clamp(FRAME_MIN_H_MM, FRAME_MAX_H_MM)
}

/// Add clickable link annotations over the footer's "YouTube reference" URL
/// on every page. genpdf/printpdf 0.3 cannot emit link annotations, so they
/// are added afterwards with lopdf. The footer URL sits at a fixed rect on
/// every page (see [`FrameDecorator`]), so one constant rect works for all.
fn add_link_annotations(pdf_path: &Path, urls: &[String]) -> Result<()> {
    let mut doc = lopdf::Document::load(pdf_path).context("re-opening rendered PDF")?;
    let pages = doc.get_pages();
    if pages.len() != urls.len() {
        // A wrong link is worse than no link: skip rather than mis-link.
        anyhow::bail!(
            "page count {} != {} blocks; skipping link annotations",
            pages.len(),
            urls.len()
        );
    }
    // Rect of the footer URL in PDF points (origin: bottom-left), generous.
    let rect = lopdf::Object::Array(vec![
        lopdf::Object::Real(42.5),
        lopdf::Object::Real(45.0),
        lopdf::Object::Real(453.5),
        lopdf::Object::Real(61.0),
    ]);
    for ((_n, page_id), url) in pages.iter().zip(urls.iter()) {
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

pub fn build_pdf(
    meta: &PdfMeta,
    blocks: &[PdfBlock],
    opts: &PdfOptions,
    out: &Path,
) -> Result<()> {
    let font_family = load_font_family(opts)?;
    let mut doc = Document::new(font_family);

    doc.set_title(&meta.title);
    doc.set_paper_size(Size::new(PAGE_W_MM, PAGE_H_MM));
    let urls: Vec<String> = blocks.iter().map(|b| b.deep_link.clone()).collect();
    doc.set_page_decorator(FrameDecorator::new(urls.clone()));

    let title_style = Style::new()
        .bold()
        .with_font_size(16)
        .with_color(rgb(INK));
    let kicker_style = Style::new().with_font_size(9).with_color(rgb(STEEL));
    let ts_style = Style::new()
        .bold()
        .with_font_size(11)
        .with_color(rgb(INK));
    let body_style = Style::new().with_font_size(11).with_color(rgb(INK));
    let vi_style = Style::new().with_font_size(10).with_color(rgb(STEEL));

    // One caption segment per landscape page: header, big centered frame,
    // subtitle right below it. The footer (divider + clickable YouTube
    // reference) is painted at a fixed position by the decorator.
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            doc.push(PageBreak::new());
        }
        doc.push(para(&meta.title.to_uppercase(), &title_style));
        doc.push(para(
            &format!("ORIGINAL VIDEO FRAME / {}", meta.uploader.to_uppercase()),
            &kicker_style,
        ));
        doc.push(Break::new(1));

        if let Some(img_path) = &b.image {
            match Image::from_path(img_path) {
                Ok(img) => {
                    let (pw, ph) = image::image_dimensions(img_path).unwrap_or((1280, 720));
                    let translated = b.caption_translated.as_deref();
                    let s = frame_scale(pw, ph, frame_box_h(&b.caption, translated));
                    doc.push(
                        img.with_scale(Scale::new(s, s))
                            .with_alignment(Alignment::Center),
                    );
                }
                Err(e) => doc.push(para(&format!("[không nhúng được ảnh: {e}]"), &vi_style)),
            }
        }

        let mut sub = Paragraph::new("");
        sub.push_styled(format!("[{}]  ", format_timestamp(b.timestamp_secs)), ts_style.clone());
        sub.push_styled(b.caption.clone(), body_style.clone());
        doc.push(sub);
        if let Some(vi) = &b.caption_translated {
            if vi != &b.caption {
                doc.push(para(&format!("VI: {vi}"), &vi_style));
            }
        }
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).context("creating PDF output dir")?;
    }
    doc.render_to_file(out)
        .with_context(|| format!("writing PDF to {}", out.display()))?;

    // Clickable "YouTube reference" links: a nicety, never fatal.
    if let Err(e) = add_link_annotations(out, &urls) {
        eprintln!("warning: could not add PDF link annotations: {e:#}");
    }
    Ok(())
}
