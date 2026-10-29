#!/usr/bin/env python3
"""Sets Postie's version in Cargo.toml and Cargo.lock, then prints it.

    python3 scripts/bump-version.py patch     0.1.0 -> 0.1.1
    python3 scripts/bump-version.py minor     0.1.3 -> 0.2.0
    python3 scripts/bump-version.py major     0.4.2 -> 1.0.0
    python3 scripts/bump-version.py current   leaves it alone, for a version already set by hand

The release workflow runs this when you press Run workflow.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KINDS = ('patch', 'minor', 'major', 'current')


def bump(version: str, kind: str) -> str:
    if kind == 'current':
        return version
    core = re.split(r'[-+]', version, maxsplit=1)[0]
    major, minor, patch = (int(part) for part in core.split('.'))
    if kind == 'major':
        return f'{major + 1}.0.0'
    if kind == 'minor':
        return f'{major}.{minor + 1}.0'
    return f'{major}.{minor}.{patch + 1}'


def main() -> None:
    kind = sys.argv[1] if len(sys.argv) > 1 else ''
    if kind not in KINDS:
        sys.exit(f'Pass one of: {", ".join(KINDS)}.')

    manifest = ROOT / 'Cargo.toml'
    text = manifest.read_text()
    # The only line that starts with `version =` is the package's own; dependencies are inline tables.
    found = re.search(r'^version = "([^"]+)"', text, re.MULTILINE)
    if not found:
        sys.exit('Cargo.toml has no version line.')
    current = found.group(1)
    new = bump(current, kind)
    if new == current:
        print(new)
        return

    manifest.write_text(text[:found.start(1)] + new + text[found.end(1):])
    lock = ROOT / 'Cargo.lock'
    entry = re.compile(r'(\[\[package\]\]\nname = "postie"\nversion = ")[^"]+(")')
    lock_text, count = entry.subn(rf'\g<1>{new}\g<2>', lock.read_text())
    if count != 1:
        sys.exit('Cargo.lock has no single entry for postie.')
    lock.write_text(lock_text)
    print(new)


if __name__ == '__main__':
    main()
