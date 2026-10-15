#!/usr/bin/env python3
"""Reject missing architectures or absolute non-system dylib dependencies."""
from pathlib import Path
import subprocess
import sys

root = Path(sys.argv[1]).resolve()
for path in root.rglob("*"):
    if not path.is_file():
        continue
    kind = subprocess.check_output(["file", "-b", str(path)], text=True)
    if "Mach-O" not in kind:
        continue
    if "arm64" not in kind:
        raise SystemExit(f"Missing arm64 architecture: {path}")
    links = subprocess.check_output(["otool", "-L", str(path)], text=True).splitlines()[1:]
    ids = {line.strip() for line in subprocess.check_output(
        ['otool', '-D', str(path)], text=True).splitlines()
        if line.strip().startswith(('/', '@')) and not line.rstrip().endswith(':')}
    for line in links:
        dependency = line.strip().split(" (", 1)[0]
        if dependency in ids or dependency.endswith(':'):
            continue
        # Some standalone binaries expose their own install name in -L, while
        # -D omits it (notably on some universal Mach-O layouts).
        if dependency.startswith(('/', '@')) and Path(dependency).resolve() == path.resolve():
            continue
        if dependency.startswith("/") and not dependency.startswith(("/usr/lib/", "/System/Library/")):
            raise SystemExit(f"Non-relocatable dependency in {path}: {dependency}")
        if dependency.startswith("@rpath/"):
            raise SystemExit(f"Unresolved runtime search path in {path}: {dependency}")
        if dependency.startswith("@loader_path/"):
            target = path.parent / dependency.removeprefix("@loader_path/")
            if not target.resolve().is_file():
                raise SystemExit(f"Missing loader-relative dependency in {path}: {dependency}")
        if dependency.startswith("@executable_path/"):
            target = root / "Contents/Resources/bin" / dependency.removeprefix("@executable_path/")
            if not target.resolve().is_file():
                raise SystemExit(f"Missing bundled dependency in {path}: {dependency}")
print("Runtime architecture and dependency checks passed")
