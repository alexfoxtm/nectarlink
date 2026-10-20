# SPDX-License-Identifier: GPL-3.0-or-later
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import plistlib
import shutil
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("prepare_notices", Path(__file__).with_name("prepare-notices.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

MACHO = b"\xcf\xfa\xed\xfe" + b"\0" * 28
QT_SBOM = """SPDXVersion: SPDX-2.3
PackageName: qtbase
PackageLicenseConcluded: NOASSERTION

PackageName: Core_Attribution_tinycbor
PackageVersion: 0.6.1
PackageLicenseConcluded: MIT
PackageCopyrightText: <text>Copyright (C) 2015 Intel
Corporation</text>

PackageName: Core_Attribution_sha1
PackageLicenseConcluded: LicenseRef-SHA1-Public-Domain
PackageCopyrightText: <text>Dominik Reichl</text>

PackageName: Core_Attribution_dual
PackageLicenseConcluded: LicenseRef-Qt-Commercial OR MIT
PackageCopyrightText: <text>The Qt Company</text>

LicenseID: LicenseRef-SHA1-Public-Domain
ExtractedText: <text>100% free public domain
implementation</text>

LicenseID: LicenseRef-Qt-Commercial
ExtractedText: <text>Qt commercial terms</text>
"""


def keg(cellar, name, version, *, sbom_license="MIT", license_file=None, qt_sbom=None, libs=()):
    root = cellar / name / version
    (root / ".brew").mkdir(parents=True)
    (root / ".brew" / f"{name}.rb").write_text(f"class F < Formula\n  url \"https://example.org/{name}.tar.gz\"\nend\n")
    (root / "INSTALL_RECEIPT.json").write_text(json.dumps({"poured_from_bottle": True, "source": {"tap": "homebrew/core"}}))
    digest = hashlib.sha256(name.encode()).hexdigest()
    sbom = {"packages": [
        {"SPDXID": f"SPDXRef-Archive-{name}-src", "versionInfo": version.split("_")[0], "licenseConcluded": sbom_license,
         "downloadLocation": f"https://example.org/{name}-{version}.tar.gz",
         "checksums": [{"algorithm": "SHA256", "checksumValue": digest}]},
        {"SPDXID": f"SPDXRef-Bottle-{name}", "downloadLocation": f"https://ghcr.io/v2/homebrew/core/{name}/blobs/x",
         "checksums": [{"algorithm": "SHA256", "checksumValue": digest}]},
    ]}
    (root / "sbom.spdx.json").write_text(json.dumps(sbom))
    if license_file:
        (root / license_file).write_text(f"{name} license\n")
    if qt_sbom:
        (root / "share/qt/sbom").mkdir(parents=True)
        (root / "share/qt/sbom" / f"{name}.spdx").write_text(qt_sbom)
    for lib in libs:
        target = root / "lib" / lib
        if lib.endswith(".framework"):
            target.mkdir(parents=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(MACHO)
    return root


class PrepareNoticesTest(unittest.TestCase):
    def scratch(self):
        base = Path.home() / "orca" / "tmp"
        base.mkdir(parents=True, exist_ok=True)
        return tempfile.TemporaryDirectory(prefix="waypair-notices-fixture-", dir=base)

    def setUp(self):
        self.saved = dict(module.SPARKLE)
        self.saved_slices = module.unsigned_slices
        module.unsigned_slices = lambda binary: dict(module.SPARKLE["unsignedSliceSha256"])

    def tearDown(self):
        module.SPARKLE.clear()
        module.SPARKLE.update(self.saved)
        module.unsigned_slices = self.saved_slices

    def inputs(self, root):
        cellar = root / "Cellar"
        kegs = [keg(cellar, "qtbase", "6.12.0_1", qt_sbom=QT_SBOM, libs=["QtCore.framework"]),
                keg(cellar, "md4c", "0.6.0", license_file="LICENSE.md", libs=["libmd4c.0.dylib"])]
        sparkle = root / "Sparkle.framework"
        (sparkle / "Resources").mkdir(parents=True)
        (sparkle / "Versions/B").mkdir(parents=True)
        (sparkle / "Resources/Info.plist").write_bytes(
            plistlib.dumps({"CFBundleShortVersionString": "2.10.0", "CFBundleVersion": "2064"}))
        (sparkle / "Versions/B/Sparkle").write_bytes(MACHO)
        license_file = root / "SPARKLE-LICENSE"
        license_file.write_text("Copyright (c) Sparkle\nMIT\n")
        module.SPARKLE["licenseSha256"] = hashlib.sha256(license_file.read_bytes()).hexdigest()
        rust = root / "rust.txt"
        rust.write_text(module.RUST_TITLE + "\n===\nRust crates (1 in the aarch64-apple-darwin build)\n")
        texts = root / "texts"
        texts.mkdir()
        (texts / "MIT.txt").write_text("MIT text\n")
        (texts / module.TEXTS_PROVENANCE).write_text(json.dumps({"commit": "c" * 40, "files": [
            {"id": "MIT", "sha256": hashlib.sha256(b"MIT text\n").hexdigest()}]}))
        repo = root / "repo"
        (repo / "LICENSES").mkdir(parents=True)
        for name in ("LGPL-3.0-only.txt", "GPL-3.0-or-later.txt", "MPL-2.0.txt"):
            (repo / "LICENSES" / name).write_text(name)
        return argparse.Namespace(
            rust_notices=rust, keg=kegs, license_texts=texts, sparkle_framework=sparkle,
            sparkle_license=license_file, adb_pin=None, source_commit="a" * 40,
            source_archive_url="https://example.org/waypair-src.tar.gz", source_archive_sha256="b" * 64,
            source_repository="https://example.org/waypair", output_dir=root / "notices", repo_root=repo)

    def app(self, root, notices, extra=()):
        contents = root / "Waypair Link.app" / "Contents"
        for relative in ["MacOS/nectarlink-desktop", "Frameworks/QtCore.framework/Versions/A/QtCore",
                         "Frameworks/libmd4c.0.dylib", "Frameworks/Sparkle.framework/Versions/B/Sparkle", *extra]:
            (contents / relative).parent.mkdir(parents=True, exist_ok=True)
            (contents / relative).write_bytes(MACHO)
        (contents / "Frameworks/Sparkle.framework/Resources").mkdir(parents=True)
        (contents / "Frameworks/Sparkle.framework/Resources/Info.plist").write_bytes(
            plistlib.dumps({"CFBundleShortVersionString": "2.10.0", "CFBundleVersion": "2064"}))
        (contents / "Resources/LICENSES").mkdir(parents=True)
        shutil.copytree(notices, contents / "Resources/LICENSES", dirs_exist_ok=True)
        return contents.parent

    def test_prepares_complete_notices_and_preserves_coverage(self):
        for case in [self.case_spdx_choice_prefers_an_available_alternative,
                     self.case_reads_multiline_tag_values,
                     self.case_prepares_verifies_and_covers_the_bundle]:
            with self.subTest(case=case.__name__):
                case()

    def test_invalid_or_incomplete_material_fails_closed(self):
        for case in [self.case_uncovered_bundled_library_fails,
                     self.case_altered_or_extra_notice_files_fail_verification,
                     self.case_missing_inputs_produce_no_notices,
                     self.case_windows_notices_are_refused,
                     self.case_unresolved_unpinned_or_mismatched_material_fails]:
            with self.subTest(case=case.__name__):
                case()

    def case_spdx_choice_prefers_an_available_alternative(self):
        self.assertEqual(module.texts_needed("FTL OR GPL-2.0-only", {"GPL-2.0-only"}), ["GPL-2.0-only"])
        self.assertEqual(module.texts_needed("(MIT OR Apache-2.0) AND Zlib", {"Apache-2.0", "Zlib"}),
                         ["Apache-2.0", "Zlib"])
        self.assertEqual(module.texts_needed("GPL-2.0-only WITH Linux-syscall-note", {"GPL-2.0-only", "Linux-syscall-note"}),
                         ["GPL-2.0-only", "Linux-syscall-note"])
        with self.assertRaisesRegex(ValueError, "no license text"):
            module.texts_needed("X11 AND HPND", {"X11"})
        with self.assertRaisesRegex(ValueError, "malformed"):
            module.parse_spdx("MIT OR")

    def case_reads_multiline_tag_values(self):
        with self.scratch() as temporary:
            path = Path(temporary) / "q.spdx"
            path.write_text(QT_SBOM)
            packages, extracted = module.read_tag_value(path)
        self.assertEqual(packages[1]["PackageCopyrightText"], "Copyright (C) 2015 Intel\nCorporation")
        self.assertEqual(extracted["LicenseRef-SHA1-Public-Domain"], "100% free public domain\nimplementation")

    def case_prepares_verifies_and_covers_the_bundle(self):
        with self.scratch() as temporary:
            root = Path(temporary)
            args = self.inputs(root)
            module.prepare(args)
            manifest = module.verify(args.output_dir)
            self.assertEqual(manifest["covers"]["libmd4c.0.dylib"], "md4c 0.6.0 (Homebrew)")
            qt = (args.output_dir / "QT-NOTICES.txt").read_text()
            self.assertIn("Core: tinycbor 0.6.1  ·  MIT", qt)
            self.assertIn("100% free public domain", qt)
            self.assertIn("Core: dual  ·  LicenseRef-Qt-Commercial OR MIT", qt)
            self.assertNotIn("Qt commercial terms", qt)
            self.assertIn("a" * 40, (args.output_dir / "SOURCE-OFFER.txt").read_text())
            sources = (args.output_dir / "THIRD-PARTY-SOURCES.txt").read_text()
            self.assertIn("md4c 0.6.0 (MIT)\n  License: third-party/homebrew/md4c-0.6.0/LICENSE.md\n"
                          "  License texts: third-party/licenses/MIT.txt", sources)
            self.assertEqual((args.output_dir / "third-party/licenses/MIT.txt").read_text(), "MIT text\n")
            provenance = json.loads((args.output_dir / "third-party/PROVENANCE.json").read_text())
            self.assertEqual(provenance["covers"], manifest["covers"])
            self.assertEqual(provenance["licenseTexts"], {"commit": "c" * 40, "files": {
                "MIT": hashlib.sha256(b"MIT text\n").hexdigest()}})
            app = self.app(root, args.output_dir, extra=["Helpers/WaypairUpdater.app/Contents/MacOS/WaypairUpdater"])
            result = module.check_bundle(args.output_dir, app)
            self.assertEqual(result["machO"], 5)
            listing = (app / "Contents/Resources/LICENSES" / module.BUNDLED_LIST).read_text()
            self.assertIn("Frameworks/libmd4c.0.dylib\n  md4c 0.6.0 (Homebrew)", listing)
            self.assertIn("Helpers/WaypairUpdater.app/Contents/MacOS/WaypairUpdater\n  Waypair Link", listing)

    def case_uncovered_bundled_library_fails(self):
        with self.scratch() as temporary:
            root = Path(temporary)
            args = self.inputs(root)
            module.prepare(args)
            app = self.app(root, args.output_dir, extra=["Frameworks/libunlisted.1.dylib",
                                                         "Helpers/WaypairUpdater.app/Contents/Frameworks/libinner.dylib"])
            (app / "Contents/Frameworks/libfat64.dylib").write_bytes(b"\xca\xfe\xba\xbf" + b"\0" * 28)
            with self.assertRaisesRegex(ValueError, "libfat64.dylib, libinner.dylib, libunlisted.1.dylib"):
                module.check_bundle(args.output_dir, app)
            self.assertFalse((app / "Contents/Resources/LICENSES" / module.BUNDLED_LIST).exists())

    def case_altered_or_extra_notice_files_fail_verification(self):
        with self.scratch() as temporary:
            root = Path(temporary)
            args = self.inputs(root)
            module.prepare(args)
            (args.output_dir / "QT-NOTICES.txt").write_text("edited")
            with self.assertRaisesRegex(ValueError, "changed"):
                module.verify(args.output_dir)
        with self.scratch() as temporary:
            root = Path(temporary)
            args = self.inputs(root)
            module.prepare(args)
            (args.output_dir / "stray.txt").write_text("x")
            with self.assertRaisesRegex(ValueError, "unlisted"):
                module.verify(args.output_dir)

    def case_missing_inputs_produce_no_notices(self):
        with self.scratch() as temporary:
            root = Path(temporary)
            args = self.inputs(root)
            (args.license_texts / "MIT.txt").unlink()
            with self.assertRaisesRegex(ValueError, "MIT"):
                module.prepare(args)
            self.assertFalse(args.output_dir.exists())
            args = self.inputs(Path(temporary) / "again")
            (args.keg[1] / "LICENSE.md").unlink()
            with self.assertRaisesRegex(ValueError, "no license file"):
                module.prepare(args)
            args.source_commit = "abc"
            with self.assertRaisesRegex(ValueError, "40-character"):
                module.prepare(args)
            self.assertFalse(args.output_dir.exists())

    def case_windows_notices_are_refused(self):
        with self.scratch() as temporary:
            args = self.inputs(Path(temporary))
            args.rust_notices.write_text("NectarLink for Windows: third-party notices\n")
            with self.assertRaisesRegex(ValueError, "macOS output"):
                module.prepare(args)

    def case_unresolved_unpinned_or_mismatched_material_fails(self):
        with self.scratch() as temporary:
            args = self.inputs(Path(temporary) / "declared")
            sbom_path = args.keg[1] / "sbom.spdx.json"
            sbom = json.loads(sbom_path.read_text())
            sbom["packages"][0]["licenseConcluded"] = "GPL-3.0-or-later AND LGPL-2.1-or-later"
            sbom_path.write_text(json.dumps(sbom))
            with self.assertRaisesRegex(ValueError, "md4c 0.6.0: GPL-3.0-or-later AND LGPL-2.1-or-later"):
                module.prepare(args)
            args = self.inputs(Path(temporary) / "unpinned")
            (args.license_texts / "MIT.txt").write_text("edited MIT\n")
            with self.assertRaisesRegex(ValueError, "pinned SHA-256"):
                module.prepare(args)
            self.assertFalse(args.output_dir.exists())

            root = Path(temporary) / "bundle"
            args = self.inputs(root)
            module.prepare(args)
            manifest_path = args.output_dir / module.MANIFEST
            manifest = json.loads(manifest_path.read_text())
            manifest_path.write_text(json.dumps({**manifest, "covers": {**manifest["covers"], "libx.dylib": "x"}}))
            with self.assertRaisesRegex(ValueError, "covers differ"):
                module.verify(args.output_dir)
            manifest_path.write_text(json.dumps(manifest))
            app = self.app(root, args.output_dir)
            plist = app / "Contents/Frameworks/Sparkle.framework/Resources/Info.plist"
            plist.write_bytes(plistlib.dumps({"CFBundleShortVersionString": "2.10.0", "CFBundleVersion": "2063"}))
            with self.assertRaisesRegex(ValueError, "not the prepared Sparkle 2.10.0 \\(2064\\)"):
                module.check_bundle(args.output_dir, app)
            plist.write_bytes(plistlib.dumps({"CFBundleShortVersionString": "2.10.0", "CFBundleVersion": "2064"}))
            module.unsigned_slices = lambda binary: {"arm64": "0" * 64, "x86_64": "0" * 64}
            with self.assertRaisesRegex(ValueError, "beyond its signature"):
                module.check_bundle(args.output_dir, app)
            self.assertFalse((app / "Contents/Resources/LICENSES" / module.BUNDLED_LIST).exists())


if __name__ == "__main__":
    unittest.main()
