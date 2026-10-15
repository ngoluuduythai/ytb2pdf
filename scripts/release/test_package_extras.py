"""Check archive contents, launcher relocation and repeatable checksums."""
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SCRIPT = Path(__file__).with_name('package-extras.py').resolve()


class PackageExtrasTests(unittest.TestCase):
    def test_packaging_and_relocated_launcher(self):
        original_run = subprocess.run
        with tempfile.TemporaryDirectory(prefix='release test ') as temporary:
            root = Path(temporary)
            prefix = 'ytb2pdf-v0.2.0-rc.1-macos-arm64'
            app = root / 'dist/ytb2pdf-app.app'
            tools = app / 'Contents/Resources/bin'
            tools.mkdir(parents=True)
            (tools / 'ffmpeg').write_text('#!/bin/sh\nprintf "bundled-ffmpeg\\n"\n')
            (tools / 'ffmpeg').chmod(0o755)
            (app / 'Contents/MacOS').mkdir(parents=True)
            binary = app / 'Contents/MacOS/ytb2pdf-cli'
            binary.write_text('#!/bin/sh\nffmpeg\nprintf "%s\\n" "$@"\n')
            binary.chmod(0o755)
            (root / f'{prefix}.dmg').write_bytes(b'original DMG')
            (root / 'release-info.json').write_text(json.dumps(dict(version='0.2.0-rc.1', commit='abc123')))
            for name in ('README.md', 'LICENSE-MIT'):
                (root / name).write_text(name)

            def run(command, **kwargs):
                if command[0] == 'ditto':
                    # macOS ditto is exercised on the runner; emulate its ZIP
                    # output here so the remaining packaging uses real tools.
                    with zipfile.ZipFile(command[-1], 'w') as archive:
                        for path in app.rglob('*'):
                            if path.is_file():
                                archive.write(path, path.relative_to(app.parent))
                    return subprocess.CompletedProcess(command, 0)
                return original_run(command, **kwargs)

            previous = Path.cwd()
            try:
                os.chdir(root)
                with patch('subprocess.run', side_effect=run):
                    runpy.run_path(str(SCRIPT), run_name='__main__')
                    runpy.run_path(str(SCRIPT), run_name='__main__')
            finally:
                os.chdir(previous)

            assets = root / 'dist/assets'
            entries = (assets / 'SHA256SUMS.txt').read_text().splitlines()
            self.assertEqual(len(entries), 5)
            for entry in entries:
                digest, name = entry.split('  ', 1)
                self.assertEqual(hashlib.sha256((assets / name).read_bytes()).hexdigest(), digest)
            self.assertEqual((assets / f'{prefix}.dmg').read_bytes(), b'original DMG')
            with zipfile.ZipFile(assets / f'{prefix}.app.zip') as archive:
                self.assertIn('ytb2pdf-app.app/Contents/Resources/bin/ffmpeg', archive.namelist())
            moved = root / 'relocated CLI'
            moved.mkdir()
            with tarfile.open(assets / f'{prefix}-cli.tar.gz') as archive:
                archive.extractall(moved, filter='data')
            launcher = moved / f'{prefix}-cli/ytb2pdf'
            result = original_run([str(launcher), 'argument with spaces'], cwd='/', capture_output=True, text=True, check=True)
            self.assertEqual(result.stdout.splitlines(), ['bundled-ffmpeg', 'argument with spaces'])
