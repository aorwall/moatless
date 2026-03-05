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
└── TASK-11: Send message to closed task
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

Sending a message to a closed task causes the system to accept it and reopen the task.

* **Given** a task exists in a closed state

* **When** the user sends a message to the closed task

* **Then** the system accepts the message
  * **And** the task is reopened to process the new message
