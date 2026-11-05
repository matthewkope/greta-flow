import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from prepare_release import prepare

ROOT = Path(__file__).resolve().parents[2]


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in ('install.sh', 'Casks/greta-flow.rb', 'desktop/src-tauri/tauri.conf.json'):
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text((ROOT / name).read_text())
        self.version = json.loads((self.root / 'desktop/src-tauri/tauri.conf.json').read_text())['version']
        self.dmg = self.root / f'Greta Flow_{self.version}_aarch64.dmg'
        self.dmg.write_bytes(b'fixture dmg bytes')
        self.out = self.root / 'output'

    def test_assets_agree_and_legacy_cask_is_migrated(self):
        prepare(self.root, f'v{self.version}', self.dmg, self.out)
        digest = hashlib.sha256(self.dmg.read_bytes()).hexdigest()
        asset = self.dmg.name.replace(' ', '.')
        self.assertEqual((self.out / 'SHA256SUMS').read_text(), f'{digest}  {asset}\n')
        installer = (self.out / 'install.sh').read_text()
        self.assertIn(f"SHA256='{digest}'", installer)
        self.assertIn(f"DMG='{asset}'", installer)
        self.assertNotIn("VERSION='@VERSION@'", installer)
        self.assertIn("[[ $VERSION != '@VERSION@' ]]", installer)
        cask = (self.out / 'greta-flow.rb').read_text()
        self.assertIn(digest, cask)
        self.assertIn('app "Greta Flow.app"', cask)
        self.assertNotIn('Gretchen.Flow_', cask)
        self.assertIn('postflight_steps', cask)
        self.assertIn('depends_on arch: :arm64', cask)
        self.assertIn('depends_on macos: :sonoma', cask)
        subprocess.run(['bash', '-n', self.out / 'install.sh'], check=True)

    def test_wrong_tag_and_artifact_fail_before_output(self):
        for tag, dmg in [('v99.0.0', self.dmg), (f'v{self.version}-beta', self.dmg),
                         (f'v{self.version}', self.root / 'other.dmg')]:
            with self.subTest(tag=tag, dmg=dmg), self.assertRaises(ValueError):
                prepare(self.root, tag, dmg, self.out)
            self.assertFalse(self.out.exists())


if __name__ == '__main__':
    unittest.main()
