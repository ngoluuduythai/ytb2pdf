#!/usr/bin/env bash
# Keep incomplete uploads as a draft; never overwrite a published release.
set -euo pipefail
: "${RELEASE_TAG:?}" "${RELEASE_PRERELEASE:?}" "${GITHUB_SHA:?}"
if existing=$(gh release view "$RELEASE_TAG" --json isDraft --jq .isDraft 2>/dev/null); then
  if [[ "$existing" != true ]]; then
    echo "Release $RELEASE_TAG is already published. Use a new version; published assets will not be overwritten." >&2
    exit 1
  fi
fi
# Create the tag explicitly for manual runs: a draft release alone need not
# materialize its tag until publication. Never move an existing tag.
remote_commit=$(git ls-remote origin "refs/tags/$RELEASE_TAG^{}" | cut -f1)
if [[ -z "$remote_commit" ]]; then
  remote_commit=$(git ls-remote origin "refs/tags/$RELEASE_TAG" | cut -f1)
fi
if [[ -z "$remote_commit" ]]; then
  git push origin "$GITHUB_SHA:refs/tags/$RELEASE_TAG"
else
  [[ "$remote_commit" == "$GITHUB_SHA" ]] || { echo 'Release tag does not match the build commit' >&2; exit 1; }
fi
if [[ -z "${existing:-}" ]]; then
  # Merge curated installation details with GitHub's generated change list.
  gh api --method POST "repos/$GITHUB_REPOSITORY/releases/generate-notes" \
    -f tag_name="$RELEASE_TAG" -f target_commitish="$GITHUB_SHA" \
    --jq .body >> dist/RELEASE_NOTES.md
  gh release create "$RELEASE_TAG" --verify-tag --target "$GITHUB_SHA" --draft \
    --title "ytb2pdf $RELEASE_TAG" --notes-file dist/RELEASE_NOTES.md
fi
gh release upload "$RELEASE_TAG" dist/assets/* --clobber
# Check that all expected assets were acknowledged before making the draft public.
python3 - <<'PY'
import json, os
from pathlib import Path
import subprocess
assets = json.loads(subprocess.check_output(['gh', 'release', 'view', os.environ['RELEASE_TAG'], '--json', 'assets']))['assets']
uploaded = {asset['name']: asset['size'] for asset in assets}
for path in Path('dist/assets').iterdir():
    assert uploaded.get(path.name) == path.stat().st_size, f'Missing or incomplete upload: {path.name}'
PY
if [[ "$RELEASE_PRERELEASE" == true ]]; then
  gh release edit "$RELEASE_TAG" --draft=false --prerelease --latest=false
else
  gh release edit "$RELEASE_TAG" --draft=false --prerelease=false --latest
fi
