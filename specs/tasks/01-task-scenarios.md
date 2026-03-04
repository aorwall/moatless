# Task Scenarios

This spec defines all testable scenarios for task management: creation, retrieval, updates, tagging, closing, and deletion. These scenarios test task management independently of agent execution.

---

## Overview

### Chain 1: Task CRUD

```
├── TASK-01: Create task with message
│   ├── TASK-03: Get task by ID
│   ├── TASK-04: Update task metadata
│   ├── TASK-05: Set and clear task tags
│   ├── TASK-08: Close task
│   └── TASK-09: Delete task
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
  * **And** the task appears in the task listing

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

### TASK-05: Set and clear task tags

Tags can be added to and removed from a task.

* **Given** TASK-01 has completed

* **When** the user sets tags on the task

* **Then** retrieving the task includes the assigned tags

* **When** the user clears all tags from the task

* **Then** retrieving the task shows no tags

---

### TASK-08: Close task

A task can be explicitly closed, ending its lifecycle.

* **Given** TASK-01 has completed (task is open)

* **When** the user closes the task

* **Then** retrieving the task shows it as closed

---

### TASK-09: Delete task

A task can be permanently deleted.

* **Given** TASK-08 has completed (task is closed)

* **When** the user deletes the task

* **Then** the system confirms the deletion
  * **And** the deleted task no longer appears in the listing

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

A closed task cannot receive new messages — it must be re-opened or a new task created.

* **Given** a task exists in a closed state

* **When** the user sends a message to the closed task

* **Then** the system returns an error indicating the operation is not allowed
  * **And** the response includes an error message explaining the task is closed
