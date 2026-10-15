import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('prepare', Path(__file__).with_name('prepare.py'))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


class VersionTests(unittest.TestCase):
    def resolve(self, event='workflow_dispatch', ref_type='branch', ref='main', requested=''):
        return prepare.resolve_version(event, ref_type, ref, requested, '0.1.0', '1234', '2')

    def test_tag_is_the_authoritative_version(self):
        info = self.resolve('push', 'tag', 'v1.2.3', '9.9.9')
        self.assertEqual(info['version'], '1.2.3')
        self.assertEqual(info['bundle_version'], '1.2.3')
        self.assertFalse(info['prerelease'])

    def test_prerelease_preserves_full_version(self):
        info = self.resolve('push', 'tag', 'v1.2.3-rc.1')
        self.assertEqual(info['tag'], 'v1.2.3-rc.1')
        self.assertEqual(info['bundle_version'], '1.2.3')
        self.assertTrue(info['prerelease'])

    def test_manual_version_and_unique_development_build(self):
        self.assertEqual(self.resolve(requested='v2.0.0')['version'], '2.0.0')
        info = self.resolve()
        self.assertEqual(info['version'], '0.1.0-dev.1234.2')
        self.assertTrue(info['prerelease'])

    def test_rejects_ambiguous_or_unsafe_versions(self):
        for value in ['1.2', '01.2.3', '1.2.3-01', '1.2.3+metadata', 'v1.2.3/evil', '1.2.3\nTAG=evil', '1.2.3-rc..1']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.resolve(requested=value)
        with self.assertRaises(ValueError):
            self.resolve('push', 'branch', 'main')


if __name__ == '__main__':
    unittest.main()
