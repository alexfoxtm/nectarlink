#!/usr/bin/env python3
"""Refresh evidence-owned parts of the Waypair notes in an Obsidian vault.

Standard library only; reads local files and never touches the network.

Feature notes (Waypair/Features/<id>.md) get their evidence-owned frontmatter
(status, horizon, platforms, owner, updated) and four sections (acceptance gates,
delivery ladder, limits, sources) refreshed from roadmap.json. Waypair/Workers.md
is generated from roadmap.json workers, an ACTIVE-SWARM checkpoint and an Orca
task-list receipt.

Dry run by default: prints a report and unified diffs. --apply writes.

Ownership rules:
  - Everything not listed above (title, summary, priority, tags, "Alex’s notes",
    any other section or property) is never written.
  - Each written item is fingerprinted in a trailing %% waypair-sync … %% comment.
    An item whose text no longer matches its fingerprint was edited by hand and is
    kept unless --take-evidence names that note.
  - Publication and acceptance are never changed by this tool in either direction:
    status "published", the published/accepted ladder rows and gates about
    acceptance, publication or the owner keep the note's value; disagreements are
    reported for a person to resolve.
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path

FEATURE_DIR = Path("Waypair/Features")
WORKERS_NOTE = Path("Waypair/Workers.md")
ALEX_HEADING = "## Alex’s notes"
STATE_RE = re.compile(r"^%% waypair-sync (\{.*\}) %%[ \t]*$", re.M)

STATUSES = {"blocked", "active", "review", "hold", "candidate", "source", "published", "queued", "deferred"}
HORIZONS = {"now", "next", "later"}
LADDER_KEYS = ("source", "tests", "built", "installed", "published", "accepted")
LADDER_VALUES = {"done", "partial", "open", "na"}
GATE_STATES = {"pass", "open", "fail", "unperformed", "hold"}
GUARDED_LADDER = ("published", "accepted")
GUARDED_GATE_RE = re.compile(r"(?i)\b(accept\w*|publish\w*|owner)\b")
ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,47}$")
SETTLED = {"completed", "failed", "cancelled", "canceled"}

SEC_GATES = "Next action and acceptance"
SEC_LADDER = "Delivery evidence"
SEC_LIMITS = "Limits"
SEC_SOURCES = "Sources"

PRIVACY_PATTERNS = [
    (re.compile(r"(?i)\b(password|passwd|passphrase|secret|api[_-]?key|bearer|access[_-]?token|refresh[_-]?token)\b"), "credential word"),
    (re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"), "private key"),
    (re.compile(r"\b(sk|pk|ghp|gho|ghs|xox[abp])[-_][A-Za-z0-9]{12,}"), "token-like string"),
    (re.compile(r"\bAKIA[0-9A-Z]{16}\b"), "cloud access key"),
    (re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\."), "JWT"),
    (re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+"), "email address"),
    (re.compile(r"\+\d[\d ()-]{7,}\d"), "phone number"),
    (re.compile(r"\b\d{3}[ .-]\d{3}[ .-]\d{4}\b"), "phone number"),
    (re.compile(r"\b\d{6}\b"), "6-digit code"),
    (re.compile(r"\b\d{14,17}\b"), "device identifier"),
    (re.compile(r"(?i)\b(imei|meid|iccid|imsi|serial ?number)\b"), "device identifier"),
    (re.compile(r"(?i)\b([0-9a-f]{2}:){5}[0-9a-f]{2}\b"), "hardware address"),
]
FORBIDDEN_PATH_PARTS = ("..", "/.ssh", "/Library/Keychains", "-AX.json", ".pem", ".key", "id_rsa", "id_ed25519")


class Refused(Exception):
    pass


# ---------------------------------------------------------------- inputs


def _load_json(path: Path, what: str):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError) as exc:
        raise Refused(f"{what} {path}: {exc}") from None


def validate_roadmap(data) -> None:
    if not isinstance(data, dict) or not isinstance(data.get("features"), list):
        raise Refused("roadmap: expected an object with a features list")
    snap = data.get("snapshot") or {}
    if not re.match(r"^\d{4}-\d{2}-\d{2}T", str(snap.get("asOf", ""))):
        raise Refused("roadmap.snapshot.asOf: expected an ISO timestamp")
    for i, f in enumerate(data["features"]):
        where = f"roadmap.features[{i}]"
        if not isinstance(f, dict) or not ID_RE.match(str(f.get("id", ""))):
            raise Refused(f"{where}: bad id")
        if f.get("status") not in STATUSES:
            raise Refused(f"{where}.status: unknown {f.get('status')!r}")
        if f.get("horizon") not in HORIZONS:
            raise Refused(f"{where}.horizon: unknown {f.get('horizon')!r}")
        ladder = f.get("ladder") or {}
        if set(ladder) != set(LADDER_KEYS) or not set(ladder.values()) <= LADDER_VALUES:
            raise Refused(f"{where}.ladder: needs {', '.join(LADDER_KEYS)} with {sorted(LADDER_VALUES)}")
        for g in f.get("gates") or []:
            if g.get("state") not in GATE_STATES or not g.get("label"):
                raise Refused(f"{where}.gates: bad gate {g!r:.80}")
        for e in f.get("evidence") or []:
            if any(p in str(e.get("path", "")) for p in FORBIDDEN_PATH_PARTS):
                raise Refused(f"{where}.evidence: path not allowed in a note")


def pick_wave(checkpoint: dict, wave: str | None) -> str | None:
    if wave:
        if not isinstance(checkpoint.get(wave), dict):
            raise Refused(f"checkpoint has no {wave} object")
        return wave
    waves = [k for k in checkpoint if re.fullmatch(r"wave\d+", k) and isinstance(checkpoint[k], dict)]
    return max(waves, key=lambda k: int(k[4:]), default=None)


def load_tasks(obj) -> list[dict]:
    if isinstance(obj, dict):
        obj = (obj.get("result") if isinstance(obj.get("result"), dict) else obj).get("tasks")
    if not isinstance(obj, list):
        raise Refused("task list: expected `orca orchestration task-list --json` output")
    out = []
    for t in obj:
        if not isinstance(t, dict) or not re.fullmatch(r"task_[0-9a-f]{6,32}", str(t.get("id", ""))):
            continue
        outcome = None
        try:
            outcome = json.loads(t.get("result") or "null")
            outcome = outcome.get("outcome") if isinstance(outcome, dict) else None
        except ValueError:
            pass
        out.append({
            "id": t["id"],
            "status": _word(t.get("status")),
            "title": t.get("display_name") or t.get("task_title") or "",
            "dispatch": t.get("dispatch_id") if re.fullmatch(r"ctx_[0-9a-f]{6,32}", str(t.get("dispatch_id"))) else None,
            "created": _stamp(t.get("created_at")),
            "completed": _stamp(t.get("completed_at")),
            "outcome": _word(outcome),
        })
    return out


def _word(value) -> str | None:
    return value if isinstance(value, str) and re.fullmatch(r"[a-z_]{1,24}", value) else None


def _stamp(value) -> str | None:
    m = re.match(r"^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2})", str(value or ""))
    return f"{m[1]} {m[2]}" if m else None


# ---------------------------------------------------------------- rendering


def inline(text) -> str:
    return re.sub(r"\s+", " ", str(text or "")).replace("%%", "%").strip()


def cell(text) -> str:
    return inline(text).replace("|", "\\|") or "—"


def link(card: str | None) -> str:
    return f"[[{FEATURE_DIR.as_posix()}/{card}|{card}]]" if card and ID_RE.match(card) else "—"


def home(path) -> str:
    p = str(path)
    h = str(Path.home())
    return "~" + p[len(h):] if p.startswith(h + "/") else p


def render_gates(gates) -> str:
    lines = []
    for g in gates:
        mark = "x" if g["state"] == "pass" else " "
        note = f": {inline(g['note'])}" if g.get("note") else ""
        lines.append(f"- [{mark}] {inline(g['label'])} — **{g['state']}**{note}")
    return "\n".join(lines)


def render_ladder(ladder) -> str:
    rows = [f"| {k} | {ladder[k]} |" for k in LADDER_KEYS]
    return "\n".join(["| Stage | State |", "|---|---|", *rows])


def render_list(items) -> str:
    return "\n".join(f"- {inline(x)}" for x in items)


def render_sources(evidence) -> str:
    return "\n".join(f"- {inline(e['label'])}: `{inline(e['path']).replace('`', '')}`" for e in evidence)


def table(head: list[str], rows: list[list], empty: str) -> str:
    if not rows:
        return empty
    out = ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out)


# ---------------------------------------------------------------- guards


def parse_gates(text: str) -> dict[str, str]:
    found = {}
    for m in re.finditer(r"^- \[[ xX]\] (.+?) — \*\*(\w+)\*\*", text, re.M):
        found[m[1].strip()] = m[2]
    return found


def parse_ladder(text: str) -> dict[str, str]:
    return {m[1]: m[2] for m in re.finditer(r"^\|\s*(\w+)\s*\|\s*(\w+)\s*\|", text, re.M) if m[1] in LADDER_KEYS}


def guard_feature(feature: dict, note_status, gates_text: str | None, ladder_text: str | None):
    """Return (status, ladder, gates, held) with publication/acceptance kept as the note has them."""
    held = []
    status = feature["status"]
    if (status == "published") != (note_status == "published"):
        held.append(f"status stays {note_status!r} (evidence says {status!r}); publication is set by a person")
        status = note_status if note_status in STATUSES else "candidate"
    ladder = dict(feature["ladder"])
    current = parse_ladder(ladder_text or "")
    for key in GUARDED_LADDER:
        mine = current.get(key, "open")
        if ladder[key] != mine:
            held.append(f"ladder {key} stays {mine} (evidence says {ladder[key]})")
            ladder[key] = mine
    gates = []
    current_gates = parse_gates(gates_text or "")
    for g in feature.get("gates") or []:
        g = dict(g)
        label = inline(g["label"])
        if GUARDED_GATE_RE.search(label):
            mine = current_gates.get(label, "open")
            if (g["state"] == "pass") != (mine == "pass"):
                held.append(f"gate {label!r} stays {mine} (evidence says {g['state']})")
                g["state"] = mine
        gates.append(g)
    return status, ladder, gates, held


# ---------------------------------------------------------------- note editing


def fingerprint(text: str) -> str:
    return hashlib.sha256(text.strip().encode("utf-8")).hexdigest()[:12]


def split_note(text: str):
    if not text.startswith("---\n"):
        raise Refused("note has no frontmatter")
    end = text.find("\n---\n", 4)
    if end < 0:
        raise Refused("note frontmatter is not closed")
    return text[4:end + 1], text[end + 5:]


def fm_get(fm: str, key: str):
    m = re.search(rf"^{re.escape(key)}:[ \t]*(.*)$", fm, re.M)
    if not m:
        return None
    raw = m[1].strip()
    if raw == "":
        items = []
        for line in fm[m.end() + 1:].splitlines():
            if not re.match(r"^\s*- ", line):
                break
            items.append(line.split("- ", 1)[1].strip().strip("\"'"))
        return items
    if raw[0] in "\"[":
        try:
            return json.loads(raw)
        except ValueError:
            return raw.strip("\"")
    return raw.strip("'")


def fm_set(fm: str, key: str, value) -> str:
    line = f"{key}: {json.dumps(value, ensure_ascii=False, separators=(',', ':'))}"
    pattern = re.compile(rf"^{re.escape(key)}:[^\n]*\n(?:[ \t]+[^\n]*\n|- [^\n]*\n)*", re.M)
    if pattern.search(fm):
        return pattern.sub(lambda _: line + "\n", fm, count=1)
    return fm + line + "\n"


def managed_end(body: str) -> int:
    """Sections are only searched before Alex's notes, the backlink and the state comment."""
    ends = [len(body)]
    for pat in (rf"^{re.escape(ALEX_HEADING)}\s*$", r"^## Alex's notes\s*$", r"^\[\[Waypair/Roadmap\|", STATE_RE.pattern):
        m = re.search(pat, body, re.M)
        if m:
            ends.append(m.start())
    return min(ends)


def section_span(body: str, heading: str):
    limit = managed_end(body)
    m = re.search(rf"^## {re.escape(heading)}[ \t]*\n", body[:limit], re.M)
    if not m:
        return None
    nxt = re.search(r"^#{1,2} ", body[m.end():limit], re.M)
    return m.end(), (m.end() + nxt.start() if nxt else limit)


def set_section(body: str, heading: str, content: str) -> str:
    block = f"\n{content}\n\n" if content else "\n\n"
    span = section_span(body, heading)
    if span:
        return body[:span[0]] + block + body[span[1]:]
    at = managed_end(body)
    return body[:at] + f"## {heading}\n{block}" + body[at:]


def read_state(body: str) -> dict:
    m = STATE_RE.search(body)
    if not m:
        return {}
    try:
        state = json.loads(m[1])
    except ValueError:
        return {}
    return state if isinstance(state, dict) else {}


def write_state(body: str, state: dict) -> str:
    line = "%% waypair-sync " + json.dumps(state, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + " %%"
    if STATE_RE.search(body):
        return STATE_RE.sub(lambda _: line, body, count=1)
    return body.rstrip("\n") + "\n\n" + line + "\n"


def reconcile(text: str, fm_targets: dict, sec_targets: dict, take: bool):
    """Apply owned targets unless hand-edited. Returns (new_text, kept)."""
    fm, body = split_note(text)
    state = read_state(body)
    last_fm, last_sec = state.get("fm", {}), state.get("sections", {})
    kept = []
    new_fm, new_state = {}, {"fm": {}, "sections": {}, "v": 1}
    for key, target in fm_targets.items():
        current = fm_get(fm, key)
        hand = current is not None and current != target and last_fm.get(key) != fingerprint(json.dumps(current))
        if hand and not take:
            kept.append(f"property {key}: {current!r} kept (evidence says {target!r})")
            if key in last_fm:
                new_state["fm"][key] = last_fm[key]
            continue
        new_fm[key] = target
        new_state["fm"][key] = fingerprint(json.dumps(target))
    for key, target in new_fm.items():
        if fm_get(fm, key) != target:
            fm = fm_set(fm, key, target)
    for heading, target in sec_targets.items():
        span = section_span(body, heading)
        current = body[span[0]:span[1]].strip() if span else None
        hand = current is not None and current != target.strip() and last_sec.get(heading) != fingerprint(current)
        if hand and not take:
            kept.append(f"section {heading!r} kept (hand-edited)")
            if heading in last_sec:
                new_state["sections"][heading] = last_sec[heading]
            continue
        if current != target.strip():
            body = set_section(body, heading, target)
        new_state["sections"][heading] = fingerprint(target)
    body = write_state(body, new_state)
    return f"---\n{fm}---\n{body}", kept


# ---------------------------------------------------------------- notes


def feature_targets(feature: dict, snapshot: dict, note_text: str):
    fm, body = split_note(note_text)
    span = section_span(body, SEC_GATES)
    gates_text = body[span[0]:span[1]] if span else None
    span = section_span(body, SEC_LADDER)
    ladder_text = body[span[0]:span[1]] if span else None
    status, ladder, gates, held = guard_feature(feature, fm_get(fm, "status"), gates_text, ladder_text)
    fm_targets = {
        "status": status,
        "horizon": feature["horizon"],
        "platforms": list(feature.get("platforms") or []),
        "owner": feature.get("lane") or "Unassigned",
        "updated": snapshot["asOf"][:10],
    }
    sec_targets = {
        SEC_GATES: render_gates(gates),
        SEC_LADDER: render_ladder(ladder),
        SEC_LIMITS: render_list(feature.get("limits") or []),
        SEC_SOURCES: render_sources(feature.get("evidence") or []),
    }
    advisories = []
    if fm_get(fm, "priority") not in (None, feature.get("priority")):
        advisories.append(f"priority {fm_get(fm, 'priority')} is manual; evidence says {feature.get('priority')} (kept)")
    return fm_targets, sec_targets, held, advisories


def workers_targets(roadmap: dict, checkpoint: dict | None, wave: str | None, tasks: list[dict] | None,
                    checkpoint_path, tasks_path):
    snapshot = roadmap["snapshot"]
    by_task = {t["id"]: t for t in tasks or []}
    card_by_dispatch = {w.get("dispatch"): w.get("card") for w in roadmap.get("workers") or [] if w.get("dispatch")}
    dates = [snapshot["asOf"][:10]]

    lanes = []
    if checkpoint and wave:
        for a in checkpoint[wave].get("active") or []:
            if not isinstance(a, dict):
                continue
            t = by_task.get(a.get("task"), {})
            dispatch = a.get("dispatch")
            if t.get("dispatch") and t["dispatch"] != dispatch:
                dispatch = f"{dispatch} (task list: {t['dispatch']})"
            provider = a.get("provider") if isinstance(a.get("provider"), dict) else {}
            agent = provider.get("id") or a.get("model")
            state = a.get("state") if isinstance(a.get("state"), dict) else {}
            state = " / ".join(str(state[k]) for k in ("dispatch", "activity") if state.get(k))
            lanes.append([cell(a.get("lane")), cell(a.get("task")), cell(dispatch), cell(agent), cell(state),
                          cell(t.get("status") or "not in receipt"), cell(t.get("title")),
                          link(card_by_dispatch.get(a.get("dispatch")))])
        if _stamp(checkpoint.get("lastUpdated")):
            dates.append(_stamp(checkpoint["lastUpdated"])[:10])
    workers = [[cell(w.get("lane")), cell(w.get("state")), link(w.get("card")), cell(w.get("dispatch")), cell(w.get("note"))]
               for w in roadmap.get("workers") or []]
    open_rows, settled_rows = [], []
    if tasks:
        for t in sorted((t for t in tasks if t["status"] not in SETTLED), key=lambda t: t["created"] or ""):
            open_rows.append([cell(t["id"]), cell(t["status"]), cell(t["title"][:90]), cell(t["created"])])
        settled = sorted((t for t in tasks if t["status"] in SETTLED and t["completed"]), key=lambda t: t["completed"], reverse=True)
        for t in settled[:8]:
            settled_rows.append([cell(t["id"]), cell(t["status"]), cell(t["outcome"]), cell(t["title"][:90]), cell(t["completed"])])
        newest = max((t["completed"] or t["created"] or "" for t in tasks), default="")
        if newest:
            dates.append(newest[:10])

    sources = [f"- Roadmap data: revision `{inline(snapshot.get('revision'))}`, as of {_stamp(snapshot['asOf'])} UTC"]
    if checkpoint is not None:
        sources.append(f"- Checkpoint: `{home(checkpoint_path)}`, {wave or 'no wave'}, last updated {_stamp(checkpoint.get('lastUpdated')) or 'unknown'} UTC")
        if wave and checkpoint[wave].get("publishedBaseline"):
            sources.append(f"- Published baseline (checkpoint): {inline(checkpoint[wave]['publishedBaseline'])}")
    if tasks is not None:
        sources.append(f"- Orca task list: `{home(tasks_path)}`, {len(tasks)} tasks")
    sources.append("- A worker_done or a task status is not proof of build, installation, publication or acceptance.")

    fm_targets = {"updated": max(dates)}
    sec_targets = {
        "Active lanes": table(["Lane", "Task", "Dispatch", "Agent", "Checkpoint state", "Task-list status", "Title", "Feature"], lanes,
                              "No active lanes in the checkpoint."),
        "Roadmap workers": table(["Worker", "State", "Feature", "Dispatch", "Note"], workers, "No workers in the roadmap data."),
        "Open tasks": table(["Task", "Status", "Title", "Created"], open_rows, "No open tasks in the receipt."),
        "Recently settled": table(["Task", "Status", "Outcome", "Title", "Settled"], settled_rows, "No settled tasks in the receipt."),
        "Sources": "\n".join(sources),
    }
    return fm_targets, sec_targets


WORKERS_SKELETON = """---
type: "workers"
generated_by: "scripts/waypair-roadmap"
tags: ["waypair","workers"]
vault: "global"
---
# Waypair — workers and tasks

Generated from the ACTIVE-SWARM checkpoint, the Orca task list and the roadmap data. Edit the roadmap data, not these tables.

## Alex’s notes


[[Waypair/Roadmap|Back to roadmap]]
"""


def privacy_scan(where: str, targets: dict) -> None:
    for name, value in targets.items():
        text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False)
        for pattern, what in PRIVACY_PATTERNS:
            if pattern.search(text):
                raise Refused(f"{where} [{name}]: generated text looks like a {what}; fix the source data")


# ---------------------------------------------------------------- main


def check_vault(vault: Path) -> Path:
    vault = vault.expanduser().resolve()
    if not (vault / "AGENTS.md").is_file():
        raise Refused(f"{vault} has no AGENTS.md; pass the vault root explicitly")
    features = (vault / FEATURE_DIR).resolve()
    if not features.is_dir() or vault not in features.parents:
        raise Refused(f"{vault / FEATURE_DIR} is missing or outside the vault")
    return vault


def plan(args) -> tuple[Path, list[tuple[Path, str, str]], list[str]]:
    vault = check_vault(args.vault)
    roadmap = _load_json(args.roadmap, "roadmap")
    validate_roadmap(roadmap)
    checkpoint = _load_json(args.checkpoint, "checkpoint") if args.checkpoint else None
    wave = pick_wave(checkpoint, args.wave) if checkpoint else None
    tasks = load_tasks(_load_json(args.tasks, "task list")) if args.tasks else None
    features = {f["id"]: f for f in roadmap["features"]}
    take = set(args.take_evidence or [])

    changes, report, seen = [], [], set()
    for path in sorted((vault / FEATURE_DIR).glob("*.md")):
        rel = path.relative_to(vault)
        old = path.read_text(encoding="utf-8")
        try:
            fid = fm_get(split_note(old)[0], "feature_id")
        except Refused as exc:
            report.append(f"{rel}: skipped ({exc})")
            continue
        if fid not in features:
            report.append(f"{rel}: no roadmap data for feature_id {fid!r}; untouched")
            continue
        seen.add(fid)
        fm_t, sec_t, held, advisories = feature_targets(features[fid], roadmap["snapshot"], old)
        privacy_scan(str(rel), {**fm_t, **sec_t})
        new, kept = reconcile(old, fm_t, sec_t, fid in take or "all" in take)
        changes.append((path, old, new))
        report.append(f"{rel}: {'changed' if new != old else 'unchanged'}")
        report += [f"  held: {h}" for h in held] + [f"  kept: {k}" for k in kept] + [f"  note: {a}" for a in advisories]
    for fid in sorted(set(features) - seen):
        report.append(f"{FEATURE_DIR / (fid + '.md')}: no note (Root creates feature notes); skipped")

    rel = Path(args.workers_note)
    path = (vault / rel).resolve()
    if path.suffix != ".md" or (vault / "Waypair").resolve() not in path.parents:
        raise Refused(f"--workers-note must be a .md file under Waypair/: {rel}")
    old = path.read_text(encoding="utf-8") if path.exists() else ""
    fm_t, sec_t = workers_targets(roadmap, checkpoint, wave, tasks, args.checkpoint, args.tasks)
    privacy_scan(str(rel), {**fm_t, **sec_t})
    new, kept = reconcile(old or WORKERS_SKELETON, fm_t, sec_t, "workers" in take or "all" in take)
    changes.append((path, old, new))
    report.append(f"{rel}: {'created' if not old else 'changed' if new != old else 'unchanged'}")
    report += [f"  kept: {k}" for k in kept]
    return vault, changes, report


def unchanged_since_plan(path: Path, old: str) -> None:
    current = path.read_text(encoding="utf-8") if path.exists() else ""
    if current != old:
        raise Refused(f"{path} changed while planning; rerun")


def write_atomic(path: Path, old: str, new: str) -> None:
    unchanged_since_plan(path, old)
    fd, tmp = tempfile.mkstemp(prefix=".waypair-sync-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(new)
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--vault", required=True, type=Path, help="Obsidian vault root (must contain AGENTS.md)")
    ap.add_argument("--roadmap", required=True, type=Path, help="roadmap.json")
    ap.add_argument("--checkpoint", type=Path, help="ACTIVE-SWARM.json")
    ap.add_argument("--wave", help="checkpoint wave key (default: highest waveNN)")
    ap.add_argument("--tasks", type=Path, help="saved `orca orchestration task-list --json` output")
    ap.add_argument("--workers-note", default=WORKERS_NOTE.as_posix(),
                    help=f"vault-relative workers note under Waypair/ (default: {WORKERS_NOTE.as_posix()})")
    ap.add_argument("--take-evidence", action="append", metavar="ID",
                    help="replace hand-edited items in this feature note (or 'workers', or 'all'); repeatable")
    ap.add_argument("--apply", action="store_true", help="write the notes (default: dry run)")
    ap.add_argument("--quiet", action="store_true", help="omit diffs")
    args = ap.parse_args(argv)
    try:
        vault, changes, report = plan(args)
    except Refused as exc:
        print(f"refused: {exc}", file=sys.stderr)
        return 2
    print("mode: " + ("apply" if args.apply else "dry run (nothing written; add --apply to write)"))
    print("\n".join(report))
    changed = [(p, o, n) for p, o, n in changes if o != n]
    if not args.quiet:
        for path, old, new in changed:
            rel = path.relative_to(vault).as_posix()
            sys.stdout.writelines(difflib.unified_diff(old.splitlines(True), new.splitlines(True), f"a/{rel}", f"b/{rel}"))
    if args.apply:
        try:
            for path, old, _ in changed:
                unchanged_since_plan(path, old)
            for path, old, new in changed:
                write_atomic(path, old, new)
        except Refused as exc:
            print(f"refused: {exc}", file=sys.stderr)
            return 2
    print(f"{len(changed)} note(s) {'written' if args.apply else 'would change'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
