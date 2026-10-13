//! `ytb2pdf-app` — native desktop UI (egui) for the YouTube -> PDF pipeline.
//!
//! The UI never blocks: the pipeline runs on a worker thread and reports
//! back through an mpsc channel. Visual language follows FilmCraft's
//! Premiere-style dark theme (see `theme.rs`).

mod theme;

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
        theme::apply(&cc.egui_ctx);
        theme::install_font(&cc.egui_ctx);
        Self {
            url: String::new(),
            translate: true,
            target_lang: "vi".to_string(),
            gemini_api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            cookies_file: None,
            running: false,
            stage: "Sẵn sàng".to_string(),
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
        cfg.target_lang = self
            .translate
            .then(|| self.target_lang.trim().to_string())
            .filter(|s| !s.is_empty());
        cfg.gemini_api_key =
            (!self.gemini_api_key.trim().is_empty()).then(|| self.gemini_api_key.trim().to_string());
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
            let res = run_pipeline(
                &cfg,
                &|ev| match ev {
                    PipelineEvent::Stage(s) => send(UiMsg::Stage(s.to_string())),
                    PipelineEvent::Progress { done, total } => {
                        send(UiMsg::Progress { done, total })
                    }
                    PipelineEvent::Log(m) => send(UiMsg::Log(m)),
                    PipelineEvent::Done(_) => {}
                },
            );
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
                        self.stage = "Hoàn thành".to_string();
                        finished = true;
                    }
                    UiMsg::Failed(e) => {
                        self.error = Some(e);
                        self.stage = "Thất bại".to_string();
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

    /// Header status: (dot colour, label).
    fn status(&self) -> (egui::Color32, &'static str) {
        if self.running {
            (theme::ACCENT, "Đang chạy")
        } else if self.result.is_some() {
            (theme::SUCCESS, "Hoàn thành")
        } else if self.error.is_some() {
            (theme::DANGER, "Lỗi")
        } else {
            (theme::TEXT_FAINT, "Sẵn sàng")
        }
    }
}

fn open_with_system_viewer(path: &PathBuf) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", &path.to_string_lossy()])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.running {
            ctx.request_repaint();
        }

        // ---- header bar ----
        egui::TopBottomPanel::top("header")
            .exact_height(56.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("YouTube → PDF")
                                .heading()
                                .strong()
                                .color(theme::TEXT),
                        );
                        ui.label(
                            egui::RichText::new("Biến video bài giảng thành tài liệu PDF")
                                .small()
                                .color(theme::TEXT_FAINT),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (dot, label) = self.status();
                        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter()
                            .circle_filled(r.center(), 4.0, dot);
                        ui.label(egui::RichText::new(label).small().color(theme::TEXT_DIM));
                        ui.add_space(8.0);
                    });
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            // ---- source ----
            theme::section(ui, "NGUỒN");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Link YouTube").color(theme::TEXT_DIM));
                ui.add_enabled(
                    !self.running,
                    egui::TextEdit::singleline(&mut self.url)
                        .hint_text("https://youtu.be/…")
                        .desired_width(f32::INFINITY),
                );
            });
            ui.add_space(4.0);
            let primary = egui::Button::new(
                egui::RichText::new(if self.running { "⏳ Đang xử lý…" } else { "▶  Tạo PDF" })
                    .color(egui::Color32::WHITE)
                    .strong(),
            )
            .fill(theme::ACCENT)
            .min_size(egui::vec2(200.0, 38.0));
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.running, primary).clicked() {
                    self.start();
                }
            });

            // ---- options ----
            theme::section(ui, "TÙY CHỌN");
            egui::Grid::new("options")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Dịch caption").color(theme::TEXT_DIM));
                    ui.horizontal(|ui| {
                        ui.add_enabled(
                            !self.running,
                            egui::Checkbox::new(&mut self.translate, ""),
                        );
                        ui.add_enabled(
                            !self.running && self.translate,
                            egui::TextEdit::singleline(&mut self.target_lang).desired_width(44.0),
                        );
                        ui.label(
                            egui::RichText::new("ngôn ngữ đích (vi, en, …)")
                                .small()
                                .color(theme::TEXT_FAINT),
                        );
                    });
                    ui.end_row();

                    ui.label(egui::RichText::new("Gemini API key").color(theme::TEXT_DIM));
                    ui.horizontal(|ui| {
                        ui.add_enabled(
                            !self.running && self.translate,
                            egui::TextEdit::singleline(&mut self.gemini_api_key)
                                .password(true)
                                .hint_text("dán key (free tại aistudio.google.com)")
                                .desired_width(280.0),
                        );
                        ui.label(
                            egui::RichText::new("chỉ lưu trong phiên chạy")
                                .small()
                                .color(theme::TEXT_FAINT),
                        );
                    });
                    ui.end_row();

                    ui.label(egui::RichText::new("Cookie").color(theme::TEXT_DIM));
                    ui.horizontal(|ui| {
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
                            egui::RichText::new(
                                self.cookies_file
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_else(|| "chưa chọn — chống 403/429 YouTube".to_string()),
                            )
                            .small()
                            .color(theme::TEXT_FAINT),
                        );
                    });
                    ui.end_row();
                });

            // ---- progress ----
            theme::section(ui, "TIẾN TRÌNH");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Trạng thái:").color(theme::TEXT_DIM));
                let stage_color = if self.error.is_some() {
                    theme::DANGER
                } else if self.result.is_some() {
                    theme::SUCCESS
                } else {
                    theme::TEXT
                };
                ui.label(egui::RichText::new(&self.stage).strong().color(stage_color));
            });
            if let Some((done, total)) = self.progress {
                let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                ui.add(
                    egui::ProgressBar::new(frac)
                        .text(format!("{done}/{total}"))
                        .desired_height(10.0),
                );
            }
            if let Some(path) = &self.result {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(theme::FIELD_BG)
                    .stroke(egui::Stroke::new(1.0, theme::FIELD_BORDER))
                    .corner_radius(4.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("✔ PDF đã tạo xong:")
                                    .strong()
                                    .color(theme::SUCCESS),
                            );
                            ui.monospace(egui::RichText::new(path.display().to_string()).small());
                            if ui.button("Mở PDF").clicked() {
                                open_with_system_viewer(path);
                            }
                        });
                    });
            }
            if let Some(err) = &self.error {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(theme::FIELD_BG)
                    .stroke(egui::Stroke::new(1.0, theme::DANGER))
                    .corner_radius(4.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(format!("Lỗi: {err}")).color(theme::DANGER));
                    });
            }

            // ---- log ----
            theme::section(ui, "NHẬT KÝ");
            egui::Frame::new()
                .fill(theme::FIELD_BG)
                .stroke(egui::Stroke::new(1.0, theme::FIELD_BORDER))
                .corner_radius(4.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            if self.logs.is_empty() {
                                ui.label(
                                    egui::RichText::new("— chưa có gì —")
                                        .small()
                                        .color(theme::TEXT_FAINT),
                                );
                            }
                            for line in &self.logs {
                                ui.monospace(
                                    egui::RichText::new(line).small().color(theme::TEXT_DIM),
                                );
                            }
                        });
                });
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([700.0, 680.0]),
        ..Default::default()
    };
    eframe::run_native(
        "YouTube → PDF",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
