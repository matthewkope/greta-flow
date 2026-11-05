#!/usr/bin/env python3
"""Read-only evidence for a real macOS 0.3.0 -> 0.4.0 upgrade."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys

BUNDLE_ID = 'com.matthewkope.gretchenflow'
NAMES = {'Gretchen Flow.app', 'Greta Flow.app'}
FORBIDDEN = Path.home() / 'gretchen-flow-signing'


def safe(path):
    path = Path(path).expanduser().absolute()
    for candidate in (path, path.resolve()):
        if candidate == FORBIDDEN or FORBIDDEN in candidate.parents:
            raise ValueError('Signing directory must never be accessed')
    return path


def digest(path):
    path = safe(path)
    if path.is_symlink():
        raise ValueError(f'Symlink requires manual review: {path}')
    with path.open('rb') as stream:
        h = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def walk_error(error):
    raise error


def manifest(root):
    root = safe(root)
    if root.is_symlink():
        raise ValueError(f'Symlink requires manual review: {root}')
    if root.is_file():
        return {'.': digest(root)}
    if not root.is_dir():
        raise ValueError(f'Required baseline data missing: {root}')
    files = {}
    for folder, dirs, names in os.walk(root, followlinks=False, onerror=walk_error):
        for name in dirs + names:
            path = safe(Path(folder) / name)
            if path.is_symlink():
                raise ValueError(f'Symlink requires manual review: {path}')
        for name in sorted(names):
            path = Path(folder) / name
            files[str(path.relative_to(root))] = digest(path)
    if not files:
        raise ValueError(f'Required baseline data empty: {root}')
    return files


def run(*args):
    result = subprocess.run(args, text=True, capture_output=True, check=True)
    return result.stdout + result.stderr


def identity(app):
    safe(app)
    plist = safe(app / 'Contents/Info.plist')
    with plist.open('rb') as stream:
        info = plistlib.load(stream)
    run('/usr/bin/codesign', '--verify', '--deep', '--strict', str(app))
    output = run('/usr/bin/codesign', '-d', '-r-', str(app))
    requirements = [line.removeprefix('designated => ').strip()
                    for line in output.splitlines() if line.startswith('designated => ')]
    if len(requirements) != 1:
        raise ValueError(f'Missing designated requirement: {app}')
    details = run('/usr/bin/codesign', '-dv', '--verbose=4', str(app))
    if 'Signature=adhoc' in details or 'Authority=' not in details:
        raise ValueError(f'Expected certificate-signed release, not ad-hoc code: {app}')
    return {'path': str(app), 'bundle_id': info.get('CFBundleIdentifier'),
            'version': info.get('CFBundleShortVersionString'),
            'requirement': requirements[0]}


def inventory(roots):
    apps = []
    for root in roots:
        root = safe(root)
        if not root.exists():
            continue
        if not root.is_dir() or root.is_symlink():
            raise ValueError(f'Invalid application root: {root}')
        for folder, dirs, _ in os.walk(root, followlinks=False, onerror=walk_error):
            for name in list(dirs):
                path = safe(Path(folder) / name)
                if path.is_symlink():
                    raise ValueError(f'Symlink requires manual review: {path}')
                if name.endswith('.app'):
                    dirs.remove(name)  # Never descend into unrelated application bundles.
                    plist = safe(path / 'Contents/Info.plist')
                    if plist.exists():
                        with plist.open('rb') as stream:
                            bundle = plistlib.load(stream).get('CFBundleIdentifier')
                    else:
                        bundle = None
                    if name in NAMES or bundle == BUNDLE_ID:
                        apps.append(path)
    return sorted(set(apps))


def capture(roots, data):
    apps = inventory(roots)
    if len(apps) != 1:
        raise ValueError(f'Expected exactly one app in configured roots; found {len(apps)}: {apps}')
    app = identity(apps[0])
    if app['bundle_id'] != BUNDLE_ID:
        raise ValueError('Unexpected bundle ID')
    return {'schema': 1, 'app_roots': roots, 'app': app, 'data': {
        label: {'path': str(safe(path)), 'files': manifest(path)}
        for label, path in data.items()}}


def compare(before, after):
    errors = []
    for field in ('bundle_id', 'requirement'):
        if before['app'][field] != after['app'][field]:
            errors.append(f'Code identity changed: {field}')
    for label, old in before['data'].items():
        new = after['data'][label]
        if old['path'] != new['path']:
            errors.append(f'{label}: storage path changed')
        for name, sha in old['files'].items():
            if new['files'].get(name) != sha:
                errors.append(f'{label}: missing or changed file: {name}')
    return errors


def save(path, value):
    path = safe(path)
    # Exclusive creation prevents accidentally replacing the baseline; private evidence.
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    before = sub.add_parser('before')
    before.add_argument('--output', type=Path, required=True)
    before.add_argument('--app-root', action='append', help='Repeat for additional install roots; replaces defaults')
    before.add_argument('--extra-model', action='append', default=[], help='Absolute custom model file/directory')
    after = sub.add_parser('after')
    after.add_argument('--baseline', type=Path, required=True)
    after.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('Run on the Mac being upgraded')
    home = Path.home()
    if args.command == 'before':
        roots = args.app_root or ['/Applications', str(home / 'Applications')]
        roots = list(dict.fromkeys(str(safe(root)) for root in roots))
        data = {'config': home / '.config/gretchen-flow/config.json',
                'models': home / '.cache/gretchen-flow/models',
                'history': home / 'Library/Application Support/gretchen-flow/history.jsonl'}
        data.update({f'extra-model-{i}': safe(path) for i, path in enumerate(args.extra_model)})
        evidence = capture(roots, data)
        if evidence['app']['version'] != '0.3.0':
            raise ValueError('Baseline must be a real 0.3.0 release')
        if not Path(data['history']).stat().st_size:
            raise ValueError('History is empty; create a non-sensitive baseline entry first')
        save(args.output, evidence)
        print('Baseline saved. Upgrade and run after; permission checks remain manual.')
    else:
        with safe(args.baseline).open() as stream:
            baseline = json.load(stream)
        if baseline['schema'] != 1 or baseline['app']['version'] != '0.3.0':
            raise ValueError('Invalid 0.3.0 baseline')
        evidence = capture(baseline['app_roots'], {
            label: entry['path'] for label, entry in baseline['data'].items()})
        errors = compare(baseline, evidence)
        if evidence['app']['version'] != '0.4.0':
            errors.append('Installed version is not 0.4.0')
        if Path(evidence['app']['path']).name != 'Greta Flow.app':
            errors.append('Expected renamed Greta Flow.app')
        # Verify the new executable actually satisfies the old requirement as well.
        run('/usr/bin/codesign', '--verify', '--strict', '-R',
            baseline['app']['requirement'], evidence['app']['path'])
        evidence['failures'] = errors
        evidence['permissions'] = 'NOT TESTED: manual confirmation required'
        save(args.output, evidence)
        if errors:
            raise ValueError('\n'.join(errors))
        print('PASS: one app in scanned roots; original data hashes and code identity preserved.\n'
              'Permissions, offline model loading and real dictation still require manual checks.')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f'FAIL: {error}', file=sys.stderr)
        sys.exit(1)
