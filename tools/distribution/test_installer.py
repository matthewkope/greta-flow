"""Exercise installer control flow in a disposable fake macOS, never /Applications."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]

STUB = '''#!/usr/bin/env python3
import os, pathlib, shutil, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['CALLS'], 'a') as log:
    log.write(name + '\\n')
if name == 'uname': print(os.environ.get('OS', 'Darwin') if args == ['-s'] else os.environ.get('ARCH', 'arm64'))
elif name == 'sysctl': print(os.environ.get('ARM64', '1'))
elif name == 'sw_vers': print(os.environ.get('MACOS', '14.0'))
elif name == 'pgrep': sys.exit(0 if os.environ.get('RUNNING') else 1)
elif name == 'curl': pathlib.Path(args[args.index('-o') + 1]).write_bytes(b'dmg')
elif name == 'hdiutil' and args[0] == 'attach':
    app = pathlib.Path(args[args.index('-mountpoint') + 1]) / 'Greta Flow.app'
    (app / 'Contents').mkdir(parents=True)
    (app / 'Contents/Info.plist').write_text('new')
elif name == 'PlistBuddy': print('com.matthewkope.gretchenflow')
elif name == 'ditto':
    if os.environ.get('FAIL_COPY'): sys.exit(1)
    shutil.copytree(args[0], args[1])
elif name == 'mv':
    if os.environ.get('FAIL_REPLACE') and args[0].endswith('/Greta Flow.app') and '/.greta-flow.' in args[0]: sys.exit(1)
    os.execv('/bin/mv', ['mv'] + args)
'''


class InstallerTests(unittest.TestCase):
    def run_installer(self, **options):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            apps = root / 'Applications'
            apps.mkdir()
            for name in ('Greta Flow.app', 'Gretchen Flow.app'):
                (apps / name / 'Contents').mkdir(parents=True)
                (apps / name / 'Contents/Info.plist').write_text('old')
            bins = root / 'bin'
            bins.mkdir()
            for name in ('uname', 'sysctl', 'sw_vers', 'pgrep', 'curl', 'hdiutil', 'PlistBuddy', 'ditto', 'xattr', 'codesign', 'open', 'mv'):
                file = bins / name
                file.write_text(STUB)
                file.chmod(0o755)
            script = (ROOT / 'install.sh').read_text()
            script = script.replace('/Applications', str(apps))
            script = script.replace('/usr/bin/codesign', 'codesign').replace('/usr/libexec/PlistBuddy', 'PlistBuddy')
            digest = '0' * 64 if options.pop('bad_hash', False) else hashlib.sha256(b'dmg').hexdigest()
            for key, value in [('VERSION', '0.4.0'), ('DMG', 'Greta.Flow_0.4.0_aarch64.dmg'), ('SHA256', digest)]:
                script = script.replace(f"{key}='@{key}@'", f"{key}='{value}'")
            installer = root / 'install.sh'
            installer.write_text(script)
            calls = root / 'calls'
            env = {**os.environ, 'PATH': f'{bins}:' + os.environ['PATH'], 'CALLS': str(calls), **options}
            result = subprocess.run(['bash', installer], env=env, capture_output=True, text=True)
            content = (apps / 'Greta Flow.app/Contents/Info.plist').read_text()
            legacy = (apps / 'Gretchen Flow.app').exists()
            return result, content, legacy, calls.read_text()

    def test_update_replaces_app_and_removes_legacy(self):
        result, content, legacy, calls = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(content, 'new')
        self.assertFalse(legacy)
        self.assertIn('open\n', calls)

    def test_platform_checks_fail_before_download(self):
        for options, message in [
            ({'OS': 'Linux'}, 'macOS is required'),
            ({'ARCH': 'x86_64', 'ARM64': '0'}, 'Apple Silicon is required'),
            ({'MACOS': '13.6'}, 'macOS 14 or later is required'),
            ({'MACOS': 'invalid'}, 'macOS 14 or later is required'),
        ]:
            with self.subTest(options=options):
                result, content, legacy, calls = self.run_installer(**options)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)
                self.assertEqual(content, 'old')
                self.assertTrue(legacy)
                self.assertNotIn('curl\n', calls)
                self.assertNotIn('open\n', calls)

    def test_supported_versions_and_rosetta(self):
        for options in ({'MACOS': '14.0'}, {'MACOS': '15.0'}, {'MACOS': '26.0'},
                        {'ARCH': 'x86_64', 'ARM64': '1'}):
            with self.subTest(options=options):
                result, content, _, _ = self.run_installer(**options)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(content, 'new')

    def test_failures_preserve_old_installation(self):
        for options in ({'bad_hash': True}, {'FAIL_COPY': '1'}, {'FAIL_REPLACE': '1'},
                        {'RUNNING': '1'}, {'MACOS': '13.6'}):
            with self.subTest(options=options):
                result, content, legacy, calls = self.run_installer(**options)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(content, 'old')
                self.assertTrue(legacy)
                self.assertNotIn('open\n', calls)
                if options.get('bad_hash'):
                    self.assertNotIn('hdiutil\n', calls)


if __name__ == '__main__':
    unittest.main()
