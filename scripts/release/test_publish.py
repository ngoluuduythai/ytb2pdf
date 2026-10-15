"""Exercise draft publication without contacting GitHub or uploading assets."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('publish.sh').resolve()


class PublishTests(unittest.TestCase):
    def run_publish(self, existing, prerelease='false', missing=False, tag_missing=False):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        (root / 'bin').mkdir()
        (root / 'dist/assets').mkdir(parents=True)
        (root / 'dist/assets/test.dmg').write_bytes(b'fake asset')
        (root / 'dist/RELEASE_NOTES.md').write_text('Installation notes\n')
        gh = root / 'bin/gh'
        gh.write_text('''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
args = sys.argv[1:]
with open('calls.jsonl', 'a') as log:
    print(json.dumps(args), file=log)
if args[:2] == ['release', 'view']:
    if 'isDraft' in args:
        existing = os.environ['EXISTING']
        if existing == 'absent': sys.exit(1)
        print(existing)
    else:
        print(json.dumps({'assets': [] if os.environ['MISSING'] == 'yes' else [{'name': 'test.dmg', 'size': 10}]}))
elif args[:2] == ['release', 'create'] and os.environ['TAG_MISSING'] == 'yes':
    assert Path('tag-created').exists(), 'Manual release needs a real tag before the draft'
elif args[0] == 'api':
    print('Generated changelog')
''')
        gh.chmod(0o755)
        git = root / 'bin/git'
        git.write_text('''#!/usr/bin/env python3
import os, sys
from pathlib import Path
if sys.argv[1] == 'push':
    Path('tag-created').touch()
elif os.environ['TAG_MISSING'] != 'yes' or Path('tag-created').exists():
    print('commit123\\trefs/tags/v1.2.3')
''')
        git.chmod(0o755)
        env = dict(os.environ, PATH=str(root / 'bin') + os.pathsep + os.environ['PATH'], RELEASE_TAG='v1.2.3', RELEASE_PRERELEASE=prerelease, GITHUB_SHA='commit123', GITHUB_REPOSITORY='owner/repo', EXISTING=existing, MISSING='yes' if missing else 'no', TAG_MISSING='yes' if tag_missing else 'no')
        result = subprocess.run(['bash', str(SCRIPT)], cwd=root, env=env, capture_output=True, text=True)
        calls = [json.loads(line) for line in (root / 'calls.jsonl').read_text().splitlines()]
        return result, calls

    def test_stable_release_is_published_only_after_upload(self):
        result, calls = self.run_publish('absent')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('--draft', next(c for c in calls if c[:2] == ['release', 'create']))
        upload = next(i for i, c in enumerate(calls) if c[:2] == ['release', 'upload'])
        publish = next(i for i, c in enumerate(calls) if c[:2] == ['release', 'edit'])
        self.assertLess(upload, publish)
        self.assertIn('--latest', calls[publish])

    def test_manual_release_creates_its_tag_before_the_draft(self):
        result, calls = self.run_publish('absent', tag_missing=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any(c[:2] == ['release', 'create'] for c in calls))

    def test_prerelease_does_not_replace_latest(self):
        result, calls = self.run_publish('true', 'true')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('--latest=false', calls[-1])

    def test_existing_public_release_is_never_overwritten(self):
        result, calls = self.run_publish('false')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(c[:2] == ['release', 'upload'] for c in calls))

    def test_incomplete_asset_upload_keeps_the_release_draft(self):
        result, calls = self.run_publish('true', missing=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(c[:2] == ['release', 'edit'] for c in calls))
