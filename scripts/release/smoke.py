#!/usr/bin/env python3
"""Verify extracted release archives with no Homebrew tools on PATH."""
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile

info = json.loads(Path('release-info.json').read_text())
prefix = f"ytb2pdf-v{info['version']}-macos-arm64"
assets = Path('dist/assets').resolve()
with tempfile.TemporaryDirectory(prefix='ytb2pdf relocated ') as temporary:
    root = Path(temporary)
    subprocess.run(['ditto', '-x', '-k', str(assets / f'{prefix}.app.zip'), str(root)], check=True)
    with tarfile.open(assets / f'{prefix}-cli.tar.gz') as archive:
        archive.extractall(root, filter='data')
    app = root / 'ytb2pdf-app.app'
    cli = root / f'{prefix}-cli/ytb2pdf'
    gui = app / 'Contents/MacOS/ytb2pdf-app'
    tools = app / 'Contents/Resources/bin'
    env = dict(os.environ, PATH='/usr/bin:/bin:/usr/sbin:/sbin')
    expected = f"ytb2pdf {info['version']} ({info['commit']})"
    for executable in (cli, gui):
        actual = subprocess.check_output([str(executable), '--version'], env=env, text=True).strip()
        assert actual == expected, (actual, expected)
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    subprocess.run([str(tools / 'ffprobe'), '-version'], env=env, check=True)
    video = root / 'test video.mp4'
    subtitles = root / 'test video.srt'
    subprocess.run([str(tools / 'ffmpeg'), '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=480x270:rate=5', '-t', '2', '-c:v', 'mpeg4', str(video)], env=env, check=True)
    subtitles.write_text('1\n00:00:00,000 --> 00:00:02,000\nOffline release verification.\n')
    cli_pdf = root / 'cli.pdf'
    subprocess.run([str(cli), str(video), '--transcript', str(subtitles), '--work-dir', str(root / 'cli-work'), '--out', str(cli_pdf)], env=env, check=True, timeout=120)
    gui_pdf = root / 'desktop.pdf'
    request = dict(url='', local_file=str(video), v2=False, target_lang=None, api_key=None, fast_whisper=False, transcript=str(subtitles), cookies=None, work_dir=str(root / 'gui-work'), destination=str(gui_pdf))
    subprocess.run([str(gui), '--conversion-worker'], input=json.dumps(request), env=env, text=True, check=True, timeout=120)
    # Exercise the production 1500-scene chunk boundary using a tiny local
    # video and text-only captions, with no Homebrew tools available on PATH.
    long_subtitles = root / 'long captions.srt'
    def timestamp(ms):
        seconds, millis = divmod(ms, 1000)
        hours, remainder = divmod(seconds, 3600)
        minutes, seconds = divmod(remainder, 60)
        return f"{hours:02}:{minutes:02}:{seconds:02},{millis:03}"
    long_subtitles.write_text(''.join(
        f"{i + 1}\n{timestamp(i * 1000)} --> {timestamp((i + 1) * 1000)}\nRelease scene {i + 1}.\n\n"
        for i in range(1501)
    ))
    long_video = root / 'long sparse video.mp4'
    subprocess.run([str(tools / 'ffmpeg'), '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=blue:size=128x72:rate=1/60', '-t', '1502', '-c:v', 'mpeg4', str(long_video)], env=env, check=True)
    merged_pdf = root / 'chunked.pdf'
    subprocess.run([str(cli), str(long_video), '--text-only', '--transcript', str(long_subtitles), '--work-dir', str(root / 'long-work'), '--out', str(merged_pdf)], env=env, check=True, timeout=240)
    metadata = subprocess.check_output([str(tools / 'pdfinfo'), str(merged_pdf)], env=env, text=True)
    pages = next(line.split(':', 1)[1].strip() for line in metadata.splitlines() if line.startswith('Pages:'))
    assert pages == '1501', f'Chunked PDF has {pages} pages instead of 1501'
    for pdf in (cli_pdf, gui_pdf, merged_pdf):
        assert pdf.read_bytes().startswith(b'%PDF-'), f'Invalid PDF: {pdf}'
print('Relocated GUI worker and CLI generated valid PDFs without Homebrew on PATH')
