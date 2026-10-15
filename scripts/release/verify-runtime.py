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
    # For dylibs the first entry is its install ID, not a dependency.
    if path.suffix == ".dylib":
        links = links[1:]
    for line in links:
        dependency = line.strip().split(" (", 1)[0]
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
