import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import waypair_obsidian_sync as sync

NOTE = """---
type: "feature"
feature_id: "demo"
status: "active"
priority: "P0"
platforms: ["mac"]
horizon: "now"
owner: "root"
updated: "2026-10-19"
tags: ["waypair","feature"]
---
# Demo

Summary written by a person.

## Next action and acceptance

- [ ] Owner acceptance on device — **open**

## Delivery evidence

| Stage | State |
|---|---|
| source | done |
| tests | open |
| built | open |
| installed | open |
| published | open |
| accepted | open |

## Limits

- Hand-written limit.

## Sources


## Alex’s notes

Keep this.

[[Waypair/Roadmap|Back to roadmap]]
"""


def roadmap(**over):
    feature = {
        "id": "demo", "title": "Demo", "status": "published", "priority": "P2", "platforms": ["mac"],
        "horizon": "next", "lane": None,
        "ladder": {"source": "done", "tests": "done", "built": "done", "installed": "done",
                   "published": "done", "accepted": "done"},
        "gates": [{"label": "Owner acceptance on device", "state": "pass"},
                  {"label": "Focused tests", "state": "pass", "note": "4 cases"}],
        "limits": ["Emulator only."],
        "evidence": [{"label": "Receipt", "path": "~/orca/tmp/demo/RESULT.json"}],
    }
    feature.update(over)
    return {"snapshot": {"revision": "r1", "asOf": "2026-10-20T09:56:00Z"}, "features": [feature],
            "workers": [{"lane": "Demo lane", "state": "implementing", "card": "demo", "dispatch": "ctx_aaaaaaaaaaaa"}]}


CHECKPOINT = {"lastUpdated": "2026-10-20T10:44:50.616757+00:00",
              "wave30": {"active": [{"task": "task_bbbbbbbbbbbb", "dispatch": "ctx_aaaaaaaaaaaa",
                                     "provider": {"id": "cursor"}, "state": {"dispatch": "dispatched"}}]}}
TASKS = {"result": {"tasks": [
    {"id": "task_bbbbbbbbbbbb", "status": "dispatched", "display_name": "Demo task", "spec": "never copied",
     "created_at": "2026-10-20 09:20:49"},
    {"id": "task_cccccccccccc", "status": "completed", "display_name": "Done task", "created_at": "2026-10-20 08:00:00",
     "completed_at": "2026-10-20T09:00:00.000Z", "result": json.dumps({"outcome": "succeeded"})}]}}


def run(*argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = sync.main(list(argv))
    return code, out.getvalue() + err.getvalue()


class SyncTest(unittest.TestCase):
    def fixture(self, tmp: Path, data: dict) -> list[str]:
        vault = tmp / "vault"
        (vault / "Waypair/Features").mkdir(parents=True)
        (vault / "AGENTS.md").write_text("# fixture\n")
        (vault / "Waypair/Features/demo.md").write_text(NOTE)
        for name, obj in (("roadmap.json", data), ("swarm.json", CHECKPOINT), ("tasks.json", TASKS)):
            (tmp / name).write_text(json.dumps(obj))
        return ["--vault", str(vault), "--roadmap", str(tmp / "roadmap.json"),
                "--checkpoint", str(tmp / "swarm.json"), "--tasks", str(tmp / "tasks.json")]

    def test_dry_run_apply_guards_and_idempotence(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            args = self.fixture(tmp, roadmap())
            note = tmp / "vault/Waypair/Features/demo.md"

            code, out = run(*args)
            self.assertEqual(code, 0, out)
            self.assertEqual(note.read_text(), NOTE)
            self.assertFalse((tmp / "vault/Waypair/Workers.md").exists())

            # Without fingerprints every difference counts as a hand edit and is kept.
            code, out = run(*args, "--apply")
            self.assertEqual(code, 0, out)
            self.assertIn("kept: section 'Limits' kept (hand-edited)", out)
            self.assertIn('horizon: "now"', note.read_text())

            code, out = run(*args, "--apply", "--take-evidence", "demo")
            self.assertEqual(code, 0, out)
            text = note.read_text()
            self.assertIn('status: "active"', text)          # publication is never set by the tool
            self.assertIn("held: status stays 'active'", out)
            self.assertIn("| published | open |", text)
            self.assertIn("| accepted | open |", text)
            self.assertIn("- [ ] Owner acceptance on device — **open**", text)
            self.assertIn("- [x] Focused tests — **pass**: 4 cases", text)
            self.assertIn('horizon: "next"', text)
            self.assertIn('priority: "P0"', text)            # manual priority kept
            self.assertIn("note: priority P0 is manual", out)
            self.assertIn("Summary written by a person.", text)
            self.assertIn("## Alex’s notes\n\nKeep this.", text)

            workers = (tmp / "vault/Waypair/Workers.md").read_text()
            self.assertIn("| — | task_bbbbbbbbbbbb | ctx_aaaaaaaaaaaa | cursor | dispatched | dispatched | Demo task |", workers)
            self.assertIn("| task_cccccccccccc | completed | succeeded | Done task | 2026-10-20 09:00 |", workers)
            self.assertNotIn("never copied", workers)

            # A later hand edit survives; untouched evidence-owned items keep flowing.
            note.write_text(note.read_text().replace("- Emulator only.", "- Emulator only.\n- Alex: also Fold."))
            (tmp / "roadmap.json").write_text(json.dumps(roadmap(horizon="later", limits=["Changed."])))
            code, out = run(*args, "--apply")
            self.assertEqual(code, 0, out)
            text = note.read_text()
            self.assertIn('horizon: "later"', text)
            self.assertIn("- Alex: also Fold.", text)
            self.assertNotIn("- Changed.", text)

            code, out = run(*args, "--apply")
            self.assertEqual(code, 0, out)
            self.assertIn("0 note(s) written", out)
            self.assertEqual(note.read_text(), text)

    def test_refuses_secret_or_missing_agents_without_writing(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            args = self.fixture(tmp, roadmap(limits=["Ping alex@example.com for the pairing code 123456"]))
            code, out = run(*args, "--apply")
            self.assertEqual(code, 2)
            self.assertIn("looks like a", out)
            self.assertNotIn("example.com", out)
            self.assertEqual((tmp / "vault/Waypair/Features/demo.md").read_text(), NOTE)
            self.assertFalse((tmp / "vault/Waypair/Workers.md").exists())

            (tmp / "vault/AGENTS.md").unlink()
            code, out = run(*args)
            self.assertEqual(code, 2)
            self.assertIn("no AGENTS.md", out)


if __name__ == "__main__":
    unittest.main()
