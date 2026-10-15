"""Ensure Mach-O install IDs and self references are not treated as libraries."""
from pathlib import Path
import runpy
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name('verify-runtime.py').resolve()


class VerifyRuntimeTests(unittest.TestCase):
    def run_verifier(self, self_reference, dependency):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / 'Contents/Resources/bin/yt-dlp'
            binary.parent.mkdir(parents=True)
            binary.touch()

            def output(command, **kwargs):
                if command[0] == 'file':
                    return 'Mach-O 64-bit executable arm64'
                if command[1] == '-D':
                    return f'{binary} (architecture arm64):\n{self_reference}\n'
                return f'{binary} (architecture arm64):\n\t{self_reference} (compatibility version 0.0.0)\n\t{dependency} (compatibility version 1.0.0)\n'

            with patch.object(sys, 'argv', [str(SCRIPT), temporary]), patch(
                'subprocess.check_output', side_effect=output
            ):
                runpy.run_path(str(SCRIPT), run_name='__main__')

    def test_extensionless_binary_self_reference_is_ignored(self):
        path = '/tmp/yt-dlp'
        self.run_verifier(path, path)

    def test_dependency_outside_bundle_is_rejected(self):
        path = '/tmp/yt-dlp'
        with self.assertRaisesRegex(SystemExit, 'Non-relocatable dependency'):
            self.run_verifier(path, '/opt/homebrew/lib/libmissing.dylib')
