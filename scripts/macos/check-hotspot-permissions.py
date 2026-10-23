# SPDX-License-Identifier: GPL-3.0-or-later
"""Validate hotspot permission descriptions in the finished macOS host plist."""
from pathlib import Path
import plistlib
import sys


def validate(path):
    with Path(path).open("rb") as stream:
        info = plistlib.load(stream)
    for key in ("NSLocationUsageDescription", "NSLocationWhenInUseUsageDescription"):
        value = info.get(key)
        if not isinstance(value, str) or not value.strip():
            raise ValueError("Hotspot discovery requires a nonempty " + key)


if __name__ == '__main__':
    try:
        validate(sys.argv[1])
    except (OSError, ValueError) as error:
        raise SystemExit(str(error))
