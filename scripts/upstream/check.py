#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
#
# check.py — read-only upstream drift report for the Waypair Link port.
#
# Default: local only, no network, writes nothing, prints a report. Opt-in:
#   --fetch   fetch canonical upstream main and the pinned PR heads into a
#             scratch bare mirror under --out (never into this checkout)
#   --github  bounded unauthenticated GET requests to the public GitHub API
#             (releases, open PRs, pinned PR state)
# Never merges, cherry-picks, checks out, pushes, publishes or comments.
# Pins and port deltas come from docs/waypair/UPSTREAM_BASE.json; the
# workflow is in docs/waypair/UPSTREAM_SYNC.md.

import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BASE = ROOT / "docs" / "waypair" / "UPSTREAM_BASE.json"

SHA_RE = re.compile(r"^[0-9a-f]{40}$")
REPO_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
GIT_TIMEOUT = 300
MAX_COMMITS = 100

EXIT_OK = 0
EXIT_FAILURE = 1
EXIT_DRIFT = 3


class Failure(Exception):
    pass


class Report:
    def __init__(self):
        self.data = {"generated_at": now_utc(), "findings": []}

    def add(self, level, message):
        assert level in ("failure", "drift", "info")
        self.data["findings"].append({"level": level, "message": message})

    def levels(self):
        return {f["level"] for f in self.data["findings"]}


def now_utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ---------------------------------------------------------------- base file

def load_base(path):
    try:
        base = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise Failure(f"cannot read base file {path}: {e}")
    if base.get("schema") != 1:
        raise Failure("base file: unsupported schema")
    up = base.get("upstream", {})
    for key in ("remote", "url", "repo", "branch"):
        if not up.get(key):
            raise Failure(f"base file: upstream.{key} missing")
    if not REPO_RE.match(up["repo"]):
        raise Failure("base file: upstream.repo must be owner/name")
    ids = set()
    for pin in base.get("pins", []):
        if pin.get("id") in ids or not pin.get("id"):
            raise Failure(f"base file: duplicate or empty pin id {pin.get('id')!r}")
        ids.add(pin["id"])
        if not SHA_RE.match(pin.get("sha", "")):
            raise Failure(f"base file: pin {pin['id']} needs a full 40-hex sha")
        if pin.get("kind") not in ("branch", "pull"):
            raise Failure(f"base file: pin {pin['id']} kind must be branch or pull")
        if pin["kind"] == "pull" and not isinstance(pin.get("number"), int):
            raise Failure(f"base file: pull pin {pin['id']} needs a number")
        if not str(pin.get("ref", "")).startswith("refs/"):
            raise Failure(f"base file: pin {pin['id']} needs a refs/ ref")
    if base.get("port", {}).get("base_pin") not in ids:
        raise Failure("base file: port.base_pin must name a pin")
    if not any(p["id"] == "main" for p in base["pins"]):
        raise Failure("base file: a pin with id 'main' is required")
    return base


def pin(base, pin_id):
    return next(p for p in base["pins"] if p["id"] == pin_id)


def check_url(url, allow_file):
    parsed = urllib.parse.urlsplit(url)
    if parsed.username or parsed.password:
        raise Failure(f"refusing URL with embedded credentials: {redact(url)}")
    if parsed.scheme == "https" or (allow_file and parsed.scheme == "file"):
        return
    raise Failure(f"refusing non-HTTPS upstream URL: {redact(url)}")


def redact(url):
    parsed = urllib.parse.urlsplit(url)
    if parsed.username or parsed.password:
        host = parsed.hostname or ""
        if parsed.port:
            host += f":{parsed.port}"
        return urllib.parse.urlunsplit((parsed.scheme, "***@" + host, parsed.path, "", ""))
    return url


# ---------------------------------------------------------------------- git

def network_env():
    env = dict(os.environ)
    for key in ("GIT_ASKPASS", "SSH_ASKPASS", "GIT_SSH_COMMAND", "GIT_SSH"):
        env.pop(key, None)
    env.update({
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_ASKPASS": "false",
        "SSH_ASKPASS": "false",
        "GCM_INTERACTIVE": "never",
        "GIT_SSH_COMMAND": "ssh -o BatchMode=yes",
    })
    return env


def network_config(allow_file):
    cfg = ["-c", "credential.helper=", "-c", "core.askPass=false",
           "-c", "protocol.allow=never", "-c", "protocol.https.allow=always",
           "-c", "http.followRedirects=initial", "-c", "fetch.recurseSubmodules=false",
           "-c", "core.hooksPath=/dev/null"]
    if allow_file:
        cfg += ["-c", "protocol.file.allow=always"]
    return cfg


def git(args, cwd, env=None, check=True, timeout=GIT_TIMEOUT, pre=()):
    try:
        proc = subprocess.run(["git", *pre, *args], cwd=cwd, env=env, capture_output=True,
                              text=True, timeout=timeout, stdin=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        raise Failure(f"git {args[0]} timed out after {timeout}s")
    if check and proc.returncode != 0:
        raise Failure(f"git {' '.join(args[:3])} failed: {proc.stderr.strip()[:500]}")
    return proc


def git_out(args, cwd, **kw):
    return git(args, cwd, **kw).stdout.strip()


def has_commit(repo, sha):
    return git(["cat-file", "-e", f"{sha}^{{commit}}"], repo, check=False).returncode == 0


def is_ancestor(repo, a, b):
    return git(["merge-base", "--is-ancestor", a, b], repo, check=False).returncode == 0


LOG_FORMAT = "%H%x1f%an%x1f%ae%x1f%aI%x1f%s%x1f%(trailers:key=Signed-off-by,valueonly,separator=%x2C)%x1e"


def commits(repo, rev_range, limit=MAX_COMMITS):
    total = int(git_out(["rev-list", "--count", rev_range], repo) or 0)
    out = git(["log", f"--max-count={limit}", f"--format={LOG_FORMAT}", rev_range], repo).stdout
    items = []
    for rec in out.split("\x1e"):
        rec = rec.lstrip("\n")
        if not rec:
            continue
        sha, name, email, date, subject, signoff = rec.split("\x1f")
        items.append({"sha": sha, "author": f"{name} <{email}>", "date": date,
                      "subject": subject, "signed_off_by": signoff.strip() or None})
    return {"total": total, "shown": len(items), "commits": items}


# -------------------------------------------------------------- local check

def classify(paths, deltas):
    cats = set()
    for path in paths:
        hit = [name for name, prefixes in deltas.items()
               if any(path.startswith(p) for p in prefixes)]
        cats.update(hit or ["generic"])
    return sorted(cats)


def local_check(repo, base, report, allow_file=False):
    up = base["upstream"]
    sect = report.data["local"] = {}
    top = git_out(["rev-parse", "--show-toplevel"], repo)
    sect["repo"] = top
    sect["head"] = git_out(["rev-parse", "HEAD"], repo)
    sect["branch"] = git_out(["rev-parse", "--abbrev-ref", "HEAD"], repo)
    sect["shallow"] = git_out(["rev-parse", "--is-shallow-repository"], repo) == "true"
    sect["dirty_paths"] = len([l for l in git_out(["status", "--porcelain"], repo).splitlines() if l])

    remotes = {}
    for line in git_out(["remote", "-v"], repo).splitlines():
        name, url, kind = line.split()
        if kind == "(fetch)":
            remotes[name] = url
    sect["remotes"] = {k: redact(v) for k, v in remotes.items()}
    url = remotes.get(up["remote"])
    if url is None:
        report.add("failure", f"remote '{up['remote']}' missing; expected {redact(up['url'])}")
    elif url != up["url"]:
        report.add("failure", f"remote '{up['remote']}' is {redact(url)}, expected {redact(up['url'])}")
    for name, u in remotes.items():
        parts = urllib.parse.urlsplit(u)
        if parts.username or parts.password:
            report.add("failure", f"remote '{name}' embeds credentials: {redact(u)}")

    pins = sect["pins"] = {}
    for p in base["pins"]:
        present = has_commit(repo, p["sha"])
        entry = {"sha": p["sha"], "present": present, "in_port": bool(p.get("in_port"))}
        if present and p.get("in_port"):
            entry["ancestor_of_head"] = is_ancestor(repo, p["sha"], "HEAD")
            if not entry["ancestor_of_head"]:
                report.add("failure", f"pin {p['id']} {p['sha'][:8]} is not an ancestor of HEAD")
        elif p.get("in_port") and not present:
            if sect["shallow"]:
                report.add("info", f"pin {p['id']} {p['sha'][:8]} is below the shallow boundary; "
                                   "verify with --fetch or --github")
            else:
                report.add("failure", f"pin {p['id']} {p['sha'][:8]} marked in_port but missing")
        pins[p["id"]] = entry

    bp = pin(base, base["port"]["base_pin"])
    if not pins[bp["id"]]["present"]:
        report.add("failure", f"port base {bp['id']} {bp['sha'][:8]} not in this checkout")
        return
    log = commits(repo, f"{bp['sha']}..HEAD", limit=1000)
    deltas = base.get("port_deltas", {})
    for c in log["commits"]:
        paths = git_out(["diff-tree", "--no-commit-id", "--name-only", "-r", c["sha"]], repo).splitlines()
        c["categories"] = classify(paths, deltas)
    sect["port_commits"] = log
    generic = [c for c in log["commits"] if "generic" in c["categories"]]
    mixed = [c for c in generic if len(c["categories"]) > 1]
    sect["upstream_candidates"] = [c["sha"] for c in generic]
    report.add("info", f"{log['total']} port commits on {bp['id']}; {len(generic)} touch generic paths "
                       "(upstream candidates after qualification)")
    for c in mixed:
        report.add("info", f"port commit {c['sha'][:8]} mixes generic and {', '.join(x for x in c['categories'] if x != 'generic')} "
                           "paths; split before offering upstream")


# -------------------------------------------------------------- fetch check

def fetch_check(base, out, report, allow_file=False):
    up = base["upstream"]
    check_url(up["url"], allow_file)
    env, cfg = network_env(), network_config(allow_file)
    mirror = Path(out) / "mirror.git"
    sect = report.data["fetch"] = {"mirror": str(mirror), "url": up["url"]}
    branch = up["branch"]
    if not (mirror / "HEAD").exists():
        git(["clone", "--bare", "--filter=blob:none", "--no-tags", "--single-branch",
             "--branch", branch, up["url"], str(mirror)], cwd=out, env=env, pre=cfg)
    else:
        git(["fetch", "--no-tags", "--filter=blob:none", "origin",
             f"+refs/heads/{branch}:refs/heads/{branch}"], cwd=mirror, env=env, pre=cfg)
    for p in base["pins"]:
        if p["kind"] == "pull":
            r = git(["fetch", "--no-tags", "--filter=blob:none", "origin", f"+{p['ref']}:{p['ref']}"],
                    cwd=mirror, env=env, pre=cfg, check=False)
            if r.returncode != 0:
                report.add("drift", f"{p['id']}: {p['ref']} not fetchable (PR closed or ref gone)")
        if not has_commit(mirror, p["sha"]):
            git(["fetch", "--no-tags", "--filter=blob:none", "origin", p["sha"]],
                cwd=mirror, env=env, pre=cfg, check=False)

    main_head = git_out(["rev-parse", f"refs/heads/{branch}"], mirror)
    sect["main_head"] = main_head
    main_pin = pin(base, "main")
    if not has_commit(mirror, main_pin["sha"]):
        report.add("failure", f"main pin {main_pin['sha'][:8]} not found upstream")
    elif not is_ancestor(mirror, main_pin["sha"], main_head):
        report.add("failure", f"upstream {branch} was rewritten: pin {main_pin['sha'][:8]} "
                              f"is not an ancestor of {main_head[:8]}")
    else:
        new = sect["new_on_main"] = commits(mirror, f"{main_pin['sha']}..{main_head}")
        if new["total"]:
            report.add("drift", f"{new['total']} new commits on upstream {branch} since pin {main_pin['sha'][:8]}")
        for c in new["commits"]:
            if not c["signed_off_by"]:
                report.add("info", f"upstream {c['sha'][:8]} has no Signed-off-by trailer")

    bp = pin(base, base["port"]["base_pin"])
    if has_commit(mirror, bp["sha"]):
        sect["main_not_in_port"] = commits(mirror, f"{bp['sha']}..{main_head}")

    pulls = sect["pulls"] = {}
    for p in base["pins"]:
        if p["kind"] != "pull":
            continue
        entry = pulls[p["id"]] = {"pin": p["sha"]}
        r = git(["rev-parse", "--verify", "--quiet", p["ref"]], mirror, check=False)
        if r.returncode != 0:
            continue
        head = entry["head"] = r.stdout.strip()
        entry["merged_into_main"] = is_ancestor(mirror, head, main_head)
        if entry["merged_into_main"]:
            report.add("drift", f"{p['id']} head {head[:8]} is now in upstream {branch}")
        if head == p["sha"]:
            continue
        if has_commit(mirror, p["sha"]) and is_ancestor(mirror, p["sha"], head):
            entry["new_commits"] = commits(mirror, f"{p['sha']}..{head}")
            report.add("drift", f"{p['id']} moved {entry['new_commits']['total']} commits "
                                f"({p['sha'][:8]} -> {head[:8]})")
        else:
            entry["force_pushed"] = True
            report.add("drift", f"{p['id']} was force-pushed: {p['sha'][:8]} is not an ancestor "
                                f"of {head[:8]}; re-review the whole PR")


# ------------------------------------------------------------- GitHub check

class GitHub:
    def __init__(self, cfg):
        self.base = cfg.get("base", "https://api.github.com").rstrip("/")
        if not self.base.startswith("https://"):
            raise Failure("github_api.base must be HTTPS")
        self.budget = int(cfg.get("max_requests", 12))
        self.per_page = min(int(cfg.get("per_page", 30)), 100)
        self.timeout = int(cfg.get("timeout_seconds", 20))
        self.used = 0

    def get(self, path, **query):
        if self.used >= self.budget:
            raise Failure(f"GitHub request budget ({self.budget}) exhausted")
        self.used += 1
        url = f"{self.base}{path}"
        if query:
            url += "?" + urllib.parse.urlencode(query)
        req = urllib.request.Request(url, method="GET", headers={
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "waypair-upstream-check",
        })
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                return json.loads(resp.read(5_000_000).decode("utf-8"))
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            raise Failure(f"GitHub GET {path} failed: HTTP {e.code}")
        except (urllib.error.URLError, TimeoutError, ValueError) as e:
            raise Failure(f"GitHub GET {path} failed: {e}")


def relevant(pr, keywords):
    text = " ".join([pr.get("title") or "", (pr.get("head") or {}).get("ref") or "",
                     " ".join(l.get("name", "") for l in pr.get("labels") or [])]).lower()
    words = set(re.findall(r"[a-z0-9]+", text))
    return sorted(k for k in keywords if k in words)


def github_check(base, report, fetched):
    up = base["upstream"]
    gh = GitHub(base.get("github_api", {}))
    repo = up["repo"]
    sect = report.data["github"] = {}
    main_pin = pin(base, "main")

    if not fetched:
        cmp = gh.get(f"/repos/{repo}/compare/{main_pin['sha']}...{up['branch']}")
        if cmp is None:
            report.add("failure", f"GitHub compare against main pin {main_pin['sha'][:8]} returned 404")
        else:
            sect["main_compare"] = {"status": cmp["status"], "ahead_by": cmp["ahead_by"],
                                    "behind_by": cmp["behind_by"],
                                    "commits": [{"sha": c["sha"], "subject": c["commit"]["message"].splitlines()[0],
                                                 "author": c["commit"]["author"]["name"]}
                                                for c in cmp.get("commits", [])[:MAX_COMMITS]]}
            if cmp["behind_by"]:
                report.add("failure", f"upstream {up['branch']} no longer contains main pin "
                                      f"(behind_by={cmp['behind_by']})")
            elif cmp["ahead_by"]:
                report.add("drift", f"{cmp['ahead_by']} new commits on upstream {up['branch']} (GitHub compare)")

    releases = gh.get(f"/repos/{repo}/releases", per_page=min(gh.per_page, 10)) or []
    known = up.get("latest_release_at_record")
    new_rel = []
    for r in releases:
        if r.get("tag_name") == known:
            break
        if not r.get("draft"):
            new_rel.append({"tag": r["tag_name"], "published_at": r.get("published_at"),
                            "prerelease": r.get("prerelease"), "url": r.get("html_url")})
    sect["new_releases"] = new_rel
    if new_rel:
        report.add("drift", f"{len(new_rel)} upstream releases after {known}: "
                            + ", ".join(r["tag"] for r in new_rel))

    pulls = gh.get(f"/repos/{repo}/pulls", state="open", sort="updated",
                   direction="desc", per_page=gh.per_page) or []
    keywords = base.get("relevance", {}).get("keywords", [])
    pinned = {p["number"]: p for p in base["pins"] if p["kind"] == "pull"}
    listed = []
    for pr in pulls:
        hits = relevant(pr, keywords)
        if not hits and pr["number"] not in pinned:
            continue
        listed.append({"number": pr["number"], "title": pr["title"], "user": (pr.get("user") or {}).get("login"),
                       "head_repo": ((pr.get("head") or {}).get("repo") or {}).get("full_name"),
                       "head_sha": (pr.get("head") or {}).get("sha"), "updated_at": pr.get("updated_at"),
                       "draft": pr.get("draft"), "keywords": hits, "url": pr.get("html_url")})
    sect["relevant_open_prs"] = listed
    sect["open_prs_scanned"] = len(pulls)

    sect["pinned_prs"] = {}
    open_by_number = {pr["number"]: pr for pr in pulls}
    for number, p in pinned.items():
        pr = open_by_number.get(number) or gh.get(f"/repos/{repo}/pulls/{number}")
        if pr is None:
            report.add("drift", f"{p['id']}: PR #{number} not found")
            continue
        head = (pr.get("head") or {}).get("sha")
        sect["pinned_prs"][p["id"]] = {"number": number, "state": pr.get("state"),
                                       "merged_at": pr.get("merged_at"), "head_sha": head}
        if pr.get("state") != "open":
            report.add("drift", f"{p['id']}: PR #{number} is {pr.get('state')}"
                                + (" (merged)" if pr.get("merged_at") else ""))
        if head and head != p["sha"] and not fetched:
            report.add("drift", f"{p['id']}: PR #{number} head moved {p['sha'][:8]} -> {head[:8]}")
    sect["requests_used"] = gh.used


# ------------------------------------------------------------------ output

def render_markdown(data):
    lines = [f"# Upstream check {data['generated_at']}", ""]
    for f in data["findings"]:
        lines.append(f"- **{f['level']}**: {f['message']}")
    loc = data.get("local", {})
    if loc:
        lines += ["", "## Local", "",
                  f"- repo `{loc['repo']}` branch `{loc['branch']}` head `{loc['head'][:12]}` "
                  f"shallow={loc['shallow']} dirty_paths={loc['dirty_paths']}"]
        for c in loc.get("port_commits", {}).get("commits", []):
            lines.append(f"- `{c['sha'][:8]}` [{', '.join(c['categories'])}] {c['subject']}")
    fet = data.get("fetch", {})
    if fet:
        lines += ["", "## Upstream git", "", f"- main head `{fet.get('main_head', '?')[:12]}`"]
        for c in fet.get("new_on_main", {}).get("commits", []):
            dco = "DCO" if c["signed_off_by"] else "no-DCO"
            lines.append(f"- `{c['sha'][:8]}` {c['author']} ({dco}) {c['subject']}")
        for pid, e in fet.get("pulls", {}).items():
            lines.append(f"- {pid}: head `{e.get('head', '?')[:12]}` pin `{e['pin'][:12]}`")
    gh = data.get("github", {})
    if gh:
        lines += ["", "## GitHub", ""]
        for r in gh.get("new_releases", []):
            lines.append(f"- release {r['tag']} {r['published_at']}")
        for pr in gh.get("relevant_open_prs", []):
            lines.append(f"- PR #{pr['number']} {pr['title']} ({pr['user']}, {pr['head_repo']}, "
                         f"`{(pr['head_sha'] or '')[:8]}`) [{', '.join(pr['keywords'])}]")
        lines.append(f"- requests used: {gh.get('requests_used')}")
    return "\n".join(lines) + "\n"


def prepare_out(out, repo):
    out = Path(out).resolve()
    top = Path(git_out(["rev-parse", "--show-toplevel"], repo)).resolve()
    if out == top or top in out.parents:
        raise Failure(f"--out must be outside the checkout ({top})")
    if out.exists() and not out.is_dir():
        raise Failure(f"--out {out} is not a directory")
    out.mkdir(parents=True, exist_ok=True)
    return out


def write_report(out, data):
    for name, text in (("report.json", json.dumps(data, indent=2, sort_keys=True) + "\n"),
                       ("report.md", render_markdown(data))):
        tmp = out / (name + ".tmp")
        tmp.write_text(text, encoding="utf-8")
        tmp.replace(out / name)


def run(base, repo, out=None, fetch=False, github=False, allow_file=False):
    report = Report()
    report.data["base_recorded_at"] = base.get("recorded_at")
    try:
        local_check(repo, base, report, allow_file=allow_file)
        if fetch:
            fetch_check(base, out, report, allow_file=allow_file)
        if github:
            github_check(base, report, fetched=fetch)
    except Failure as e:
        report.add("failure", str(e))
    return report


# --------------------------------------------------------------- self-test

def self_test(scratch):
    scratch = Path(tempfile.mkdtemp(prefix="upstream-selftest-", dir=scratch))
    env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
               GIT_AUTHOR_NAME="Synthetic", GIT_AUTHOR_EMAIL="synthetic@example.invalid",
               GIT_COMMITTER_NAME="Synthetic", GIT_COMMITTER_EMAIL="synthetic@example.invalid")
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(env)
    checks = []

    def ok(name, cond):
        checks.append((name, bool(cond)))
        print(f"{'PASS' if cond else 'FAIL'} {name}")

    def commit(repo, path, text, msg):
        f = repo / path
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text(text)
        git(["add", path], repo)
        git(["commit", "-q", "-m", msg], repo)
        return git_out(["rev-parse", "HEAD"], repo)

    try:
        up = scratch / "upstream"
        up.mkdir()
        git(["init", "-q", "-b", "main"], up)
        commit(up, "core/a.txt", "1", "core: one\n\nSigned-off-by: A <a@example.invalid>")
        main_pin = commit(up, "core/a.txt", "2", "core: two\n\nSigned-off-by: A <a@example.invalid>")
        git(["checkout", "-q", "-b", "pr"], up)
        pr_pin = commit(up, "desktop/mac.txt", "m", "mac: port\n\nSigned-off-by: B <b@example.invalid>")
        git(["update-ref", "refs/pull/1/head", pr_pin], up)
        git(["checkout", "-q", "main"], up)
        url = up.as_uri()

        port = scratch / "port"
        git(["clone", "-q", "--branch", "pr", url, str(port)], scratch,
            pre=["-c", "protocol.file.allow=always"])
        git(["remote", "rename", "origin", "upstream"], port)
        commit(port, "brand/logo.txt", "w", "brand: waypair")
        commit(port, "core/fix.txt", "f", "core: generic fix")
        (port / "core" / "mixed.txt").write_text("m")
        git(["add", "core/mixed.txt"], port)
        commit(port, "brand/name.txt", "n", "mixed: brand and core")
        commit(port, "core/fix.txt", "f2", "core: second fix")
        port_head = git_out(["rev-parse", "HEAD"], port)

        base = {
            "schema": 1, "recorded_at": "synthetic",
            "port": {"branch": "pr", "base_pin": "pr1"},
            "upstream": {"remote": "upstream", "url": url, "repo": "synthetic/repo", "branch": "main"},
            "pins": [
                {"id": "main", "kind": "branch", "ref": "refs/heads/main", "sha": main_pin, "in_port": True},
                {"id": "pr1", "kind": "pull", "number": 1, "ref": "refs/pull/1/head", "sha": pr_pin, "in_port": True},
            ],
            "port_deltas": {"branding": ["brand/"]},
        }
        out = scratch / "out"
        out.mkdir()

        r = run(base, port, out, fetch=True, allow_file=True)
        ok("clean baseline has no failure or drift", not ({"failure", "drift"} & r.levels()))
        ok("port commits counted", r.data["local"]["port_commits"]["total"] == 4)
        cats = {c["subject"]: c["categories"] for c in r.data["local"]["port_commits"]["commits"]}
        ok("branding commit classified", cats["brand: waypair"] == ["branding"])
        ok("generic commit classified", cats["core: generic fix"] == ["generic"])
        ok("mixed commit classified", cats["mixed: brand and core"] == ["branding", "generic"])
        ok("no new upstream commits", r.data["fetch"]["new_on_main"]["total"] == 0)

        new_main = commit(up, "core/a.txt", "3", "core: three (unsigned)")
        git(["checkout", "-q", "pr"], up)
        pr_new = commit(up, "desktop/mac.txt", "m2", "mac: follow-up\n\nSigned-off-by: B <b@example.invalid>")
        git(["update-ref", "refs/pull/1/head", pr_new], up)
        git(["checkout", "-q", "main"], up)
        r = run(base, port, out, fetch=True, allow_file=True)
        new = r.data["fetch"]["new_on_main"]
        ok("new upstream commit detected", new["total"] == 1 and new["commits"][0]["sha"] == new_main)
        ok("missing Signed-off-by flagged", new["commits"][0]["signed_off_by"] is None)
        ok("PR head move detected", r.data["fetch"]["pulls"]["pr1"].get("new_commits", {}).get("total") == 1)
        ok("drift reported without failure", "drift" in r.levels() and "failure" not in r.levels())

        git(["reset", "-q", "--hard", git_out(["rev-parse", "HEAD~2"], up)], up)
        commit(up, "core/a.txt", "x", "rewrite")
        r = run(base, port, out, fetch=True, allow_file=True)
        ok("rewritten upstream main is a failure",
           any("rewritten" in f["message"] for f in r.data["findings"] if f["level"] == "failure"))

        bad = json.loads(json.dumps(base))
        bad["upstream"]["url"] = (scratch / "missing").as_uri()
        (out / "bad").mkdir()
        r = run(bad, port, out / "bad", fetch=True, allow_file=True)
        ok("remote mismatch is a failure", any("expected" in f["message"] for f in r.data["findings"]))
        ok("unreachable upstream is a failure",
           any("failed" in f["message"] for f in r.data["findings"] if f["level"] == "failure"))

        bad = json.loads(json.dumps(base))
        bad["pins"][0]["sha"] = "0" * 40
        r = run(bad, port)
        ok("missing in_port pin is a failure",
           any("missing" in f["message"] for f in r.data["findings"] if f["level"] == "failure"))

        bad = json.loads(json.dumps(base))
        bad["upstream"]["url"] = "https://user:token@example.invalid/x.git"
        r = run(bad, port, out, fetch=True, allow_file=False)
        ok("credential URL refused", any("embedded credentials" in f["message"] for f in r.data["findings"]))

        try:
            prepare_out(port / "scratch", port)
            ok("--out inside checkout refused", False)
        except Failure:
            ok("--out inside checkout refused", True)

        env_net = network_env()
        ok("network env disables prompts", env_net["GIT_TERMINAL_PROMPT"] == "0"
           and env_net["GCM_INTERACTIVE"] == "never")
        ok("checkout untouched", git_out(["rev-parse", "HEAD"], port) == port_head
           and git_out(["status", "--porcelain"], port) == "")
    finally:
        os.environ.clear()
        os.environ.update(saved)
        shutil.rmtree(scratch, ignore_errors=True)
    failed = [n for n, passed in checks if not passed]
    print(f"self-test: {len(checks) - len(failed)}/{len(checks)} passed")
    return EXIT_OK if not failed else EXIT_FAILURE


# --------------------------------------------------------------------- main

def main(argv=None):
    ap = argparse.ArgumentParser(description="Read-only upstream drift report (see docs/waypair/UPSTREAM_SYNC.md).")
    ap.add_argument("--base", default=str(DEFAULT_BASE), help="provenance JSON (default: %(default)s)")
    ap.add_argument("--repo", default=str(ROOT), help="checkout to inspect read-only")
    ap.add_argument("--out", help="scratch directory outside the checkout for report.{json,md} and mirror.git")
    ap.add_argument("--fetch", action="store_true", help="fetch upstream main and pinned PR heads into --out/mirror.git")
    ap.add_argument("--github", action="store_true", help="bounded unauthenticated GitHub API GETs")
    ap.add_argument("--strict", action="store_true", help=f"exit {EXIT_DRIFT} when drift is found")
    ap.add_argument("--self-test", action="store_true", help="offline synthetic test; uses --out or the system temp dir")
    args = ap.parse_args(argv)

    if args.self_test:
        scratch = Path(args.out).resolve() if args.out else None
        if scratch:
            scratch.mkdir(parents=True, exist_ok=True)
        return self_test(scratch)
    if args.fetch and not args.out:
        ap.error("--fetch needs --out (the mirror lives there, never in the checkout)")
    try:
        base = load_base(args.base)
        out = prepare_out(args.out, args.repo) if args.out else None
    except Failure as e:
        print(f"failure: {e}", file=sys.stderr)
        return EXIT_FAILURE
    report = run(base, args.repo, out, fetch=args.fetch, github=args.github)
    if out:
        write_report(out, report.data)
        print(f"wrote {out / 'report.md'} and {out / 'report.json'}")
    print(render_markdown(report.data), end="")
    levels = report.levels()
    if "failure" in levels:
        return EXIT_FAILURE
    if args.strict and "drift" in levels:
        return EXIT_DRIFT
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
