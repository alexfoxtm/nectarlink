#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline assembly and checking of the macOS bundle's third-party notices.

prepare       builds a notices folder from explicit local inputs: the macOS
              `cargo xtask notices` output, Homebrew kegs, Sparkle, the ADB pin
              and the fork's source archive. Nothing is downloaded or built.
verify        checks a prepared folder against its manifest.
check-bundle  fails when a Mach-O in an assembled .app has no notice, then
              lists every bundled component in Resources/LICENSES.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile

MANIFEST = "third-party/notices-manifest.json"
REQUIRED = ("THIRD-PARTY-NOTICES.txt", "QT-NOTICES.txt", "THIRD-PARTY-SOURCES.txt", "SOURCE-OFFER.txt",
            "third-party/PROVENANCE.json")
RUST_TITLE = "Waypair Link for macOS: third-party notices"
BUNDLED_LIST = "BUNDLED-COMPONENTS.txt"
MACHO = {b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xfe\xed\xfa\xce", b"\xca\xfe\xba\xbe",
         b"\xbe\xba\xfe\xca", b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca"}
# Waypair's own executables; every other Mach-O needs a notice.
OWN_CODE = {("MacOS", "nectarlink-desktop"), ("Helpers", "WaypairUpdater.app", "Contents", "MacOS", "WaypairUpdater"),
            ("Library", "SystemExtensions", "com.waypair.mac.CameraExtension.systemextension", "Contents", "MacOS", "WaypairCameraExtension")}
LICENSE_NAME = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)|^[\w.+-]*GPL[\w.+-]*\.txt$", re.IGNORECASE)
SPDX_ID = re.compile(r"^[A-Za-z0-9.+-]+$")
# Waypair holds no commercial Qt license, so that choice is never taken.
UNSELECTABLE = {"LicenseRef-Qt-Commercial"}
TEXTS_PROVENANCE = "PROVENANCE.json"
# Texts the repository's LICENSES/ already ships beside these notices.
BESIDE = {"LGPL-3.0-only": "LGPL-3.0-only.txt", "LGPL-3.0-or-later": "LGPL-3.0-only.txt",
          "GPL-3.0-only": "GPL-3.0-or-later.txt", "GPL-3.0-or-later": "GPL-3.0-or-later.txt",
          "MPL-2.0": "MPL-2.0.txt", "CC-BY-4.0": "CC-BY-4.0.txt"}

# Sparkle 2.10.0, checked on this Mac: tag 2.10.0 of the official repository
# at the commit below, whose Package.swift pins the release artifact. With
# signatures removed, both slices of that artifact's Sparkle binary have
# these hashes, as did the Root-signed copy shipped in build 32.
SPARKLE = {
    "version": "2.10.0",
    "build": "2064",
    "repository": "https://github.com/sparkle-project/Sparkle",
    "tag": "2.10.0",
    "commit": "eef1a539a373c1f1a320624b1130fc5de7b2e100",
    "artifact": "https://github.com/sparkle-project/Sparkle/releases/download/2.10.0/Sparkle-for-Swift-Package-Manager.zip",
    "artifactSha256": "17e28312b8e18ab7cdbbe09a6fb28cc55a5479ec6c371dbc07cdecd2a14fd959",
    "licenseSha256": "389a4e4e9a32f059775b13a06e25a591445ba229d2838d26dd3e7c0c45127cfe",
    "unsignedSliceSha256": {
        "arm64": "3a684bd908fce13d28f952badeec82910f0d03d1bef0c4c6ab345c6b1891f417",
        "x86_64": "b8ec2baf9a63b102ae756041ea641f91d58d049710491cb58482bbe62534f64e",
    },
}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def regular_file(path: Path, what: str) -> Path:
    if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
        raise ValueError(f"{what} must be a non-empty regular file: {path}")
    return path


def clean_url(url: str, what: str) -> str:
    if not re.fullmatch(r"https://[^\s<>\"'\x00-\x1f\x7f]+", url or ""):
        raise ValueError(f"{what} must be a clean https URL: {url!r}")
    return url


# --- SPDX expressions ---------------------------------------------------------

def parse_spdx(expression: str):
    """('or'|'and', [nodes]) or ('id', [ids]); WITH keeps both identifiers."""
    tokens = re.findall(r"\(|\)|[^\s()]+", expression)
    position = 0

    def take():
        nonlocal position
        position += 1
        return tokens[position - 1]

    def peek():
        return tokens[position] if position < len(tokens) else None

    def parse_or():
        nodes = [parse_and()]
        while peek() == "OR":
            take()
            nodes.append(parse_and())
        return nodes[0] if len(nodes) == 1 else ("or", nodes)

    def parse_and():
        nodes = [parse_atom()]
        while peek() == "AND":
            take()
            nodes.append(parse_atom())
        return nodes[0] if len(nodes) == 1 else ("and", nodes)

    def parse_atom():
        token = peek()
        if token is None or token in ("OR", "AND", "WITH", ")"):
            raise ValueError(f"malformed SPDX expression: {expression!r}")
        if take() == "(":
            node = parse_or()
            if peek() != ")":
                raise ValueError(f"malformed SPDX expression: {expression!r}")
            take()
            return node
        ids = [token]
        if peek() == "WITH":
            take()
            ids.append(take() if peek() not in (None, "(", ")") else "")
        if not all(SPDX_ID.fullmatch(i) for i in ids):
            raise ValueError(f"malformed SPDX expression: {expression!r}")
        return ("id", ids)

    node = parse_or()
    if position != len(tokens):
        raise ValueError(f"malformed SPDX expression: {expression!r}")
    return node


def texts_needed(expression: str, available) -> list:
    """The identifiers whose texts satisfy the expression: the first OR choice
    that is fully available, every AND part. Raises when none is."""
    def solve(node):
        kind, parts = node
        if kind == "id":
            return parts if all(i in available for i in parts) else None
        if kind == "and":
            chosen = [solve(p) for p in parts]
            return None if None in chosen else [i for c in chosen for i in c]
        for part in parts:
            chosen = solve(part)
            if chosen is not None:
                return chosen
        return None

    chosen = solve(parse_spdx(expression))
    if chosen is None:
        raise ValueError(f"no license text for {expression!r}; supply its SPDX texts with --license-texts")
    return list(dict.fromkeys(chosen))


def pinned_texts(texts: Path) -> dict:
    """The SPDX texts `texts/PROVENANCE.json` pins (license-list-data commit, id -> SHA-256), each checked.
    Only pinned texts can satisfy a license."""
    record = json.loads(regular_file(texts / TEXTS_PROVENANCE, "license texts PROVENANCE.json").read_text())
    if not re.fullmatch(r"[0-9a-f]{40}", str(record.get("commit", ""))):
        raise ValueError("license texts PROVENANCE.json must name the license-list-data commit")
    files = {}
    for entry in record.get("files") or []:
        license_id, digest = str(entry.get("id", "")), str(entry.get("sha256", ""))
        if not SPDX_ID.fullmatch(license_id) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ValueError(f"invalid license texts PROVENANCE.json entry: {entry}")
        if sha256(regular_file(texts / f"{license_id}.txt", f"{license_id}.txt")) != digest:
            raise ValueError(f"{license_id}.txt differs from its pinned SHA-256")
        files[license_id] = digest
    if not files:
        raise ValueError("license texts PROVENANCE.json pins no texts")
    return {"commit": record["commit"], "files": files}


def text_reference(license_id: str, beside: set) -> str:
    return f"{BESIDE[license_id]} in this folder" if license_id in beside else f"third-party/licenses/{license_id}.txt"


# --- Qt SBOMs (SPDX tag-value) ------------------------------------------------

def read_tag_value(path: Path):
    """Packages and extracted LicenseRef texts of a Qt SPDX tag-value SBOM."""
    packages, extracted, current, license_id = [], {}, None, None
    lines = path.read_text().splitlines()
    index = 0
    while index < len(lines):
        key, _, value = lines[index].partition(": ")
        index += 1
        if value.startswith("<text>"):
            value = value[len("<text>"):]
            while "</text>" not in value and index < len(lines):
                value += "\n" + lines[index]
                index += 1
            value = value.split("</text>", 1)[0]
        if key == "PackageName":
            current = {"name": value}
            packages.append(current)
        elif key == "LicenseID":
            license_id = value
        elif key == "ExtractedText" and license_id:
            extracted[license_id] = value
        elif current is not None and key in ("PackageVersion", "PackageLicenseConcluded", "PackageCopyrightText",
                                              "PackageDownloadLocation"):
            current[key] = value
    return packages, extracted


# --- Homebrew kegs ------------------------------------------------------------

def keg_covers(keg: Path) -> set:
    names = set()
    for directory, subdirs, files in os.walk(keg):
        for sub in list(subdirs):
            if sub.endswith(".framework"):
                names.add(sub)
                subdirs.remove(sub)
        names.update(f for f in files if f.endswith(".dylib"))
    return names


def read_keg(keg: Path) -> dict:
    keg = keg.resolve(strict=True)
    if not keg.is_dir() or keg.parent.parent.name != "Cellar":
        raise ValueError(f"keg must resolve to a Cellar/<formula>/<version> directory: {keg}")
    name, version = keg.parent.name, keg.name
    receipt = json.loads(regular_file(keg / "INSTALL_RECEIPT.json", "keg receipt").read_text())
    source = receipt.get("source") or {}
    if source.get("tap") != "homebrew/core":
        raise ValueError(f"{name} {version} is not from homebrew/core")
    sbom_path = regular_file(keg / "sbom.spdx.json", "keg SBOM")
    sbom = {p["SPDXID"]: p for p in json.loads(sbom_path.read_text()).get("packages", [])}
    archive = sbom.get(f"SPDXRef-Archive-{name}-src")
    if not archive or archive.get("versionInfo") != re.sub(r"_\d+$", "", version):
        raise ValueError(f"{name} {version}: the keg SBOM does not record its source archive")

    def located(package):
        checksum = [c["checksumValue"] for c in package.get("checksums", []) if c.get("algorithm") == "SHA256"]
        if len(checksum) != 1 or not re.fullmatch(r"[0-9a-f]{64}", checksum[0]):
            raise ValueError(f"{name} {version}: {package['SPDXID']} has no SHA-256")
        return {"url": clean_url(package.get("downloadLocation"), f"{name} source"), "sha256": checksum[0]}

    patches = [located(p) for i, p in sorted(sbom.items()) if i.startswith(f"SPDXRef-Patch-{name}-")]
    bottle = sbom.get(f"SPDXRef-Bottle-{name}")
    formula = regular_file(keg / ".brew" / f"{name}.rb", "keg formula")
    licenses = sorted(p for p in keg.iterdir() if p.is_file() and not p.is_symlink() and LICENSE_NAME.search(p.name))
    qt_sboms = sorted((keg / "share/qt/sbom").glob("*.spdx"))
    if not licenses and not qt_sboms:
        raise ValueError(f"{name} {version}: no license file in the keg")
    covers = keg_covers(keg)
    if not covers:
        raise ValueError(f"{name} {version}: the keg holds no framework or dylib")
    return {
        "name": name, "version": version, "path": keg, "license": archive.get("licenseConcluded", "NOASSERTION"),
        "source": located(archive), "patches": patches, "bottle": located(bottle) if bottle else None,
        "pouredFromBottle": bool(receipt.get("poured_from_bottle")), "formula": formula,
        "licenses": licenses, "qtSboms": qt_sboms, "covers": covers,
    }


# --- Sparkle --------------------------------------------------------------------

def unsigned_slices(binary: Path) -> dict:
    hashes = {}
    with tempfile.TemporaryDirectory(prefix="waypair-sparkle-") as scratch:
        for arch in SPARKLE["unsignedSliceSha256"]:
            thin = Path(scratch) / arch
            subprocess.run(["lipo", str(binary), "-thin", arch, "-output", str(thin)], check=True,
                           capture_output=True)
            subprocess.run(["codesign", "--remove-signature", str(thin)], check=True, capture_output=True)
            hashes[arch] = sha256(thin)
    return hashes


def check_sparkle(framework: Path, license_file: Path) -> dict:
    if framework.is_symlink() or not framework.is_dir() or framework.name != "Sparkle.framework":
        raise ValueError(f"not a Sparkle.framework directory: {framework}")
    info = plistlib.loads(regular_file(framework / "Resources/Info.plist", "Sparkle Info.plist").read_bytes())
    if (info.get("CFBundleShortVersionString"), info.get("CFBundleVersion")) != (SPARKLE["version"], SPARKLE["build"]):
        raise ValueError("Sparkle.framework is not the checked 2.10.0 (2064) release")
    if sha256(regular_file(license_file, "Sparkle LICENSE")) != SPARKLE["licenseSha256"]:
        raise ValueError("Sparkle LICENSE is not the one at tag 2.10.0")
    binary = framework / "Versions/B/Sparkle"
    if unsigned_slices(regular_file(binary, "Sparkle binary")) != SPARKLE["unsignedSliceSha256"]:
        raise ValueError("Sparkle binary differs from the 2.10.0 release artifact beyond its signature")
    return {**SPARKLE, "frameworkBinarySha256": sha256(binary),
            "checked": "main binary slices without signatures; helper executables inside the framework not compared"}


# --- prepare ------------------------------------------------------------------

def source_lines(component: dict) -> list:
    lines = [f"  Source: {component['source']['url']}", f"    SHA-256 {component['source']['sha256']}"]
    for patch in component["patches"]:
        lines += [f"  Homebrew patch: {patch['url']}", f"    SHA-256 {patch['sha256']}"]
    lines.append(f"  Homebrew formula as built: third-party/homebrew/{component['name']}-{component['version']}/"
                 f"{component['formula'].name}")
    if component["bottle"]:
        lines += [f"  Homebrew bottle: {component['bottle']['url']}", f"    SHA-256 {component['bottle']['sha256']}"]
    else:
        lines.append("  Homebrew bottle: not recorded in the keg SBOM")
    return lines


def qt_notices(qt: list, texts: Path, available: set, beside: set):
    """QT-NOTICES.txt, and the pinned texts it includes."""
    version = sorted({re.sub(r"_\d+$", "", k["version"]) for k in qt})
    if len(version) != 1:
        raise ValueError(f"Qt kegs disagree on the Qt version: {version}")
    out = ["Qt in Waypair Link for macOS", "=" * 28, "",
           f"Waypair Link uses Qt {version[0]}, deployed from the Homebrew (homebrew/core) packages",
           "  " + ", ".join(f"{k['name']} {k['version']}" for k in qt) + ",",
           "under the GNU Lesser General Public License version 3 (LGPL-3.0-only.txt in this",
           "folder, which builds on GPL-3.0-or-later.txt). The Qt libraries are the separate",
           "Qt*.framework bundles in Contents/Frameworks and the plugins in Contents/PlugIns;",
           f"{BUNDLED_LIST} in this folder lists exactly which ones this build carries.", "",
           "Corresponding source", "--------------------"]
    for keg in qt:
        out += ["", f"{keg['name']} {keg['version']} ({keg['license']})", *source_lines(keg)]
    out += ["", "Qt's own repositories: https://code.qt.io", "",
            "Third-party code inside Qt", "--------------------------",
            "Qt's SBOMs (third-party/qt/*.spdx, verbatim) declare the code below inside these",
            "modules. The list covers each whole module; some entries may not be compiled into",
            "this macOS build.", ""]
    extracted, needed, unmet = {}, [], set()
    for keg in qt:
        for sbom in keg["qtSboms"]:
            packages, refs = read_tag_value(sbom)
            extracted.update(refs)
            for package in packages:
                if "_Attribution_" not in package["name"]:
                    continue
                expression = package.get("PackageLicenseConcluded", "NOASSERTION")
                try:
                    needed += texts_needed(expression, available | (set(refs) - UNSELECTABLE))
                except ValueError:
                    unmet.add(expression)
                module, _, what = package["name"].partition("_Attribution_")
                out.append(f"{module}: {what} {package.get('PackageVersion', '')}".rstrip() + f"  ·  {expression}")
                out += [f"  {line}" for line in package.get("PackageCopyrightText", "NOASSERTION").splitlines()]
    if unmet:
        raise ValueError(f"--license-texts {texts} has no pinned <SPDX id>.txt satisfying: {'; '.join(sorted(unmet))}")
    out += ["", "Licenses named above", "--------------------"]
    used = []
    for license_id in dict.fromkeys(needed):
        if license_id in beside and license_id not in extracted:
            out.append(f"\n--- {license_id}: {BESIDE[license_id]} in this folder ---")
        elif license_id.startswith("LicenseRef-") and license_id in extracted:
            out += [f"\n--- {license_id} (from Qt's SBOM) ---", extracted[license_id].rstrip()]
        else:
            body = regular_file(texts / f"{license_id}.txt", license_id).read_text()
            out += [f"\n--- {license_id} ---", body.rstrip()]
            used.append(license_id)
    return "\n".join(out) + "\n", used


def prepare(args) -> dict:
    output = args.output_dir
    if output.exists() or output.is_symlink():
        raise ValueError(f"output exists: {output}")
    rust = regular_file(args.rust_notices, "Rust notices")
    if not rust.read_text().startswith(RUST_TITLE + "\n") or "apple-darwin build)" not in rust.read_text():
        raise ValueError("Rust notices are not the macOS output of `cargo xtask notices --target <apple target>`")
    texts = args.license_texts
    if texts.is_symlink() or not texts.is_dir():
        raise ValueError(f"--license-texts must be a directory of <SPDX id>.txt files: {texts}")
    pinned = pinned_texts(texts)
    licenses_dir = args.repo_root / "LICENSES"
    beside = {i for i, f in BESIDE.items() if (licenses_dir / f).is_file()}
    available = (set(pinned["files"]) | beside) - UNSELECTABLE
    if not re.fullmatch(r"[0-9a-f]{40}", args.source_commit or ""):
        raise ValueError("--source-commit must be the full 40-character commit")
    if not re.fullmatch(r"[0-9a-f]{64}", args.source_archive_sha256 or ""):
        raise ValueError("--source-archive-sha256 must be 64 lowercase hex characters")
    archive_url = clean_url(args.source_archive_url, "--source-archive-url")
    repository = clean_url(args.source_repository, "--source-repository") if args.source_repository else None

    kegs = [read_keg(k) for k in args.keg]
    names = [k["name"] for k in kegs]
    if len(set(names)) != len(names):
        raise ValueError("a formula is given twice")
    qt = [k for k in kegs if k["qtSboms"]]
    if "qtbase" not in [k["name"] for k in qt]:
        raise ValueError("the qtbase keg is required")
    # Each keg's declared license must resolve to texts; its own files keep the copyright lines.
    unmet = []
    for keg in kegs:
        try:
            keg["texts"] = texts_needed(keg["license"], available)
        except ValueError:
            unmet.append(f"{keg['name']} {keg['version']}: {keg['license']}")
    if unmet:
        raise ValueError(f"--license-texts {texts} has no pinned <SPDX id>.txt satisfying: {'; '.join(unmet)}")
    sparkle = check_sparkle(args.sparkle_framework, args.sparkle_license)
    adb = json.loads(regular_file(args.adb_pin, "ADB pin").read_text()) if args.adb_pin else None
    if adb is not None and (adb.get("schema") != 1 or not all(
            re.fullmatch(r"[0-9a-f]{64}", str(adb.get(k, ""))) for k in ("archiveSha256", "noticeSha256"))):
        raise ValueError("invalid ADB pin")

    covers = {}
    for keg in kegs:
        for name in sorted(keg["covers"]):
            covers.setdefault(name, f"{keg['name']} {keg['version']} (Homebrew)")
    covers["Sparkle.framework"] = f"Sparkle {SPARKLE['version']}"
    if adb is not None:
        covers["adb"] = f"Android platform-tools adb {adb.get('version')}"

    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".notices-", dir=output.parent) as scratch:
        stage = Path(scratch) / "stage"
        (stage / "third-party/qt").mkdir(parents=True)
        shutil.copyfile(rust, stage / "THIRD-PARTY-NOTICES.txt")
        qt_text, qt_used = qt_notices(qt, texts, available, beside)
        (stage / "QT-NOTICES.txt").write_text(qt_text)
        keg_used = [i for keg in kegs for i in keg["texts"] if i not in beside]
        (stage / "third-party/licenses").mkdir()
        for license_id in dict.fromkeys(keg_used):
            shutil.copyfile(texts / f"{license_id}.txt", stage / "third-party/licenses" / f"{license_id}.txt")
        for keg in kegs:
            folder = stage / "third-party/homebrew" / f"{keg['name']}-{keg['version']}"
            folder.mkdir(parents=True)
            for path in [*keg["licenses"], keg["formula"], keg["path"] / "sbom.spdx.json"]:
                shutil.copyfile(path, folder / path.name)
            for sbom in keg["qtSboms"]:
                shutil.copyfile(sbom, stage / "third-party/qt" / sbom.name)
        (stage / "third-party/sparkle").mkdir()
        shutil.copyfile(args.sparkle_license, stage / "third-party/sparkle/LICENSE")

        sources = ["Sources of the third-party components in Waypair Link for macOS", "=" * 64, "",
                   "Every bundled framework, library and helper is listed with the exact version",
                   "shipped and where its source is. Nothing here was downloaded while packaging:",
                   "the records come from the Homebrew kegs the app was deployed from, the checked",
                   "Sparkle release and the pinned ADB archive."]
        for keg in kegs:
            folder = f"third-party/homebrew/{keg['name']}-{keg['version']}"
            licensed = ", ".join(f"{folder}/{p.name}" for p in keg["licenses"])
            sources += ["", f"{keg['name']} {keg['version']} ({keg['license']})",
                        f"  License: {licensed or 'see QT-NOTICES.txt'}",
                        f"  License texts: {', '.join(text_reference(i, beside) for i in keg['texts'])}",
                        *source_lines(keg)]
        sources += ["", f"Sparkle {SPARKLE['version']} (build {SPARKLE['build']}, MIT and the licenses in its LICENSE)",
                    "  License: third-party/sparkle/LICENSE",
                    f"  Source: {SPARKLE['repository']} tag {SPARKLE['tag']}, commit {SPARKLE['commit']}",
                    f"  Release artifact: {SPARKLE['artifact']}", f"    SHA-256 {SPARKLE['artifactSha256']}"]
        if adb is not None:
            sources += ["", f"Android platform-tools adb {adb.get('version')} (licenses in ADB-NOTICE.txt)",
                        f"  Archive: {clean_url(adb.get('url'), 'ADB archive')}", f"    SHA-256 {adb['archiveSha256']}",
                        f"  Source: {clean_url(adb.get('source'), 'ADB source')}",
                        "    The pin records no tag or commit for this release; the exact source",
                        "    revision of this adb is not recorded here."]
        (stage / "THIRD-PARTY-SOURCES.txt").write_text("\n".join(sources) + "\n")

        offer = ["Source code for Waypair Link", "=" * 28, "",
                 "This build of Waypair Link was made from commit",
                 f"  {args.source_commit}",
                 "of the Waypair fork of NectarLink. The complete source tree of that commit is",
                 "published as", f"  {archive_url}", f"  SHA-256 {args.source_archive_sha256}"]
        if repository:
            offer += ["and in the repository", f"  {repository} (commit {args.source_commit})"]
        offer += ["", "Waypair Link is free software under the GNU General Public License, version 3",
                  "or later (LICENSE.txt in the app's Resources folder); its core libraries are",
                  "under the Mozilla Public License 2.0 (MPL-2.0.txt in this folder). Where the",
                  "exact sources of the bundled third-party components are: THIRD-PARTY-SOURCES.txt."]
        (stage / "SOURCE-OFFER.txt").write_text("\n".join(offer) + "\n")

        provenance = {
            "schema": 1,
            "source": {"commit": args.source_commit, "archive": archive_url,
                       "archiveSha256": args.source_archive_sha256, "repository": repository},
            "homebrew": [{k: keg[k] for k in ("name", "version", "license", "texts", "source", "patches", "bottle",
                                              "pouredFromBottle")} for keg in kegs],
            "sparkle": sparkle,
            "adb": adb,
            "licenseTexts": {"commit": pinned["commit"],
                             "files": {i: pinned["files"][i] for i in dict.fromkeys(keg_used + qt_used)}},
            "covers": dict(sorted(covers.items())),
        }
        (stage / "third-party/PROVENANCE.json").write_text(json.dumps(provenance, indent=2) + "\n")
        files = {str(p.relative_to(stage)): sha256(p) for p in sorted(stage.rglob("*")) if p.is_file()}
        manifest = {"schema": 1, "files": files, "covers": provenance["covers"]}
        (stage / MANIFEST).write_text(json.dumps(manifest, indent=2) + "\n")
        stage.rename(output)
    return {"notices": str(output), "files": len(files), "covers": len(covers)}


# --- verify / check-bundle ------------------------------------------------------

def verify(folder: Path) -> dict:
    manifest = json.loads(regular_file(folder / MANIFEST, "notices manifest").read_text())
    if manifest.get("schema") != 1 or not isinstance(manifest.get("files"), dict) or not manifest.get("covers"):
        raise ValueError("invalid notices manifest")
    for required in REQUIRED:
        if required not in manifest["files"]:
            raise ValueError(f"notices lack {required}")
    present = {str(p.relative_to(folder)) for p in folder.rglob("*") if p.is_file() or p.is_symlink()}
    unexpected = present - set(manifest["files"]) - {MANIFEST}
    if unexpected:
        raise ValueError(f"unlisted files in the notices: {sorted(unexpected)}")
    for name, digest in manifest["files"].items():
        if Path(name).is_absolute() or ".." in Path(name).parts:
            raise ValueError(f"invalid manifest path: {name}")
        if sha256(regular_file(folder / name, name)) != digest:
            raise ValueError(f"{name} changed since it was prepared")
    # The manifest itself is not hashed; its covers must equal the hashed PROVENANCE.json's.
    if json.loads((folder / "third-party/PROVENANCE.json").read_text()).get("covers") != manifest["covers"]:
        raise ValueError("the manifest's covers differ from third-party/PROVENANCE.json")
    for name in ("QtCore.framework", "Sparkle.framework"):
        if name not in manifest["covers"]:
            raise ValueError(f"notices do not cover {name}")
    return manifest


def component_key(relative: Path):
    """The notice entry a bundled Mach-O needs, or None for Waypair's own code."""
    parts = relative.parts
    if parts in OWN_CODE:
        return None
    if parts[:2] == ("Helpers", "adb") and len(parts) == 2:
        return "adb"
    return next((p for p in parts if p.endswith(".framework")), relative.name)


def check_bundled_sparkle(framework: Path, recorded: dict):
    """The bundled Sparkle must be the release `prepare` checked: same version and build, and the same
    binary slices once signatures are removed (from copies; the bundle is not touched)."""
    info = plistlib.loads(regular_file(framework / "Resources/Info.plist", "bundled Sparkle Info.plist").read_bytes())
    if (info.get("CFBundleShortVersionString"), info.get("CFBundleVersion")) != (recorded.get("version"),
                                                                                recorded.get("build")):
        raise ValueError(f"bundled {framework} is not the prepared Sparkle {recorded.get('version')} "
                         f"({recorded.get('build')})")
    if unsigned_slices(regular_file(framework / "Versions/B/Sparkle", "bundled Sparkle binary")) != \
            recorded.get("unsignedSliceSha256"):
        raise ValueError(f"bundled {framework} differs from the prepared Sparkle beyond its signature")


def check_bundle(folder: Path, app: Path) -> dict:
    manifest = verify(folder)
    provenance = json.loads((folder / "third-party/PROVENANCE.json").read_text())
    contents = app / "Contents"
    licenses = contents / "Resources/LICENSES"
    for name, digest in manifest["files"].items():
        if sha256(regular_file(licenses / name, f"bundled {name}")) != digest:
            raise ValueError(f"bundled {name} differs from the prepared notices")
    if (contents / "Helpers/adb").exists():
        adb = provenance.get("adb") or {}
        if sha256(regular_file(licenses / "ADB-NOTICE.txt", "bundled ADB-NOTICE.txt")) != adb.get("noticeSha256"):
            raise ValueError("bundled ADB-NOTICE.txt is not the pinned platform-tools notice")
    rows, missing, sparkles = [], set(), set()
    for path in sorted(contents.rglob("*")):
        if path.is_symlink() or not path.is_file():
            continue
        with path.open("rb") as handle:
            if handle.read(4) not in MACHO:
                continue
        relative = path.relative_to(contents)
        key = component_key(relative)
        if key is None:
            rows.append((str(relative), "Waypair Link (LICENSE.txt)"))
        elif key in manifest["covers"]:
            rows.append((str(relative), manifest["covers"][key]))
        else:
            missing.add(key)
        if key == "Sparkle.framework":
            sparkles.add(contents.joinpath(*relative.parts[:relative.parts.index(key) + 1]))
    if missing:
        raise ValueError(f"no notice covers these bundled components: {', '.join(sorted(missing))}")
    for framework in sorted(sparkles):
        check_bundled_sparkle(framework, provenance.get("sparkle") or {})
    listing = licenses / BUNDLED_LIST
    if listing.exists() or listing.is_symlink():
        raise ValueError(f"{BUNDLED_LIST} already exists in the bundle")
    lines = ["Components bundled in this build of Waypair Link", "=" * 48, "",
             "Each executable file in the app, relative to Contents/, and the notice that covers it.", ""]
    listing.write_text("\n".join(lines + [f"{path}\n  {who}" for path, who in rows]) + "\n")
    return {"machO": len(rows), "listing": str(listing)}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    build = commands.add_parser("prepare")
    build.add_argument("--rust-notices", type=Path, required=True)
    build.add_argument("--keg", type=Path, action="append", required=True)
    build.add_argument("--license-texts", type=Path, required=True)
    build.add_argument("--sparkle-framework", type=Path, required=True)
    build.add_argument("--sparkle-license", type=Path, required=True)
    build.add_argument("--adb-pin", type=Path)
    build.add_argument("--source-commit", required=True)
    build.add_argument("--source-archive-url", required=True)
    build.add_argument("--source-archive-sha256", required=True)
    build.add_argument("--source-repository")
    build.add_argument("--output-dir", type=Path, required=True)
    build.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[2])
    commands.add_parser("verify").add_argument("notices", type=Path)
    bundle = commands.add_parser("check-bundle")
    bundle.add_argument("notices", type=Path)
    bundle.add_argument("app", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            result = prepare(args)
        elif args.command == "verify":
            result = {"files": len(verify(args.notices)["files"])}
        else:
            result = check_bundle(args.notices, args.app)
    except (ValueError, OSError, KeyError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"notices refused: {error}\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
