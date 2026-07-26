# Workspaces

A workspace is the run-configuration a task executes against — the sandbox image, setup commands, environment variables, preview servers, plugins, and the set of repositories placed on disk. Tasks reference a workspace, not a repository.

A workspace can be authored in two places, and this document is about what happens when both want to own the same one.

## What Users Can Do

- **Create a workspace by hand** — name it, give it run-configuration, and place one or more repositories in it.
- **Declare workspaces in git** — a repository can commit a workspaces file that declares one or more workspaces. These are reconciled into real workspaces and kept in step with the file.
- **Override a git-declared workspace** — take a copy under manual control so it can be edited from the UI or CLI. It keeps a record of where it came from.
- **Reset an overridden workspace** — discard the local edits and take the configuration from git again.
- **Sync a repository's configuration** — reconcile everything a repository declares, on demand.

## How It Works

1. A repository that commits a workspaces file has those workspaces reconciled into the system. They are marked as coming from git and are read-only: editing them means editing the file.
2. Reconciliation is a full replace, not a merge. Every field, and the whole set of repository placements, is taken from the file — a field removed from the file is cleared on the workspace.
3. Reconciliation runs automatically when a task starts (only for repositories that ship a configuration directory) and on demand when a user asks for it.
4. Overriding a workspace marks it as manually managed while keeping its origin — which repository declared it, in which file, under which entry. The origin is what makes a later reset possible.
5. The automatic sync leaves overridden workspaces alone. That is the point of an override: it survives everyday task activity.
6. The two explicit, user-initiated syncs do reclaim an override, replacing the local edits with the committed configuration: syncing a repository's configuration (all of that repository's overrides) and resetting a single workspace (just that one).
7. A reset is a re-sync, not a relabel. A workspace returns to git-managed only when the repository still declares it — so a workspace can never claim to come from git while holding configuration git never produced.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Workspace | The run-configuration a task executes against, plus its repository placements |
| Repository placement | One repository inside a workspace: which branch, which subdirectory, and whether it is the primary one |
| Git-declared workspace | A workspace reconciled from a repository's committed workspaces file; read-only |
| Override | A git-declared workspace taken under manual control so it can be edited here instead |
| Origin | The repository, file, and entry a workspace was declared by; kept through an override |
| Reclaim | An explicit sync taking an overridden workspace back under git control, replacing its local edits |
| Automatic sync | The reconciliation that runs when a task starts; respects overrides |
| Explicit sync | A reconciliation a user asked for; reclaims overrides |

An override is deliberately all-or-nothing: the whole workspace is either
authored in git or authored here. There is no per-field override — a workspace
that is half-synced would make it impossible to say what a reset does.

Deleting an overridden workspace discards the override along with it. If the
repository still declares that entry, a later sync brings it back as a
git-declared workspace; the deletion does not remove it from git.
