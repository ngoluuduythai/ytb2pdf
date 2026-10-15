#!/usr/bin/env python3
"""Resolve a release identity without changing the checked-in Cargo versions."""
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

SEMVER = re.compile(
    r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)"
    r"(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?"
)


def resolve_version(event, ref_type, ref_name, requested, base, run_id, attempt):
    if event == "push":
        if ref_type != "tag" or not ref_name.startswith("v"):
            raise ValueError("Release pushes must use a v-prefixed version tag")
        version = ref_name[1:]
    else:
        version = requested.strip().removeprefix("v")
        if not version:
            version = f"{base.split('-')[0]}-dev.{run_id}.{attempt}"
    match = SEMVER.fullmatch(version)
    if not match:
        raise ValueError("Use SemVer such as 0.2.0 or 0.2.0-rc.1 (no build metadata)")
    return {
        "version": version,
        "bundle_version": ".".join(match.group(i) for i in (1, 2, 3)),
        "tag": f"v{version}",
        "prerelease": bool(match.group(4)),
    }


def main():
    base = tomllib.loads(Path("Cargo.toml").read_text())["workspace"]["package"]["version"]
    info = resolve_version(
        os.environ["GITHUB_EVENT_NAME"], os.environ["GITHUB_REF_TYPE"],
        os.environ["GITHUB_REF_NAME"], os.environ.get("REQUESTED_VERSION", ""),
        base, os.environ["GITHUB_RUN_ID"], os.environ["GITHUB_RUN_ATTEMPT"],
    )
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    existing = subprocess.run(
        ["git", "rev-parse", "--verify", f"refs/tags/{info['tag']}^{{commit}}"],
        capture_output=True, text=True,
    )
    if existing.returncode == 0 and existing.stdout.strip() != sha:
        raise ValueError(f"{info['tag']} already points to a different commit; choose a new version")
    info.update(commit=sha, target="aarch64-apple-darwin", minimum_macos="14.0",
                run_url=f"{os.environ['GITHUB_SERVER_URL']}/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{os.environ['GITHUB_RUN_ID']}",
                signing="ad-hoc (not notarized)")
    Path("release-info.json").write_text(json.dumps(info, indent=2) + "\n")
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        for key in ("version", "bundle_version", "tag", "prerelease"):
            print(f"{key}={str(info[key]).lower()}", file=output)
    with open(os.environ["GITHUB_ENV"], "a") as env:
        print(f"YTB2PDF_VERSION={info['version']}", file=env)
        print(f"YTB2PDF_COMMIT={sha}", file=env)
    print(f"Preparing {info['tag']} at {sha}")


if __name__ == "__main__":
    main()
