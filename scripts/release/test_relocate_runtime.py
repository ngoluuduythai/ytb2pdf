"""Regression checks for Poppler @rpath and extensionless Mach-O install IDs."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('relocate', Path(__file__).with_name('relocate-runtime.py'))
relocate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(relocate)


class RelocationTests(unittest.TestCase):
    def test_extensionless_install_id_is_not_a_dependency(self):
        def output(*args):
            if args[1] == '-D':
                return '/build/yt-dlp:\n/build/yt-dlp\n'
            return '/build/yt-dlp:\n\t/build/yt-dlp (compatibility version 0.0.0)\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n'
        with patch.object(relocate, 'output', side_effect=output):
            self.assertEqual(relocate.dependencies(Path('/build/yt-dlp')), ['/usr/lib/libSystem.B.dylib'])

    def test_rpath_uses_homebrew_search_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            library = root / 'libpoppler.163.dylib'
            library.touch()
            with patch.object(relocate, 'rpaths', return_value=[]):
                self.assertEqual(relocate.resolve('@rpath/libpoppler.163.dylib', root / 'pdfunite', root / 'pdfunite', [root]), library.resolve())

    def test_loader_relative_rpath_is_resolved_at_original_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'lib').mkdir()
            (root / 'bin').mkdir()
            library = root / 'lib/libggml.dylib'
            library.touch()
            with patch.object(relocate, 'rpaths', return_value=['@loader_path/../lib']):
                self.assertEqual(relocate.resolve('@rpath/libggml.dylib', root / 'bin/whisper-cli', root / 'bin/whisper-cli', []), library.resolve())

    def test_missing_dependency_fails_instead_of_prompting(self):
        with patch.object(relocate, 'rpaths', return_value=[]):
            with self.assertRaisesRegex(RuntimeError, 'Cannot resolve.*libmissing'):
                relocate.resolve('@rpath/libmissing.dylib', Path('/missing/tool'), Path('/missing/tool'), [])

    def test_rpath_load_command_parser(self):
        with patch.object(relocate, 'output', return_value='Load command 1\n          cmd LC_RPATH\n      cmdsize 48\n         path @loader_path/../lib (offset 12)\n'):
            self.assertEqual(relocate.rpaths(Path('/tool')), ['@loader_path/../lib'])
