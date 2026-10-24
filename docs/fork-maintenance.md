# Fork Maintenance Runbook

soaplabs/moatless-vibe is a deploy-target fork of aorwall/moatless-vibe.
This document is the day-to-day playbook for keeping them in sync without
painful merges, and for shipping fork-only customizations cleanly.

## Branch model

| Branch | Owner | Purpose |
|---|---|---|
| `main` | soaplabs | Fork's deployed branch. Carries upstream + fork customizations. **Never rebased.** |
| `upstream/main` | aorwall | Read-only ref via the `upstream` remote. Never checked out, only fetched. |
| `feat/<name>`, `fix/<name>` | soaplabs | Fork-only feature branches. PR'd to `soaplabs/main`. |
| `upstream/<name>` | soaplabs | Upstream-bound branches. Branched off `upstream/main`, pushed to `aorwall` remote, PR'd against `aorwall/moatless-vibe`. |
| `sync/upstream-YYYYMMDD` | github-actions[bot] | Short-lived sync branches opened by `sync-upstream.yml`. Deleted after merge. |

### Decision tree: where does this change go?

```
Is the change fork-specific (deploy config, fork CD, fork helm overlay)?
├── Yes  → feat/<name> off soaplabs/main → PR to soaplabs/main
└── No   → upstream/<name> off upstream/main → push to aorwall → PR to aorwall/main
```

After an upstream PR merges, **drop the local branch**. The next weekly sync
PR will pull the change into soaplabs/main automatically.

### One-time setup

```bash
git remote add upstream https://github.com/aorwall/moatless-vibe.git
git remote -v   # verify both remotes
```

The `aorwall` remote (writable, used for upstream PRs) is configured separately
on owner machines that have push access to aorwall/moatless-vibe.

## Upstream sync

`.github/workflows/sync-upstream.yml` runs Mondays 09:00 UTC. It:

1. Fetches `upstream/main`.
2. Creates `sync/upstream-YYYYMMDD` off `origin/main`.
3. Merges `upstream/main` with `--no-ff`.
4. If conflicts: commits the conflicted state, labels the PR `has-conflicts`.
5. Pushes + opens (or updates) the PR.

Trigger manually: **Actions → Sync upstream/main → Run workflow**. Tick
**dry-run** to see incoming commits without opening a PR.

### Conflict resolution recipe

```bash
gh pr checkout <sync-pr-number>
git status                    # see conflicted paths
```

Common conflicts and how to resolve them:

#### `drizzle/meta/_journal.json`

Both upstream and fork add migration entries → JSON merge conflict on the
`entries` array.

**Recipe:** take both sets of entries, sort by `idx` ascending, renumber if
duplicates exist (rename the **fork** migration file + entry — never upstream).
Confirm `idx` is dense and contiguous from 0.

```bash
# After resolving, start the stack to run the migrations and confirm checksums:
docker-compose/dev-docker.sh --no-logs
```

If the backend reports a checksum mismatch on startup, a migration file was
edited post-merge — undo the edit. **Never modify a migration that has shipped.**

#### Workflow files (`.github/workflows/*.yml`)

Most fork-side gating is `if: github.repository == 'aorwall/moatless-vibe'`
on existing jobs — easy to merge. Conflicts usually mean upstream restructured
the job; manually re-apply the gate.

#### `Taskfile.yml`, `docker/build-*.sh`

The fork uses `${REGISTRY_ORG}` (parameterized in
[aorwall#100](https://github.com/aorwall/moatless-vibe/pull/100)). Conflicts
here are usually upstream tweaking image-build args — keep the parameterization.

### Manual sync (no workflow)

```bash
git fetch upstream
git fetch origin
git switch -c sync/upstream-$(date +%Y%m%d) origin/main
git merge --no-ff upstream/main
# resolve any conflicts...
git push -u origin HEAD
gh pr create --base main --title "chore: merge upstream/main $(date +%Y-%m-%d)"
```

## Migration discipline

The fork already burned itself once (`fix: restore migration 0042 checksum`).
Avoid the next collision:

- **Never edit a migration file already merged from upstream** — changes its
  checksum and breaks every existing database.
- **Fork-only migrations start at `9000` and go up** (`9000_fork_xyz.sql`).
  Leaves `0xxx`–`8xxx` to upstream and makes ordering deterministic.
- **If upstream lands a migration at a number we've already used**, rename the
  **fork** migration to a higher number in a one-off
  `chore(db): renumber fork migration` PR. Update the entry in
  `drizzle/meta/_journal.json` to match.
- The Drizzle journal will conflict on every sync that adds a migration —
  unavoidable. See the recipe above.

## Helm overlay rule

- `k8s/helm/moatless-vibe/values.yaml` is **upstream's defaults**. Don't edit
  it for fork-specific values.
- The **fork's nonprod overlay** lives in `soaplabs/soap-platform-gitops` at
  `apps/nonprod/moatless-vibe/`: an umbrella chart whose `values.yaml` sets
  this deployment's values, deployed by the Argo CD application
  `platform-moatless-vibe`. Edit that repo for environment-specific overrides
  — never the upstream chart.
- Same rule for any future fork-specific config: prefer additive overlay files
  over editing upstream defaults. Easier to merge, easier to audit, easier to
  rip out if we ever close the fork.

## Image publishing flow

`ci.yml` builds and pushes every image to Docker Hub under `REGISTRY_ORG`
(repo variable, default `aorwall`), and `publish-helm-dev` pushes the chart to
the same org as an OCI artifact. The deployment pulls from there —
`sandboxImage.repository` in the Helm values names `aorwall/moatless-sandbox`.

| Image | Dockerfile | Trigger |
|---|---|---|
| `moatless-backend` | `docker/Dockerfile.backend-cross` | push main, paths filter |
| `moatless-sandbox` (+ `-js` alias) | `docker/Dockerfile.sandbox` | push main, paths filter |
| `moatless-sandbox-base` | `docker/Dockerfile.sandbox-base` | push main, paths filter |
| `moatless-sandbox-tools` | `docker/Dockerfile.sandbox-tools` | push main, paths filter |
| `moatless-sandbox-recipe:<hash12>` (+ alias, e.g. `:soaplabs`) | generated from `docker/recipes/*.json` via `docker/generate-recipe.sh` (`build-sandboxes.yml`) | `workflow_run` off `ci.yml`'s completion (push to main), workflow_dispatch |

### Local sandbox build (for testing)

```bash
# Override the registry org for your dev push:
REGISTRY_ORG=myuser ./docker/build-sandbox.sh --tag dev --push
```

The fork's image-name segments are `moatless-*` (no `-vibe`); upstream uses
`moatless-vibe-*`. This rename is in
[aorwall#101](https://github.com/aorwall/moatless-vibe/pull/101).

## Cross-repo coordination

| Need | Repo | Path |
|---|---|---|
| Add an upstream-bound feature | moatless-vibe (this repo) | `upstream/<name>` branch off `upstream/main`, push to `aorwall` |
| Add a fork-only feature | moatless-vibe (this repo) | `feat/<name>` branch off `soaplabs/main` |
