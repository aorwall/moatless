# Plan: Persist the whole workspace directory (multi-repo + loose files) to S3

## Context

Today the sandbox conflates two concepts into a single path,
`WORKSPACE_DIR=/opt/moatless/repo`:

1. **The repo root** — the agent's cwd, all git/diff operations, the file API,
   transcript-path derivation, setup-command cwd, and the server-container `cd`.
2. **The persistence unit** — what the `s3` storage backend snapshots to S3
   (see `sandbox/src/init/workspace_sync.rs`, added in #238).

Because the persistence unit *is* the single repo, anything the agent creates
outside that one repo — a second cloned repo, a downloaded dataset, scratch
files — is **not** persisted and is lost on a pod restart/eviction.

We want the agent to be able to clone additional repos and download files into a
durable working area and have them survive restarts. To do that we split the two
concepts:

- **`workspace_dir`** = `/opt/moatless/workspace` — a *container* directory that
  is snapshotted to S3 in its entirety. This is also the **agent's cwd**.
- **`repo_dir`** = `/opt/moatless/workspace/<repo-name>` — where the primary repo
  is cloned; the target of all repo-scoped git/diff operations.

The K8s `workspace-data` volume already mounts at `/opt/moatless` (not
`/opt/moatless/repo`), so the repo is *already* a subdirectory of the persisted
volume — **no volume/mount changes are required**. This is purely about moving
the repo into a named subdir and splitting the two concepts in code.

## Goals

- The agent can `git clone` other repos and download files anywhere under
  `workspace_dir` and have them persisted across stop/start/eviction.
- The primary repo continues to work exactly as before for git status, diffs,
  PRs, previews, and `--resume`.
- Seamless rollout: existing single-repo snapshots keep working.

## Non-goals

- Persisting things outside `workspace_dir` (e.g. installed toolchains under
  `/opt/moatless/data`, which is a separate volume) — out of scope.
- Submodule / nested-repo-inside-a-repo semantics — only top-level repos and
  loose files under `workspace_dir` are considered.

---

## Key design decision: cwd = `workspace_dir`

The agent's cwd is the **container** (`/opt/moatless/workspace`), **not** the
repo subdir. Rationale:

- The agent's "current directory" is the durable scratch area, so `git clone
  <other>` and file downloads land in a persisted place by default.
- **It removes the sharpest risk in this change.** The Claude/Codex transcript
  and `--resume` machinery derive the CLI project-dir by dashing the cwd
  (`transcript_sync::transcript_dir()`, `session::has_resumable_transcript()`,
  and the SDK's `resolve_project_dir`). Because cwd stays on `workspace_dir`,
  those keep keying off `workspace_dir` and **need no change** — no divergence
  between the CLI's cwd and the transcript path.

Tradeoff: Claude Code's out-of-the-box ergonomics assume cwd is the repo root
(CLAUDE.md discovery, ambient `git` context). With cwd at the container level the
agent must be *explicitly told* where the repo is — hence the prompt addition
below. The primary repo's `CLAUDE.md` is still surfaced via the mirrored
`AGENTS.md`/system prompt, and the repo path is stated up front.

### What stays on `workspace_dir` vs moves to `repo_dir`

| Concern | Path used | Change? |
|---|---|---|
| Agent cwd (Claude + Codex) | `workspace_dir` | value changes, code unchanged |
| Transcript dir / `--resume` derivation | `workspace_dir` | **no change** |
| File API root (tree/search/read/write, `validate_path` security root) | `workspace_dir` | value changes → agent can browse the whole workspace |
| S3 sync (build/restore) | `workspace_dir` | rewritten (Phase 2) |
| Directory creation on init | `workspace_dir` | value changes |
| **Clone target** | `repo_dir` | **new** |
| **Git status / diff / numstat / base-branch** (`files/git.rs`) | `repo_dir` | **new** |
| **Setup commands cwd** (e.g. `bun install`) | `repo_dir` | **new** |
| **Server container `cd`** (dev/preview servers) | `repo_dir` | **new** (via `server.env`) |
| **CLAUDE.md → AGENTS.md mirror** | `repo_dir` | **new** |
| **`workspace_recloned` `.git` check + note** | `repo_dir` | **new** |

---

## Agent prompt addition (initial system prompt)

On session start the agent must be told the layout. Since `repo_dir` depends on
the repo name (derived at clone time), the **sandbox** appends this note locally
in `build_prompt_append` (`sandbox/src/agent/options.rs`) — it knows both paths
after cloning; the backend does not need the repo name.

Wording is **honest about what actually persists in the current phase**. In
Phase 1 the S3 snapshot is still git-based and scoped to `repo_dir`, so only the
repository survives a restart — loose files elsewhere under `workspace_dir` do
not. The note therefore does **not** promise arbitrary-file persistence yet; that
claim becomes true once the Phase 2 whole-workspace sync lands, at which point the
wording is broadened.

Phase 1 wording (paths interpolated from config in `build_workspace_layout_note`):

```
## Workspace layout

- Workspace dir: `/opt/moatless/workspace` — your working directory (cwd). The
  task repository is checked out in a subdirectory of it (see below).
- Repo dir: `/opt/moatless/workspace/<repo-name>` — the checkout of the task's
  repository; do your repository work here. Its committed history plus your
  uncommitted, non-gitignored working-tree changes are snapshotted to durable
  storage and survive sandbox restarts.
Only the repository is persisted right now: loose files elsewhere under the
workspace dir (and gitignored build artifacts) are ephemeral and may be lost
when the sandbox is recycled.
```

Phase 2 broadens the first bullet to "anything you save under the workspace dir
persists" once the sync covers the whole directory.

The Codex path (`AGENTS.md` mirror) carries the same note.

---

## Impact map

Grouped by phase. File:line references are indicative (as of the #238 merge).

### Phase 1 — Split `workspace_dir` vs `repo_dir` in the sandbox

Foundation. No change to persistence behavior yet (sync still targets the single
repo, now at `repo_dir`).

1. **Derive `repo_dir`.** Add `repo_dir: String` to `sandbox` `Config`
   (`config.rs`), computed as `workspace_dir/<repo-name>`. Repo name = last path
   segment of the clone URL (mirror `derive_plugin_name` in
   `init/workspace.rs`; the existing `extract_github_repo_path` returns
   `owner/repo`, not what we need). Fall back to a stable default (e.g. `repo`)
   when there's no clone URL.
2. **Clone into `repo_dir`** — `init/git.rs` (clone target + the
   `.git`-already-exists skip check), `init/mod.rs` `repo_preexisted` check.
3. **Repoint repo-scoped ops to `repo_dir`:**
   - `files/git.rs` — all git status/diff/numstat/base-branch functions.
   - `init/workspace.rs:~109` — setup-command `current_dir`.
   - `init/mod.rs` — CLAUDE.md→AGENTS.md mirror root; `workspace_recloned_note`.
4. **Agent cwd stays `workspace_dir`** — `agent/options.rs:82`,
   `agent/codex_exec.rs:411,420` unchanged in code (only the value flips). Keep
   file API + transcript on `workspace_dir`. The Dockerfile `ENV WORKSPACE_DIR`
   flip to `/opt/moatless/workspace` (plus Helm `workspaceDir`, `run-sandbox.sh`,
   Docker bind-mount targets) ships **in this phase** — it must move together with
   the code, or `repo_dir` would nest under the old `/opt/moatless/repo`.
5. **Server containers** — write the resolved `repo_dir` into
   `/opt/moatless/data/server.env` as `WORKSPACE_PATH` from the sandbox, and
   change the start wrapper in `k8s/pod_builder.rs:~724` and
   `docker/container_builder.rs:~348` from `cd /opt/moatless/repo` to
   `cd "$WORKSPACE_PATH"` (server.env is already sourced first). This keeps the
   backend ignorant of the dynamic repo name — single source of truth is the
   sandbox.
6. **System prompt addition** — `agent/options.rs` `build_prompt_append` appends
   the workspace-layout note.
7. **Repo-config bridge** — `init/workspace_bridge.rs`. Claude/Codex discover
   project config (skills, `CLAUDE.md`, `AGENTS.md`) **relative to cwd**, but cwd
   is now the workspace and the repo is a subdir, so those would be invisible.
   After the clone, symlink them up to the workspace root:
   - **Skills, for both agents**, from the repo's `.claude/skills`:
     `workspace_dir/.claude/skills/<name>` (Claude) **and**
     `workspace_dir/.agents/skills/<name>` (Codex) → `repo_dir/.claude/skills/<name>`
     (real `skills` parent, per-subdir symlinks). Skills are discovered relative
     to cwd, so covering them once at the workspace level is enough — the old
     in-repo `.agents/skills` mirror in `codex_compat` is **removed** (it lived
     under `repo_dir`, which is no longer cwd, so it was never read).
   - `CLAUDE.md`/`AGENTS.md` file symlinks at the workspace root.

   `codex_compat` keeps only the `CLAUDE.md → AGENTS.md` mirror, which stays
   repo-wide because Codex reads the `AGENTS.md` in the directory of files it
   edits (nested instructions), unlike skills. All bridge links are idempotent,
   regenerated every init, and live outside `repo_dir`, so the Phase 1 snapshot
   excludes them for free — **but the Phase 2 whole-workspace sync must add
   `workspace_dir/.claude`, `workspace_dir/.agents`, `workspace_dir/AGENTS.md`
   (and bridged `CLAUDE.md`) to its exclude list**, or it would snapshot dangling
   links / duplicate the repo's own config.
8. Tests updated for new clone target / server `cd`.

### Phase 2 — Rewrite the S3 sync for a multi-repo + loose-file workspace

`sandbox/src/init/workspace_sync.rs`. The current `build_tar` runs a single
`git ls-files` at the workspace root and appends one top-level `.git`; if the
workspace is a container (not a repo) that command fails and captures nothing,
and loose files / second repos are invisible.

- **Build:** discover each `.git` directory one level under `workspace_dir`; for
  each, run `git -C <repo> ls-files -c -o --exclude-standard -z` (preserves that
  repo's `.gitignore`) and append its files under `<repo>/…`, plus its `.git` via
  `append_dir_all("<repo>/.git", …)`. Then a plain walk of `workspace_dir` for
  **loose files not under any discovered repo** (no git ignore context applies to
  them). Archive paths are workspace-relative with the repo-name as the leading
  component, so `unpack(workspace_dir)` places everything correctly (no change to
  the extract call).
- **Restore detection:** the "full repo?" probe currently matches a *top-level*
  `.git/`. Change it to detect any path component named `.git` (nested
  `<repo>/.git/…`), or add a format sentinel (`.workspace-meta.json`) to the
  archive.
- **Guards:** a coarse exclude list for loose files (or a per-file size
  threshold) to avoid accidentally snapshotting multi-GB artifacts; `log()`
  anything dropped so truncation is never silent. Note that agent-cloned repos
  may be full-history (the primary repo is `--depth 1`) — size can grow. The
  exclude list **must** also drop the Phase 1 repo-config bridge symlinks
  (`workspace_dir/.claude`, `workspace_dir/.agents`, `workspace_dir/AGENTS.md`,
  bridged `CLAUDE.md`): they are regenerated every init and point into `repo_dir`,
  so snapshotting them would either duplicate the repo's config or capture
  dangling links.
- **Backward compatibility:** existing single-repo snapshots (paths at the
  archive root, top-level `.git`) must still restore. Detect old-format archives
  and extract them into `repo_dir` (or keep the pre-#238 behavior) rather than
  the workspace root.

### Phase 3 — Docker / Helm / flip the default

- Flip `ENV WORKSPACE_DIR=/opt/moatless/repo` → `/opt/moatless/workspace` in
  `docker/Dockerfile.sandbox`, `-full`, `-java`, `-soap`; the base image's
  "keep `/opt/moatless/repo` empty for clone" guard; `docker/run-sandbox.sh`;
  `k8s/helm/moatless-vibe/values.yaml:544`; `Dockerfile.sandbox-soap`'s
  hardcoded `REPO_DIR` warm-up script.
- Backend: `WORKSPACE_PATH` env for server containers becomes dynamic (Phase 1
  `server.env` change), and the bind-mount target for Docker mode
  (`container_builder.rs:149,388`) moves from `/opt/moatless/repo` to
  `/opt/moatless/workspace` (repo in a subdir).
- Frontend file browser: no hardcoded path, but the tree root now shows
  `<repo-name>/` plus any loose files. Decide whether the default view opens at
  the workspace root or auto-descends into the repo. Diff/git views remain
  repo-scoped regardless.

---

## Risks & open questions

1. **Agent ergonomics with cwd at the container level.** Mitigated by the prompt
   note and the `AGENTS.md`/`CLAUDE.md` mirror. Worth an e2e check that the agent
   reliably operates in `repo_dir`.
2. **Snapshot size.** Loose files and full-history agent-cloned repos can bloat
   the tar. Needs the exclude list + size cap + drop logging (Phase 2).
3. **File-browser UX** (Phase 3 open question above).
4. **Non-GitHub repo names / name collisions.** Two repos with the same last
   segment would collide; derive a unique subdir if needed.
5. **`--depth 1` primary repo.** Restoring `.git` of a shallow clone is fine, but
   agent operations that need history (e.g. `git log` across the base) already
   have this constraint today; unchanged.

## Rollout order

Phase 1 → Phase 2 → Phase 3, each independently shippable and testable. Phase 1
lands the concept split with no user-visible persistence change; Phase 2 makes
the whole workspace durable; Phase 3 flips the path default and cleans up
Docker/Helm/UX. Each phase requires a new **sandbox image** to take effect (it's
the sandbox binary that runs this), verified per the `dev-loop` / deploy flow.
