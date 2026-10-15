# Local Video Input Support — Implementation Plan

**Status:** Proposed (not implemented)
**Scope:** Rust core, CLI, and native desktop app
**Related design:** [Architecture V2 Plan](architecture-v2-plan.md)
**Primary goal:** Support a user-selected local video file in addition to an existing YouTube URL.

> In the current native desktop application, **"upload" means choosing a file on the user's device for local processing**. This plan does not introduce a website, cloud storage, a remote upload API, or a hosted processing service. Files should not leave the user's device unless the user explicitly enables a network-dependent feature such as translation.

## 1. Problem and opportunity

ytb2pdf currently assumes every input is a YouTube URL. Both `run_pipeline` (V1) and `run_v2_pipeline` parse that URL with `parse_youtube_url`, fetch metadata and captions through `yt-dlp`, and use YouTube-specific URLs in generated PDFs.

This prevents users from converting their own recordings, downloaded lectures, training videos, conference talks, or offline files to PDF, even though the existing FFmpeg, frame deduplication, subtitle parsing, Whisper, and PDF-building components are largely reusable.

**Desired experience:** Choose either **YouTube URL** or **Local Video File**, optionally provide subtitles, and generate a PDF using the same core features.

### Representative use cases

1. Convert a locally recorded university lecture (`.mp4`) with a separate `.srt` transcript.
2. Convert a meeting recording (`.mov`) without subtitles using local Whisper transcription.
3. Convert an offline tutorial (`.mkv` or `.webm`) into a PDF with meaningful frames.
4. Use scene-aware extraction on a long local training video without downloading or re-uploading it.
5. Keep the existing YouTube URL workflow unchanged.

**Format promise:** Support common containers such as MP4, MOV, MKV, WebM, and AVI **when the installed FFmpeg build can decode their streams**. Do not promise literally every video format, codec, encrypted file, or corrupted recording.

## 2. Scope

### In scope — initial release

- Native desktop **Choose Video File** flow with clear source selection.
- CLI support for local file paths while maintaining the existing YouTube URL syntax.
- Metadata extraction from local videos using `ffprobe`.
- Reuse V1 frame extraction, dHash deduplication, scene compaction, and PDF generation.
- Accept explicit sidecar `.srt`/`.vtt` files via `--transcript`.
- Optional same-basename subtitle discovery for local input.
- Local Whisper fallback when no subtitle file is available and audio exists.
- Optional Gemini caption translation using the existing V1 mechanism.
- Clear, local-video-appropriate PDF metadata and timestamps.
- Useful progress, validation errors, cancellation/cleanup behavior where supported.

### Next iterations

- Local-file support for V2 scene-aware processing.
- Embedded text-subtitle stream selection and extraction.
- Desktop drag-and-drop, recently used paths (opt-in), and better long-file progress.
- More precise codec/stream selection and platform testing.

### Explicitly out of scope

- Cloud uploads, browser-based file upload, remote storage, or a backend service.
- Downloading arbitrary non-YouTube websites or network URLs.
- DRM-protected videos and bypassing copy protection.
- Streaming a file while it is still being uploaded.
- Background indexing, automatic AI summaries, or a general video editor.
- Guaranteeing conversion of files FFmpeg cannot decode.

## 3. Current architecture and necessary changes

The repository currently has these relevant boundaries:

| Component | Current behavior | Required change |
| --- | --- | --- |
| `crates/ytb2pdf-core/src/pipeline.rs` | `PipelineConfig.url: String`; V1 assumes `YouTube` | Introduce source-aware orchestration |
| `crates/ytb2pdf-core/src/youtube.rs` | URL parsing, `yt-dlp` metadata, subtitles, audio, video | Keep as the YouTube-only adapter |
| `crates/ytb2pdf-core/src/subtitles.rs` | Existing SRT/VTT/JSON3 parser | Reuse for file-based transcripts |
| `crates/ytb2pdf-core/src/whisper.rs` | Accepts an audio path and preprocesses with FFmpeg | Feed it audio from the local video, without `yt-dlp` |
| `crates/ytb2pdf-core/src/frames.rs` | Accepts video paths and timestamps | Reuse on the original local file |
| `crates/ytb2pdf-core/src/pdf.rs` | Requires `PdfMeta.source_url` and `PdfBlock.deep_link` strings | Make video links optional; keep timestamp labels for local files |
| `crates/ytb2pdf-core/src/v2/*` | Downloads 240p YouTube proxy and high-res sections | Add an FFmpeg local-proxy path; seek into local source rather than download sections |
| `crates/ytb2pdf-cli/src/main.rs` | First argument is assumed to be a URL | Classify URL vs path and validate |
| `crates/ytb2pdf-app/src/main.rs` | Text field for URL only | Add source selector, file picker, path display, source-aware UI |

The goal is **one PDF engine with multiple input adapters**, not a second, copy-pasted local-video pipeline.

## 4. Proposed design

### 4.1. Explicit input representation

Introduce a typed input at the core boundary:

```rust
pub enum VideoInput {
    YouTubeUrl(String),
    LocalFile(PathBuf),
}
```

Add a source-aware field to `PipelineConfig`, or introduce a new configuration type with a compatibility constructor. Migrate CLI and app call sites deliberately; do **not** keep a confusing `url` field for paths indefinitely.

Suggested helpers:

- `VideoInput::parse_cli(raw: &str) -> Result<Self>` — classify supported YouTube URLs versus local filesystem paths.
- `VideoInput::validate(&self) -> Result<()>` — check path existence, file type, and YouTube URL shape.
- `local_video::probe_metadata(path: &Path) -> Result<VideoMeta>` — invoke `ffprobe` with JSON output and validate duration/video stream.
- `local_video::extract_audio(...)` — invoke `ffmpeg`, or reuse a source-independent Whisper helper.

**Backward compatibility:** Existing commands using a positional YouTube URL must continue working without any new flags.

### 4.2. Separate source acquisition from document generation

Recommended internal model (illustrative, not committed API):

```text
                 +--------------------+
YouTube URL ---->| YouTube adapter    |---+
                 +--------------------+   |
                                          v
                                     Source metadata
                                     Captions / audio
                                     Media path / link
                                          |
                 +--------------------+   |
Local file ----->| Local-file adapter |---+
                 +--------------------+   |
                                          v
                                  Shared PDF pipeline
                                 /                   \
                             V1 frames          V2 scenes
                                 \                   /
                                  +-----------------+
                                          |
                                          v
                                    Generated PDF
```

For local input, the original video path is already the high-resolution media asset. **Do not copy or download the full video just to start processing.** Use a dedicated scratch directory for derived audio, proxy, frames, and PDF intermediates.

Use source-neutral metadata such as title (filename stem or embedded title), duration, and an optional creator/uploader. Preserve source type, but keep sensitive absolute filesystem paths out of the visible PDF by default.

### 4.3. Captions and audio

Use a predictable precedence order for local videos:

1. Explicit `--transcript <file.srt|file.vtt>` or selected transcript in the GUI.
2. An unambiguous same-basename sidecar file, e.g. `lecture.mp4` and `lecture.srt`.
3. (Follow-up phase) Supported embedded *text* subtitle tracks, selected explicitly when there are multiple.
4. Local Whisper transcription if the file contains an audio stream.
5. Clear error if neither usable captions nor an audio stream is available.

Do not silently combine or switch between different-language subtitle tracks. For Whisper, prefer reusing the existing `whisper::transcribe_audio` preprocessing path; it already converts audio input to 16 kHz mono WAV. If the installed FFmpeg can read the local video directly, passing the video path to that conversion avoids a redundant intermediate extraction. Confirm this behavior with an integration test.

A local file with captions should not require internet access. Whisper's first model download and optional Gemini translation are exceptions that must be visible in the UI.

### 4.4. PDF metadata and links

The current PDF layout assumes every page contains a YouTube reference. Make the reference optional:

```rust
pub struct PdfMeta {
    // Existing title, duration, etc.
    pub source_url: Option<String>,
}

pub struct PdfBlock {
    // Existing frame, caption, and timestamp fields.
    pub deep_link: Option<String>,
}
```

Adapt the footer, page decorator, link annotations, and any PDF-merging/chunking implementation accordingly:

- **YouTube source:** Preserve timestamped YouTube links exactly as before.
- **Local source:** Show `00:12:34` and a neutral label such as `VIDEO TIMESTAMP`; omit hyperlinks.
- Do not fabricate a `file://` link or place the user's absolute local path in the PDF.
- A missing uploader should produce a neutral label such as `LOCAL VIDEO`, not `unknown channel`.

Cover both layouts with snapshot/render tests to prevent footer overlap or broken annotations.

## 5. CLI and GUI experience

### 5.1. CLI

Prefer retaining the first positional input for both URL and file:

```bash
# Existing YouTube command remains valid
ytb2pdf "https://youtu.be/W25_BbZCZD0" --out lecture.pdf

# New local-file input
ytb2pdf "/Users/me/Videos/lecture.mp4" --out lecture.pdf

# Explicit subtitles; avoids speech recognition
ytb2pdf "./conference-recording.mov" --transcript "./conference-recording.srt" --out conference.pdf

# Whisper fallback when no transcript is found
ytb2pdf "./meeting.mkv" --whisper-model medium --out meeting.pdf

# Phase 3: local V2 mode
ytb2pdf "./long-course.mp4" --v2 --out long-course-v2.pdf
```

An optional explicit `--file <path>` alias can improve scripting but should not be mandatory.

Define error messages for missing files, directories instead of files, unknown URL schemes, unsupported codecs, missing audio/captions, absent `ffprobe`, and unreadable media. YouTube-only options (e.g., cookies) should be rejected or clearly documented as ignored for local input; silently passing them to FFmpeg is incorrect.

### 5.2. Desktop application

Add a source selector:

```text
SOURCE
  (o) YouTube URL     ( ) Local Video File

  [ URL field                         ]   (YouTube selected)

  [ Choose Video File... ]  lecture.mp4    (Local selected)
  [ Choose Transcript... ]  Optional

OPTIONS
  Translation   [on/off]    Target language [...]
  Whisper model [Fast / Balanced]
  Processing    [V1 / V2 (when supported)]

[ Generate PDF ]

Progress: Preparing local video -> Captions -> Frames -> PDF
```

Implementation notes:

- Reuse the existing `rfd::FileDialog` pattern already used for transcript/cookie selection.
- Show filename, duration after probing, and selected source type before starting.
- Disable the YouTube-cookie controls for local mode.
- Preserve the YouTube input when toggling modes.
- Use the selected filename to propose a safe output name; avoid collisions.
- Keep processing off the UI thread, following the current `mpsc` worker design.
- Add drag-and-drop only after the basic picker is stable.

## 6. V1 versus V2 handling for local files

### First delivery: local V1

Reuse the original media file with `frames::capture_frames`; the rest of the frame-deduplication and PDF-building stages should require minimal changes. This is the fastest route to reliable local-video support.

### Follow-up: local V2

The V2 remote architecture uses a 240p **download** and selected high-resolution **section downloads**. These network optimizations do not apply to a local video already on disk.

Instead:

1. Use FFmpeg to generate a low-resolution analysis proxy (or measure whether direct decoding is faster for smaller files).
2. Reuse `v2::scene::detect_scenes`, timestamp grouping, and `v2::mapping`.
3. Extract selected high-resolution frames **directly from the original file** by FFmpeg seeking. Avoid creating per-scene high-resolution segment files unless benchmarks prove that caching segments is faster.
4. Reuse shared PDF block assembly and PDF generation.
5. Expose `--v2` for local files only after correctness and performance tests pass.

**Important:** V2's local optimization target is **CPU time, disk I/O, and temporary disk usage**, not network bandwidth. Measure local V1 versus local V2 separately from the YouTube benchmarks.

## 7. Implementation phases and deliverables

### Phase 0 — Baseline and API design

- [ ] Capture regression fixtures and expected PDF output from existing YouTube V1/V2 commands.
- [ ] Finalize `VideoInput` and configuration migration strategy.
- [ ] Document supported container/codec combinations and the subtitle precedence policy.
- [ ] Identify all YouTube-specific assumptions in PDF footer and metadata code.

**Exit criteria:** API design agreed; existing behavior protected by tests.

### Phase 1 — Local file support in core and CLI (MVP)

- [ ] Implement `local_video.rs` for `ffprobe` metadata and source validation.
- [ ] Accept a local path in CLI while preserving positional YouTube URLs.
- [ ] Bypass `yt-dlp` entirely for local input.
- [ ] Reuse V1 frame extraction directly on local media.
- [ ] Support explicit/sidecar subtitles and local Whisper fallback.
- [ ] Support optional existing Gemini translation.
- [ ] Make PDF references source-aware; render local timestamps without hyperlinks.
- [ ] Generate the PDF without copying the source video into the work directory.
- [ ] Add CLI usage and source-specific errors.

**Exit criteria:** A local MP4 with SRT and a local MP4 without captions (with Whisper) each produce a usable PDF. Existing YouTube V1 remains unchanged.

### Phase 2 — Native desktop local-file picker

- [ ] Add YouTube/Local source selector and video file dialog.
- [ ] Show selected file and validation results.
- [ ] Hide/disable inapplicable cookie settings.
- [ ] Pass the source through the shared pipeline worker; retain progress and errors.
- [ ] Add basic UI tests/manual acceptance checklist.

**Exit criteria:** A user can pick a local video and create a PDF without entering any URL.

### Phase 3 — Local scene-aware V2

- [ ] Add local proxy generation or direct low-res decode option.
- [ ] Reuse V2 scene detection and caption mapping.
- [ ] Extract chosen full-resolution frames directly from local media.
- [ ] Avoid unnecessary section copies and temporary files.
- [ ] Evaluate frame quality, PDF page-count parity, wall time, RAM, and scratch usage.

**Exit criteria:** `--v2` works on local video and produces valid scene-aligned output; benchmarks are documented.

### Phase 4 — Subtitle interoperability and polish

- [ ] Read embedded text-subtitle streams via FFmpeg, with explicit track selection.
- [ ] Handle subtitle languages and ambiguous multiple-track cases.
- [ ] Add drag-and-drop to desktop, clear missing-audio/caption guidance, and optional cancel controls.
- [ ] Improve video rotation, variable-frame-rate, and short/zero-duration behavior.

**Exit criteria:** Documented common media and subtitle combinations work predictably.

### Phase 5 — Quality, documentation, and release

- [ ] Add unit tests for input classification, probe JSON, filename generation, and subtitle precedence.
- [ ] Add local-video integration tests with deterministic, generated media fixtures.
- [ ] Add YouTube regression tests for V1 and V2.
- [ ] Test on macOS ARM64 and representative Windows/Linux setups where supported.
- [ ] Run `cargo fmt --all -- --check`, `cargo check --workspace`, and `cargo test --workspace`.
- [ ] Update README, CLI help, desktop onboarding, and GitHub release notes.
- [ ] Confirm GitHub Actions bundles the required FFmpeg/`ffprobe` binaries.

**Exit criteria:** Tested, documented builds and a released package.

## 8. Acceptance criteria

| Area | Acceptance criterion |
| --- | --- |
| Compatibility | Existing YouTube URL commands and PDF links still work |
| Local operation | Local file + sidecar transcript requires no YouTube request or `yt-dlp` |
| No captions | Whisper can transcribe a local video containing supported audio |
| PDF | Generated PDF includes frames, captions, and meaningful timestamps |
| Local metadata | PDF contains no invented YouTube URLs or private filesystem paths |
| UI | Selecting a local video does not require a URL |
| Common formats | Test MP4, MOV, MKV, WebM with codecs supported by the bundled FFmpeg |
| Input failures | Invalid media, missing file, no decodable video stream, and no audio/subtitles yield actionable errors |
| Long videos | A multi-hour local video can complete without unbounded temporary storage or uncontrolled memory use |
| Privacy | No automatic file upload or remote processing; external translation/model download is explicit |
| V2 | Local V2 performance and output differences are measured, not presumed |

## 9. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| FFmpeg supports containers but not every codec | Probe streams early and surface decoder errors |
| Local videos have no subtitles | Sidecar discovery, embedded text-subtitle support later, then Whisper |
| Whisper requires an initial model download | Explain the first-run network requirement and cache the model |
| Long WAV conversion consumes disk space | Clean temp files, stream/chunk transcription in a future optimization, monitor disk usage |
| PDF footer assumes YouTube links | Represent links as optional and test local/YouTube rendering |
| V2 high-resolution sections are download-specific | Seek directly in local media instead of using `yt-dlp --download-sections` |
| Very long files trigger high RAM usage | Reuse/test large-PDF chunking and impose bounded intermediate buffers |
| Untrusted or malformed media | Pass paths as command arguments (not shell text), probe safely, enforce limits and fail clearly |
| Privacy and path disclosure | Keep processing local by default; avoid embedding absolute paths |

## 10. Suggested milestones and priority

| Milestone | Priority | Minimum demo |
| --- | --- | --- |
| M1: Local CLI + V1 | P0 | `ytb2pdf ./lecture.mp4 --transcript ./lecture.srt` |
| M2: Local GUI file picker | P0 | Pick MP4 -> Generate PDF |
| M3: Local Whisper fallback | P0 | Pick video without captions -> PDF after transcription |
| M4: Local V2 | P1 | `ytb2pdf ./course.mp4 --v2` |
| M5: Embedded subtitles + drag/drop | P2 | Choose subtitle track from local MKV |

**Recommendation:** Deliver **M1–M3 first**. They solve the user's upload/local-file use case with the least architectural risk. Do not block the MVP on V2 optimization or embedded subtitle stream parsing.

## 11. Decisions to record during implementation

1. Should the long-term public interface become `VideoInput` immediately, or use a transitional adapter for existing `PipelineConfig.url` users?
2. Should local media display an optional user-editable document title in the desktop app?
3. When both an external sidecar and embedded subtitles exist, is the sidecar always preferred?
4. Should V2 be exposed in the GUI or remain CLI-only until larger-video benchmarks stabilize?
5. Should the product name/description eventually move from **YouTube-to-PDF** to **video-to-PDF**, while retaining `ytb2pdf` for compatibility?

---

**Definition of done:** Users can convert their own locally stored video files into timestamped PDF documents through the CLI and desktop app, while the existing YouTube workflow remains fully functional. This document is a **proposal**, not a claim that local input is currently implemented.
