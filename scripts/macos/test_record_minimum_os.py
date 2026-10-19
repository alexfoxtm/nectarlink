# SPDX-License-Identifier: GPL-3.0-or-later
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('minimum_os', Path(__file__).with_name('record-minimum-os.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class MinimumOsTest(unittest.TestCase):
    def test_reads_all_architectures_and_legacy_deployment_versions(self):
        values = module.versions('app (architecture x86_64):\nminos 14.4\napp (architecture arm64):\nminos 27.0\napp (architecture arm64e):\ncmd LC_VERSION_MIN_MACOSX\ncmdsize 16\nversion 11.0\n')
        self.assertEqual(max(values), (27, 0, 0))
        self.assertEqual(values[0], (14, 4, 0))
        self.assertIn((11, 0, 0), values)

    def test_missing_deployment_metadata_is_refused(self):
        with self.assertRaises(ValueError):
            module.versions('sdk 27.0\nversion 27037.1\n')
        with self.assertRaisesRegex(ValueError, 'arm64'):
            module.versions('app (architecture x86_64):\nminos 14.4\napp (architecture arm64):\nsdk 27.0\n')


if __name__ == '__main__':
    unittest.main()
