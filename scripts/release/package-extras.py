#!/usr/bin/env python3
"""Add archives and checksums without changing the existing DMG builder."""
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
    shutil.copy2(f'{prefix}.dmg', assets / f'{prefix}.dmg')
    subprocess.run([
        'ditto', '-c', '-k', '--sequesterRsrc', '--keepParent',
        str(app), str(assets / f'{prefix}.app.zip'),
    ], check=True)

    cli = Path('dist') / f'{prefix}-cli'
    if cli.exists():
        shutil.rmtree(cli)
    shutil.copytree(app / 'Contents/Resources', cli / 'Contents/Resources', symlinks=True)
    frameworks = app / 'Contents/Frameworks'
    if frameworks.exists():
        shutil.copytree(frameworks, cli / 'Contents/Frameworks', symlinks=True)
    (cli / 'Contents/MacOS').mkdir(parents=True)
    shutil.copy2(app / 'Contents/MacOS/ytb2pdf-cli', cli / 'Contents/MacOS/ytb2pdf-cli')
    launcher = cli / 'ytb2pdf'
    launcher.write_text('''#!/bin/sh
set -eu
release_dir=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
PATH="$release_dir/Contents/Resources/bin:$PATH"
export PATH
exec "$release_dir/Contents/MacOS/ytb2pdf-cli" "$@"
''')
    launcher.chmod(0o755)
    installation = f'''ytb2pdf {info['version']} — Apple Silicon, macOS 14 or newer
Source commit: {info['commit']}

DMG: drag ytb2pdf-app.app into Applications.
App ZIP: extract, then move ytb2pdf-app.app into Applications.
CLI archive: extract and keep the entire folder together. Run:
  ./ytb2pdf --version
  ./ytb2pdf --help

This build uses ad-hoc signing and is not Apple-notarized.
If macOS blocks the app, review System Settings > Privacy & Security.
Included tools: yt-dlp, FFmpeg, ffprobe, Poppler (pdfunite/pdfinfo), Deno
and whisper-cli, with their non-system dynamic libraries bundled.
The release workflow verifies relocated tools and offline PDF export.
Whisper models download on first use. Translation requires a Gemini API key.

Download all release assets into one folder, then verify:
  shasum -a 256 -c SHA256SUMS.txt
'''
    (cli / 'INSTALL.txt').write_text(installation)
    shutil.copy2('README.md', cli / 'README.md')
    shutil.copy2('LICENSE-MIT', cli / 'LICENSE-MIT')
    subprocess.run([
        'tar', '-czf', str(assets / f'{prefix}-cli.tar.gz'),
        '-C', 'dist', cli.name,
    ], check=True)
    shutil.copy2('release-info.json', assets / f'{prefix}-build-info.json')
    (assets / 'INSTALL.txt').write_text(installation)
    # Hash only this release's explicit inventory; exclude stale files and
    # SHA256SUMS.txt itself if packaging is repeated.
    names = [f'{prefix}{suffix}' for suffix in (
        '.dmg', '.app.zip', '-cli.tar.gz', '-build-info.json',
    )] + ['INSTALL.txt']
    checksums = []
    for name in sorted(names):
        with (assets / name).open('rb') as source:
            digest = hashlib.file_digest(source, 'sha256').hexdigest()
        checksums.append(f'{digest}  {name}\n')
    (assets / 'SHA256SUMS.txt').write_text(''.join(checksums))


if __name__ == '__main__':
    main()
