//! Native desktop interface with an isolated, cancellable conversion worker.
mod model;
mod theme;
mod worker;

use eframe::egui::{self, RichText};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use worker::{Job, Message};

struct ConversionResult {
    path: PathBuf,
    bytes: u64,
    seconds: u64,
}

struct App {
    url: String,
    local: bool,
    local_file: Option<PathBuf>,
    transcript: Option<PathBuf>,
    v2: bool,
    translate: bool,
    language: String,
    api_key: String,
    show_key: bool,
    fast_whisper: bool,
    cookies: Option<PathBuf>,
    output: Option<PathBuf>,
    output_custom: bool,
    job: Option<Job>,
    cancelling: bool,
    started: Option<Instant>,
    elapsed: u64,
    stages: Vec<String>,
    progress: Option<(usize, usize)>,
    logs: VecDeque<String>,
    result: Option<ConversionResult>,
    error: Option<String>,
    notice: Option<String>,
    cancelled: bool,
    details_open: bool,
    advanced_open: bool,
    viewer_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            url: String::new(),
            local: false,
            local_file: None,
            transcript: None,
            v2: false,
            translate: false,
            language: "vi".into(),
            api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            show_key: false,
            fast_whisper: false,
            cookies: None,
            output: None,
            output_custom: false,
            job: None,
            cancelling: false,
            started: None,
            elapsed: 0,
            stages: Vec::new(),
            progress: None,
            logs: VecDeque::new(),
            result: None,
            error: None,
            notice: None,
            cancelled: false,
            details_open: false,
            advanced_open: false,
            viewer_rx: None,
        }
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        theme::install_font(&cc.egui_ctx);
        Self::default()
    }

    fn running(&self) -> bool {
        self.job.is_some()
    }

    fn source_error(&self) -> Option<String> {
        if self.local {
            match &self.local_file {
                Some(path) if path.is_file() => None,
                Some(_) => Some("The selected video no longer exists. Choose another file.".into()),
                None => Some("Choose a video file or drop one into the app.".into()),
            }
        } else {
            model::validate_url(&self.url).err().map(str::to_owned)
        }
    }

    fn settings_error(&self) -> Option<&'static str> {
        if self.translate && self.v2 && !self.local {
            Some("YouTube scene-aware processing does not support translation yet. Turn it off under Advanced, or keep captions in their original language.")
        } else if self.translate && self.api_key.trim().is_empty() {
            Some("Enter a Gemini API key to translate, or turn off Translate captions.")
        } else if self.transcript.as_ref().is_some_and(|p| !p.is_file()) {
            Some("The selected subtitle file no longer exists. Remove it or choose another file.")
        } else if !self.local && self.cookies.as_ref().is_some_and(|p| !p.is_file()) {
            Some("The selected cookies file no longer exists. Remove it or choose another file under Advanced.")
        } else {
            None
        }
    }

    fn refresh_output(&mut self) {
        if self.output_custom {
            return;
        }
        let stem = if self.local {
            self.local_file
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
        } else {
            model::validate_url(&self.url)
                .ok()
                .and_then(|_| ytb2pdf_core::youtube::parse_youtube_url(&self.url).ok())
                .map(|v| v.id)
        };
        self.output = stem.map(|s| model::available_output(&model::default_folder(), &s));
    }

    fn choose_video(&mut self, path: PathBuf) {
        self.local = true;
        self.local_file = Some(path);
        // A manually imported subtitle belongs to the previous source.
        self.transcript = None;
        self.result = None;
        self.cancelled = false;
        self.error = None;
        self.notice = None;
        self.refresh_output();
    }

    fn start(&mut self) {
        if self.running() {
            return;
        }
        let validation = self
            .source_error()
            .or_else(|| self.settings_error().map(str::to_owned))
            .or_else(|| {
                self.output
                    .as_ref()
                    .and_then(|p| model::validate_output(p).err())
                    .map(str::to_owned)
            });
        if let Some(error) = validation {
            self.notice = Some(error);
            return;
        }
        let Some(destination) = self.output.clone() else {
            return;
        };
        let root = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(model::default_folder)
            .join(".ytb2pdf");
        let request = worker::Request {
            url: self.url.trim().into(),
            local_file: if self.local {
                self.local_file.clone()
            } else {
                None
            },
            v2: self.v2,
            target_lang: self.translate.then(|| self.language.clone()),
            api_key: self.translate.then(|| self.api_key.trim().into()),
            fast_whisper: self.fast_whisper,
            transcript: self.transcript.clone(),
            cookies: if self.local {
                None
            } else {
                self.cookies.clone()
            },
            work_dir: root,
            destination,
        };
        match Job::start(request) {
            Ok(job) => {
                self.job = Some(job);
                self.cancelling = false;
                self.started = Some(Instant::now());
                self.elapsed = 0;
                self.stages = vec!["Preparing conversion".into()];
                self.progress = None;
                self.logs.clear();
                self.result = None;
                self.error = None;
                self.notice = None;
                self.cancelled = false;
                self.details_open = false;
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    fn log(&mut self, mut line: String) {
        if !self.api_key.trim().is_empty() {
            line = line.replace(self.api_key.trim(), "[API key hidden]");
        }
        // Bound both the number and size of lines from external processes.
        if line.len() > 4096 {
            let boundary = line.floor_char_boundary(4096);
            line.truncate(boundary);
            line.push('…');
        }
        if self.logs.len() >= 400 {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    fn poll(&mut self) {
        if let Some(result) = self.viewer_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.viewer_rx = None;
            if let Err(error) = result {
                self.notice = Some(error);
            }
        }
        if self.running() {
            self.elapsed = self.started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
        }
        let mut terminal = false;
        while let Some(message) = self.job.as_ref().and_then(|j| j.rx.try_recv().ok()) {
            match message {
                Message::Stage(stage) => {
                    self.log(format!("Step: {stage}"));
                    if self.stages.last() != Some(&stage) {
                        self.stages.push(stage);
                    }
                    self.progress = None;
                }
                Message::Progress { done, total } => self.progress = Some((done, total)),
                Message::Log(line) => self.log(line),
                Message::Done(path) => {
                    self.result = Some(ConversionResult {
                        bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                        path,
                        seconds: self.elapsed,
                    });
                    terminal = true;
                }
                Message::Failed(error) => {
                    self.log(error.clone());
                    self.error = Some(error);
                    self.details_open = true;
                    terminal = true;
                }
                Message::Cancelled => {
                    self.cancelled = true;
                    terminal = true;
                }
            }
        }
        if terminal {
            self.job = None;
            self.cancelling = false;
            self.progress = None;
            self.started = None;
        }
    }

    fn reset(&mut self) {
        self.result = None;
        self.error = None;
        self.notice = None;
        self.cancelled = false;
        self.url.clear();
        self.local_file = None;
        self.transcript = None;
        self.output = None;
        self.output_custom = false;
        self.logs.clear();
        self.stages.clear();
        self.details_open = false;
    }

    fn source_ui(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "01  SOURCE VIDEO");
        theme::card(ui, |ui| {
            let before = self.local;
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.local, false, "YouTube link");
                ui.selectable_value(&mut self.local, true, "Video on this device");
            });
            if before != self.local {
                self.transcript = None;
                self.refresh_output();
            }
            ui.add_space(10.0);
            if self.local {
                egui::Frame::new()
                    .fill(theme::FIELD_BG)
                    .stroke(egui::Stroke::new(1.0_f32, theme::FIELD_BORDER))
                    .corner_radius(6.0)
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        ui.set_min_width((ui.available_width() - 1.0).max(0.0));
                        if let Some(path) = &self.local_file {
                            ui.label(
                                RichText::new(
                                    path.file_name().unwrap_or_default().to_string_lossy(),
                                )
                                .strong(),
                            );
                            ui.label(
                                RichText::new(path.display().to_string())
                                    .small()
                                    .color(theme::TEXT_DIM),
                            );
                        } else {
                            ui.label(RichText::new("Drop a video here").strong());
                            ui.label(
                                RichText::new(
                                    "MP4, MOV, MKV, WebM and other FFmpeg-supported formats",
                                )
                                .small()
                                .color(theme::TEXT_DIM),
                            );
                        }
                        if ui
                            .button(if self.local_file.is_some() {
                                "Change video…"
                            } else {
                                "Choose video…"
                            })
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Video", &["mp4", "mov", "mkv", "webm", "avi", "m4v"])
                                .pick_file()
                            {
                                self.choose_video(path);
                            }
                        }
                    });
                ui.label(RichText::new("Video processing stays on your device. Translation shares captions with Gemini only when enabled.")
                    .small().color(theme::TEXT_DIM));
            } else {
                ui.label("YouTube video URL");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.url)
                            .hint_text("https://www.youtube.com/watch?v=…")
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.transcript = None;
                    self.refresh_output();
                }
                if !self.url.trim().is_empty() {
                    if let Some(error) = self.source_error() {
                        inline_error(ui, &error);
                    }
                } else {
                    ui.label(
                        RichText::new("Paste a video link to get started.")
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Subtitles").strong());
            ui.label(RichText::new(if self.transcript.is_some() {
                "Use the selected file instead of automatic captions or speech recognition."
            } else if self.local {
                "Automatically finds matching SRT/VTT files or embedded subtitles, then uses speech recognition if needed."
            } else if self.v2 {
                "Uses YouTube captions. Scene-aware YouTube processing needs captions or an imported subtitle file."
            } else {
                "Uses YouTube captions, then speech recognition if needed."
            }).small().color(theme::TEXT_DIM));
            let mut remove = false;
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if self.transcript.is_some() {
                        "Change subtitles…"
                    } else {
                        "Import SRT / VTT…"
                    })
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Subtitles", &["srt", "vtt"])
                        .pick_file()
                    {
                        self.transcript = Some(path);
                    }
                }
                if let Some(path) = &self.transcript {
                    ui.label(
                        RichText::new(path.file_name().unwrap_or_default().to_string_lossy())
                            .color(theme::SUCCESS),
                    )
                    .on_hover_text(path.display().to_string());
                    remove = ui.small_button("Remove").clicked();
                }
            });
            if remove {
                self.transcript = None;
            }
        });
    }

    fn options_ui(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "02  PDF OPTIONS");
        theme::card(ui, |ui| {
            ui.checkbox(&mut self.translate, "Translate captions");
            if self.translate {
                ui.label(
                    RichText::new("Keep original captions and add a translation.")
                        .small()
                        .color(theme::TEXT_DIM),
                );
                egui::ComboBox::from_id_salt("language")
                    .selected_text(
                        model::LANGUAGES
                            .iter()
                            .find(|(code, _)| *code == self.language)
                            .map(|(_, label)| *label)
                            .unwrap_or("Vietnamese"),
                    )
                    .show_ui(ui, |ui| {
                        for (code, name) in model::LANGUAGES {
                            ui.selectable_value(&mut self.language, (*code).into(), *name);
                        }
                    });
                ui.add_space(4.0);
                ui.label("Gemini API key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.api_key)
                        .password(!self.show_key)
                        .hint_text("Enter your Gemini API key")
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.show_key, "Show key");
                    ui.label(
                        RichText::new("Kept in memory for this session.")
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                });
                ui.label(RichText::new("Captions are sent to Gemini. Long videos may take more time and use more API quota.")
                    .small().color(theme::TEXT_DIM));
                if self.v2 && !self.local {
                    inline_error(
                        ui,
                        "Translation is unavailable with YouTube scene-aware processing.",
                    );
                    if ui.small_button("Use standard processing").clicked() {
                        self.v2 = false;
                    }
                }
            } else {
                ui.label(
                    RichText::new("Keep captions in their original language. No API key needed.")
                        .small()
                        .color(theme::TEXT_DIM),
                );
            }
            ui.add_space(6.0);
            egui::CollapsingHeader::new("Advanced").id_salt("advanced")
                .open(self.advanced_open.then_some(true))
                .show(ui, |ui| {
                    ui.checkbox(&mut self.v2, "Scene-aware processing · experimental");
                    ui.label(RichText::new(if self.local {
                        "Selects images around scene changes instead of subtitle timestamps. Output can differ from standard processing."
                    } else {
                        "Downloads selected video sections. Requires captions; translation and automatic speech recognition are unavailable in this mode."
                    }).small().color(theme::TEXT_DIM));
                    ui.add_space(8.0);
                    ui.label(RichText::new("Speech recognition fallback").strong());
                    ui.add_enabled_ui(self.local || !self.v2, |ui| {
                        egui::ComboBox::from_id_salt("whisper")
                            .selected_text(if self.fast_whisper { "Fast · ~75 MB" } else { "Balanced · ~147 MB" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.fast_whisper, true, "Fast · ~75 MB");
                                ui.selectable_value(&mut self.fast_whisper, false, "Balanced · ~147 MB");
                            });
                    });
                    ui.label(RichText::new("Used only when subtitles are missing. Downloads the model on first use.").small().color(theme::TEXT_DIM));
                    if !self.local {
                        ui.add_space(8.0);
                        ui.label(RichText::new("YouTube authentication").strong());
                        ui.label(RichText::new("Optional cookies.txt for sign-in or access errors.").small().color(theme::TEXT_DIM));
                        let mut remove = false;
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Choose cookies.txt…").clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("Cookies", &["txt"]).pick_file() { self.cookies = Some(path); }
                            }
                            if let Some(path) = &self.cookies {
                                ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                                remove = ui.small_button("Remove").clicked();
                            }
                        });
                        if remove { self.cookies = None; }
                    }
                });
            self.advanced_open = false;
        });
    }

    fn output_ui(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "03  SAVE PDF");
        theme::card(ui, |ui| {
            if let Some(path) = &self.output {
                ui.label(
                    RichText::new(path.file_name().unwrap_or_default().to_string_lossy()).strong(),
                );
                ui.label(
                    RichText::new(path.parent().unwrap_or(Path::new("")).display().to_string())
                        .small()
                        .color(theme::TEXT_DIM),
                );
                if let Err(error) = model::validate_output(path) {
                    inline_error(ui, error);
                }
            } else {
                ui.label(
                    RichText::new(
                        "Choose a source to suggest a filename, or select a destination below.",
                    )
                    .color(theme::TEXT_DIM),
                );
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button("Choose where to save…").clicked() {
                    let suggested = self.output.clone().unwrap_or_else(|| {
                        model::available_output(&model::default_folder(), "video")
                    });
                    if let Some(path) = rfd::FileDialog::new()
                        .set_directory(suggested.parent().unwrap_or(Path::new(".")))
                        .set_file_name(suggested.file_name().unwrap_or_default().to_string_lossy())
                        .add_filter("PDF document", &["pdf"])
                        .save_file()
                    {
                        self.output = Some(path.with_extension("pdf"));
                        self.output_custom = true;
                    }
                }
                if self.output_custom && ui.small_button("Use suggested name").clicked() {
                    self.output_custom = false;
                    self.refresh_output();
                }
            });
        });
    }

    fn progress_ui(&mut self, ui: &mut egui::Ui) {
        theme::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(if self.cancelling {
                        "Stopping conversion…"
                    } else {
                        "Creating your PDF"
                    })
                    .strong(),
                );
                ui.label(
                    RichText::new(model::duration(self.elapsed))
                        .small()
                        .color(theme::TEXT_DIM),
                );
            });
            ui.add_space(6.0);
            // Actual emitted steps are shown rather than an invented overall percentage.
            let first = self.stages.len().saturating_sub(3);
            for (index, stage) in self.stages.iter().enumerate().skip(first) {
                let current = index + 1 == self.stages.len();
                ui.label(
                    RichText::new(format!(
                        "{}  {stage}",
                        if current { "Now:" } else { "Done:" }
                    ))
                    .color(if current {
                        theme::TEXT
                    } else {
                        theme::TEXT_DIM
                    }),
                );
            }
            if let Some((done, total)) = self.progress.filter(|(_, total)| *total > 0) {
                let fraction = (done as f32 / total as f32).clamp(0.0, 1.0);
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_height(22.0)
                        .text(format!("Current step: {done} / {total}")),
                );
            } else {
                ui.label(
                    RichText::new("Working… progress will appear when this step reports a total.")
                        .small()
                        .color(theme::TEXT_DIM),
                );
            }
            ui.add_space(4.0);
            if ui
                .add_enabled(!self.cancelling, egui::Button::new("Cancel conversion"))
                .clicked()
            {
                if let Some(job) = &self.job {
                    job.cancel();
                }
                self.cancelling = true;
            }
        });
    }

    fn result_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = &self.result {
            let path = result.path.clone();
            let seconds = result.seconds;
            let bytes = result.bytes;
            theme::card(ui, |ui| {
                ui.label(
                    RichText::new("Your PDF is ready")
                        .heading()
                        .color(theme::SUCCESS),
                );
                ui.label(
                    RichText::new(path.file_name().unwrap_or_default().to_string_lossy()).strong(),
                );
                ui.label(
                    RichText::new(format!(
                        "{:.1} MB · completed in {}",
                        bytes as f64 / 1_048_576.0,
                        model::duration(seconds)
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
                ui.label(
                    RichText::new(path.display().to_string())
                        .small()
                        .color(theme::TEXT_DIM),
                );
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.add(primary_button("Open PDF")).clicked() {
                        self.open(&path);
                    }
                    if ui.button("Open folder").clicked() {
                        if let Some(folder) = path.parent() {
                            self.open(folder);
                        }
                    }
                    if ui.button("Convert another video").clicked() {
                        self.reset();
                    }
                });
            });
        }
        if let Some(error) = &self.error {
            let (title, help) = model::error_guidance(error);
            theme::card(ui, |ui| {
                ui.label(RichText::new(title).strong().color(theme::DANGER));
                ui.label(help);
                ui.label("Completed stages are saved. Resume reuses them with the same source and settings.");
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Show processing details").clicked() {
                        self.details_open = true;
                    }
                    if ui.button("Review advanced settings").clicked() {
                        self.advanced_open = true;
                    }
                });
            });
        }
        if self.cancelled {
            theme::card(ui, |ui| {
                ui.label(RichText::new("Conversion cancelled").strong());
                ui.label(
                    "Completed stages are saved. Resume to continue without repeating finished work.",
                );
            });
        }
    }

    fn open(&mut self, path: &Path) {
        let path = path.to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        self.viewer_rx = Some(rx);
        self.notice = None;
        std::thread::spawn(move || {
            let result = open_with_system_viewer(&path)
                .map_err(|error| format!("Could not open {}. {error}", path.display()));
            let _ = tx.send(result);
        });
    }

    fn ui(&mut self, ctx: &egui::Context) {
        self.poll();
        let running = self.running();
        if running || self.viewer_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        if !running {
            let dropped = ctx.input(|i| i.raw.dropped_files.clone());
            for dropped in dropped {
                if let Some(path) = dropped.path {
                    if path.is_file() {
                        match path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
                            "srt" | "vtt" => self.transcript = Some(path),
                            "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "mpeg" | "mpg" | "wmv" | "flv" | "ts" | "mts" => self.choose_video(path),
                            _ => self.notice = Some("Drop a supported video or SRT/VTT subtitle file. For other video formats, use Choose video.".into()),
                        }
                    }
                }
            }
        }
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(22, 16)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("ytb2pdf {}", ytb2pdf_core::VERSION))
                            .heading()
                            .strong(),
                    );
                    let (label, color) = if running {
                        ("PROCESSING", theme::ACCENT_HOVER)
                    } else if self.result.is_some() {
                        ("COMPLETE", theme::SUCCESS)
                    } else {
                        ("VIDEO / DOCUMENT", theme::TEXT_DIM)
                    };
                    ui.label(RichText::new(label).small().color(color));
                });
                ui.label(
                    RichText::new("Turn lectures and tutorials into illustrated study notes.")
                        .small()
                        .color(theme::TEXT_DIM),
                );
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::CANVAS_BG)
                    .inner_margin(egui::Margin::symmetric(18, 12)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Full-page scrolling keeps every control reachable at small sizes and high DPI.
                        ui.set_max_width(ui.available_width().min(860.0));
                        if running {
                            self.progress_ui(ui);
                        } else {
                            self.result_ui(ui);
                            if self.result.is_none() {
                                self.source_ui(ui);
                                self.options_ui(ui);
                                self.output_ui(ui);
                                ui.add_space(12.0);
                                let issue = self
                                    .source_error()
                                    .or_else(|| self.settings_error().map(str::to_owned))
                                    .or_else(|| {
                                        self.output
                                            .as_ref()
                                            .and_then(|p| model::validate_output(p).err())
                                            .map(str::to_owned)
                                    });
                                let ready = issue.is_none() && self.output.is_some();
                                if let Some(error) = self.settings_error() {
                                    inline_error(ui, error);
                                }
                                let action = if self.error.is_some() || self.cancelled {
                                    "Resume conversion"
                                } else {
                                    "Generate PDF"
                                };
                                if ui
                                    .add_enabled(ready, primary_button(action))
                                    .on_disabled_hover_text(
                                        issue
                                            .as_deref()
                                            .unwrap_or("Choose a source and output file first."),
                                    )
                                    .clicked()
                                {
                                    self.start();
                                }
                            }
                        }
                        if let Some(notice) = self.notice.clone() {
                            ui.add_space(8.0);
                            inline_error(ui, &notice);
                            if ui.small_button("Dismiss").clicked() {
                                self.notice = None;
                            }
                        }
                        if !self.logs.is_empty() {
                            ui.add_space(12.0);
                            egui::CollapsingHeader::new("Processing details")
                                .id_salt("details")
                                .open(self.details_open.then_some(true))
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.small_button("Copy details").clicked() {
                                            ui.ctx().copy_text(
                                                self.logs
                                                    .iter()
                                                    .cloned()
                                                    .collect::<Vec<_>>()
                                                    .join("\n"),
                                            );
                                        }
                                        ui.label(
                                            RichText::new("Most recent 400 messages")
                                                .small()
                                                .color(theme::TEXT_DIM),
                                        );
                                    });
                                    egui::ScrollArea::vertical()
                                        .id_salt("log-scroll")
                                        .max_height(220.0)
                                        .stick_to_bottom(true)
                                        .show(ui, |ui| {
                                            for line in &self.logs {
                                                ui.label(
                                                    RichText::new(line)
                                                        .monospace()
                                                        .small()
                                                        .color(theme::TEXT_DIM),
                                                );
                                            }
                                        });
                                });
                        }
                        self.details_open = false;
                        ui.add_space(18.0);
                    });
            });
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            egui::Area::new("drop-overlay".into())
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(24.0)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(if running {
                                    "Wait for the current conversion to finish"
                                } else {
                                    "Drop a video or subtitle file"
                                })
                                .heading(),
                            );
                        });
                });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }
}

fn inline_error(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(theme::DANGER));
}

fn primary_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).strong().color(egui::Color32::WHITE))
        .fill(theme::ACCENT)
        .min_size(egui::vec2(160.0, 42.0))
}

fn open_with_system_viewer(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(path.exists(), "The file or folder no longer exists.");
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(path).status()?;
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("explorer").arg(path).status()?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let status = std::process::Command::new("xdg-open").arg(path).status()?;
    anyhow::ensure!(
        status.success(),
        "The system viewer could not open it. Open the saved file manually."
    );
    Ok(())
}

/// Include the tools shipped beside the executable in the macOS app bundle.
fn ensure_bundled_tools_on_path() {
    let bin = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("../Resources/bin")));
    if let Some(bin) = bin {
        if bin.join("yt-dlp").exists() || bin.join("ffmpeg").exists() {
            let bin = std::fs::canonicalize(&bin).unwrap_or(bin);
            let mut paths = vec![bin];
            if let Some(path) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&path));
            }
            if let Ok(path) = std::env::join_paths(paths) {
                std::env::set_var("PATH", path);
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    if std::env::args()
        .nth(1)
        .is_some_and(|arg| arg == "--version" || arg == "-V")
    {
        println!(
            "ytb2pdf {} ({})",
            ytb2pdf_core::VERSION,
            ytb2pdf_core::BUILD_COMMIT
        );
        return Ok(());
    }
    ensure_bundled_tools_on_path();
    if std::env::args().nth(1).as_deref() == Some(worker::ARG) {
        std::process::exit(if worker::run().is_ok() { 0 } else { 1 });
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780.0, 860.0])
            .with_min_inner_size([400.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        &format!("ytb2pdf {} — Video to PDF", ytb2pdf_core::VERSION),
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_block_missing_keys_and_unsupported_translation() {
        let mut app = App {
            translate: true,
            api_key: String::new(),
            ..Default::default()
        };
        assert!(app.settings_error().unwrap().contains("API key"));
        app.api_key = "test-key".into();
        app.v2 = true;
        assert!(app.settings_error().unwrap().contains("YouTube"));
        app.local = true;
        assert!(app.settings_error().is_none());
    }
    #[test]
    fn new_local_source_clears_previous_transcript_and_updates_filename() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("New lesson.mp4");
        std::fs::write(&source, []).unwrap();
        let mut app = App {
            transcript: Some(folder.path().join("old.srt")),
            ..Default::default()
        };
        app.choose_video(source);
        assert!(app.transcript.is_none());
        assert!(app
            .output
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("New lesson"));
    }
    #[test]
    fn logs_are_bounded_and_redact_api_keys() {
        let mut app = App {
            api_key: "secret-test-key".into(),
            ..Default::default()
        };
        for _ in 0..450 {
            app.log("request with secret-test-key".into());
        }
        assert_eq!(app.logs.len(), 400);
        assert!(app.logs.iter().all(|l| !l.contains("secret-test-key")));
    }
    #[test]
    fn renders_empty_translated_error_and_result_states_at_small_and_large_sizes() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        theme::install_font(&ctx);
        let mut app = App::default();
        for width in [400.0, 780.0, 1280.0] {
            for state in 0..4 {
                app.translate = state == 1;
                app.error = (state == 2).then(|| "YouTube returned 429".into());
                app.result = (state == 3).then(|| ConversionResult {
                    path: PathBuf::from("/a/very/long/path/lecture.pdf"),
                    bytes: 1024,
                    seconds: 30,
                });
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.ui(ctx),
                );
                assert!(!output.shapes.is_empty());
                for shape in output.shapes {
                    // Catch cards and controls escaping the window when the
                    // viewport gets narrower than the preferred content width.
                    let bounds = shape.shape.visual_bounding_rect();
                    if !bounds.is_finite() {
                        continue;
                    }
                    assert!(
                        bounds.max.x <= width + 1.0,
                        "Horizontal overflow at width {width}, state {state}: {:?}",
                        shape.shape
                    );
                }
            }
        }
    }
}
