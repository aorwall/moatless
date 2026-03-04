# Sandbox Scenarios

This spec defines all testable scenarios for sandbox lifecycle management: provisioning, stopping, restarting, and cleanup. Uses a task without an initial message so the agent stays idle and doesn't interfere with sandbox operations.

**Prerequisite:** This chain starts from [TASK-02](../tasks/01-task-scenarios.md#task-02-create-task-without-message) — a task created without a message.

---

## Overview

### Chain: Sandbox Lifecycle

```
├── TASK-02: Create task without message (in tasks/01-task-scenarios.md)
│   └── SANDBOX-01: Sandbox provisions (agent stays idle)
│       ├── SANDBOX-03: Stop and start sandbox (files persist)
│       ├── SANDBOX-04: Restart sandbox (files persist)
│       └── SANDBOX-05: Cleanup sandbox (all resources removed)
```

### Standalone: Error Cases

```
└── SANDBOX-06: Sandbox provision failure (invalid image)
```

---

## Chain: Sandbox Lifecycle

Sandbox lifecycle management: provisioning, stopping, restarting, and cleanup. Uses a task without an initial message so the agent stays idle and doesn't interfere with sandbox operations.

Each scenario below depends on its parent as a precondition.

---

### SANDBOX-01: Sandbox provisions (agent stays idle)

The sandbox provisions regardless of whether a message was included. Without a message, the agent remains idle.

* **Given** TASK-02 has completed

* **When** checking the sandbox status

* **Then** the sandbox becomes ready
  * **And** the agent remains idle (no message to process)

---

### SANDBOX-03: Stop and start sandbox (files persist)

A sandbox can be stopped and restarted, with workspace files preserved across the cycle.

* **Given** SANDBOX-01 has completed (the sandbox is ready)

* **When** the user stops the sandbox

* **Then** the sandbox transitions to stopped

* **When** the user starts the sandbox

* **Then** the sandbox transitions back to ready
  * **And** workspace files from before the stop are still present

---

### SANDBOX-04: Restart sandbox (files persist)

Restarting a sandbox recreates the execution environment while preserving workspace files.

* **Given** SANDBOX-03 has completed (the sandbox is ready)

* **When** the user restarts the sandbox

* **Then** the sandbox transitions through initializing to ready
  * **And** workspace files are preserved across the restart

---

### SANDBOX-05: Cleanup sandbox (all resources removed)

Cleaning up a sandbox removes all associated resources.

* **Given** SANDBOX-04 has completed (the sandbox is ready)

* **When** the user cleans up the sandbox

* **Then** the sandbox transitions to not created
  * **And** all associated resources are removed

---

## Standalone: Error Cases

These scenarios are independent — each creates and tears down its own resources.

---

### SANDBOX-06: Sandbox provision failure (invalid image)

When a sandbox is configured with a nonexistent image, provisioning fails and the sandbox never becomes ready.

* **Given** a repository exists with a nonexistent sandbox image configured

* **When** creating a task and waiting for the sandbox to become ready

* **Then** the sandbox does not become ready
