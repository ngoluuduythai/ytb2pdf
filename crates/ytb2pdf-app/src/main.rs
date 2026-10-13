//! `ytb2pdf-app` — native desktop UI (egui) for the YouTube -> PDF pipeline.
//!
//! The UI never blocks: the pipeline runs on a worker thread and reports
//! back through an mpsc channel.

use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc;
use ytb2pdf_core::{run_pipeline, PipelineConfig, PipelineEvent};

enum UiMsg {
    Stage(String),
    Progress { done: usize, total: usize },
    Log(String),
    Done(PathBuf),
    Failed(String),
}

struct App {
    url: String,
    translate: bool,
    target_lang: String,
    gemini_api_key: String,
    cookies_file: Option<PathBuf>,
    running: bool,
    stage: String,
    progress: Option<(usize, usize)>,
    logs: Vec<String>,
    result: Option<PathBuf>,
    error: Option<String>,
    rx: Option<mpsc::Receiver<UiMsg>>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_vietnamese_font(&cc.egui_ctx);
        Self {
            url: String::new(),
            translate: true,
            target_lang: "vi".to_string(),
            gemini_api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            cookies_file: None,
            running: false,
            stage: "Nhập link YouTube rồi bấm Tạo PDF".to_string(),
            progress: None,
            logs: Vec::new(),
            result: None,
            error: None,
            rx: None,
        }
    }

    fn start(&mut self) {
        if self.running {
            return;
        }
        let url = self.url.trim().to_string();
        if url.is_empty() {
            self.error = Some("Anh ơi, dán link YouTube vào trước đã~".to_string());
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.running = true;
        self.logs.clear();
        self.result = None;
        self.error = None;
        self.progress = None;

        let mut cfg = PipelineConfig::default();
        cfg.url = url;
        cfg.target_lang = self.translate.then(|| self.target_lang.trim().to_string()).filter(|s| !s.is_empty());
        cfg.gemini_api_key = (!self.gemini_api_key.trim().is_empty())
            .then(|| self.gemini_api_key.trim().to_string());
        cfg.cookies_file = self.cookies_file.clone();
        let video_id = ytb2pdf_core::youtube::parse_youtube_url(&cfg.url)
            .map(|v| v.id)
            .unwrap_or_else(|_| "video".to_string());
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        cfg.work_dir = PathBuf::from(format!("{home}/.ytb2pdf/work-{video_id}"));
        cfg.out_pdf = PathBuf::from(format!("{home}/Desktop/ytb2pdf-{video_id}.pdf"));

        std::thread::spawn(move || {
            let send = |m: UiMsg| {
                let _ = tx.send(m);
            };
            let res = run_pipeline(&cfg, &|ev| match ev {
                PipelineEvent::Stage(s) => send(UiMsg::Stage(s.to_string())),
                PipelineEvent::Progress { done, total } => send(UiMsg::Progress { done, total }),
                PipelineEvent::Log(m) => send(UiMsg::Log(m)),
                PipelineEvent::Done(_) => {}
            });
            match res {
                Ok(p) => send(UiMsg::Done(p)),
                Err(e) => send(UiMsg::Failed(format!("{e:#}"))),
            }
        });
    }

    fn poll(&mut self) {
        let mut finished = false;
        if let Some(rx) = &self.rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    UiMsg::Stage(s) => {
                        self.stage = s;
                        self.progress = None;
                    }
                    UiMsg::Progress { done, total } => self.progress = Some((done, total)),
                    UiMsg::Log(m) => {
                        self.logs.push(m);
                        if self.logs.len() > 400 {
                            self.logs.remove(0);
                        }
                    }
                    UiMsg::Done(p) => {
                        self.result = Some(p);
                        self.stage = "Xong rồi nè~".to_string();
                        finished = true;
                    }
                    UiMsg::Failed(e) => {
                        self.error = Some(e);
                        self.stage = "Có lỗi xảy ra".to_string();
                        finished = true;
                    }
                }
            }
        }
        if finished {
            self.running = false;
            self.rx = None;
        }
    }
}

/// egui's bundled font may lack Vietnamese glyphs; try to load a system
/// TTF with Vietnamese coverage (DejaVu / Arial / Noto) at startup.
fn install_vietnamese_font(ctx: &egui::Context) {
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{home}/Library/Fonts/Arial.ttf"),
        "/Library/Fonts/Arial.ttf".to_string(),
        "/System/Library/Fonts/Supplemental/Arial.ttf".to_string(),
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".to_string(),
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "vietnamese".to_owned(),
                egui::FontData::from_owned(bytes).into(),
            );
            if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                prop.insert(0, "vietnamese".to_owned());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

fn open_with_system_viewer(path: &PathBuf) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/c", "start", "", &path.to_string_lossy()]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.running {
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.heading("YouTube → PDF");
            ui.label("Biến video bài giảng thành tài liệu PDF có caption + hình ảnh~");
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Link YouTube:");
                ui.add_enabled(
                    !self.running,
                    egui::TextEdit::singleline(&mut self.url)
                        .hint_text("https://youtu.be/…")
                        .desired_width(ui.available_width() - 110.0),
                );
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_enabled(!self.running, egui::Checkbox::new(&mut self.translate, "Dịch caption sang:"));
                ui.add_enabled(
                    !self.running && self.translate,
                    egui::TextEdit::singleline(&mut self.target_lang).desired_width(40.0),
                );
                ui.label("File cookie (chống 403/429):");
                if ui
                    .add_enabled(!self.running, egui::Button::new("Chọn cookies.txt…"))
                    .clicked()
                {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("cookies.txt", &["txt"])
                        .pick_file()
                    {
                        self.cookies_file = Some(p);
                    }
                }
                ui.monospace(
                    self.cookies_file
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(không bắt buộc)".to_string()),
                );
            });
            ui.label("Xuất file bằng extension Get cookies.txt LOCALLY khi đang mở youtube.com");

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Gemini API key:");
                ui.add_enabled(
                    !self.running && self.translate,
                    egui::TextEdit::singleline(&mut self.gemini_api_key)
                        .password(true)
                        .hint_text("dán key vào đây (free tại aistudio.google.com)")
                        .desired_width(ui.available_width() - 120.0),
                );
            });
            ui.label("Key chỉ lưu trong phiên chạy này, không ghi ra file.");

            ui.add_space(8.0);
            let btn = ui.add_enabled(
                !self.running,
                egui::Button::new("Tạo PDF").min_size(egui::vec2(160.0, 36.0)),
            );
            if btn.clicked() {
                self.start();
            }

            ui.add_space(8.0);
            ui.separator();
            ui.label(format!("{}", self.stage));
            if let Some((done, total)) = self.progress {
                let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(frac).text(format!("{done}/{total}")));
            }

            if let Some(err) = &self.error {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("Lỗi: {err}"));
            }
            if let Some(path) = &self.result {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.colored_label(
                        egui::Color32::from_rgb(80, 180, 100),
                        format!("{}", path.display()),
                    );
                    if ui.button("Mở PDF").clicked() {
                        open_with_system_viewer(path);
                    }
                });
            }

            ui.add_space(6.0);
            ui.label("Nhật ký:");
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.logs {
                        ui.monospace(line);
                    }
                });
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "YouTube → PDF",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
