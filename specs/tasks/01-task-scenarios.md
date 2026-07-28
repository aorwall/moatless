# Task Scenarios

This spec defines all testable scenarios for task management: creation, retrieval, updates, tagging, visibility, closing, and deletion. These scenarios test task management independently of agent execution.

---

## Overview

### Chain 1: Task CRUD

```
├── TASK-01: Create task with message
│   ├── TASK-01.1: Task appears in listing
│   ├── TASK-01.2: Task creation event emitted
│   ├── TASK-03: Get task by ID
│   │   └── TASK-03.1: Whether the sandbox is running is asked for separately
│   ├── TASK-04: Update task metadata
│   ├── TASK-05: Set task tags
│   │   └── TASK-05.1: Clear tags
│   ├── TASK-06: Update task visibility
│   ├── TASK-07: List tasks with filters
│   ├── TASK-08: Close task
│   │   └── TASK-08.1: Task closed event emitted
│   └── TASK-09: Delete task
│       └── TASK-09.1: Task removed from listing
```

### Chain 2: Task Without Message

```
├── TASK-02: Create task without message
```

### Standalone: Error Cases

```
├── TASK-10: Create task with invalid repository
├── TASK-11: Send message to closed task
├── TASK-12: Send a follow-up message and read the conversation back
├── TASK-13: A task with no external connection reports none
└── TASK-14: Connections are readable by whoever can read the task
```

### Standalone: Feedback

```
├── TASK-20: Feedback at task, message and tool-call level
├── TASK-21: A tool call named without its message is rejected
└── TASK-22: Preview-failure feedback carries its metadata
```

---

## Chain 1: Task CRUD

Task creation, retrieval, updates, and deletion. This chain tests task management without requiring agent execution.

Each scenario below depends on its parent as a precondition.

---

### TASK-01: Create task with message

Creating a task with an initial message.

* **Given** a valid repository exists

* **When** the user creates a task with a repository and an initial message

* **Then** the task is created in an open state
  * **And** the task has a unique identifier

---

### TASK-01.1: Task appears in listing

A newly created task is visible in the task listing.

* **Given** TASK-01 has completed

* **When** the user retrieves the task listing

* **Then** the created task appears in the listing

---

### TASK-01.2: Task creation event emitted

A real-time event is emitted when a task is created.

* **Given** TASK-01 has completed

* **When** listening for real-time events

* **Then** a task creation event has been emitted for the new task

---

### TASK-03: Get task by ID

A task can be retrieved by its ID with full details.

* **Given** TASK-01 has completed

* **When** the user retrieves the task by its ID

* **Then** the response contains the task with the correct ID
  * **And** the response includes the status, repository, and creation date

---

### TASK-03.1: Whether the sandbox is running is asked for separately

A task is a durable record; whether its sandbox is up right now is not. Reading a task must not answer the second question, or a stale listing would look like a live one.

* **Given** TASK-01 has completed

* **When** the user retrieves the task, on its own and in a listing

* **Then** neither answer says anything about the sandbox or the agent running right now

* **And when** the user asks for the task's live status

* **Then** that answer reports the sandbox, the agent, and any sandbox failure — and nothing else
  * **And** the fuller status report adds the running servers to those three

---

### TASK-04: Update task metadata

Task metadata (title, description) can be updated after creation.

* **Given** TASK-01 has completed

* **When** the user updates the task with a new title

* **Then** the response reflects the updated title
  * **And** retrieving the task confirms the change

---

### TASK-05: Set task tags

Tags can be added to a task.

* **Given** TASK-01 has completed

* **When** the user sets tags on the task

* **Then** retrieving the task includes the assigned tags

---

### TASK-05.1: Clear tags

Tags can be removed from a task.

* **Given** TASK-05 has completed

* **When** the user clears all tags from the task

* **Then** retrieving the task shows no tags

---

### TASK-06: Update task visibility

A task's visibility can be changed.

* **Given** TASK-01 has completed

* **When** the user updates the task visibility

* **Then** the response reflects the new visibility setting
  * **And** retrieving the task confirms the change

---

### TASK-07: List tasks with filters

The task listing supports filtering by status.

* **Given** TASK-01 has completed (task is open)

* **When** the user lists tasks filtered by open status

* **Then** all returned tasks have open status
  * **And** the result includes at least one task

---

### TASK-08: Close task

A task can be explicitly closed, ending its lifecycle.

* **Given** TASK-01 has completed (task is open)

* **When** the user closes the task

* **Then** retrieving the task shows it as closed

---

### TASK-08.1: Task closed event emitted

A real-time event is emitted when a task is closed.

* **Given** TASK-08 has completed

* **When** listening for real-time events

* **Then** a task closed event has been emitted for the task

---

### TASK-09: Delete task

A task can be permanently deleted.

* **Given** TASK-08 has completed (task is closed)

* **When** the user deletes the task

* **Then** the system confirms the deletion

---

### TASK-09.1: Task removed from listing

A deleted task no longer appears in the listing.

* **Given** TASK-09 has completed

* **When** the user retrieves the task listing

* **Then** the deleted task no longer appears in the listing

---

## Chain 2: Task Without Message

A task created without a message provisions the sandbox but does not start the agent. This scenario is the entry point for the [sandbox lifecycle chain](../sandbox/01-sandbox-scenarios.md).

---

### TASK-02: Create task without message

A task created without a message provisions the sandbox but does not start the agent.

* **Given** a valid repository exists

* **When** the user creates a task with a repository but without an initial message

* **Then** the task is created in an open state
  * **And** the agent does not automatically start

---

## Standalone: Error Cases

These scenarios are independent — each creates and tears down its own resources.

---

### TASK-10: Create task with invalid repository

Creating a task with a non-existent repository returns a clear error.

* **Given** no repository exists with the specified ID

* **When** the user creates a task with the invalid repository

* **Then** the system returns an error indicating the repository was not found
  * **And** the response includes an error message

---

### TASK-11: Send message to closed task

Sending a message to a closed task is rejected — a closed task is terminal and has no reopen path.

* **Given** a task exists in a closed state

* **When** the user sends a message to the closed task

* **Then** the system rejects the message with a conflict error (409)
  * **And** no message is persisted on the task
  * **And** the task remains closed

---

### TASK-12: Send a follow-up message and read the conversation back

A task's conversation accumulates: a message sent after creation joins the
initial one, and the listing returns them.

* **Given** a task exists, created with an initial message

* **When** the user sends a second message to the task

* **Then** the task's message listing is not empty

---

### TASK-13: A task with no external connection reports none

The per-task connections endpoint answers for a task nothing is wired to.

* **Given** a freshly created task with no adapter binding

* **When** the user reads the task's connections

* **Then** the list is empty

---

### TASK-14: Connections are readable by whoever can read the task

The connections endpoint is scope-checked like reading the task itself, not
admin-gated: the owner reads it, and a user who cannot see the task gets a 404
rather than a 403 — an invisible task is *not found*.

* **Given** a user-scoped repository holding a task owned by one user

* **When** that user reads the task's connections

* **Then** the list is returned

* **When** a user who cannot see the task reads its connections

* **Then** the system answers 404

---

## Standalone: Feedback

Feedback is filed against a task and optionally narrowed to one message or one
tool call within it. These scenarios are independent.

---

### TASK-20: Feedback at task, message and tool-call level

The three surfaces are stored distinctly and all belong to the task.

* **Given** a task exists

* **When** the user files task-level, message-level and tool-call-level feedback

* **Then** each response carries the surface it was filed under
  * **And** each is attributed to the user rather than the agent
  * **And** the task's feedback listing returns all three

---

### TASK-21: A tool call named without its message is rejected

A tool call has no identity outside the message that contains it.

* **Given** a task exists

* **When** the user files tool-call feedback naming a tool call but no message

* **Then** the system rejects the request

---

### TASK-22: Preview-failure feedback carries its metadata

Preview failures are reported with the diagnostic detail that explains them.

* **Given** a task exists

* **When** the user files preview-failure feedback with error metadata

* **Then** the stored feedback carries the surface and the metadata unchanged
