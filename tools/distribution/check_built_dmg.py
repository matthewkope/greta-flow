#!/usr/bin/env python3
"""Exercise a built DMG in disposable app directories; never launch the app."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import tempfile

from prepare_release import prepare

ROOT = Path(__file__).resolve().parents[2]


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def verify_app(app):
    run('/usr/bin/codesign', '--verify', '--deep', '--strict', str(app))
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    assert info['CFBundleIdentifier'] == 'com.matthewkope.gretchenflow'
    assert info['CFBundleShortVersionString'] == VERSION
    assert info['LSMinimumSystemVersion'] == '14.0'
    run('/usr/bin/lipo', str(app / 'Contents/MacOS' / info['CFBundleExecutable']), '-verify_arch', 'arm64')
    attrs = run('xattr', '-r', str(app), capture_output=True).stdout
    assert 'com.apple.quarantine' not in attrs, attrs


def check(dmg):
    with tempfile.TemporaryDirectory(prefix='greta-install-ci-') as directory:
        root = Path(directory)
        assets = root / 'assets'
        prepare(ROOT, f'v{VERSION}', dmg, assets)
        run('shasum', '-a', '256', '-c', 'SHA256SUMS', cwd=assets)
        pinned = assets / dmg.name.replace(' ', '.')
        run('xattr', '-w', 'com.apple.quarantine', '0081;00000000;CI;', str(pinned))
        bins = root / 'bin'
        bins.mkdir()
        # Only transport, running-process detection, and launch are substituted.
        # hdiutil, codesign, PlistBuddy, ditto, xattr and filesystem changes are real.
        stubs = {
            'curl': 'while [[ $1 != -o ]]; do shift; done; cp "$CI_DMG" "$2"',
            'pgrep': 'exit 1',
            'open': 'printf "%s\\n" "$*" >> "$CI_OPEN_LOG"',
        }
        for name, body in stubs.items():
            path = bins / name
            path.write_text('#!/bin/bash\nset -eu\n' + body + '\n')
            path.chmod(0o755)
        apps = root / 'Applications'
        apps.mkdir()
        env = {**os.environ, 'PATH': f'{bins}:' + os.environ['PATH'],
               'CI_DMG': str(pinned), 'CI_OPEN_LOG': str(root / 'opened')}
        script = (assets / 'install.sh').read_text().replace('/Applications', str(apps))
        installer = root / 'install.sh'
        installer.write_text(script)
        app = apps / 'Greta Flow.app'
        run('bash', str(installer), env=env)
        verify_app(app)
        # A stale file distinguishes replacement from a no-op/overlay copy.
        marker = app / 'stale-ci-file'
        marker.write_text('old installation')
        run('xattr', '-w', 'com.apple.quarantine', '0081;00000000;CI;', str(app))
        before = (app / 'Contents/Info.plist').read_bytes()
        bad = root / 'bad-install.sh'
        digest = hashlib.sha256(pinned.read_bytes()).hexdigest()
        bad.write_text(script.replace(f"SHA256='{digest}'", "SHA256='" + '0' * 64 + "'"))
        result = subprocess.run(['bash', str(bad)], env=env, text=True, capture_output=True)
        assert result.returncode != 0 and 'SHA-256 mismatch' in result.stderr, result
        assert marker.exists() and (app / 'Contents/Info.plist').read_bytes() == before
        assert len((root / 'opened').read_text().splitlines()) == 1
        run('bash', str(installer), env=env)
        assert not marker.exists()
        verify_app(app)
        assert len((root / 'opened').read_text().splitlines()) == 2
        assert not list(apps.glob('.greta-flow.*')), 'installer staging leaked'

        # Audit the actual release URL, then install an otherwise identical cask
        # with a local artifact URL. No release publication or signing secrets.
        tap = f'greta-ci/install-{os.getpid()}'
        token = f'{tap}/greta-flow'
        brew_env = {**os.environ, 'HOMEBREW_NO_AUTO_UPDATE': '1',
                    'HOMEBREW_NO_INSTALL_CLEANUP': '1'}
        run('brew', 'tap-new', '--no-git', tap, env=brew_env)
        install_attempted = False
        try:
            tap_dir = Path(run('brew', '--repository', tap, capture_output=True, env=brew_env).stdout.strip())
            cask = tap_dir / 'Casks/greta-flow.rb'
            cask.parent.mkdir(exist_ok=True)
            release_cask = (assets / 'greta-flow.rb').read_text()
            cask.write_text(release_cask)
            run('ruby', '-c', str(cask))
            run('brew', 'style', str(cask), env=brew_env)
            run('brew', 'audit', '--cask', token, env=brew_env)
            # A unique test token cannot replace a user's Homebrew receipt.
            local_name = f'greta-flow-ci-{os.getpid()}'
            token = f'{tap}/{local_name}'
            cask = cask.with_name(f'{local_name}.rb')
            local_cask = release_cask.replace('cask "greta-flow"', f'cask "{local_name}"', 1)
            cask.write_text(re.sub(r'^  url .*$', f'  url "{pinned.as_uri()}"', local_cask, flags=re.M))
            cask_apps = root / 'cask-apps'
            cask_apps.mkdir()
            install_attempted = True
            run('brew', 'install', '--cask', '--appdir', str(cask_apps), token, env=brew_env)
            cask_app = cask_apps / 'Greta Flow.app'
            verify_app(cask_app)
            (cask_app / 'stale-ci-file').write_text('old installation')
            run('xattr', '-w', 'com.apple.quarantine', '0081;00000000;CI;', str(cask_app))
            run('brew', 'reinstall', '--cask', '--appdir', str(cask_apps), token, env=brew_env)
            verify_app(cask_app)
            assert not (cask_app / 'stale-ci-file').exists()
        finally:
            # No --zap: never touch user settings or model caches.
            if install_attempted:
                subprocess.run(['brew', 'uninstall', '--cask', token], env=brew_env, check=False)
            run('brew', 'untap', tap, env=brew_env)
    print('Built-DMG installer and cask checks passed (no app launch or Gatekeeper validation).')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dmg', type=Path, required=True)
    args = parser.parse_args()
    VERSION = json.loads((ROOT / 'desktop/src-tauri/tauri.conf.json').read_text())['version']
    check(args.dmg.resolve())
