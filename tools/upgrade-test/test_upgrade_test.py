import copy
import importlib.util
import json
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('upgrade', Path(__file__).with_name('upgrade_test.py'))
upgrade = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upgrade)


class UpgradeTests(unittest.TestCase):
    def test_data_loss_and_identity_regressions(self):
        before = {'app': {'bundle_id': upgrade.BUNDLE_ID, 'requirement': 'certificate leaf = H"abc"'},
                  'data': {'models': {'path': '/cache', 'files': {'weights': 'abc'}}}}
        self.assertEqual(upgrade.compare(before, copy.deepcopy(before)), [])
        for mutation in ('missing', 'changed', 'moved', 'bundle_id', 'requirement'):
            after = copy.deepcopy(before)
            if mutation == 'missing':
                after['data']['models']['files'].clear()
            elif mutation == 'changed':
                after['data']['models']['files']['weights'] = 'bad'
            elif mutation == 'moved':
                after['data']['models']['path'] = '/new-cache'
            else:
                after['app'][mutation] = 'different'
            with self.subTest(mutation=mutation):
                self.assertTrue(upgrade.compare(before, after))
        after = copy.deepcopy(before)
        after['data']['models']['files']['new-model'] = 'xyz'
        self.assertEqual(upgrade.compare(before, after), [])

    def test_inventory_finds_legacy_and_renamed_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, bundle in [('Gretchen Flow.app', upgrade.BUNDLE_ID),
                                 ('nested/Copy.app', upgrade.BUNDLE_ID),
                                 ('Unrelated.app', 'other')]:
                contents = root / name / 'Contents'
                contents.mkdir(parents=True)
                (contents / 'Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier': bundle}))
            self.assertEqual(len(upgrade.inventory([root])), 2)
            with self.assertRaisesRegex(ValueError, 'exactly one'):
                upgrade.capture([str(root)], {})

    def test_manifest_missing_empty_modified_and_symlink(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(ValueError):
                upgrade.manifest(root)
            with self.assertRaises(ValueError):
                upgrade.manifest(root / 'missing')
            item = root / 'weights'
            item.write_bytes(b'original')
            original = upgrade.manifest(root)
            item.write_bytes(b'modified')
            self.assertNotEqual(original, upgrade.manifest(root))
            (root / 'link').symlink_to(item)
            with self.assertRaises(ValueError):
                upgrade.manifest(root)

    def test_signing_directory_is_rejected_without_read(self):
        with self.assertRaises(ValueError):
            upgrade.manifest(upgrade.FORBIDDEN / 'anything')

    def test_signature_failure_and_adhoc_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            (app / 'Contents').mkdir()
            (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({
                'CFBundleIdentifier': upgrade.BUNDLE_ID, 'CFBundleShortVersionString': '0.3.0'}))
            with patch.object(upgrade, 'run', side_effect=['', 'designated => identifier x', 'Signature=adhoc']):
                with self.assertRaisesRegex(ValueError, 'ad-hoc'):
                    upgrade.identity(app)
            with patch.object(upgrade, 'run', side_effect=upgrade.subprocess.CalledProcessError(1, 'codesign')):
                with self.assertRaises(upgrade.subprocess.CalledProcessError):
                    upgrade.identity(app)

    def test_after_cli_checks_version_and_old_requirement(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            before = {'schema': 1, 'app_roots': ['/Applications'],
                      'app': {'path': '/Applications/Gretchen Flow.app',
                              'version': '0.3.0', 'bundle_id': upgrade.BUNDLE_ID,
                              'requirement': 'identifier "production"'},
                      'data': {'history': {'path': '/history', 'files': {'.': 'abc'}}}}
            baseline = root / 'before.json'
            baseline.write_text(json.dumps(before))
            for version in ('0.3.0', '0.4.0'):
                after = copy.deepcopy(before)
                after['app'].update(version=version, path='/Applications/Greta Flow.app')
                output = root / (version + '.json')
                args = ['upgrade_test.py', 'after', '--baseline', str(baseline), '--output', str(output)]
                with patch.object(upgrade.sys, 'argv', args), patch.object(upgrade.sys, 'platform', 'darwin'), \
                        patch.object(upgrade, 'capture', return_value=after), patch.object(upgrade, 'run') as run, \
                        patch('builtins.print'):
                    if version == '0.3.0':
                        with self.assertRaisesRegex(ValueError, 'not 0.4.0'):
                            upgrade.main()
                    else:
                        upgrade.main()
                    run.assert_called_once_with('/usr/bin/codesign', '--verify', '--strict', '-R',
                                                before['app']['requirement'], after['app']['path'])
                    self.assertIn('NOT TESTED', json.loads(output.read_text())['permissions'])

    def test_evidence_cannot_overwrite(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'evidence.json'
            upgrade.save(path, {'original': True})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                upgrade.save(path, {})


if __name__ == '__main__':
    unittest.main()
