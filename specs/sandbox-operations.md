# Sandbox Operations

This spec defines all testable scenarios for sandbox lifecycle management: provisioning, idempotent re-provision, stopping, restarting, and cleanup. Uses a task without an initial message so the agent stays idle and doesn't interfere with sandbox operations.

---

## Overview

### Chain: Sandbox Lifecycle

```
├── TASK-02: Create task without message
│   └── SANDBOX-01: Sandbox provisions (agent stays idle)
│       ├── SANDBOX-02: Idempotent re-provision returns ready
│       ├── SANDBOX-03: Stop sandbox
│       │   └── SANDBOX-03.1: Start stopped sandbox (files persist)
│       ├── SANDBOX-04: Restart sandbox (files persist)
│       └── SANDBOX-05: Cleanup sandbox (all resources removed)
```

### Standalone: Error Cases

```
├── SANDBOX-06: Sandbox provision failure (invalid image)
└── SANDBOX-07: Sandbox crash loop detected early
```

---

## Chain: Sandbox Lifecycle

Sandbox lifecycle management: provisioning, stopping, restarting, and cleanup. Uses a task without an initial message so the agent stays idle and doesn't interfere with sandbox operations.

Each scenario below depends on its parent as a precondition.

---

### TASK-02: Create task without message

A task created without a message provisions the sandbox but does not start the agent.

* **Given** a valid repository exists

* **When** the user creates a task with a repository but without an initial message

* **Then** the task is created in an open state
  * **And** the agent does not automatically start

---

### SANDBOX-01: Sandbox provisions (agent stays idle)

The sandbox provisions regardless of whether a message was included. Without a message, the agent remains idle.

* **Given** TASK-02 has completed

* **When** checking the sandbox status

* **Then** the sandbox becomes ready
  * **And** the agent remains idle (no message to process)

---

### SANDBOX-02: Idempotent re-provision returns ready

Requesting sandbox provisioning when the sandbox is already ready is idempotent.

* **Given** SANDBOX-01 has completed (the sandbox is ready)

* **When** the user requests sandbox provisioning again for the same task

* **Then** the sandbox remains ready
  * **And** no error is returned
  * **And** the existing sandbox is reused (not recreated)

---

### SANDBOX-03: Stop sandbox

A sandbox can be stopped to save resources while preserving workspace files.

* **Given** SANDBOX-01 has completed (the sandbox is ready)

* **When** the user stops the sandbox

* **Then** the sandbox transitions to stopped
  * **And** the agent can no longer execute

---

### SANDBOX-03.1: Start stopped sandbox (files persist)

A stopped sandbox can be restarted, and workspace files are preserved.

* **Given** SANDBOX-03 has completed (the sandbox is stopped)

* **When** the user starts the sandbox

* **Then** the sandbox transitions back to ready
  * **And** the agent can execute again
  * **And** workspace files from before the stop are still present

---

### SANDBOX-04: Restart sandbox (files persist)

Restarting a sandbox recreates the execution environment while preserving workspace files.

* **Given** SANDBOX-03.1 has completed (the sandbox is ready)

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

---

### SANDBOX-07: Sandbox crash loop detected early

When a sandbox enters a crash loop after provisioning, the error is reported through the status so clients fail fast instead of timing out.

* **Given** a repository exists with a sandbox image that immediately crashes on startup

* **When** creating a task and waiting for the sandbox to become ready

* **Then** the sandbox does not become ready
  * **And** the status includes an error message describing the crash loop
  * **And** the wait completes quickly rather than timing out
