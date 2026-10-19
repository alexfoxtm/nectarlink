# SPDX-License-Identifier: GPL-3.0-or-later
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile


spec = importlib.util.spec_from_file_location("prepare_adb", Path(__file__).with_name("prepare-adb.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PrepareAdbTest(unittest.TestCase):
    def fixture(self, root):
        archive = root / "fixture.zip"
        helper, notice = b"synthetic helper; never executed", b"synthetic Apache license notice"
        with zipfile.ZipFile(archive, "w") as output:
            output.writestr("platform-tools/adb", helper)
            output.writestr("platform-tools/NOTICE.txt", notice)
            output.writestr("platform-tools/fastboot", b"must not ship")
        pin = {"schema": 1, "member": "platform-tools/adb", "notice": "platform-tools/NOTICE.txt", "archiveBytes": archive.stat().st_size,
               "archiveSha256": hashlib.sha256(archive.read_bytes()).hexdigest(), "memberBytes": len(helper),
               "memberSha256": hashlib.sha256(helper).hexdigest(), "noticeSha256": hashlib.sha256(notice).hexdigest()}
        return archive, pin, helper

    def scratch(self):
        base = Path.home() / "orca" / "tmp"
        base.mkdir(parents=True, exist_ok=True)
        return tempfile.TemporaryDirectory(prefix="waypair-adb-fixture-", dir=base)

    def test_prepares_only_the_pinned_helper_and_notices(self):
        with self.scratch() as temporary:
            root = Path(temporary)
            archive, pin, helper = self.fixture(root)
            output = root / "prepared"
            module.prepare(archive, pin, output)
            self.assertEqual((output / "adb").read_bytes(), helper)
            self.assertTrue((output / "adb").stat().st_mode & 0o111)
            self.assertEqual({p.name for p in output.iterdir()}, {"adb", "ADB-NOTICE.txt", "origin.json"})

    def test_tampered_archive_produces_no_helper(self):
        with self.scratch() as temporary:
            root = Path(temporary)
            archive, pin, _ = self.fixture(root)
            pin["archiveSha256"] = "0" * 64
            output = root / "prepared"
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                module.prepare(archive, pin, output)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
