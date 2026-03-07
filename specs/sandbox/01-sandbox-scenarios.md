# Sandbox Scenarios

This spec defines all testable scenarios for sandbox lifecycle management: provisioning, idempotent re-provisioning, stopping, restarting, and cleanup. Uses a task without an initial message so the agent stays idle and doesn't interfere with sandbox operations.

**Prerequisite:** This chain starts from [TASK-02](../tasks/01-task-scenarios.md#task-02-create-task-without-message) — a task created without a message.

---

## Overview

### Chain: Sandbox Lifecycle

```
├── TASK-02: Create task without message (in tasks/01-task-scenarios.md)
│   └── SANDBOX-01: Sandbox provisions (agent stays idle)
│       ├── SANDBOX-01.1: Sandbox ready event emitted
│       ├── SANDBOX-02: Idempotent re-provision returns ready
│       ├── SANDBOX-03: Stop sandbox
│       │   ├── SANDBOX-03.1: Sandbox stopped event emitted
│       │   └── SANDBOX-03.2: Start stopped sandbox (files persist)
│       ├── SANDBOX-04: Restart sandbox (files persist)
│       └── SANDBOX-05: Cleanup sandbox (all resources removed)
│           └── SANDBOX-05.1: Sandbox removed event emitted
```

### Standalone: Error Cases

```
├── SANDBOX-06: Sandbox provision failure (invalid image)
├── SANDBOX-07: Sandbox crash loop detected early
└── SERVER-02: Server readiness timeout detected as failure
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

### SANDBOX-01.1: Sandbox ready event emitted

A real-time event is emitted when the sandbox becomes ready.

* **Given** SANDBOX-01 has completed

* **When** listening for real-time events

* **Then** a sandbox ready event has been emitted for the task

---

### SANDBOX-02: Idempotent re-provision returns ready

Requesting provisioning on an already-ready sandbox returns the ready state without side effects.

* **Given** SANDBOX-01 has completed (the sandbox is ready)

* **When** the user requests provisioning again

* **Then** the sandbox remains ready (idempotent)

---

### SANDBOX-03: Stop sandbox

A sandbox can be stopped, transitioning to a stopped state.

* **Given** SANDBOX-02 has completed (the sandbox is ready)

* **When** the user stops the sandbox

* **Then** the sandbox transitions to stopped

---

### SANDBOX-03.1: Sandbox stopped event emitted

A real-time event is emitted when the sandbox is stopped.

* **Given** SANDBOX-03 has completed

* **When** listening for real-time events

* **Then** a sandbox stopped event has been emitted

---

### SANDBOX-03.2: Start stopped sandbox (files persist)

A stopped sandbox can be started, with workspace files preserved.

* **Given** SANDBOX-03 has completed (the sandbox is stopped)

* **When** the user starts the sandbox

* **Then** the sandbox transitions back to ready
  * **And** workspace files from before the stop are still present

---

### SANDBOX-04: Restart sandbox (files persist)

Restarting a sandbox recreates the execution environment while preserving workspace files.

* **Given** SANDBOX-03.2 has completed (the sandbox is ready)

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

### SANDBOX-05.1: Sandbox removed event emitted

A real-time event is emitted when the sandbox is removed.

* **Given** SANDBOX-05 has completed

* **When** listening for real-time events

* **Then** a sandbox removed event has been emitted for the task

---

## Standalone: Error Cases

These scenarios are independent — each creates and tears down its own resources.

---

### SANDBOX-06: Sandbox provision failure (invalid image)

When a sandbox is configured with a nonexistent image, provisioning fails and the sandbox never becomes ready.

* **Given** a repository exists with a nonexistent sandbox image configured

* **When** creating a task and waiting for the sandbox to become ready

* **Then** the sandbox does not become ready

---

### SANDBOX-07: Sandbox crash loop detected early

When a sandbox container crashes repeatedly, the system detects the crash loop and reports an error without waiting for the full timeout.

* **Given** a repository is configured with a sandbox image that crashes immediately on start

* **When** creating a task and waiting for the sandbox to become ready

* **Then** the sandbox does not become ready
  * **And** the system reports a crash loop error
  * **And** the error is detected well before the full timeout

---

### SERVER-02: Server readiness timeout detected as failure

When a server process fails to bind its port, the readiness probe stays unhealthy. After the configured startup timeout, the system marks the server as failed.

* **Given** a repository with a preview server configured with a short startup timeout and a start command that does not bind the expected port

* **When** creating a task and waiting for the server status

* **Then** the server transitions to failed
  * **And** the system reports an error indicating the server failed to become ready within the timeout
