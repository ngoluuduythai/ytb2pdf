# ytb2pdf

**Turn long videos into structured, readable PDF documents.**

ytb2pdf is an open-source Rust application that converts YouTube videos **and local video files**—lectures, technical tutorials, presentations, recordings, and educational content—into PDF documents combining meaningful video frames with timestamped transcripts.

Instead of repeatedly scrubbing through a multi-hour video to find a diagram or explanation, you can review its visual content and accompanying text in a document.

> **Project status (October 2026):** Active development. Caption-driven **V1**, experimental scene-aware **V2**, and **local video input** are implemented in the CLI and desktop app. Local input passed Rust workspace compilation and offline PDF integration tests on macOS CI. Long-video benchmarks and wider platform validation remain open. AI-powered summarization is planned, not implemented.

## Why ytb2pdf?

Video is excellent for demonstrations, but not always ideal for reference and review. A two-hour lecture may contain useful diagrams, explanations, and examples that are difficult to revisit quickly.

ytb2pdf helps make that content easier to work with:

- **Study and review:** Convert long lectures into materials you can read at your own pace.
- **Technical documentation:** Preserve important frames and explanations from programming tutorials and engineering walkthroughs.
- **Research and note-taking:** Keep timestamped visual references from educational videos in one document.
- **Language learning:** Read original subtitles alongside optional translations.
- **Offline reference:** Save a PDF for later reading without replaying the entire video.

The goal is **not** to replace the original video. It is to make valuable video content easier to revisit, organize, and learn from.

## Features

- Convert **YouTube URLs or local video files** into PDF documents (MP4, MOV, MKV, WebM and other formats supported by your FFmpeg build).
- Select local videos in the desktop app using a file picker or drag and drop.
- Extract and normalize subtitles in SRT, VTT, and JSON3 formats; for local videos, use explicit transcripts, same-basename sidecars, or the first compatible embedded text subtitle track.
- Capture video frames aligned with subtitle timestamps in the default V1 pipeline.
- Use **experimental V2 (`--v2`)** to detect scenes on low-resolution proxies. For YouTube, download selected higher-resolution segments; for local files, extract full-resolution frames directly from the source.
- Fall back to local Whisper speech recognition when captions are unavailable (requires `whisper-cli` and a downloaded model; YouTube V1 and local files).
- Reduce redundant images using perceptual hashing (dHash).
- Include timestamps, original captions, and optional translations.
- Translate subtitles with Gemini when configured.
- Run through a command-line interface or a native desktop application.
- Track pipeline progress while processing a video.

### How it works

```text
YouTube URL or local video file
    |
    v
Source metadata + captions
(yt-dlp for YouTube / FFmpeg and files for local video)
    |
    v
Subtitle parsing and normalization
    |
    +----> Optional translation (Gemini)
    |
    v
Video acquisition (YouTube only) + frame extraction (FFmpeg)
    |
    v
Duplicate-frame filtering (dHash)
    |
    v
PDF generation
    |
    v
Timestamped frames + captions + optional translations
```

**V1 (default)** captures frames around subtitle timestamps and filters duplicates. **V2 (opt-in)** detects scene changes using a low-resolution proxy, extracts selected frames, and maps captions to the resulting scenes. V2 is implemented but remains under evaluation, and output can differ from V1. See the [Architecture V2 Plan](docs/architecture-v2-plan.md).

### Architecture V2: bandwidth-aware frame extraction

```text
YouTube URL + captions
         |
         v
Download low-resolution proxy (240p target)
         |
         v
Detect visual changes (FFmpeg scene score + periodic sampling)
         |
         v
Group scene timestamps into download segments
         |
         v
Download selected higher-resolution segments in parallel
         |
         v
Extract scene frames and map captions to frames
         |
         v
Build illustrated, timestamped PDF
```

V2 remains optional. With YouTube input, it aims to reduce network downloads by selecting video sections. With **local input**, it creates a small analysis proxy and seeks directly in the original file instead of downloading segments. The YouTube V2 pipeline does not currently translate subtitles; use V1 when you need translation of YouTube captions.

## Getting started

### Prerequisites

- Rust stable (tested with Rust 1.99)
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) available on your `PATH` (**YouTube URLs only**)
- [FFmpeg](https://ffmpeg.org/) available on your `PATH` (required for frame extraction, metadata probing, and local video processing)
- [Poppler](https://poppler.freedesktop.org/) `pdfunite` on your `PATH` for documents with more than 1,500 scenes (`brew install poppler` on macOS; `apt install poppler-utils` on Debian/Ubuntu). Included in the packaged macOS app and CLI runtime.
- Optional: [whisper.cpp](https://github.com/ggml-org/whisper.cpp) `whisper-cli` for transcription when captions are missing (bundled in the macOS packaging workflow; model downloaded on first use)
- Optional: a Gemini API key for subtitle translation

Make sure the required commands are available:

```bash
rustc --version
cargo --version
yt-dlp --version
ffmpeg -version
```

### Clone the repository

```bash
git clone https://github.com/ngoluuduythai/ytb2pdf.git
cd ytb2pdf
```

### Run the CLI

```bash
cargo run -p ytb2pdf-cli -- "https://youtu.be/W25_BbZCZD0"
```

To request Vietnamese subtitle translation:

```bash
export GEMINI_API_KEY="your-api-key"
cargo run -p ytb2pdf-cli -- "https://youtu.be/W25_BbZCZD0" --to vi
```

### Try the V2 pipeline (CLI)

```bash
cargo run -p ytb2pdf-cli -- "https://youtu.be/W25_BbZCZD0" --v2 --out lecture-v2.pdf
```

Optional controls include `--quality 720`, `--jobs 4`, `--transcript path/to/file.srt`, and `--json` for machine-readable progress events. **YouTube V2 currently requires captions** (either downloaded or supplied with `--transcript`); unlike V1, it does not yet fall back to Whisper.

### Convert a local video file

Local video files are processed **on your device**. You do not need YouTube or `yt-dlp` for local conversion. Supported containers and codecs depend on your FFmpeg build.

```bash
# Convert a local video with an explicit subtitle file
cargo run -p ytb2pdf-cli -- "./lecture.mp4" \
  --transcript "./lecture.srt" --out lecture.pdf

# Same-basename sidecars are discovered automatically:
# lecture.mp4 + lecture.srt (or lecture.vtt)
cargo run -p ytb2pdf-cli -- "./lecture.mp4" --out lecture.pdf

# If no usable subtitles exist, transcribe locally with Whisper
cargo run -p ytb2pdf-cli -- "./meeting.mov" \
  --whisper-model medium --out meeting.pdf

# Experimental scene-aware processing for a local recording
cargo run -p ytb2pdf-cli -- "./long-course.mkv" --v2 --out long-course.pdf
```

For local media, ytb2pdf tries an explicit transcript, a matching SRT/VTT sidecar, then the first compatible embedded text-subtitle stream. If none works and the file has audio, it falls back to Whisper. First-time Whisper model downloads require internet access; **Gemini translation sends captions to a third-party service only when enabled**.

Local PDFs display **video timestamps** without fabricated YouTube links or embedded absolute file paths. The local V2 pipeline analyzes a low-resolution proxy but extracts final images from the original file. See the [Local Video Input Plan](docs/local-video-input-plan.md).

### Run the desktop app

```bash
cargo run -p ytb2pdf-app
```

The desktop app guides you through **Source video → PDF options → Save PDF**:

- Paste a YouTube link, choose a local video, or drag and drop a video into the app. You can also import or drop an SRT/VTT subtitle file.
- Keep the original captions by default, or enable translation and choose a language. A Gemini API key is required for translation and stays in memory for the session; captions are sent to Gemini only when translation is enabled.
- Expand **Advanced** for experimental scene-aware V2 processing, the speech-recognition model, and optional YouTube cookies. YouTube V2 requires captions and does not support translation or Whisper fallback; incompatible settings are blocked before conversion.
- Choose the PDF destination. The suggested filename uses the video ID or local filename, with a numeric suffix if needed. The default folder is Desktop, then Documents or your home folder when Desktop is unavailable. Existing PDFs are never overwritten.
- Follow the current processing step and elapsed time, expand processing details, or **Cancel conversion** to stop the conversion worker and its media tools. Closing the app also stops the worker. Completed stages are retained under `~/.ytb2pdf/resume/`. **Resume conversion** reuses matching saved work after errors, cancellation, or restarting the app. Source/configuration changes select a separate cache; API keys and cookies are not stored in checkpoints. Per-attempt temporary files are cleaned up; resumable downloads and saved progress remain.
- Once complete, **Open PDF**, **Open folder**, or **Convert another video**. Saved progress remains available for another export; remove unneeded folders under `~/.ytb2pdf/resume/` manually to reclaim disk space. PDFs are written to a temporary file in the destination folder and saved to the chosen filename only after successful conversion.

The interface scrolls at small window sizes and larger display scales. Errors include suggested next steps, while technical output stays under **Processing details**.

### Resume after a failed PDF export

Completed downloads, normalized captions, translation batches, frames and PDF inputs are saved. If PDF creation fails, retry with the same video and settings to resume directly at PDF generation, without downloading, transcribing, translating or extracting frames again. Completed PDF parts are checksum-verified and reused if a later part or the final merge fails. Changing the output filename does not invalidate saved work.

The desktop app retains these checkpoints across restarts. The CLI uses the same mechanism under `--work-dir` (default `ytb2pdf-work/`). To resume, keep that work directory and use the same source and processing options. Local source metadata and subtitle content identify the input; changed source/options use a separate cache. Corrupted cached frames are extracted again, and incomplete stages are retried. Source video files and imported subtitles must remain available to validate the cache identity. A work-directory lock prevents concurrent conversions from modifying the same saved progress.

Checkpoints contain captions, derived media and source metadata, but not API keys or cookies. Deleting a cache folder removes its saved progress. Intermediate files deleted by older desktop versions cannot be recovered automatically.

### YouTube authentication issues

If YouTube returns HTTP 403 or asks you to confirm that you are not a bot, yt-dlp may need access to cookies from a browser where you are already signed in.

For example:

```bash
cargo run -p ytb2pdf-cli -- "https://youtu.be/W25_BbZCZD0" --cookies-from-browser chrome
```

Only use accounts and videos you are authorized to access. Keep browser cookies, credentials, and API keys private.

## V2 implementation status and measured results

The [V2 implementation plan](docs/architecture-v2-plan.md) reports the following status as of **October 15, 2026**:

| Workstream | Status |
| --- | --- |
| Validate selective downloading with `yt-dlp --download-sections` | Complete |
| Low-resolution proxy, scene detection, and segment planning | Implemented |
| Parallel higher-resolution segment downloads | Implemented |
| Frame extraction, caption-to-scene mapping, and `--v2` CLI integration | Implemented |
| End-to-end test on a 30-minute video | Completed |
| Validation on videos longer than three hours | Pending |
| Further tuning for output parity and bandwidth savings | Ongoing |

**Reported 30-minute benchmark (not a general performance guarantee):**

| Metric | V1 | V2 |
| --- | --- | --- |
| Downloaded data | 101 MB | 76 MB (about 25% less) |
| Visual/caption-derived scenes | 110 | 94 |
| PDF output | 104 MB / 110 pages | 80 MB / 85 pages |
| Frame resolution | 720p | 720p |

V2 produced **about 23% fewer pages**, exceeding the plan's target of staying within ±10% of V1. The goal of downloading under 20% of V1's data on videos longer than one hour **has not yet been validated**. Longer-video savings remain a hypothesis, not a measured result.

The plan also documents robustness work: less frequent fallback sampling, skipping frames that cannot be extracted, and chunked PDF generation for very large documents. It reports a completed 7,628-page CS50 PDF and a much smaller text-only PDF experiment; however, **the current CLI does not expose a `--text-only` flag**, so it is not documented here as a runnable CLI option.

## Architecture

ytb2pdf uses a Rust workspace with a reusable processing engine separated from its interfaces.

```text
crates/
├── ytb2pdf-core/       # Processing engine independent of the UI
│   ├── youtube.rs      # YouTube URL parsing, metadata, subtitles, downloads
│   ├── local_video.rs  # Local source validation, FFmpeg metadata, subtitles, V1/V2
│   ├── subtitles.rs    # SRT/VTT/JSON3 parsing and normalization
│   ├── frames.rs       # FFmpeg frame extraction and dHash filtering
│   ├── translate.rs    # Translator abstraction and Gemini implementation
│   ├── pdf.rs          # PDF generation with genpdf
│   ├── pipeline.rs     # V1 pipeline orchestration and progress callbacks
│   ├── whisper.rs      # Local speech-to-text fallback
│   └── v2/             # Proxy, scene detection, segment planning, high-res extraction, mapping, pipeline
├── ytb2pdf-cli/        # Command-line interface
└── ytb2pdf-app/        # Native desktop UI built with egui
```

**Design principles**

- **Separation of concerns:** The core engine does not depend on the desktop UI.
- **Composable stages:** Subtitle processing, translation, frame extraction, and PDF generation are distinct steps.
- **Pragmatic dependencies:** yt-dlp (YouTube) and FFmpeg (all video sources) are called as external executables rather than linked into the Rust binary.
- **Local-first processing:** Local video files are processed on the user's machine, without uploading the video to a remote service.
- **Extensibility:** New frame-selection strategies, translation providers, and document-processing stages can be added over time.

## Roadmap

The longer-term vision is to turn lengthy educational videos into **useful, searchable learning documents**, rather than simply exporting screenshots.

### Planned improvements

- [x] Scene-aware V2 pipeline with low-resolution proxy and selective higher-resolution downloads (opt-in CLI)
- [x] Speech-to-text fallback in V1 using Whisper
- [ ] Validate V2 on 3+ hour videos and measure actual bandwidth/time savings
- [ ] Improve V2 scene selection and V1/V2 PDF page-count parity
- [x] Support local video input through CLI and desktop file picker / drag and drop
- [x] Add local scene-aware V2 processing and an offline PDF integration test
- [x] Expose experimental V2 processing in the desktop app
- [ ] Improve YouTube V2 translation and Whisper support
- [ ] Support explicit selection among multiple embedded subtitle tracks
- [ ] Validate local files across platforms and a broader set of codecs
- [ ] Improve bundled font coverage, including Vietnamese
- [ ] macOS code signing and notarization
- [ ] Improved application branding and packaging

### Future exploration: AI-assisted documents

- [ ] Summarize long videos into concise, structured notes
- [ ] Identify key concepts, chapters, and takeaways
- [ ] Produce topic-based summaries with references to video timestamps
- [ ] Preserve relevant diagrams and screenshots alongside summaries
- [ ] Offer different document formats for learning, research, and technical reference

Unchecked items are future improvements and are **not yet implemented**. Priorities may change based on feedback and contributions.

## Contributing

Contributions are welcome, whether you are fixing a bug, improving documentation, proposing a feature, or working on the processing pipeline.

### Ways to contribute

- Report reproducible bugs and platform-specific issues.
- Suggest improvements to frame selection, subtitle handling, translation, or PDF layout.
- Improve documentation, examples, and accessibility.
- Add tests and improve performance.
- Discuss and prototype future AI-assisted summarization features.

### Development workflow

1. Check [existing issues](https://github.com/ngoluuduythai/ytb2pdf/issues) and [pull requests](https://github.com/ngoluuduythai/ytb2pdf/pulls) to avoid duplicate work.
2. For substantial changes, open an issue first to discuss the approach.
3. Fork the repository and create a focused feature branch.
4. Make your changes, including tests or documentation where appropriate.
5. Run local checks:

   ```bash
   cargo fmt --all -- --check
   cargo check --workspace
   cargo test --workspace
   ```

6. Open a pull request with a clear description of the problem, the solution, and how you tested it.

Please keep pull requests reasonably scoped. For changes that affect generated PDFs, before/after examples are especially helpful. Do not commit API keys, browser cookies, downloaded copyrighted videos, or large generated artifacts.

### Good first contributions

Not sure where to start? Documentation improvements, small bug fixes, test coverage, and clearer error messages are all valuable. Browse the [issue tracker](https://github.com/ngoluuduythai/ytb2pdf/issues) or open a discussion through an issue to propose an idea.

## Building for macOS

The [macOS release workflow](.github/workflows/macos.yml) builds on an Apple Silicon runner for **macOS 14 or newer**. Linux builds cannot produce these native macOS packages.

### Release versions

- Push a SemVer tag such as `v0.2.0` for a stable release, or `v0.2.0-rc.1` for a prerelease.
- Or open **Actions → macOS release → Run workflow** and enter a version. A blank version produces a unique development prerelease, for example `0.1.0-dev.123456.1`.
- Disable **Publish to GitHub Releases** for a manual build that only uploads Actions artifacts. Tagged builds always publish after validation.
- Reuse a draft to retry an interrupted upload. Published releases and tags pointing at other commits are protected; use a new version for changed builds.

The version is embedded in the desktop header, window title, CLI `--version`, app `--version`, macOS bundle metadata, artifact names, and release title. The release workflow injects `YTB2PDF_VERSION` and `YTB2PDF_COMMIT` at compile time; local builds use the shared version in `[workspace.package]` and a `local` commit marker. Prerelease suffixes remain visible in the app and build-info file; macOS's numeric version fields use the numeric version and workflow build number.

### Release assets

For `v0.2.0`, a complete release contains:

| Asset | Contents |
| --- | --- |
| `ytb2pdf-v0.2.0-macos-arm64.dmg` | Desktop app, installation instructions, and Applications shortcut |
| `ytb2pdf-v0.2.0-macos-arm64.app.zip` | The desktop app bundle |
| `ytb2pdf-v0.2.0-macos-arm64-cli.tar.gz` | CLI launcher and bundled runtime; extract and run `./ytb2pdf --help` |
| `ytb2pdf-v0.2.0-macos-arm64-build-info.json` | Version, source commit, build URL, runtime versions, and signing status |
| `SHA256SUMS.txt` | SHA-256 checksums for all release assets above |

Both desktop and CLI packages include **yt-dlp, FFmpeg, ffprobe, Poppler (pdfunite/pdfinfo), Deno, Whisper CLI, and their non-system dynamic libraries**. Whisper's Metal library is embedded; models download on first use when needed. No Homebrew installation is required on the user's Mac. The packaging checks reject build-runner library paths, and the workflow verifies both extracted archives by creating offline PDFs with Homebrew removed from `PATH`.

Release notes include download guidance, requirements, signing status, and GitHub-generated changes. Uploads remain drafts until all expected assets are present. Stable releases become Latest; prereleases do not replace Latest. Before installing, place the downloaded assets and checksum file together and run `shasum -a 256 -c SHA256SUMS.txt`.

Packages are **ad-hoc signed, not Apple Developer ID signed or notarized**. Gatekeeper may require approval through System Settings → Privacy & Security. Trusted distribution still requires Apple Developer credentials and notarization.

Release-script tests can run locally with Python 3.11 or newer:

```bash
python3 -m unittest discover -s scripts/release -p 'test_*.py'
bash -n scripts/release/build-runtime.sh scripts/release/publish.sh
```

## Acknowledgments

The workspace architecture draws inspiration from [FilmCraft](https://github.com/storytold/filmcraft).

ytb2pdf builds on the work of the open-source communities behind [Rust](https://www.rust-lang.org/), [yt-dlp](https://github.com/yt-dlp/yt-dlp), [FFmpeg](https://ffmpeg.org/), and related libraries.

## License

See the repository's license information for applicable terms. If no license has been published, please note that public source availability alone does not grant permission to redistribute or modify the project.

---

**Feedback and contributions help shape the project.** If ytb2pdf is useful to you, consider opening an issue, contributing a fix, or starring the repository.
