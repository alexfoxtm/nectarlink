# SPDX-License-Identifier: GPL-3.0-or-later
import hashlib
import json
import runpy
import tempfile
import unittest
from pathlib import Path

publisher = runpy.run_path(str(Path(__file__).with_name("publish-r2.py")))


class PersonalBuildVersionTest(unittest.TestCase):
    def test_upstream_based_personal_version(self):
        self.assertEqual(publisher["version_of"]({"version": "0.1.4-local.2"}, "android"), "0.1.4-local.2")

    def test_path_component_in_version_is_refused(self):
        with self.assertRaises(publisher["Failure"]):
            publisher["version_of"]({"version": "0.1.4-local.2/.."}, "android")


class ReleaseNotesTest(unittest.TestCase):
    def test_android_notes_upload_and_missing_notes_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            apk_path = base / "Waypair-Link-0.47.0-android.apk"
            apk_path.write_bytes(b"fake-apk-bytes")
            src_path = base / "waypair-link-0.47.0-source.tar.gz"
            src_path.write_bytes(b"fake-source-bytes")
            notes_path = base / "CHANGELOG.txt"
            notes_path.write_bytes("Waypair Link 0.47.0\n\n- Fix A\n".encode("utf-8"))

            def entry(p):
                return {"path": str(p), "sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
                        "size": p.stat().st_size}

            version = "0.47.0"
            a_prefix = f"waypair/nectar/android/releases/v{version}/"
            section = {"version": version, "apk": entry(apk_path), "source": entry(src_path),
                       "release_notes": entry(notes_path)}
            immutable, manifests = publisher["plan_android"](section, base)
            self.assertEqual([u.key for u in immutable],
                             [a_prefix + apk_path.name, a_prefix + src_path.name,
                              a_prefix + "CHANGELOG.txt", a_prefix + "SHA256SUMS.txt"])
            self.assertFalse(any(u.mutable for u in immutable))
            self.assertEqual(immutable[2].content_type, "text/plain; charset=utf-8")
            sums_text = immutable[3].data.decode("utf-8")
            self.assertIn("CHANGELOG.txt", sums_text)
            self.assertIn(apk_path.name, sums_text)
            feed = json.loads(manifests[0].data.decode("utf-8"))
            self.assertEqual(feed["html_url"], publisher["public_url"](a_prefix + "CHANGELOG.txt"))
            self.assertEqual(manifests[0].key, publisher["ANDROID_MANIFEST_KEY"])

            with self.assertRaises(publisher["Failure"]):
                publisher["plan_android"](
                    {"version": version, "apk": entry(apk_path), "source": entry(src_path)}, base)

            dmg_path = base / "Waypair-Link-0.47.0.dmg"
            dmg_path.write_bytes(b"fake-dmg-bytes")
            msrc_path = base / "waypair-link-0.47.0-macos-source.tar.gz"
            msrc_path.write_bytes(b"fake-macos-source-bytes")
            dmg_url = publisher["public_url"](f"waypair/nectar/macos/releases/v{version}/{dmg_path.name}")
            appcast_path = base / "appcast.xml"
            appcast_path.write_text(
                f'<?xml version="1.0"?><rss><channel><item>'
                f'<enclosure url="{dmg_url}" length="{dmg_path.stat().st_size}" '
                f'sparkle:edSignature="AAA="/></item></channel></rss>', encoding="utf-8")
            m_prefix = f"waypair/nectar/macos/releases/v{version}/"
            m_immutable, m_manifests = publisher["plan_macos"](
                {"version": version, "dmg": entry(dmg_path), "source": entry(msrc_path),
                 "appcast": entry(appcast_path), "release_notes": entry(notes_path)}, base)
            self.assertEqual([u.key for u in m_immutable],
                             [m_prefix + dmg_path.name, m_prefix + msrc_path.name,
                              m_prefix + "CHANGELOG.txt", m_prefix + "SHA256SUMS.txt"])
            self.assertIn("CHANGELOG.txt", m_immutable[3].data.decode("utf-8"))
            self.assertEqual(m_manifests[0].key, publisher["MACOS_APPCAST_KEY"])
            self.assertEqual(m_manifests[0].body(), appcast_path.read_bytes())
            with self.assertRaises(publisher["Failure"]):
                publisher["plan_macos"](
                    {"version": version, "dmg": entry(dmg_path), "source": entry(msrc_path),
                     "appcast": entry(appcast_path)}, base)

    def test_release_notes_malformed_and_oversize_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            apk_path = base / "Waypair-Link-0.47.0-android.apk"
            apk_path.write_bytes(b"fake-apk-bytes")
            src_path = base / "waypair-link-0.47.0-source.tar.gz"
            src_path.write_bytes(b"fake-source-bytes")

            def entry(p):
                return {"path": str(p), "sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
                        "size": p.stat().st_size}

            def section_for(notes_entry):
                return {"version": "0.47.0", "apk": entry(apk_path),
                        "source": entry(src_path), "release_notes": notes_entry}

            wrong = base / "NOTES.txt"
            wrong.write_text("notes\n", encoding="utf-8")
            with self.assertRaises(publisher["Failure"]):
                publisher["plan_android"](section_for(entry(wrong)), base)
            bad_dir = base / "bad"
            bad_dir.mkdir()
            bad = bad_dir / "CHANGELOG.txt"
            bad.write_bytes(b"\xff\xfe\x00bad")
            with self.assertRaises(publisher["Failure"]):
                publisher["plan_android"](section_for(entry(bad)), base)
            blank = base / "blank" / "CHANGELOG.txt"
            blank.parent.mkdir(parents=True)
            blank.write_text("   \n  ", encoding="utf-8")
            with self.assertRaises(publisher["Failure"]):
                publisher["plan_android"](section_for(entry(blank)), base)
            big = base / "big" / "CHANGELOG.txt"
            big.parent.mkdir(parents=True)
            big.write_bytes(b"x" * (publisher["NOTES_MAX_BYTES"] + 1))
            with self.assertRaises(publisher["Failure"]):
                publisher["plan_android"](section_for(entry(big)), base)
            notes_path = base / "CHANGELOG.txt"
            notes_path.write_text("ok\n", encoding="utf-8")
            for heading in ("Waypair Link 0.46.0", "Waypair Link 0.47.0-local.1", "Waypair Link 0.47.01", "\nWaypair Link 0.47.0"):
                notes_path.write_text(heading + "\n\n- A change\n", encoding="utf-8")
                with self.subTest(heading=heading), self.assertRaises(publisher["Failure"]):
                    publisher["plan_android"](section_for(entry(notes_path)), base)
            notes_path.write_text("Waypair Link 0.47.0\n\n- A change\n", encoding="utf-8")
            bad_hash = entry(notes_path)
            bad_hash["sha256"] = "0" * 64
            with self.assertRaises(publisher["Failure"]):
                publisher["plan_android"](section_for(bad_hash), base)


if __name__ == "__main__":
    unittest.main()
