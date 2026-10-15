# ytb2pdf

**Turn hours of YouTube videos into structured, readable PDF documents.**

ytb2pdf is an open-source Rust application that transforms long-form YouTube videos—lectures, technical tutorials, presentations, and educational content—into PDF documents that combine video frames with timestamped transcripts.

Instead of repeatedly scrubbing through a multi-hour video to find a diagram or explanation, you can review its visual content and accompanying text in a document.

> **Project status:** Active development. The current focus is reliable video-to-PDF conversion. AI-powered summarization is a planned future capability, not a current feature.

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

- Convert YouTube videos into PDF documents.
- Extract and normalize subtitles in SRT, VTT, and JSON3 formats.
- Capture video frames aligned with subtitle timestamps.
- Reduce redundant images using perceptual hashing (dHash).
- Include timestamps, original captions, and optional translations.
- Translate subtitles with Gemini when configured.
- Run through a command-line interface or a native desktop application.
- Track pipeline progress while processing a video.

### How it works

```text
YouTube URL
    |
    v
Video metadata and subtitles (yt-dlp)
    |
    v
Subtitle parsing and normalization
    |
    +----> Optional translation (Gemini)
    |
    v
Video download and frame extraction (FFmpeg)
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

The current pipeline captures frames around subtitle timestamps and filters duplicates. More advanced scene-aware frame selection is planned.

## Getting started

### Prerequisites

- Rust stable (tested with Rust 1.99)
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) available on your `PATH`
- [FFmpeg](https://ffmpeg.org/) available on your `PATH`
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

### Run the desktop app

```bash
cargo run -p ytb2pdf-app
```

The app currently writes generated PDFs to `~/Desktop/ytb2pdf-<video-id>.pdf` and uses `~/.ytb2pdf/` for temporary files.

### YouTube authentication issues

If YouTube returns HTTP 403 or asks you to confirm that you are not a bot, yt-dlp may need access to cookies from a browser where you are already signed in.

For example:

```bash
cargo run -p ytb2pdf-cli -- "https://youtu.be/W25_BbZCZD0" --cookies-from-browser chrome
```

Only use accounts and videos you are authorized to access. Keep browser cookies, credentials, and API keys private.

## Architecture

ytb2pdf uses a Rust workspace with a reusable processing engine separated from its interfaces.

```text
crates/
├── ytb2pdf-core/       # Processing engine independent of the UI
│   ├── youtube.rs      # URL parsing, metadata, subtitles, downloads
│   ├── subtitles.rs    # SRT/VTT/JSON3 parsing and normalization
│   ├── frames.rs       # FFmpeg frame extraction and dHash filtering
│   ├── translate.rs    # Translator abstraction and Gemini implementation
│   ├── pdf.rs          # PDF generation with genpdf
│   └── pipeline.rs     # Pipeline orchestration and progress callbacks
├── ytb2pdf-cli/        # Command-line interface
└── ytb2pdf-app/        # Native desktop UI built with egui
```

**Design principles**

- **Separation of concerns:** The core engine does not depend on the desktop UI.
- **Composable stages:** Subtitle processing, translation, frame extraction, and PDF generation are distinct steps.
- **Pragmatic dependencies:** yt-dlp and FFmpeg are invoked as external executables rather than linked into the Rust binary.
- **Extensibility:** New frame-selection strategies, translation providers, and document-processing stages can be added over time.

## Roadmap

The longer-term vision is to turn lengthy educational videos into **useful, searchable learning documents**, rather than simply exporting screenshots.

### Planned improvements

- [ ] Scene-change detection and smarter selection of meaningful frames
- [ ] Speech-to-text fallback for videos without subtitles
- [ ] Better bundled font support, including Vietnamese
- [ ] macOS code signing and notarization
- [ ] Improved application branding and packaging

### Future exploration: AI-assisted documents

- [ ] Summarize long videos into concise, structured notes
- [ ] Identify key concepts, chapters, and takeaways
- [ ] Produce topic-based summaries with references to video timestamps
- [ ] Preserve relevant diagrams and screenshots alongside summaries
- [ ] Offer different document formats for learning, research, and technical reference

These items describe future directions and are **not yet implemented**. Priorities may change based on feedback and contributions.

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

A Linux build does not produce a native macOS executable. The project includes a macOS GitHub Actions workflow at [`.github/workflows/macos.yml`](.github/workflows/macos.yml) for building and packaging a `.dmg`.

Code signing and notarization remain planned improvements and require appropriate Apple Developer credentials.

## Acknowledgments

The workspace architecture draws inspiration from [FilmCraft](https://github.com/storytold/filmcraft).

ytb2pdf builds on the work of the open-source communities behind [Rust](https://www.rust-lang.org/), [yt-dlp](https://github.com/yt-dlp/yt-dlp), [FFmpeg](https://ffmpeg.org/), and related libraries.

## License

See the repository's license information for applicable terms. If no license has been published, please note that public source availability alone does not grant permission to redistribute or modify the project.

---

**Feedback and contributions help shape the project.** If ytb2pdf is useful to you, consider opening an issue, contributing a fix, or starring the repository.
