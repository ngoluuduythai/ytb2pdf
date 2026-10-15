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
        std::fs::create_dir_all(&request.work_dir).context("Could not create the work folder")?;
        // Each attempt gets fresh scratch space: cached frames from different
        // videos, subtitle timestamps or interrupted writes must never be reused.
        let workspace = tempfile::Builder::new()
            .prefix("conversion-")
            .tempdir_in(&request.work_dir)
            .context("Could not create conversion scratch space")?;
        request.work_dir = workspace.path().to_path_buf();
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
            let _workspace = workspace;
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
    let cfg = PipelineConfig {
        url: request.url,
        local_file: request.local_file,
        v2: request.v2,
        target_lang: request.target_lang,
        gemini_api_key: request.api_key,
        whisper_model: if request.fast_whisper {
            WhisperModel::Fast
        } else {
            WhisperModel::Medium
        },
        transcript_file: request.transcript,
        cookies_file: request.cookies,
        work_dir: request.work_dir,
        out_pdf: request.destination,
        ..Default::default()
    };
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
        assert_eq!(std::fs::read_dir(dir.join("work")).unwrap().count(), 0);
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
        let Message::Log(line) = job.rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("Expected child PID");
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
