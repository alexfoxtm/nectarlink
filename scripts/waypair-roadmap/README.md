# Waypair roadmap → Obsidian sync

`waypair_obsidian_sync.py` refreshes the evidence-owned parts of the Waypair notes in the global Obsidian vault. It uses only the Python standard library, reads local files, and never makes network calls. It is a dry run unless you pass `--apply`.

## Update command

Save a fresh task list first, run a dry run, read the report, then apply:

```sh
VAULT="$HOME/Library/Mobile Documents/iCloud~md~obsidian/Documents/global"
orca orchestration task-list --brief --json > ~/orca/tmp/<wave-dir>/TASKS-NOW.json

python3 scripts/waypair-roadmap/waypair_obsidian_sync.py \
  --vault "$VAULT" \
  --roadmap ~/orca/tmp/waypair-roadmap-thgudgbg/roadmap.json \
  --checkpoint ~/orca/tmp/waypair-feature-expansion-20261018/ACTIVE-SWARM.json \
  --tasks ~/orca/tmp/<wave-dir>/TASKS-NOW.json

# same command plus --apply once the report and diffs look right
```

| Option | Meaning |
| --- | --- |
| `--vault` | Required. The vault root, which must contain `AGENTS.md` and `Waypair/Features/`. There is no default. |
| `--roadmap` | Required. `roadmap.json`, the canonical feature data. |
| `--checkpoint` | Optional. `ACTIVE-SWARM.json`. Its highest `waveNN.active` list fills the active-lanes table. Use `--wave wave30` to pick a different wave. |
| `--tasks` | Optional. Saved output of `orca orchestration task-list --json`. Only the id, status, title, dispatch, timestamps and outcome are read; specs and result bodies are never copied. |
| `--workers-note PATH` | Vault-relative workers note under `Waypair/`. The default is `Waypair/Workers.md`. |
| `--take-evidence ID` | Replace hand-edited items in that feature note. `workers` targets the workers note and `all` targets every note. Repeatable. Publication and acceptance guards still apply. |
| `--apply` | Write the notes. Without it nothing is written. |
| `--quiet` | Print the report without unified diffs. |

Exit code 0 means success. Exit code 2 means the tool refused before writing, for example because the vault has no `AGENTS.md`, the input is invalid, generated text looks private, or a note changed after it was read. Every note is rechecked just before the batch is written, and each file is replaced atomically.

## What it writes

Feature notes are matched by `feature_id` in `Waypair/Features/*.md`. The tool only updates existing notes. A feature without a note is reported and skipped, because Root creates feature notes.

| Owned by the tool | Never touched |
| --- | --- |
| Properties `status`, `horizon`, `platforms`, `owner` (roadmap `lane`, otherwise `Unassigned`) and `updated` (the date of roadmap `snapshot.asOf`) | `priority` (manual; a mismatch is only reported), `type`, `feature_id`, `title`, `tags`, `vault` and any other property |
| Sections `Next action and acceptance`, `Delivery evidence`, `Limits` and `Sources` | The H1, the summary, `## Alex’s notes`, the back link and every other section |

The tool regenerates the workers note (`--workers-note`) from the checkpoint's active lanes, the roadmap `workers` list, open tasks, the last eight settled tasks and a list of the input sources. It creates this note when it is missing. Its `## Alex’s notes` section is preserved the same way. `Roadmap.md` and `Tasks.base` are never written; Root maintains the visible roadmap. `Workers.md` has `type: "workers"` and sits outside `Waypair/Features`, so the `Tasks.base` filter (`file.inFolder("Waypair/Features")` and `type == "feature"`) does not pick it up.

## Hand edits are kept

Each item the tool writes is fingerprinted in a trailing Obsidian comment, `%% waypair-sync {…} %%`, which is hidden in reading view. On the next run:

- If an item still matches its fingerprint, the tool refreshes it from the evidence.
- If an item was changed by hand, for example a status edited in the Base table or a line added under Limits, the tool keeps it and reports `kept: …`. Use `--take-evidence <id>` when the evidence should win.
- If an item has no fingerprint yet (the first run on a note) and differs from the evidence, the tool treats it as a hand edit. Items that already match are adopted silently.
- If the comment is deleted, the next run behaves like a first run.

## Publication and acceptance are set by a person

The tool never changes these in either direction:

- frontmatter `status` to or from `published`;
- the `published` and `accepted` rows of the delivery ladder;
- the pass state of any gate whose label mentions acceptance, publication or the owner.

When the evidence disagrees, the report prints `held: …` and the note keeps its value. A missing value counts as `open`. A `worker_done`, a task status or a source change is never proof of a build, installation, publication or acceptance.

## Privacy

All generated text is checked before anything is written. The tool refuses credential words, private keys, token-like strings, cloud keys, JWTs, email addresses, phone numbers, 6-digit codes, long numeric device IDs, IMEI or serial numbers, and MAC addresses. Evidence paths may not contain `..`, `.ssh`, keychains, key files or `-AX.json` accessibility dumps. The refusal names the note and section but not the matched text. Timestamps are rendered to the minute.

## Tests

```sh
cd scripts/waypair-roadmap && python3 -m unittest -v test_waypair_obsidian_sync
```

There are two tests, both on temporary fixtures. The first covers the dry run, first-run adoption, `--take-evidence`, the publication and acceptance guards, manual priority, Alex's notes, a later hand edit and idempotence. The second checks that the tool refuses private text or a vault without `AGENTS.md` and writes nothing.
