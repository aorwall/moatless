# Workspace Git Override Scenarios

Covers the lifecycle of a workspace declared in a repository's git tree: it is
read-only until overridden, the override survives automatic syncs, and an
explicit sync or a reset takes it back under git control.

See [00-design.md](./00-design.md) for the design overview.

## Chain 1: Override and reset a git-declared workspace
├── WS-01: Git-declared workspace is reconciled and read-only
│   ├── WS-02: Overriding a workspace makes it editable
│   │   ├── WS-03: An overridden workspace keeps its origin
│   │   ├── WS-04: An overridden workspace can be edited
│   │   ├── WS-05: The automatic sync leaves an override alone
│   │   ├── WS-06: Resetting takes the configuration from git again
│   │   └── WS-07: Syncing the repository reclaims every override
│   └── WS-08: Deleting a git-origin workspace creates a tombstone

## Standalone
├── WS-09: Creating a task after git sync re-elected the primary still works
├── WS-20: Resetting a workspace that was never in git is rejected
└── WS-21: Resetting a workspace git no longer declares leaves it overridden

---

### WS-01: Git-declared workspace is reconciled and read-only

A repository that commits a workspaces file gets those workspaces created for it, and they cannot be edited directly.

* **Given** a repository whose git tree declares a workspace

* **When** the user syncs the repository's configuration

* **Then** a workspace with the declared name and run-configuration exists
  * **And** it is shown as synced from git
  * **And** editing or deleting it is rejected because it is defined in git

---

### WS-02: Overriding a workspace makes it editable

Overriding takes the workspace under manual control without changing any of its values.

* **Given** WS-01 has completed

* **When** the user overrides the workspace

* **Then** the workspace is no longer shown as synced from git
  * **And** its run-configuration is unchanged
  * **And** it can now be edited

---

### WS-03: An overridden workspace keeps its origin

The record of where a workspace came from is what makes a later reset possible, so an override preserves it.

* **Given** WS-02 has completed

* **When** the user views the workspace

* **Then** it still names the repository, file, and entry it was declared by

---

### WS-04: An overridden workspace can be edited

Once overridden, a workspace behaves like any manually created one.

* **Given** WS-02 has completed

* **When** the user changes the workspace's run-configuration and saves

* **Then** the change is accepted
  * **And** viewing the workspace shows the edited values

---

### WS-05: The automatic sync leaves an override alone

An override exists to survive everyday task activity — starting a task must not undo it.

* **Given** WS-04 has completed

* **When** a task starts against the repository, triggering the automatic sync

* **Then** the workspace keeps the edited values
  * **And** it is still shown as manually managed

---

### WS-06: Resetting takes the configuration from git again

A reset discards the local edits in favour of the committed configuration.

* **Given** WS-05 has completed

* **When** the user resets the workspace to git

* **Then** the workspace is shown as synced from git again
  * **And** its run-configuration matches what the repository declares
  * **And** the edits made in WS-04 are gone

---

### WS-07: Syncing the repository reclaims every override

The explicit repository sync is authoritative: it takes back every override belonging to that repository.

* **Given** WS-06 has completed
  * **And** the user has overridden the workspace again and edited it

* **When** the user syncs the repository's configuration

* **Then** the workspace is shown as synced from git again
  * **And** the sync reports how many overridden workspaces it reclaimed
  * **And** the local edits are gone

---

### WS-08: Deleting a git-origin workspace creates a tombstone

Deleting a git-origin workspace keeps a tombstone with its provenance. This prevents automatic or repository-wide sync from recreating the same source key without an explicit per-workspace reset.

* **Given** WS-01 has completed

* **When** the user deletes the workspace

* **Then** the workspace is soft-deleted
  * **And** it is shown as an overridden git-origin workspace in admin
  * **And** its source repository, config path, and source key are preserved
  * **And** later automatic or repository-wide sync does not recreate it
  * **And** resetting this workspace to Git restores it from the committed config

---

## Standalone

### WS-20: Resetting a workspace that was never in git is rejected

Without an origin there is nothing to reset to, and marking such a workspace as git-managed would leave it uneditable and unreachable by any sync.

* **Given** a manually created workspace that was never declared in git

* **When** the user resets it to git

* **Then** the system returns an error explaining there is no git configuration to reset to
  * **And** the workspace is unchanged

---

### WS-21: Resetting a workspace git no longer declares leaves it overridden

A reset only takes effect when the repository still declares the workspace, so a failed reset never leaves a workspace claiming an origin it no longer has.

* **Given** an overridden workspace whose entry has been removed from the repository's workspaces file

* **When** the user resets it to git

* **Then** the system reports that the reset did not take effect and why
  * **And** the workspace stays overridden with its local edits intact

---

### WS-09: Creating a task after git sync re-elected the primary still works

A repository's shadow workspace is written by two paths. Creating a task
inserts the repository placement under a deterministic id; git sync deletes
every placement and re-elects the primary under a random one. Once sync has
run, the deterministic id is gone, so the next task create must not add a
second primary.

* **Given** a repository whose shadow workspace was created by a first task

* **When** git sync has replaced the placement with a re-elected primary
  * **And** the user creates a second task for the repository

* **Then** the task is created
  * **And** the workspace still has exactly one primary placement
