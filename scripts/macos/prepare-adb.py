#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline preparation of the pinned Google ADB helper; never executes it."""

import argparse
import hashlib
import json
from pathlib import Path
import zipfile


DEFAULT_PIN = Path(__file__).with_name("adb-platform-tools.json")
MAX_ARCHIVE = 64 * 1024 * 1024


def prepare(archive: Path, pin: dict, output: Path) -> dict:
    if pin.get("schema") != 1 or pin.get("member") != "platform-tools/adb" or pin.get("notice") != "platform-tools/NOTICE.txt":
        raise ValueError("invalid platform-tools pin")
    size = pin.get("archiveBytes")
    member_size = pin.get("memberBytes")
    if not isinstance(size, int) or not 0 < size <= MAX_ARCHIVE or not isinstance(member_size, int) or not 0 < member_size <= MAX_ARCHIVE:
        raise ValueError("invalid archive size pin")
    if archive.is_symlink() or archive.stat().st_size != size:
        raise ValueError("archive size or file type mismatch")
    data = archive.read_bytes()
    if hashlib.sha256(data).hexdigest() != pin.get("archiveSha256"):
        raise ValueError("archive checksum mismatch")
    with zipfile.ZipFile(archive) as source:
        if source.namelist().count(pin["member"]) != 1 or source.namelist().count(pin["notice"]) != 1:
            raise ValueError("archive members missing or duplicated")
        if source.getinfo(pin["member"]).file_size != member_size or source.getinfo(pin["notice"]).file_size > 4 * 1024 * 1024:
            raise ValueError("member size mismatch")
        helper = source.read(pin["member"])
        notice = source.read(pin["notice"])
    if hashlib.sha256(helper).hexdigest() != pin.get("memberSha256") or hashlib.sha256(notice).hexdigest() != pin.get("noticeSha256"):
        raise ValueError("member checksum mismatch")
    output.mkdir(parents=True, exist_ok=False)
    executable = output / "adb"
    executable.write_bytes(helper)
    executable.chmod(0o755)
    (output / "ADB-NOTICE.txt").write_bytes(notice)
    (output / "origin.json").write_text(json.dumps(pin, indent=2) + "\n")
    return {"helper": str(executable), "upstreamSha256": pin["memberSha256"], "notice": str(output / "ADB-NOTICE.txt")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--pin", type=Path, default=DEFAULT_PIN)
    args = parser.parse_args()
    try:
        result = prepare(args.archive, json.loads(args.pin.read_text()), args.output_dir)
    except (ValueError, OSError, zipfile.BadZipFile) as error:
        parser.exit(1, f"ADB preparation refused: {error}\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
