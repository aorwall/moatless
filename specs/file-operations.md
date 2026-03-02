# File Operations

This spec defines all testable scenarios for workspace file operations: listing, reading, searching, writing, and git status overlay. These scenarios test file operations independently of agent execution.

---

## Overview

### Chain: File Operations

```
├── SANDBOX-01: Sandbox ready (from any task creation)
│   ├── FILE-01: List workspace files
│   ├── FILE-02: Read workspace file
│   ├── FILE-03: File tree with git status overlay
│   ├── FILE-04: Search files by name
│   └── FILE-05: Write file to workspace
```

---

## Chain: File Operations

Workspace file operations available once a sandbox is provisioned. These scenarios test file operations independently of agent execution.

Each scenario below depends on a sandbox being ready.

---

### FILE-01: List workspace files

The file listing returns the workspace directory structure.

* **Given** a sandbox is ready

* **When** the user lists workspace files

* **Then** the response contains a list of file entries
  * **And** each entry includes at minimum a file path
  * **And** known template files are present (e.g., package.json, index.html)

---

### FILE-02: Read workspace file

Individual files can be read from the workspace.

* **Given** FILE-01 has completed (file listing is available)

* **When** the user reads a known file from the workspace

* **Then** the response contains the file content as text
  * **And** the content is non-empty

---

### FILE-03: File tree with git status overlay

The file listing can include git status information for each file.

* **Given** a sandbox is ready

* **When** the user lists workspace files with git status enabled

* **Then** file entries include git status indicators (unmodified, modified, added, etc.)

---

### FILE-04: Search files by name

Files can be searched by name or glob pattern.

* **Given** a sandbox is ready

* **When** the user searches for files by name pattern

* **Then** the response includes files matching the pattern

---

### FILE-05: Write file to workspace

Files can be written to the workspace.

* **Given** a sandbox is ready

* **When** the user writes a file with a path and content

* **Then** reading that file back returns the written content
