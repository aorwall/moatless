# Task CRUD

This spec defines all testable scenarios for task management: creation, retrieval, updates, tagging, visibility, and deletion. These scenarios test task management independently of agent execution.

---

## Overview

### Chain: Task CRUD

```
├── TASK-01: Create task with message
│   ├── TASK-01.1: Task appears in listing
│   ├── TASK-01.2: Task-created notification emitted
│   ├── TASK-03: Get task by ID
│   ├── TASK-04: Update task metadata
│   ├── TASK-05: Set task tags
│   │   └── TASK-05.1: Clear tags
│   ├── TASK-06: Update task visibility
│   ├── TASK-07: List tasks with filters
│   ├── TASK-08: Close task
│   │   └── TASK-08.1: Task-closed notification
│   └── TASK-09: Delete task
│       └── TASK-09.1: Task removed from listing
```

### Standalone: Error Cases

```
├── TASK-10: Create task with invalid repository
└── TASK-11: Send message to closed task
```

---

## Chain: Task CRUD

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

* **When** the user lists all tasks

* **Then** the response includes the created task
  * **And** the task is in an open state

---

### TASK-01.2: Task-created notification emitted

Task creation emits a notification.

* **Given** a live event connection is established

* **When** a task is created

* **Then** a task-created notification is received with the task's ID

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

Tags can be added to a task for categorization and filtering.

* **Given** TASK-01 has completed

* **When** the user sets tags on the task

* **Then** retrieving the task includes the assigned tags

---

### TASK-05.1: Clear tags

Tags can be removed from a task.

* **Given** TASK-05 has completed (task has tags)

* **When** the user clears all tags from the task

* **Then** retrieving the task shows no tags

---

### TASK-06: Update task visibility

Task visibility can be toggled between private and public.

* **Given** TASK-01 has completed (task defaults to private)

* **When** the user updates visibility to public

* **Then** retrieving the task shows visibility as public

---

### TASK-07: List tasks with filters

The task listing supports filtering by status and other criteria.

* **Given** TASK-01 has completed (at least one task exists)

* **When** the user lists tasks with filter parameters

* **Then** only tasks matching the filter criteria are returned

---

### TASK-08: Close task

A task can be explicitly closed, ending its lifecycle.

* **Given** TASK-01 has completed (task is open)

* **When** the user closes the task

* **Then** retrieving the task shows it as closed

---

### TASK-08.1: Task-closed notification

Closing a task emits a notification.

* **Given** a live event connection is established

* **When** TASK-08 is executed

* **Then** a task-closed notification is received with the task's ID

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

* **When** the user lists all tasks

* **Then** the deleted task is not in the response

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
