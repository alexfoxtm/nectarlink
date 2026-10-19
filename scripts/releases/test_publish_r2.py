# SPDX-License-Identifier: GPL-3.0-or-later
import runpy
import unittest
from pathlib import Path

publisher = runpy.run_path(str(Path(__file__).with_name("publish-r2.py")))


class PersonalBuildVersionTest(unittest.TestCase):
    def test_upstream_based_personal_version(self):
        self.assertEqual(publisher["version_of"]({"version": "0.1.4-local.2"}, "android"), "0.1.4-local.2")

    def test_path_component_in_version_is_refused(self):
        with self.assertRaises(publisher["Failure"]):
            publisher["version_of"]({"version": "0.1.4-local.2/.."}, "android")


if __name__ == "__main__":
    unittest.main()
