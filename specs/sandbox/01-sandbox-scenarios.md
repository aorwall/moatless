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
├── SERVER-02: Server readiness timeout detected as failure
└── SERVER-09: An overridden server that crashes still auto-disables
```

### Chain: Live Server Config

```
├── SERVER-03: Start command override restarts the server in place
│   ├── SERVER-04: Override survives a sandbox stop/start
│   ├── SERVER-05: Env vars are set and unset live
│   └── SERVER-07: Reset drops the override and restores workspace config
├── SERVER-06: Changing port or image requires a redeploy
└── SERVER-08: Reserved env key is rejected
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

---

## Chain: Live Server Config

A preview server's start command and environment can be changed while the sandbox
runs. Only the server's own process restarts — the agent container is untouched,
and no pod or container is replaced.

Each scenario below depends on its parent as a precondition.

---

### SERVER-03: Start command override restarts the server in place

* **Given** a ready sandbox with a running preview server

* **When** updating the server's start command

* **Then** the response reports the new override and an advanced generation
  * **And** the server becomes ready again serving the new command
  * **And** the sandbox container is not restarted
  * **And** no pod or container is replaced

---

### SERVER-04: Override survives a sandbox stop/start

The override is task state, not container state, so a rebuilt sandbox starts its
servers on the overridden config.

* **Given** SERVER-03 has completed

* **When** stopping and starting the sandbox

* **Then** the server starts on the overridden command
  * **And** reading the server status still reports the override

---

### SERVER-05: Env vars are set and unset live

* **Given** SERVER-03 has completed

* **When** setting an env var and unsetting one that the container spec injected

* **Then** the restarted server process sees the new value
  * **And** the unset variable is absent from the server process's environment

---

### SERVER-06: Changing port or image requires a redeploy

The port is wired into the Service and IngressRoute, and the image is the
container's identity — neither can change without rebuilding the pod.

* **Given** a ready sandbox with a running preview server

* **When** updating the server's port or docker image

* **Then** the request is rejected as requiring a redeploy
  * **And** the server keeps running unchanged

---

### SERVER-07: Reset drops the override and restores workspace config

* **Given** SERVER-03 has completed

* **When** resetting the server's override

* **Then** the response reports an empty override
  * **And** the server restarts on the workspace's configured start command

---

### SERVER-08: Reserved env key is rejected

Overrides cannot shadow the credentials and wiring the sandbox depends on.

* **Given** a ready sandbox with a running preview server

* **When** setting an env var whose key is reserved

* **Then** the request is rejected
  * **And** no override is persisted for that server

---

### SERVER-09: An overridden server that crashes still auto-disables

The supervisor restarts the server process only when the config generation
advances. A process that exits on its own still takes the container down, so
crash-loop detection and auto-disable behave exactly as without an override.

* **Given** a ready sandbox with a running preview server

* **When** overriding the server's start command with one that exits immediately

* **Then** the server container crash-loops

* **And** after the crash-restart threshold the server is auto-disabled and the sandbox is redeployed
