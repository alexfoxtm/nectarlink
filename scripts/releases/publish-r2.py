#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
#
# publish-r2.py — publish a verified Waypair Link (Nectar base) release to R2.
#
# Dry-run by default: standard library only, no network. `--publish` imports
# boto3 (Root's pinned publisher venv) and uploads to the dedicated prefixes
# waypair/nectar/android/ and waypair/nectar/macos/ only. See
# docs/waypair/R2_RELEASE.md for the plan format, key layout and gates.

import argparse
import datetime
import hashlib
import json
import os
import re
import sys
import traceback
from pathlib import Path

PUBLIC_ORIGIN = "https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev"
ALLOWED_PREFIXES = ("waypair/nectar/android/", "waypair/nectar/macos/")
ANDROID_MANIFEST_KEY = "waypair/nectar/android/stable/update.json"
MACOS_APPCAST_KEY = "waypair/nectar/macos/stable/appcast.xml"
SUMS_NAME = "SHA256SUMS.txt"
NOTES_NAME = "CHANGELOG.txt"
NOTES_MAX_BYTES = 8 * 1024

VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-local\.[1-9][0-9]*)?$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
SAFE_NAME_RE = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9._-]*$")
SOURCE_SUFFIXES = (".tar.gz", ".tar.xz", ".tgz", ".zip")

IMMUTABLE_CACHE = "public, max-age=31536000, immutable"
MUTABLE_CACHE = "no-cache, max-age=0"
CONTENT_TYPES = {
    ".apk": "application/vnd.android.package-archive",
    ".dmg": "application/x-apple-diskimage",
    ".xml": "application/xml",
    ".json": "application/json",
    ".txt": "text/plain; charset=utf-8",
    ".zip": "application/zip",
    ".gz": "application/gzip",
    ".tgz": "application/gzip",
    ".xz": "application/x-xz",
}

EXIT_PREFLIGHT = 1
EXIT_REFUSED = 2
EXIT_UPLOAD = 3


class Failure(Exception):
    pass


class Log:
    def __init__(self, path):
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)
        self.file = path.open("a", encoding="utf-8")

    def __call__(self, line):
        print(line, flush=True)
        self.file.write(line + "\n")
        self.file.flush()

    def detail(self, text):
        self.file.write(text if text.endswith("\n") else text + "\n")
        self.file.flush()


class Upload:
    def __init__(self, key, mutable, sha256, size, path=None, data=None):
        if not key.startswith(ALLOWED_PREFIXES) or ".." in key or "//" in key or "%" in key:
            raise Failure(f"refusing key outside dedicated prefixes: {key}")
        self.key = key
        self.mutable = mutable
        self.sha256 = sha256
        self.size = size
        self.path = path
        self.data = data

    @property
    def content_type(self):
        return CONTENT_TYPES.get(Path(self.key).suffix, "application/octet-stream")

    def body(self):
        return self.data if self.data is not None else self.path.read_bytes()


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def public_url(key):
    return f"{PUBLIC_ORIGIN}/{key}"


def verified_file(entry, label, base):
    """The plan entry's file, after checking its exact size and SHA-256."""
    if not isinstance(entry, dict):
        raise Failure(f"{label}: missing from plan")
    raw, sha, size = entry.get("path"), entry.get("sha256"), entry.get("size")
    if not isinstance(raw, str) or not raw:
        raise Failure(f"{label}: plan has no path")
    if not isinstance(sha, str) or not SHA_RE.match(sha):
        raise Failure(f"{label}: plan sha256 must be 64 lowercase hex characters")
    if not isinstance(size, int) or isinstance(size, bool) or size <= 0:
        raise Failure(f"{label}: plan size must be a positive integer")
    path = Path(raw).expanduser()
    if not path.is_absolute():
        path = base / path
    if not path.is_file():
        raise Failure(f"{label}: file missing: {path}")
    actual_size = path.stat().st_size
    if actual_size != size:
        raise Failure(f"{label}: size {actual_size} != plan {size}: {path}")
    actual_sha = sha256_file(path)
    if actual_sha != sha:
        raise Failure(f"{label}: sha256 {actual_sha} != plan {sha}: {path}")
    if not SAFE_NAME_RE.match(path.name) or ".." in path.name:
        raise Failure(f"{label}: unsafe file name {path.name!r}")
    return path, sha, size


def sums_upload(prefix, files):
    lines = "".join(f"{sha}  {path.name}\n" for path, sha, _ in sorted(files, key=lambda f: f[0].name))
    data = lines.encode("utf-8")
    return Upload(prefix + SUMS_NAME, False, hashlib.sha256(data).hexdigest(), len(data), data=data)


def release_notes_of(section, label, base):
    """Optional per-platform CHANGELOG.txt, or None when absent.

    Verified with the same path/size/SHA-256 guards as every other file,
    plus: exact name CHANGELOG.txt, UTF-8 non-empty text, at most 8 KiB.
    """
    if "release_notes" not in section:
        return None
    notes = verified_file(section.get("release_notes"), f"{label} release notes", base)
    if notes[0].name != NOTES_NAME:
        raise Failure(f"{label} release notes: file name must be exactly {NOTES_NAME}")
    if notes[2] > NOTES_MAX_BYTES:
        raise Failure(f"{label} release notes: size {notes[2]} exceeds {NOTES_MAX_BYTES} bytes")
    try:
        text = notes[0].read_bytes().decode("utf-8")
    except (OSError, UnicodeDecodeError) as e:
        raise Failure(f"{label} release notes: must be UTF-8 text: {e}")
    if not text or not text.strip():
        raise Failure(f"{label} release notes: must be non-empty text")
    return notes


def version_of(section, label):
    version = section.get("version")
    if not isinstance(version, str) or not VERSION_RE.match(version):
        raise Failure(f"{label}: version must be MAJOR.MINOR.PATCH with an optional -local.N suffix")
    return version


def source_of(section, label, base):
    source = verified_file(section.get("source"), f"{label} source archive", base)
    if not source[0].name.endswith(SOURCE_SUFFIXES):
        raise Failure(f"{label} source archive: expected {', '.join(SOURCE_SUFFIXES)}")
    return source


def plan_android(section, base):
    version = version_of(section, "android")
    apk = verified_file(section.get("apk"), "android apk", base)
    name = apk[0].name
    if not name.endswith("-android.apk") or not 12 <= len(name) <= 128:
        raise Failure(f"android apk: name must end in -android.apk (12-128 chars): {name}")
    source = source_of(section, "android", base)
    if source[0].name == name or source[0].name == SUMS_NAME:
        raise Failure("android: source archive name collides with another asset")
    notes = release_notes_of(section, "android", base)
    if notes is not None and source[0].name == NOTES_NAME:
        raise Failure("android: source archive name collides with another asset")

    prefix = f"waypair/nectar/android/releases/v{version}/"
    if notes is None:
        sums = sums_upload(prefix, [apk, source])
        html_url = section.get("html_url", public_url(sums.key))
    else:
        sums = sums_upload(prefix, [apk, source, notes])
        html_url = section.get("html_url", public_url(prefix + NOTES_NAME))
    if not isinstance(html_url, str) or not html_url.startswith("https://") or any(c.isspace() for c in html_url):
        raise Failure("android: html_url must be an https URL without whitespace")

    feed = {
        "tag_name": f"v{version}",
        "html_url": html_url,
        "draft": False,
        "prerelease": False,
        "assets": [
            {"name": name, "size": apk[2], "browser_download_url": public_url(prefix + name)},
            {"name": SUMS_NAME, "size": sums.size, "browser_download_url": public_url(sums.key)},
        ],
    }
    feed_data = (json.dumps(feed, indent=2) + "\n").encode("utf-8")
    immutable = [
        Upload(prefix + name, False, apk[1], apk[2], path=apk[0]),
        Upload(prefix + source[0].name, False, source[1], source[2], path=source[0]),
    ]
    if notes is not None:
        immutable.append(Upload(prefix + NOTES_NAME, False, notes[1], notes[2], path=notes[0]))
    immutable.append(sums)
    manifest = Upload(ANDROID_MANIFEST_KEY, True, hashlib.sha256(feed_data).hexdigest(), len(feed_data), data=feed_data)
    return immutable, [manifest]


def plan_macos(section, base):
    version = version_of(section, "macos")
    dmg = verified_file(section.get("dmg"), "macos dmg", base)
    if not dmg[0].name.endswith(".dmg"):
        raise Failure(f"macos dmg: expected a .dmg file: {dmg[0].name}")
    source = source_of(section, "macos", base)
    if source[0].name in (dmg[0].name, SUMS_NAME):
        raise Failure("macos: source archive name collides with another asset")
    notes = release_notes_of(section, "macos", base)
    if notes is not None and source[0].name == NOTES_NAME:
        raise Failure("macos: source archive name collides with another asset")
    appcast = verified_file(section.get("appcast"), "macos signed appcast", base)

    prefix = f"waypair/nectar/macos/releases/v{version}/"
    dmg_url = public_url(prefix + dmg[0].name)
    # Consistency only: Sparkle (and Root) verify the Ed25519 signature.
    text = appcast[0].read_bytes().decode("utf-8", errors="replace")
    if dmg_url not in text:
        raise Failure(f"macos appcast: does not reference the published DMG URL {dmg_url}")
    if f'length="{dmg[2]}"' not in text:
        raise Failure(f'macos appcast: no length="{dmg[2]}" matching the DMG size')
    if "sparkle:edSignature=" not in text:
        raise Failure("macos appcast: no sparkle:edSignature attribute; Root must sign it first")

    immutable = [
        Upload(prefix + dmg[0].name, False, dmg[1], dmg[2], path=dmg[0]),
        Upload(prefix + source[0].name, False, source[1], source[2], path=source[0]),
    ]
    if notes is not None:
        immutable.append(Upload(prefix + NOTES_NAME, False, notes[1], notes[2], path=notes[0]))
        immutable.append(sums_upload(prefix, [dmg, source, notes]))
    else:
        immutable.append(sums_upload(prefix, [dmg, source]))
    manifest = Upload(MACOS_APPCAST_KEY, True, appcast[1], appcast[2], path=appcast[0])
    return immutable, [manifest]


def preflight(plan_path, log):
    try:
        plan = json.loads(plan_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise Failure(f"plan unreadable: {plan_path}: {e}")
    if not isinstance(plan, dict):
        raise Failure("plan must be a JSON object")
    base = plan_path.parent
    unknown = set(plan) - {"qualification_receipt", "android", "macos"}
    if unknown:
        raise Failure(f"plan has unknown keys: {sorted(unknown)}")
    receipt = verified_file(plan.get("qualification_receipt"), "qualification receipt", base)
    log(f"PREFLIGHT qualification receipt present, sha256 matches: {receipt[0]}")
    if "android" not in plan and "macos" not in plan:
        raise Failure("plan has neither an android nor a macos section")

    immutable, manifests = [], []
    for name, build in (("android", plan_android), ("macos", plan_macos)):
        if name in plan:
            if not isinstance(plan[name], dict):
                raise Failure(f"{name}: section must be an object")
            i, m = build(plan[name], base)
            immutable += i
            manifests += m
            log(f"PREFLIGHT {name}: local files match plan sizes and SHA-256")
    keys = [u.key for u in immutable + manifests]
    if len(keys) != len(set(keys)):
        raise Failure("plan produces duplicate object keys")
    return immutable, manifests


def describe(uploads, log, run_dir):
    for u in uploads:
        kind = "mutable  " if u.mutable else "immutable"
        log(f"  {kind} {u.key}  size={u.size} sha256={u.sha256} type={u.content_type}")
        if u.data is not None:
            out = run_dir / "generated" / u.key
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(u.data)


def r2_client(args):
    for var in ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"):
        if not os.environ.get(var):
            raise Failure(f"{var} is not set (values are never printed)")
    endpoint = os.environ.get("AWS_ENDPOINT_URL_S3") or os.environ.get("AWS_ENDPOINT_URL")
    if not endpoint or not re.match(r"^https://[0-9a-f]{32}\.r2\.cloudflarestorage\.com/?$", endpoint):
        raise Failure("AWS_ENDPOINT_URL_S3 (or AWS_ENDPOINT_URL) must be https://<account>.r2.cloudflarestorage.com")
    try:
        import boto3
        from botocore.config import Config
    except ImportError:
        raise Failure("boto3 is not importable; run --publish from Root's pinned publisher venv")
    return boto3.client("s3", endpoint_url=endpoint, region_name="auto",
                        config=Config(retries={"max_attempts": 3, "mode": "standard"}))


def error_code(e):
    return getattr(e, "response", {}).get("Error", {}).get("Code", "")


def remote_object(client, bucket, key, expected_size):
    """(size, sha256) of an existing object, or None when absent."""
    try:
        head = client.head_object(Bucket=bucket, Key=key)
    except Exception as e:
        if error_code(e) in ("404", "NoSuchKey", "NotFound"):
            return None
        raise
    if head["ContentLength"] != expected_size:
        return head["ContentLength"], "(not read: size differs)"
    body = client.get_object(Bucket=bucket, Key=key)["Body"]
    digest = hashlib.sha256()
    for chunk in iter(lambda: body.read(1 << 20), b""):
        digest.update(chunk)
    return head["ContentLength"], digest.hexdigest()


def put(client, bucket, u):
    body = u.body()
    if len(body) != u.size or hashlib.sha256(body).hexdigest() != u.sha256:
        raise Failure(f"{u.key}: local bytes changed since preflight")
    client.put_object(Bucket=bucket, Key=u.key, Body=body, ContentType=u.content_type,
                      CacheControl=MUTABLE_CACHE if u.mutable else IMMUTABLE_CACHE,
                      Metadata={"sha256": u.sha256})
    size = client.head_object(Bucket=bucket, Key=u.key)["ContentLength"]
    if size != u.size:
        raise Failure(f"{u.key}: stored size {size} != {u.size} after upload")


def publish(args, immutable, manifests, log, run_dir):
    client = r2_client(args)
    bucket = args.bucket

    log("REMOTE-PREFLIGHT checking immutable keys (read-only)")
    pending, conflicts = [], []
    for u in immutable:
        existing = remote_object(client, bucket, u.key, u.size)
        if existing is None:
            pending.append(u)
            log(f"  absent     {u.key}")
        elif existing == (u.size, u.sha256):
            log(f"  identical  {u.key} (will skip)")
        else:
            conflicts.append(u.key)
            log(f"  CONFLICT   {u.key}: remote size={existing[0]} sha256={existing[1]}")
    if conflicts:
        log(f"REFUSED: {len(conflicts)} immutable key(s) already hold different bytes; nothing uploaded")
        return EXIT_REFUSED

    uploaded = []
    try:
        for u in pending:
            put(client, bucket, u)
            uploaded.append(u.key)
            log(f"UPLOADED immutable {u.key}")
        for u in manifests:
            prior = None
            try:
                prior = client.get_object(Bucket=bucket, Key=u.key)["Body"].read()
            except Exception as e:
                if error_code(e) not in ("404", "NoSuchKey", "NotFound"):
                    raise
            if prior is not None:
                backup = run_dir / "prior" / u.key
                backup.parent.mkdir(parents=True, exist_ok=True)
                backup.write_bytes(prior)
                if hashlib.sha256(prior).hexdigest() == u.sha256:
                    log(f"SKIPPED mutable {u.key} (already identical)")
                    continue
                log(f"  prior {u.key} saved to {backup}")
            put(client, bucket, u)
            uploaded.append(u.key)
            log(f"UPLOADED mutable {u.key}")
    except Exception as e:
        log.detail(traceback.format_exc())
        log(f"UPLOAD FAILED after {len(uploaded)} upload(s): {type(e).__name__}: {error_code(e) or e}")
        log(f"  uploaded before failure: {uploaded or 'none'}")
        return EXIT_UPLOAD
    log(f"PUBLISH COMPLETE: {len(uploaded)} object(s) uploaded; public readback is still Root's gate")
    return 0


def main():
    parser = argparse.ArgumentParser(
        description="Publish a verified Waypair Link (Nectar) release to R2. Dry-run unless --publish.")
    parser.add_argument("--plan", required=True, type=Path, help="release plan JSON from Root")
    parser.add_argument("--publish", action="store_true", help="upload (default: dry-run, no network)")
    parser.add_argument("--bucket", help="R2 bucket name (required with --publish)")
    parser.add_argument("--log-dir", type=Path,
                        help="where run logs, generated manifests and prior manifests go "
                             "(default: <plan dir>/r2-publish-logs)")
    args = parser.parse_args()

    mode = "publish" if args.publish else "dry-run"
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    plan_path = args.plan.expanduser().resolve()
    run_dir = (args.log_dir or plan_path.parent / "r2-publish-logs").expanduser() / f"{stamp}-{mode}"
    log = Log(run_dir / "run.log")
    log(f"MODE {mode.upper()} plan={plan_path} origin={PUBLIC_ORIGIN} log={run_dir / 'run.log'}")

    try:
        if args.publish and not args.bucket:
            raise Failure("--publish needs --bucket")
        immutable, manifests = preflight(plan_path, log)
    except Failure as e:
        log(f"PREFLIGHT FAILED: {e}")
        return EXIT_PREFLIGHT
    log("PREFLIGHT OK. Upload order (immutable artifacts and SHA256SUMS first, then manifests):")
    describe(immutable + manifests, log, run_dir)

    if not args.publish:
        log(f"DRY-RUN COMPLETE: nothing uploaded; generated files in {run_dir / 'generated'}")
        return 0
    try:
        return publish(args, immutable, manifests, log, run_dir)
    except Failure as e:
        log(f"PUBLISH FAILED before any write: {e}")
        return EXIT_PREFLIGHT
    except Exception as e:
        log.detail(traceback.format_exc())
        log(f"REMOTE-PREFLIGHT FAILED, nothing uploaded: {type(e).__name__}: {error_code(e) or e}")
        return EXIT_REFUSED


if __name__ == "__main__":
    sys.exit(main())
