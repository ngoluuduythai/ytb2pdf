//! Durable checkpoints. Secrets and the destination never participate in cache identity.
use crate::pdf::{self, PdfBlock, PdfMeta, PdfOptions};
use crate::{PipelineConfig, PipelineEvent};
use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

// Bump when the interpretation of cached pipeline inputs or PDF layout changes.
const CACHE_SCHEMA: u32 = 1;
const PDF_SCHEMA: u32 = 2;

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn file_digest(path: &Path) -> Result<String> {
    let mut source = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn file_identity(path: &Path, content: bool) -> Result<serde_json::Value> {
    let path = std::fs::canonicalize(path)?;
    let metadata = std::fs::metadata(&path)?;
    Ok(serde_json::json!({"path": path, "size": metadata.len(),
        "modified": metadata.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_nanos().to_string(),
        "digest": if content { Some(file_digest(&path)?) } else { None }}))
}

pub fn workspace_key(cfg: &PipelineConfig) -> Result<String> {
    let source = if let Some(path) = &cfg.local_file {
        let mut sidecars = Vec::new();
        if cfg.transcript_file.is_none() {
            for extension in ["srt", "vtt"] {
                let candidate = path.with_extension(extension);
                if candidate.is_file() {
                    sidecars.push(file_identity(&candidate, true)?);
                }
            }
        }
        serde_json::json!({"local": file_identity(path, false)?, "sidecars": sidecars})
    } else {
        let url = crate::youtube::parse_youtube_url(&cfg.url)
            .map(|video| video.id)
            .unwrap_or_else(|_| cfg.url.trim().to_owned());
        serde_json::json!({"youtube": url})
    };
    let transcript = cfg
        .transcript_file
        .as_ref()
        .map(|p| file_identity(p, true))
        .transpose()?;
    let identity = serde_json::json!({"schema": CACHE_SCHEMA, "source": source,
        "transcript": transcript, "source_lang": cfg.source_lang, "target_lang": cfg.target_lang,
        "whisper": cfg.whisper_model.file_name(), "quality": cfg.max_height,
        "dedupe": cfg.dhash_threshold, "v2": cfg.v2, "text_only": cfg.text_only});
    Ok(digest(&serde_json::to_vec(&identity)?))
}

/// Namespace all intermediate files by validated source/config identity.
pub fn prepare_workspace(cfg: &PipelineConfig) -> Result<PathBuf> {
    let key = workspace_key(cfg)?;
    let marker = cfg.work_dir.join("cache-identity.json");
    if std::fs::read_to_string(&marker).ok().as_deref() == Some(&key) {
        return Ok(std::fs::canonicalize(&cfg.work_dir)?);
    }
    let directory = cfg.work_dir.join("resume").join(&key);
    std::fs::create_dir_all(&directory)?;
    atomic_write(&directory.join("cache-identity.json"), key.as_bytes())?;
    Ok(std::fs::canonicalize(directory)?)
}

pub fn scoped_config(cfg: &PipelineConfig) -> Result<PipelineConfig> {
    let mut scoped = cfg.clone();
    scoped.work_dir = prepare_workspace(cfg)?;
    Ok(scoped)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Checkpoint has no parent folder")?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Stage<T> {
    key: String,
    value: T,
}

/// Commit only a complete stage; interrupted writes never replace its checkpoint.
pub fn stage<T: Serialize + DeserializeOwned>(
    cfg: &PipelineConfig,
    name: &str,
    emit: &(dyn Fn(PipelineEvent) + Sync),
    produce: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let key = workspace_key(cfg)?;
    let path = cfg.work_dir.join(format!("stage-{name}.json"));
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(saved) = serde_json::from_slice::<Stage<T>>(&bytes) {
            if saved.key == key {
                emit(PipelineEvent::Log(format!(
                    "Resuming completed stage: {name}"
                )));
                return Ok(saved.value);
            }
        }
    }
    let value = produce()?;
    atomic_write(&path, &serde_json::to_vec(&Stage { key, value: &value })?)?;
    Ok(value)
}

#[derive(Serialize, Deserialize)]
struct Artifact {
    path: PathBuf,
    hash: String,
}
#[derive(Serialize, Deserialize)]
struct PreparedPdf {
    schema: u32,
    key: String,
    meta: PdfMeta,
    blocks: Vec<PdfBlock>,
    artifacts: Vec<Artifact>,
    text_only: bool,
}

pub fn try_pdf(
    cfg: &PipelineConfig,
    emit: &(dyn Fn(PipelineEvent) + Sync),
) -> Result<Option<PathBuf>> {
    let path = cfg.work_dir.join("prepared-pdf.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(None);
    };
    let saved = match serde_json::from_slice::<PreparedPdf>(&bytes) {
        Ok(saved) if saved.schema == PDF_SCHEMA && saved.key == workspace_key(cfg)? => saved,
        _ => {
            emit(PipelineEvent::Log(
                "Saved PDF inputs are outdated; rebuilding from completed earlier stages.".into(),
            ));
            return Ok(None);
        }
    };
    emit(PipelineEvent::Stage("Checking saved PDF inputs"));
    if saved
        .artifacts
        .iter()
        .any(|artifact| file_digest(&artifact.path).ok().as_deref() != Some(&artifact.hash))
    {
        emit(PipelineEvent::Log(
            "A saved frame is missing or changed; repairing from the earlier stages.".into(),
        ));
        return Ok(None);
    }
    emit(PipelineEvent::Log("Resuming PDF creation: reusing captions, translations and frames; skipping download and extraction.".into()));
    render(cfg, &saved, &bytes, emit).map(Some)
}

pub fn save_and_render(
    cfg: &PipelineConfig,
    meta: &PdfMeta,
    blocks: &[PdfBlock],
    opts: &PdfOptions,
    emit: &(dyn Fn(PipelineEvent) + Sync),
) -> Result<PathBuf> {
    let mut seen = HashSet::new();
    let mut artifacts = Vec::new();
    for image in blocks.iter().filter_map(|block| block.image.as_ref()) {
        if seen.insert(image) {
            artifacts.push(Artifact {
                path: image.clone(),
                hash: file_digest(image)?,
            });
        }
    }
    let saved = PreparedPdf {
        schema: PDF_SCHEMA,
        key: workspace_key(cfg)?,
        meta: meta.clone(),
        blocks: blocks.to_vec(),
        artifacts,
        text_only: opts.text_only,
    };
    let bytes = serde_json::to_vec(&saved)?;
    atomic_write(&cfg.work_dir.join("prepared-pdf.json"), &bytes)?;
    render(cfg, &saved, &bytes, emit)
}

fn render(
    cfg: &PipelineConfig,
    saved: &PreparedPdf,
    bytes: &[u8],
    emit: &(dyn Fn(PipelineEvent) + Sync),
) -> Result<PathBuf> {
    let opts = PdfOptions {
        text_only: saved.text_only,
        cache_dir: Some(
            cfg.work_dir
                .join("pdf-parts")
                .join(digest(&serde_json::to_vec(&(
                    PDF_SCHEMA,
                    crate::VERSION,
                    crate::BUILD_COMMIT,
                    bytes,
                ))?)),
        ),
        ..Default::default()
    };
    emit(PipelineEvent::Stage("Generating PDF"));
    pdf::build_pdf(&saved.meta, &saved.blocks, &opts, &cfg.out_pdf)?;
    emit(PipelineEvent::Done(cfg.out_pdf.clone()));
    Ok(cfg.out_pdf.clone())
}

pub fn lock_workspace(cfg: &PipelineConfig) -> Result<std::fs::File> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(cfg.work_dir.join("pipeline.lock"))?;
    lock.try_lock()
        .context("This saved conversion is already running in another process")?;
    Ok(lock)
}

fn artifact_marker(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".sha256");
    PathBuf::from(name)
}

pub fn artifact_valid(path: &Path) -> bool {
    match (
        std::fs::read_to_string(artifact_marker(path)),
        file_digest(path),
    ) {
        (Ok(expected), Ok(actual)) => expected == actual,
        _ => false,
    }
}

pub fn commit_artifact(path: &Path) -> Result<()> {
    anyhow::ensure!(
        std::fs::metadata(path)?.len() > 0,
        "Completed media file is empty: {}",
        path.display()
    );
    atomic_write(&artifact_marker(path), file_digest(path)?.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn identity_excludes_credentials_and_destination_but_invalidates_changed_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("captions.srt");
        std::fs::write(&transcript, b"original captions").unwrap();
        let mut cfg = PipelineConfig {
            url: "https://youtu.be/W25_BbZCZD0".into(),
            transcript_file: Some(transcript.clone()),
            ..Default::default()
        };
        let key = workspace_key(&cfg).unwrap();
        cfg.gemini_api_key = Some("private-api-key".into());
        cfg.cookies_file = Some(dir.path().join("cookies.txt"));
        cfg.out_pdf = dir.path().join("another-output.pdf");
        cfg.jobs = 17;
        assert_eq!(workspace_key(&cfg).unwrap(), key);
        cfg.target_lang = Some("vi".into());
        assert_ne!(workspace_key(&cfg).unwrap(), key);
        cfg.target_lang = None;
        std::fs::write(&transcript, b"changed captions").unwrap();
        assert_ne!(workspace_key(&cfg).unwrap(), key);
    }

    #[test]
    fn a_saved_workspace_can_only_be_processed_by_one_worker() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = scoped_config(&PipelineConfig {
            work_dir: dir.path().to_path_buf(),
            ..Default::default()
        })
        .unwrap();
        let first = lock_workspace(&cfg).unwrap();
        assert!(lock_workspace(&cfg).is_err());
        drop(first);
        assert!(lock_workspace(&cfg).is_ok());
    }

    #[test]
    fn incomplete_stages_are_retried_and_completed_stages_are_reused() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = scoped_config(&PipelineConfig {
            work_dir: dir.path().to_path_buf(),
            ..Default::default()
        })
        .unwrap();
        let failed: Result<Vec<String>> =
            stage(&cfg, "captions", &|_| {}, || anyhow::bail!("Interrupted"));
        assert!(failed.is_err());
        assert!(!cfg.work_dir.join("stage-captions.json").exists());
        let original: Vec<String> =
            stage(&cfg, "captions", &|_| {}, || Ok(vec!["saved".into()])).unwrap();
        let reused: Vec<String> = stage(&cfg, "captions", &|_| {}, || {
            anyhow::bail!("Must not repeat the completed stage")
        })
        .unwrap();
        assert_eq!(reused, original);
        std::fs::write(cfg.work_dir.join("stage-captions.json"), b"partial JSON").unwrap();
        let repaired: Vec<String> =
            stage(&cfg, "captions", &|_| {}, || Ok(vec!["repaired".into()])).unwrap();
        assert_eq!(repaired, vec!["repaired"]);
    }

    #[test]
    fn incomplete_or_changed_downloads_are_not_reused() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("video.mp4");
        std::fs::write(&media, b"partial").unwrap();
        assert!(!artifact_valid(&media));
        commit_artifact(&media).unwrap();
        assert!(artifact_valid(&media));
        std::fs::write(&media, b"changed").unwrap();
        assert!(!artifact_valid(&media));
    }

    #[test]
    fn pdf_failure_resumes_offline_with_completed_parts_even_at_a_new_destination() {
        let root = tempfile::tempdir().unwrap();
        let blocked = root.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        // Invalid URL deliberately proves retry bypasses URL parsing/network.
        let mut cfg = PipelineConfig {
            url: "saved-source".into(),
            work_dir: root.path().join("work"),
            out_pdf: blocked.join("result.pdf"),
            ..Default::default()
        };
        let scoped = scoped_config(&cfg).unwrap();
        let meta = PdfMeta {
            title: "Resume regression".into(),
            uploader: "ytb2pdf".into(),
            duration_secs: 1501,
            source_url: None,
        };
        let blocks: Vec<_> = (0..1501)
            .map(|i| PdfBlock {
                timestamp_secs: i as f64,
                deep_link: None,
                caption: format!("Scene {i}"),
                caption_translated: None,
                image: None,
                merged: false,
            })
            .collect();
        let failed = save_and_render(
            &scoped,
            &meta,
            &blocks,
            &PdfOptions {
                text_only: true,
                ..Default::default()
            },
            &|_| {},
        );
        assert!(failed.is_err());
        assert!(scoped.work_dir.join("prepared-pdf.json").is_file());
        let parts_dir = std::fs::read_dir(scoped.work_dir.join("pdf-parts"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let parts: Vec<_> = std::fs::read_dir(parts_dir)
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
            .collect();
        assert_eq!(parts.len(), 2);
        let saved_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1234);
        for part in &parts {
            std::fs::File::options()
                .write(true)
                .open(part)
                .unwrap()
                .set_modified(saved_time)
                .unwrap();
        }
        cfg.out_pdf = root.path().join("recovered.pdf");
        let events = Mutex::new(Vec::new());
        crate::run_pipeline(&cfg, &|event| events.lock().unwrap().push(event)).unwrap();
        assert_eq!(
            lopdf::Document::load(&cfg.out_pdf)
                .unwrap()
                .get_pages()
                .len(),
            1501
        );
        for part in parts {
            assert_eq!(
                std::fs::metadata(part).unwrap().modified().unwrap(),
                saved_time,
                "A completed PDF part was rebuilt"
            );
        }
        assert!(events.lock().unwrap().iter().any(
            |e| matches!(e, PipelineEvent::Log(log) if log.contains("Resuming PDF creation"))
        ));
        assert!(!events.lock().unwrap().iter().any(|e| matches!(e, PipelineEvent::Stage(stage) if stage.contains("Downloading") || stage.contains("Extracting"))));
    }
}
