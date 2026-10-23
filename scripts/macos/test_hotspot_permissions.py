# SPDX-License-Identifier: GPL-3.0-or-later
import importlib.util
from pathlib import Path
import plistlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('hotspot_permissions', Path(__file__).with_name('check-hotspot-permissions.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class HotspotPermissionsTest(unittest.TestCase):
    def test_finished_plist_accepts_permission_descriptions(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'Info.plist'
            path.write_bytes(plistlib.dumps({
                'NSLocationUsageDescription': 'Find the phone hotspot you choose.',
                'NSLocationWhenInUseUsageDescription': 'Find the phone hotspot you choose.',
            }))
            self.assertIsNone(module.validate(path))

    def test_finished_plist_rejects_missing_permission_description(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'Info.plist'
            path.write_bytes(plistlib.dumps({
                'NSLocationWhenInUseUsageDescription': 'Find the phone hotspot you choose.',
            }))
            with self.assertRaisesRegex(ValueError, 'NSLocationUsageDescription'):
                module.validate(path)


if __name__ == '__main__':
    unittest.main()
