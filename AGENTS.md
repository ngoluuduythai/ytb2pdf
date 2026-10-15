# AGENTS.md — ytb2pdf

Instructions for AI agents using or modifying this repository.

## What this is

`ytb2pdf` turns a YouTube video into a captioned, illustrated PDF for study:
`yt-dlp` (video + captions) → `ffmpeg` (frame capture at caption timestamps) →
perceptual-hash scene dedupe → A4-landscape PDF. Optional EN→VI translation
via Gemini. Rust workspace (`ytb2pdf-core` library, `ytb2pdf-cli`, `ytb2pdf-app`
egui GUI).

## Agent workflow: generate a PDF for a user

This is the primary agent use case. The user gives you a YouTube URL and
(optionally) an SRT file; you produce the PDF.

```sh
# 1. Build (after pulling latest main)
cargo build --release -p ytb2pdf-cli

# 2. Run — use --json for machine-readable progress (one JSON object per line)
ytb2pdf-cli "<youtube-url>" \
  --transcript <captions.srt> \   # optional: skips YouTube captions AND Whisper
  --cookies <cookies.txt> \       # Netscape format; REQUIRED on datacenter IPs
  --out <output.pdf> \
  [--to vi] \                     # optional: EN->VI translation (needs GEMINI_API_KEY)
  [--quality 480] \               # optional: 480|720|1080 max height (default 720;
                                  #   use 480 for very long videos — smaller download)
  [--jobs 4] \                    # optional: parallel ffmpeg workers (default: CPU count)
  [--v2] \                       # optional: Architecture V2 — low-res scene detection,
                                  #   high-res segment download (saves bandwidth on long videos)
  [--text-only] \                 # optional: captions only, no images (tiny PDF for mobile)
  --json
```

JSON event types: `{"type":"stage","stage":"..."}`, `{"type":"progress","done":n,"total":m}`,
`{"type":"log","message":"..."}`, `{"type":"done","pdf":"/path/to.pdf"}`.
Exit code is non-zero on failure; the stderr message explains what went wrong.

## Architecture V2 (`--v2`)

For long videos (>1h), V2 saves download bandwidth:
1. Downloads 240p proxy → detects scenes with ffmpeg
2. Plans segments around scene changes
3. Downloads only those segments at high-res via `yt-dlp --download-sections`
4. Extracts frames, maps captions, builds PDF (same layout as V1)

Tested on 30-min video: 25% bandwidth savings (101MB → 76MB).
Best for long videos with sparse scenes (e.g. 12h lecture).

## Hard requirements & gotchas

- **Dependencies on PATH**: `yt-dlp`, `ffmpeg`, and `deno` (Deno solves
  YouTube's JS `n` challenge; the pipeline passes `--remote-components
  ejs:github` automatically — do NOT remove that flag or downloads fail with
  "n challenge solving failed" / "The page needs to be reloaded" even with
  valid cookies).
- **YouTube bot-checks**: datacenter/cloud IPs are routinely 429'd or
  challenged. A fresh `cookies.txt` (exported while logged into youtube.com)
  is required in that case. Do NOT hammer on 429 — back off and retry later.
- **Long videos**: frame capture is parallel (default: CPU count, `--jobs N`
  to override) and resumable — already-captured frames are skipped, so a
  restarted run continues where it left off. Video download also resumes
  partial `.part` files. Keep logs and work dirs under `~/workspace`
  (persistent), NOT `/tmp` (wiped on VM replacement).
- **Copyright**: only process videos the user is authorized to use.
- **No browser-cookie access**: never use `--cookies-from-browser` unless the
  user explicitly asks; prefer a scoped `cookies.txt` file.
- **Translation**: `--to <lang>` needs `GEMINI_API_KEY`; hundreds of segments
  mean hundreds of API calls — warn about time/rate limits on long videos.

## Pipeline behavior (so you can explain results)

- Scene compacting: consecutive segments whose frames are >90% similar
  (dHash Hamming ≤ 6) merge into ONE page — one image, captions joined into a
  clean paragraph with NO timestamps. The footer YouTube link carries the time
  reference (`?t=...s`).
- Provided `--transcript` (.srt/.vtt) skips both YouTube caption download and
  the Whisper fallback entirely.
- Whisper (bundled `whisper-cli`) runs ONLY when no captions exist at all.

## Modifying the code

- `cargo test -p ytb2pdf-core` must stay green (unit + offline pipeline tests).
- The PDF layout is A4 landscape with clickable per-page YouTube links added
  via `lopdf` post-processing (genpdf/printpdf 0.3 has no native link support).
- Do NOT touch `.github/workflows/*` via API — the GitHub App cannot write
  workflow files; ask the user to edit them manually.
- Commit messages: short imperative, e.g. `pdf: ...`, `pipeline: ...`.
