//! `ytb2pdf-app` — native desktop UI (egui) for the YouTube -> PDF pipeline.
//!
//! The UI never blocks: the pipeline runs on a worker thread and reports
//! back through an mpsc channel. Visual language follows FilmCraft's
//! Premiere-style dark theme (see `theme.rs`).

mod theme;

use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc;
use ytb2pdf_core::{run_pipeline, whisper::WhisperModel, PipelineConfig, PipelineEvent};

enum UiMsg {
    Stage(String),
    Progress { done: usize, total: usize },
    Log(String),
    Done(PathBuf),
    Failed(String),
}

struct App {
    url: String,
    use_local_file: bool,
    local_file: Option<PathBuf>,
    use_v2: bool,
    translate: bool,
    target_lang: String,
    gemini_api_key: String,
    whisper_model: WhisperModel,
    cookies_file: Option<PathBuf>,
    transcript_file: Option<PathBuf>,
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
            use_local_file: false,
            local_file: None,
            use_v2: false,
            translate: true,
            target_lang: "vi".to_string(),
            gemini_api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            whisper_model: WhisperModel::default(),
            cookies_file: None,
            transcript_file: None,
            running: false,
            stage: "Ready".to_string(),
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
        if self.use_local_file {
            if self.local_file.as_ref().map(|p| p.is_file()) != Some(true) {
                self.error = Some("Please select a valid local video file.".to_string());
                return;
            }
        } else if url.is_empty() {
            self.error = Some("Please enter a YouTube URL.".to_string());
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
        cfg.local_file = if self.use_local_file { self.local_file.clone() } else { None };
        cfg.v2 = self.use_v2;
        cfg.target_lang = self
            .translate
            .then(|| self.target_lang.trim().to_string())
            .filter(|s| !s.is_empty());
        cfg.gemini_api_key =
            (!self.gemini_api_key.trim().is_empty()).then(|| self.gemini_api_key.trim().to_string());
        cfg.whisper_model = self.whisper_model;
        cfg.cookies_file = self.cookies_file.clone();
        cfg.transcript_file = self.transcript_file.clone();
        let video_id = if let Some(p) = &cfg.local_file {
            p.file_stem().and_then(|s| s.to_str()).unwrap_or("video").to_string()
        } else {
            ytb2pdf_core::youtube::parse_youtube_url(&cfg.url)
                .map(|v| v.id).unwrap_or_else(|_| "video".to_string())
        };
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
                        self.stage = "Completed".to_string();
                        finished = true;
                    }
                    UiMsg::Failed(e) => {
                        self.error = Some(e);
                        self.stage = "Failed".to_string();
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
            (theme::ACCENT, "Running")
        } else if self.result.is_some() {
            (theme::SUCCESS, "Completed")
        } else if self.error.is_some() {
            (theme::DANGER, "Error")
        } else {
            (theme::TEXT_FAINT, "Ready")
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
            .exact_height(64.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Video » PDF")
                                .heading()
                                .strong()
                                .color(theme::TEXT),
                        );
                        ui.label(
                            egui::RichText::new("Turn video lectures into PDF documents")
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
            theme::section(ui, "SOURCE");
            theme::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!self.running, |ui| {
                        ui.radio_value(&mut self.use_local_file, false, "YouTube URL");
                        ui.radio_value(&mut self.use_local_file, true, "Local Video File");
                    });
                });
                if self.use_local_file {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!self.running, egui::Button::new("Choose Video File…")).clicked() {
                            if let Some(file) = rfd::FileDialog::new()
                                .add_filter("Video files", &["mp4", "mov", "mkv", "webm", "avi"])
                                .pick_file()
                            {
                                self.local_file = Some(file);
                            }
                        }
                        if let Some(file) = &self.local_file {
                            ui.label(file.file_name().unwrap_or_default().to_string_lossy().to_string());
                        } else {
                            ui.label("No video selected");
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("YouTube URL").color(theme::TEXT_DIM));
                        ui.add_enabled(!self.running,
                            egui::TextEdit::singleline(&mut self.url)
                                .hint_text("https://youtu.be/…")
                                .desired_width(f32::INFINITY));
                    });
                }
                ui.add_space(4.0);
                // Optional transcript import: skips YouTube captions AND Whisper.
                ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Transcript").color(theme::TEXT_DIM));
                if ui
                .add_enabled(!self.running, egui::Button::new("Choose subtitle file…"))
                .clicked()
                {
                if let Some(p) = rfd::FileDialog::new()
                .add_filter("Subtitles", &["srt", "vtt"])
                .pick_file()
                {
                self.transcript_file = Some(p);
                }
                }
                match &self.transcript_file {
                Some(p) => {
                ui.label(
                egui::RichText::new(format!(
                "• {}",
                p.file_name().unwrap_or_default().to_string_lossy()
                ))
                .color(theme::SUCCESS),
                );
                if ui
                .add_enabled(!self.running, egui::Button::new("×"))
                .on_hover_text("Remove transcript file")
                .clicked()
                {
                self.transcript_file = None;
                }
                }
                None => {
                ui.label(
                egui::RichText::new("(optional — skips YouTube captions and Whisper)")
                .small()
                .color(theme::TEXT_FAINT),
                );
                }
                }
                });
                ui.add_space(8.0);
                let primary = egui::Button::new(
                egui::RichText::new(if self.running { "Processing…" } else { "Generate PDF" })
                .color(egui::Color32::WHITE)
                .strong(),
                )
                .fill(theme::ACCENT)
                .min_size(egui::vec2(240.0, 44.0));
                if ui.add_enabled(!self.running, primary).clicked() {
                self.start();
                }
            }); // end card

            // ---- options ----
            theme::section(ui, "OPTIONS");
            ui.add_enabled(!self.running, egui::Checkbox::new(&mut self.use_v2, "Scene-aware V2 processing (experimental)"));
            egui::Grid::new("options")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Translate captions").color(theme::TEXT_DIM));
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
                            egui::RichText::new("target language (vi, en, …)")
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
                                .hint_text("Enter API key (aistudio.google.com)")
                                .desired_width(280.0),
                        );
                        ui.label(
                            egui::RichText::new("stored for this session only")
                                .small()
                                .color(theme::TEXT_FAINT),
                        );
                    });
                    ui.end_row();

                    ui.label(egui::RichText::new("Whisper").color(theme::TEXT_DIM));
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("whisper_model")
                            .selected_text(format!(
                                "{} ({})",
                                self.whisper_model.label(),
                                self.whisper_model.size_hint()
                            ))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.whisper_model,
                                    WhisperModel::Fast,
                                    "Fast (tiny, ~75 MB)",
                                );
                                ui.selectable_value(
                                    &mut self.whisper_model,
                                    WhisperModel::Medium,
                                    "Balanced (base, ~147 MB)",
                                );
                            });
                        ui.label(
                            egui::RichText::new("model downloads once if captions are unavailable")
                                .small()
                                .color(theme::TEXT_FAINT),
                        );
                    });
                    ui.end_row();

                    ui.label(egui::RichText::new("Cookie").color(theme::TEXT_DIM));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!self.running, egui::Button::new("Choose cookies.txt…"))
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
                                    .unwrap_or_else(|| "not selected — may help with YouTube 403/429 errors".to_string()),
                            )
                            .small()
                            .color(theme::TEXT_FAINT),
                        );
                    });
                    ui.end_row();
                });

            // ---- progress ----
            theme::section(ui, "PROGRESS");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Status:").color(theme::TEXT_DIM));
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
                                egui::RichText::new("✔ PDF created:")
                                    .strong()
                                    .color(theme::SUCCESS),
                            );
                            ui.monospace(egui::RichText::new(path.display().to_string()).small());
                            if ui.button("Open PDF").clicked() {
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
                        ui.label(egui::RichText::new(format!("Error: {err}")).color(theme::DANGER));
                    });
            }

            // ---- log ----
            theme::section(ui, "LOGS");
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
                                    egui::RichText::new("— no logs yet —")
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

/// The .dmg ships `yt-dlp` + `ffmpeg` inside the .app bundle
/// (`Contents/Resources/bin/`). Put that dir on PATH so the pipeline's
/// child processes find them without any Homebrew install.
fn ensure_bundled_tools_on_path() {
    let bin = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
        .map(|macos_dir| macos_dir.join("../Resources/bin"));
    if let Some(bin) = bin {
        if bin.join("yt-dlp").exists() || bin.join("ffmpeg").exists() {
            let bin = std::fs::canonicalize(&bin).unwrap_or(bin);
            let path = std::env::var("PATH").unwrap_or_default();
            std::env::set_var("PATH", format!("{}:{path}", bin.display()));
        }
    }
}

fn main() -> eframe::Result<()> {
    ensure_bundled_tools_on_path();
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
