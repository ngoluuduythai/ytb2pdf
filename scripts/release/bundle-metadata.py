#!/usr/bin/env python3
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess

app = Path('dist/ytb2pdf-app.app')
resources = app / 'Contents/Resources'
info = json.loads(Path('release-info.json').read_text())
for tool, args in {
    'yt-dlp': ['--version'], 'ffmpeg': ['-version'], 'ffprobe': ['-version'],
    'deno': ['--version'],
}.items():
    result = subprocess.check_output([str(resources / 'bin' / tool), *args], text=True)
    info.setdefault('runtime', {})[tool] = result.splitlines()[0]
info['runtime']['whisper.cpp'] = 'v1.8.2 (static, embedded Metal library)'
plist = {
    'CFBundleName': 'ytb2pdf', 'CFBundleDisplayName': 'ytb2pdf',
    'CFBundleIdentifier': 'com.github.ngoluuduythai.ytb2pdf',
    'CFBundleExecutable': 'ytb2pdf-app', 'CFBundlePackageType': 'APPL',
    'CFBundleShortVersionString': info['bundle_version'],
    'CFBundleVersion': f"{os.environ['GITHUB_RUN_NUMBER']}.{os.environ['GITHUB_RUN_ATTEMPT']}",
    'Ytb2pdfReleaseVersion': info['version'],
    'CFBundleGetInfoString': f"ytb2pdf {info['version']} ({info['commit'][:12]})",
    'LSMinimumSystemVersion': info['minimum_macos'],
    'NSHighResolutionCapable': True,
}
with (app / 'Contents/Info.plist').open('wb') as output:
    plistlib.dump(plist, output)
(resources / 'build-info.json').write_text(json.dumps(info, indent=2) + '\n')
Path('release-info.json').write_text(json.dumps(info, indent=2) + '\n')
shutil.copy('README.md', resources / 'README.md')
shutil.copy('LICENSE-MIT', resources / 'LICENSE-MIT')
(resources / 'THIRD_PARTY_NOTICES.txt').write_text('''Bundled runtime components retain their respective licenses.
Source code and license information:
- yt-dlp: https://github.com/yt-dlp/yt-dlp
- FFmpeg and ffprobe: https://ffmpeg.org/legal.html
  Homebrew build and dependency sources: https://formulae.brew.sh/formula/ffmpeg
- Deno: https://github.com/denoland/deno
- whisper.cpp v1.8.2: https://github.com/ggml-org/whisper.cpp/tree/v1.8.2
This package is ad-hoc signed, not Apple Developer ID signed or notarized.
''')
shutil.copy(Path(os.environ['RUNNER_TEMP']) / 'ytb2pdf-whisper/LICENSE', resources / 'whisper-LICENSE')
