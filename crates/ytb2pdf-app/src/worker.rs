//! Isolate a conversion so cancellation stops the worker and its media processes.
//! Configuration (including the API key) travels over stdin, never command arguments.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::JoinHandle;
use std::time::Duration;
use ytb2pdf_core::{whisper::WhisperModel, PipelineConfig, PipelineEvent};

pub const ARG: &str = "--conversion-worker";

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub url: String,
    pub local_file: Option<PathBuf>,
    pub v2: bool,
    pub target_lang: Option<String>,
    pub api_key: Option<String>,
    pub fast_whisper: bool,
    pub transcript: Option<PathBuf>,
    pub cookies: Option<PathBuf>,
    pub work_dir: PathBuf,
    pub destination: PathBuf,
}

impl Request {
    fn config(&self) -> PipelineConfig {
        PipelineConfig {
            url: self.url.clone(),
            local_file: self.local_file.clone(),
            v2: self.v2,
            target_lang: self.target_lang.clone(),
            gemini_api_key: self.api_key.clone(),
            whisper_model: if self.fast_whisper {
                WhisperModel::Fast
            } else {
                WhisperModel::Medium
            },
            transcript_file: self.transcript.clone(),
            cookies_file: self.cookies.clone(),
            work_dir: self.work_dir.clone(),
            out_pdf: self.destination.clone(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    Stage(String),
    Progress { done: usize, total: usize },
    Log(String),
    Done(PathBuf),
    Failed(String),
    Cancelled,
}

pub struct Job {
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub rx: mpsc::Receiver<Message>,
}

impl Job {
    pub fn start(request: Request) -> Result<Self> {
        Self::start_with_executable(request, &std::env::current_exe()?)
    }

    fn start_with_executable(mut request: Request, executable: &std::path::Path) -> Result<Self> {
        let workspace = ytb2pdf_core::resume::prepare_workspace(&request.config())?;
        request.work_dir = workspace.clone();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(workspace.join("conversion.lock"))?;
        lock.try_lock()
            .context("This source and settings are already being processed in another window")?;
        // Temporary writes are disposable; completed downloads/frames/checkpoints
        // live beside them and survive failures, cancellation and app restarts.
        let attempt = tempfile::Builder::new()
            .prefix("attempt-")
            .tempdir_in(&workspace)?;
        let parent = request
            .destination
            .parent()
            .context("Choose an output folder")?;
        let staging = tempfile::Builder::new()
            .prefix(".ytb2pdf-")
            .suffix(".pdf")
            .tempfile_in(parent)
            .context("Cannot write to the selected output folder")?;
        let destination = request.destination.clone();
        request.destination = staging.path().to_path_buf();
        let payload = serde_json::to_vec(&request)?;
        let mut command = Command::new(executable);
        command
            .env("TMPDIR", attempt.path())
            .env("TMP", attempt.path())
            .env("TEMP", attempt.path())
            .arg(ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .spawn()
            .context("Could not start the conversion worker")?;
        if let Err(error) = child.stdin.take().unwrap().write_all(&payload) {
            stop_tree(&mut child);
            let _ = child.wait();
            return Err(error.into());
        }
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let token = Arc::clone(&cancel);
        // Bounded channel prevents verbose external tools from growing memory indefinitely.
        let (tx, rx) = mpsc::sync_channel(256);
        let thread = std::thread::spawn(move || {
            let _ = tx.send(Message::Log(format!(
                "Saved progress: {}",
                workspace.display()
            )));
            let _lock = lock;
            let _attempt = attempt;
            let out_tx = tx.clone();
            let output = std::thread::spawn(move || {
                let mut completed = false;
                let mut failure = None;
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let message =
                        serde_json::from_str::<Message>(&line).unwrap_or(Message::Log(line));
                    match message {
                        Message::Done(_) => completed = true,
                        Message::Failed(error) => failure = Some(error),
                        other => {
                            let _ = out_tx.send(other);
                        }
                    }
                }
                (completed, failure)
            });
            let err_tx = tx.clone();
            let errors = std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let _ = err_tx.send(Message::Log(line));
                }
            });
            let status = loop {
                if token.load(Ordering::Relaxed) {
                    stop_tree(&mut child);
                    break child.wait();
                }
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) => std::thread::sleep(Duration::from_millis(80)),
                    Err(error) => {
                        stop_tree(&mut child);
                        let _ = child.wait();
                        break Err(error);
                    }
                }
            };
            let (completed, failure) = output.join().unwrap_or_default();
            let _ = errors.join();
            let final_message = if token.load(Ordering::Relaxed) {
                Message::Cancelled
            } else if let Some(error) = failure {
                Message::Failed(error)
            } else if matches!(status, Ok(s) if s.success()) && completed {
                match staging.persist_noclobber(&destination) {
                    Ok(_) => Message::Done(destination),
                    Err(error) => Message::Failed(format!(
                        "Could not save PDF. Choose another filename or output folder. {}",
                        error.error
                    )),
                }
            } else {
                Message::Failed(format!("The conversion worker stopped unexpectedly ({status:?}). See processing details."))
            };
            let _ = tx.send(final_message);
        });
        Ok(Self {
            cancel,
            thread: Some(thread),
            rx,
        })
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
        // Keep draining while joining, including when the window closes.
        if let Some(thread) = self.thread.take() {
            while !thread.is_finished() {
                let _ = self.rx.recv_timeout(Duration::from_millis(20));
            }
            let _ = thread.join();
        }
    }
}

fn stop_tree(child: &mut Child) {
    #[cfg(unix)]
    // SAFETY: this PID is the process-group leader created above. The negative
    // PID targets only that conversion's group, including ffmpeg/yt-dlp children.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .output();
    }
    let _ = child.kill();
}

pub fn run() -> Result<()> {
    let request: Request =
        serde_json::from_reader(std::io::stdin().lock()).context("Invalid conversion request")?;
    let cfg = request.config();
    let emit = |event| {
        let message = match event {
            PipelineEvent::Stage(stage) => Message::Stage(stage.to_owned()),
            PipelineEvent::Progress { done, total } => Message::Progress { done, total },
            PipelineEvent::Log(log) => Message::Log(log),
            PipelineEvent::Done(_) => return,
        };
        send(&message);
    };
    let result = if cfg.v2 && cfg.local_file.is_none() {
        ytb2pdf_core::v2::pipeline::run_v2_pipeline(&cfg, &emit)
    } else {
        ytb2pdf_core::run_pipeline(&cfg, &emit)
    };
    match result {
        Ok(path) => send(&Message::Done(path)),
        Err(error) => {
            send(&Message::Failed(format!("{error:#}")));
            return Err(error);
        }
    }
    Ok(())
}

fn send(message: &Message) {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    if serde_json::to_writer(&mut stdout, message).is_ok() {
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    fn fixture(mode: &str) -> (tempfile::TempDir, Request, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("worker.py");
        std::fs::write(
            &executable,
            format!(
                r#"#!/usr/bin/python3
import json, pathlib, subprocess, sys, time
request = json.load(sys.stdin)
mode = {mode:?}
if mode == 'cancel':
    child = subprocess.Popen(['sleep', '60'])
    print(json.dumps({{'Log': 'child-pid:' + str(child.pid)}}), flush=True)
    time.sleep(60)
elif mode == 'failure':
    pathlib.Path(request['destination']).write_bytes(b'partial')
    print(json.dumps({{'Failed': 'Test conversion failed'}}), flush=True)
    sys.exit(1)
else:
    pathlib.Path(request['destination']).write_bytes(b'%PDF-test')
    print(json.dumps({{'Done': request['destination']}}), flush=True)
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let request = Request {
            url: String::new(),
            local_file: None,
            v2: false,
            target_lang: None,
            api_key: None,
            fast_whisper: false,
            transcript: None,
            cookies: None,
            work_dir: dir.path().join("work"),
            destination: dir.path().join("output.pdf"),
        };
        (dir, request, executable)
    }

    fn terminal(job: &Job) -> Message {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let message = job
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if matches!(
                message,
                Message::Done(_) | Message::Failed(_) | Message::Cancelled
            ) {
                return message;
            }
        }
    }

    fn assert_clean(dir: &std::path::Path) {
        let cache = std::fs::read_dir(dir.join("work/resume"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(cache.join("cache-identity.json").is_file());
        assert!(!std::fs::read_dir(cache).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("attempt-")));
        assert!(!std::fs::read_dir(dir).unwrap().any(|p| p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".ytb2pdf-")));
    }

    #[test]
    fn successful_conversion_commits_output_and_cleans_scratch_space() {
        let (dir, request, executable) = fixture("success");
        let destination = request.destination.clone();
        let job = Job::start_with_executable(request, &executable).unwrap();
        assert!(matches!(terminal(&job), Message::Done(p) if p == destination));
        drop(job);
        assert_eq!(std::fs::read(destination).unwrap(), b"%PDF-test");
        assert_clean(dir.path());
    }

    #[test]
    fn failed_conversion_removes_partial_pdf() {
        let (dir, request, executable) = fixture("failure");
        let destination = request.destination.clone();
        let job = Job::start_with_executable(request, &executable).unwrap();
        assert!(matches!(terminal(&job), Message::Failed(_)));
        drop(job);
        assert!(!destination.exists());
        assert_clean(dir.path());
    }

    #[test]
    fn saving_never_overwrites_an_existing_pdf() {
        let (dir, request, executable) = fixture("success");
        let destination = request.destination.clone();
        std::fs::write(&destination, b"original").unwrap();
        let job = Job::start_with_executable(request, &executable).unwrap();
        assert!(matches!(terminal(&job), Message::Failed(_)));
        drop(job);
        assert_eq!(std::fs::read(destination).unwrap(), b"original");
        assert_clean(dir.path());
    }

    #[test]
    fn cancelling_stops_descendants_and_removes_partial_files() {
        let (dir, request, executable) = fixture("cancel");
        let destination = request.destination.clone();
        let job = Job::start_with_executable(request, &executable).unwrap();
        let line = loop {
            if let Message::Log(line) = job.rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                if line.starts_with("child-pid:") {
                    break line;
                }
            }
        };
        let pid: i32 = line.strip_prefix("child-pid:").unwrap().parse().unwrap();
        job.cancel();
        assert!(matches!(terminal(&job), Message::Cancelled));
        drop(job);
        assert!(!destination.exists());
        assert_clean(dir.path());
        #[cfg(target_os = "linux")]
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // A dead child may briefly remain a zombie until the host's init reaps it.
            assert_eq!(stat.split_whitespace().nth(2), Some("Z"));
        }
    }
}
