#!/usr/bin/env python3
"""Package a signed macOS app, CLI runtime, provenance and checksums."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def main():
    info = json.loads(Path('release-info.json').read_text())
    prefix = f"ytb2pdf-v{info['version']}-macos-arm64"
    app = Path('dist/ytb2pdf-app.app')
    assets = Path('dist/assets')
    assets.mkdir(parents=True, exist_ok=True)
    subprocess.run(['ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', str(app), str(assets / f'{prefix}.app.zip')], check=True)

    image = Path('dist/dmg-root')
    image.mkdir()
    shutil.copytree(app, image / app.name, symlinks=True)
    (image / 'Applications').symlink_to('/Applications')
    (image / 'INSTALL.txt').write_text(f'''ytb2pdf {info['version']} — Apple Silicon, macOS {info['minimum_macos']} or newer

Drag ytb2pdf-app.app into Applications, then open it.
The app includes yt-dlp, FFmpeg, ffprobe, Deno and whisper-cli.
Whisper models are downloaded on first use, when captions are unavailable.
Translation is optional and requires a Gemini API key.
This build is ad-hoc signed and not notarized by Apple.
If macOS blocks the app, review it under System Settings > Privacy & Security.

CLI: use the separate CLI archive and run ./ytb2pdf --help.
Verify downloads using: shasum -a 256 -c SHA256SUMS.txt
''')
    subprocess.run(['hdiutil', 'create', '-fs', 'HFS+', '-volname', f"ytb2pdf {info['version']}", '-format', 'UDZO', '-srcfolder', str(image), '-ov', str(assets / f'{prefix}.dmg')], check=True)
    subprocess.run(['hdiutil', 'verify', str(assets / f'{prefix}.dmg')], check=True)

    cli = Path('dist') / f'{prefix}-cli'
    for directory in ['MacOS', 'Resources', 'Frameworks']:
        shutil.copytree(app / 'Contents' / directory, cli / 'Contents' / directory, symlinks=True)
    (cli / 'Contents/MacOS/ytb2pdf-app').unlink()
    launcher = cli / 'ytb2pdf'
    launcher.write_text('''#!/bin/sh
set -eu
release_dir=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
PATH="$release_dir/Contents/Resources/bin:$PATH"
export PATH
exec "$release_dir/Contents/MacOS/ytb2pdf-cli" "$@"
''')
    launcher.chmod(0o755)
    (cli / 'INSTALL.txt').write_text(f'''ytb2pdf CLI {info['version']} — Apple Silicon, macOS {info['minimum_macos']} or newer

Keep this entire extracted folder together; the launcher uses Contents/ for its runtime.
Run from this directory:
  ./ytb2pdf --version
  ./ytb2pdf --help
  ./ytb2pdf "./lecture.mp4" --transcript "./lecture.srt" --out lecture.pdf

Bundled tools: yt-dlp, FFmpeg, ffprobe, Deno and Whisper CLI.
Whisper models download on first use if captions are unavailable.
Third-party notices and license files are under Contents/Resources/.
This build is ad-hoc signed, not Apple-notarized.
''')
    subprocess.run(['tar', '-czf', str(assets / f'{prefix}-cli.tar.gz'), '-C', 'dist', cli.name], check=True)
    shutil.copy('release-info.json', assets / f'{prefix}-build-info.json')
    sums = []
    for path in sorted(assets.iterdir()):
        if path.is_file():
            with path.open('rb') as source:
                digest = hashlib.file_digest(source, 'sha256').hexdigest()
            sums.append(f"{digest}  {path.name}\n")
    (assets / 'SHA256SUMS.txt').write_text(''.join(sums))

    notes = f'''## ytb2pdf {info['version']}

Convert YouTube videos and local recordings into illustrated PDFs with captions.

### Downloads

| Asset | Use |
| --- | --- |
| `{prefix}.dmg` | Recommended: drag the app into Applications |
| `{prefix}.app.zip` | The same desktop app as a ZIP archive |
| `{prefix}-cli.tar.gz` | CLI with bundled media tools; extract and run `./ytb2pdf --help` |
| `{prefix}-build-info.json` | Exact source commit and bundled runtime versions |
| `SHA256SUMS.txt` | SHA-256 checksums for all download assets |

### Requirements and installation

- Apple Silicon (arm64), macOS {info['minimum_macos']} or newer.
- FFmpeg, ffprobe, yt-dlp, Deno and Whisper CLI are included; no Homebrew setup is required.
- Whisper models download on first use when captions are missing. Gemini translation is optional and needs your API key.
- Ad-hoc signed, **not Apple-notarized**. macOS may require approval in System Settings → Privacy & Security.
- Verify downloads: `shasum -a 256 -c SHA256SUMS.txt` in the folder containing all assets.

Source commit: `{info['commit']}`. [Build details]({info['run_url']}).

### Changes

'''
    Path('dist/RELEASE_NOTES.md').write_text(notes)
    print('\n'.join(str(path) for path in sorted(assets.iterdir())))


if __name__ == '__main__':
    main()
