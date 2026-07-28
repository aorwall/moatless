# Workspace Multi-Repo Scenarios

Covers what a workspace's repository placements mean at runtime: every repo the
workspace places is checked out into the task's sandbox, side by side, and the
primary one stays the repo all repo-scoped work targets.

See [00-design.md](./00-design.md) for the design overview.

## Chain 1: A workspace with several repos
├── WS-30: A multi-repo workspace checks out every repo
│   ├── WS-30.1: Every placement is checked out under its mount name
│   ├── WS-30.2: Each mount is a real checkout, not an empty directory
│   ├── WS-31: The primary repo stays the repo-scoped target
│   └── WS-32: A repo added to the workspace is checked out on the next start

## Standalone
├── WS-33: A single-repo workspace is unchanged
└── WS-34: Each repo is checked out on its own branch

---

### WS-30: A multi-repo workspace checks out every repo

The repositories a workspace places are the repositories the task's sandbox
holds — not just the primary one.

* **Given** a workspace with two repository placements, each with its own mount name

* **When** the workspace is created

* **Then** both placements are stored
  * **And** the workspace reads as multi-repo

---

### WS-30.1: Every placement is checked out under its mount name

* **Given** WS-30 has completed

* **When** creating a task against that workspace and waiting for its sandbox to be ready

* **Then** the workspace directory contains a directory for each placement, named by its mount name

---

### WS-30.2: Each mount is a real checkout, not an empty directory

* **Given** WS-30.1 has completed

* **When** reading a file the repository contains from each mount

* **Then** each read returns content

---

### WS-31: The primary repo stays the repo-scoped target

Adding repos to a workspace must not move the repo everything else points at.

* **Given** WS-30 has completed

* **When** reading the sandbox's git status and diff

* **Then** they describe the primary placement's checkout
  * **And** the agent is told where the other checkouts are, so it treats them as part of the task

---

### WS-32: A repo added to the workspace is checked out on the next start

A workspace is edited over time; a sandbox that already exists picks up the
addition without losing what is on disk.

* **Given** WS-30 has completed, and its sandbox is stopped

* **When** adding a third repository placement to the workspace and starting the sandbox again

* **Then** the new placement is checked out
  * **And** the existing checkouts are left as they were, uncommitted changes included

---

### WS-33: A single-repo workspace is unchanged

The common case must not move: a one-repo workspace produces exactly the layout
it did before workspaces could hold several repos.

* **Given** a workspace with one repository placement and no mount name set

* **When** creating a task against it and waiting for the sandbox to be ready

* **Then** the repository is checked out in a directory named after the repository

---

### WS-34: Each repo is checked out on its own branch

A placement's branch belongs to that placement; the task's branch applies to the
primary repo only.

* **Given** a workspace with two placements, the second pinned to a non-default branch

* **When** creating a task against that workspace on a branch of its own and waiting for the sandbox

* **Then** the primary checkout is on the task's branch
  * **And** the second checkout is on the branch its placement pinned
