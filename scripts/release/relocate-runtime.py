#!/usr/bin/env python3
"""Copy Mach-O dependencies and rewrite each reference relative to its loader."""
import os
from pathlib import Path
import shutil
import subprocess
import sys


def output(*args):
    return subprocess.check_output(args, text=True)


def install_ids(path):
    return {line.strip() for line in output('otool', '-D', str(path)).splitlines()
            if line.strip().startswith(('/', '@')) and not line.rstrip().endswith(':')}


def dependencies(path):
    ids = install_ids(path)
    return list(dict.fromkeys(line.strip().split(' (', 1)[0]
                             for line in output('otool', '-L', str(path)).splitlines()[1:]
                             if line.strip() and not line.rstrip().endswith(':')
                             and line.strip().split(' (', 1)[0] not in ids))


def rpaths(path):
    lines = output('otool', '-l', str(path)).splitlines()
    return [lines[i + 2].strip().split('path ', 1)[1].rsplit(' (offset', 1)[0]
            for i, line in enumerate(lines)
            if line.strip() == 'cmd LC_RPATH']


def resolve(dependency, source, executable, search):
    def expand(value):
        return Path(value.replace('@loader_path', str(source.parent))
                    .replace('@executable_path', str(executable.parent)))
    if dependency.startswith('@rpath/'):
        name = dependency.removeprefix('@rpath/')
        candidates = [expand(p) / name for p in rpaths(source)]
        candidates += [directory / name for directory in search]
    else:
        candidates = [expand(dependency)]
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    raise RuntimeError(f'Cannot resolve {dependency} required by {source}; searched {candidates}')


def main():
    root = Path(sys.argv[1]).resolve()
    frameworks = root / 'Contents/Frameworks'
    frameworks.mkdir(exist_ok=True)
    prefix = Path(output('brew', '--prefix').strip())
    search = [prefix / 'lib', *sorted((prefix / 'opt').glob('*/lib'))]
    copied = {}
    processed = set()
    changed = []

    def relocate(source, destination, executable):
        if destination in processed:
            return
        processed.add(destination)
        links = dependencies(source)
        if install_ids(source):
            subprocess.run(['install_name_tool', '-id', '@loader_path/' + destination.name,
                            str(destination)], check=True)
        for dependency in links:
            if dependency.startswith(('/usr/lib/', '/System/Library/')):
                continue
            resolved = resolve(dependency, source, executable, search)
            target = frameworks / resolved.name
            if target.name in copied and copied[target.name] != resolved:
                raise RuntimeError(f'Conflicting runtime libraries: {copied[target.name]} and {resolved}')
            if target.name not in copied:
                copied[target.name] = resolved
                shutil.copy2(resolved, target)
                target.chmod(target.stat().st_mode | 0o200)
                relocate(resolved, target, executable)
            relative = os.path.relpath(target, destination.parent)
            subprocess.run(['install_name_tool', '-change', dependency,
                            '@loader_path/' + relative, str(destination)], check=True)
        changed.append(destination)

    binaries = [p for p in root.rglob('*') if p.is_file()
                and 'Mach-O' in output('file', '-b', str(p))]
    for binary in binaries:
        relocate(binary, binary, binary)
    for binary in changed:
        subprocess.run(['codesign', '--force', '--sign', '-', str(binary)], check=True)


if __name__ == '__main__':
    main()
