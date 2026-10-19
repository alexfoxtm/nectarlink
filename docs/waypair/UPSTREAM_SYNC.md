# Upstream sync — Waypair Link (Nectar base)

The port on `waypair/macos-port` is the product. Upstream NectarLink
(`thtbee/nectarlink`) keeps moving, and other people send useful fixes. This
page is how new upstream work gets into the port, and how generic port fixes
go back, without vendoring snapshots or trusting arbitrary forks.

Pins and provenance live in `docs/waypair/UPSTREAM_BASE.json`:

| Pin | Source | SHA | In port |
| --- | --- | --- | --- |
| `main` | `thtbee/nectarlink` `main` | `f8f72547` | yes, under PR24 |
| `macos-pr24` | PR #24, `saivinay1805/nectarlink` `feature/macos-port` | `96370de1` | yes, port base |
| `linux-pr12` | PR #12, `jochenkirstaetter/nectarlink` `linux` | `1de43ef1` | no, reference |

The `upstream` remote stays `https://github.com/thtbee/nectarlink.git`. PR
heads are read from that repository's `refs/pull/<n>/head`, not from fork
remotes.

## 1. Check (read-only)

```sh
# Local only: no network, writes nothing.
python3 scripts/upstream/check.py

# Full report into an explicit scratch dir outside the checkout.
python3 scripts/upstream/check.py --fetch --github \
  --out ~/orca/tmp/upstream-check-$(date +%Y%m%d)

# Offline synthetic test of the detection and failure paths.
python3 scripts/upstream/check.py --self-test --out ~/orca/tmp/upstream-selftest
```

- Local: verifies the `upstream` remote URL, that no remote has credentials
  embedded in its URL, that in-port pins are ancestors of `HEAD`, and lists
  port commits since the port base. Each commit is tagged by path with its
  `port_deltas` category (`branding`, `r2_updater`, `cloud`,
  `port_docs_and_tooling`) or `generic`. The checkout is shallow, so pins
  below the boundary are reported as unverifiable locally, not as failures.
- `--fetch`: blobless clone/fetch of upstream `main` and the pinned PR refs
  into `--out/mirror.git`. Prompts and credential helpers are disabled, and
  only HTTPS is allowed. Reports new commits on `main` since the pin, `main`
  commits not yet under the port base, moved, force-pushed or merged PR heads,
  and commits with no `Signed-off-by`. It never writes refs into the checkout.
- `--github`: unauthenticated `GET` only, capped by `github_api.max_requests`.
  Reports releases newer than `latest_release_at_record`, open PRs matching
  `relevance.keywords`, and the state of the pinned PRs. It never posts or
  comments.
- `--out` must be outside the checkout. Without `--out`, the report goes to
  stdout only.

Exit codes: `0` ok (drift is listed in the report), `1` check failure (wrong
remote, missing or rewritten pin, network error), `2` usage, `3` drift with
`--strict`.

Routine runs piggyback on the coordinator's existing heartbeat. Do not create
new schedules, cron jobs or launch agents for this.

The tool never merges, cherry-picks, checks out, pushes, installs or
publishes. Everything below is a reviewed, manual step.

## 2. Integration branch

Use a separate worktree. Never touch the product checkout or another worker's
tree.

```sh
git worktree add ~/orca/workspaces/nectarlink/upstream-sync-YYYYMMDD \
  -b waypair/upstream-sync-YYYYMMDD waypair/macos-port
cd ~/orca/workspaces/nectarlink/upstream-sync-YYYYMMDD
GIT_TERMINAL_PROMPT=0 git fetch --no-tags upstream \
  +refs/heads/main:refs/remotes/upstream/main \
  +refs/pull/24/head:refs/remotes/upstream/pr/24
# The product checkout is shallow; deepen only in the integration worktree.
GIT_TERMINAL_PROMPT=0 git fetch --unshallow upstream
git merge --no-ff upstream/main          # or upstream/pr/24 for port-base moves
```

Merge, don't rebase `waypair/macos-port`: port history and upstream SHAs stay
intact, so later syncs and upstream PRs stay cheap.

## 3. Focused diff and conflict review

- `git log --format='%h %an %s' <pin>..upstream/main` is the list of changes
  under review. `git diff <pin> upstream/main --stat` gives the touched paths.
- Compare against port commits that touch the same paths. The check report
  tags every port commit, so overlap with `generic` port commits is where
  conflicts and duplicate fixes show up. Use `git range-diff` when a PR was
  force-pushed.
- Resolve conflicts in favor of upstream structure and carry the port's
  behavior forward. If a port fix duplicates an upstream fix, keep upstream's
  version and drop ours.
- Keep `branding`, `r2_updater` and `cloud` deltas in their own commits. A
  sync commit contains upstream changes and conflict resolutions only, never
  new branding, R2 or Cloud changes.

## 4. Existing checks

Root is the sole compiler. Workers do not build, run the app, touch devices or
use private credentials. Root runs the checks that already exist:

- `scripts/check.ps1`, the same as upstream CI (fmt, clippy, tests, license
  headers via `scripts/check-headers.ps1`).
- Android: `docs/waypair/ANDROID_BUILD.md`.
- macOS bundle: `docs/waypair/MACOS_PACKAGING.md`.

## 5. Artifact signing and R2 update proof

A sync that ships must be proven through the real update path:

- Sign with the preserved release signers as described in
  `ANDROID_BUILD.md` and `MACOS_PACKAGING.md`.
- `scripts/releases/publish-r2.py`: run the dry-run first, then `--publish`
  from Root's publisher venv (`docs/waypair/R2_RELEASE.md`).
- Proof means a previously installed build sees and verifies the new version
  from the R2 manifest or appcast. A successful upload alone is not proof.

## 6. Adopt

- Fast-forward `waypair/macos-port` to the reviewed integration branch.
- In the same change, update `UPSTREAM_BASE.json`: move the `sha` of each pin
  that was adopted, refresh `state_at_record`, `recorded_at` and
  `latest_release_at_record`, and set `in_port` correctly.
- Remove the integration worktree.

## Unmerged contributions (cherry-picks)

Useful open PRs against `thtbee/nectarlink` can be taken before upstream merges
them, with provenance:

- Source: only `refs/pull/<n>/head` of the canonical repository, at an exact
  SHA. A fork branch without an upstream PR is out of scope unless Alex
  approves it explicitly, and the approval reason is recorded.
- Licence: every commit must carry the original author's `Signed-off-by` (DCO
  1.1, `CONTRIBUTING.md`), and the files must be under the licenses in
  `LICENSE`. Without a sign-off, ask upstream; don't take the commit.
- Mechanics: `git cherry-pick -x <sha>` in an integration worktree. `-x`
  records the origin SHA, and the original author is kept. Don't squash other
  people's commits into ours.
- Record each adoption in `adopted_contributions`:

```json
{
  "pr": 22,
  "origin_sha": "<40-hex>",
  "head_repo": "owner/nectarlink",
  "author": "Name <email>",
  "signed_off_by": "Name <email>",
  "license": "GPL-3.0-or-later",
  "local_sha": "<40-hex>",
  "reviewed_by": "Root",
  "date": "YYYY-MM-DD"
}
```

When upstream later merges the PR, the next sync drops our cherry-pick in
favor of the upstream commit.

## Sending fixes upstream

Port commits tagged `generic` are upstream candidates once Root has qualified
them on device. Split mixed commits first. Rebase the generic part onto
upstream `main` on a separate branch, with no Waypair name, R2 or Cloud
references. Opening the upstream PR is Alex's call; workers never push,
comment or open PRs upstream.
