"""Check that macOS metadata and runtime provenance use the resolved release."""
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('bundle-metadata.py').resolve()


class BundleMetadataTests(unittest.TestCase):
    def test_bundle_has_version_build_identity_and_runtime_inventory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            resources = root / 'dist/ytb2pdf-app.app/Contents/Resources'
            tools = resources / 'bin'
            tools.mkdir(parents=True)
            for tool in ('yt-dlp', 'ffmpeg', 'ffprobe', 'deno', 'pdfunite', 'pdfinfo'):
                path = tools / tool
                path.write_text(f'#!/bin/sh\nprintf "{tool} test-version\\n"\n')
                path.chmod(0o755)
            info = dict(version='1.2.3-rc.2', bundle_version='1.2.3', commit='abcdef0123456789', minimum_macos='14.0')
            (root / 'release-info.json').write_text(json.dumps(info))
            (root / 'README.md').write_text('Readme')
            (root / 'LICENSE-MIT').write_text('License')
            (root / 'ytb2pdf-whisper').mkdir()
            (root / 'ytb2pdf-whisper/LICENSE').write_text('Whisper license')
            env = dict(os.environ, RUNNER_TEMP=str(root), GITHUB_RUN_NUMBER='42', GITHUB_RUN_ATTEMPT='2')
            subprocess.run(['python3', str(SCRIPT)], cwd=root, env=env, check=True)
            with (resources.parent / 'Info.plist').open('rb') as source:
                plist = plistlib.load(source)
            self.assertEqual(plist['CFBundleShortVersionString'], '1.2.3')
            self.assertEqual(plist['CFBundleVersion'], '42.2')
            self.assertEqual(plist['Ytb2pdfReleaseVersion'], '1.2.3-rc.2')
            inventory = json.loads((resources / 'build-info.json').read_text())
            self.assertEqual(inventory['commit'], info['commit'])
            self.assertEqual(set(inventory['runtime']), {'yt-dlp', 'ffmpeg', 'ffprobe', 'pdfunite', 'pdfinfo', 'deno', 'whisper.cpp'})
            self.assertTrue((resources / 'whisper-LICENSE').exists())
