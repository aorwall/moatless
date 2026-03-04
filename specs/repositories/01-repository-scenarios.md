# Repository Scenarios

This spec defines all testable scenarios for repository management: creation from a GitHub URL, retrieval, updates, listing, and deletion. These scenarios test repository management using real GitHub repos.

---

## Overview

### Chain: Repository CRUD

```
├── REPO-01: Create repository from GitHub URL
│   ├── REPO-02: Get repository by ID
│   ├── REPO-03: Update repository metadata
│   └── REPO-05: Delete repository
```

### Standalone: Error Cases

```
├── REPO-06: Create repository with invalid URL
└── REPO-07: Get non-existent repository
```

---

## Chain: Repository CRUD

Repository creation from a real GitHub URL, retrieval, updates, and deletion. This chain creates a private GitHub repo in setup and cleans it up in teardown.

Each scenario below depends on its parent as a precondition.

---

### REPO-01: Create repository from GitHub URL

Creating a repository from a real GitHub remote URL exercises the full clone pipeline.

* **Given** a private GitHub repository has been created

* **When** the user creates a repository with the GitHub URL

* **Then** the response contains a repository with a valid ID
  * **And** the repository name matches the request
  * **And** the repository is a remote repository
  * **And** the repository URL matches the provided URL
  * **And** the repository appears in the repository listing

---

### REPO-02: Get repository by ID

A repository can be retrieved by its ID with full details.

* **Given** REPO-01 has completed

* **When** the user retrieves the repository by its ID

* **Then** the response contains the repository with the correct ID
  * **And** the response includes the name, kind, URL, and creation date

---

### REPO-03: Update repository metadata

Repository metadata (name, description) can be updated after creation.

* **Given** REPO-01 has completed

* **When** the user updates the repository with a new name and description

* **Then** the response reflects the updated name and description
  * **And** retrieving the repository confirms the changes

---

### REPO-05: Delete repository

A repository can be deleted and no longer appears in the listing.

* **Given** REPO-01 has completed

* **When** the user deletes the repository

* **Then** the system confirms the deletion
  * **And** the deleted repository is not in the listing

---

## Standalone: Error Cases

These scenarios are independent — each creates and tears down its own resources.

---

### REPO-06: Create repository with invalid URL

Creating a repository with an invalid remote URL returns a clear error.

* **Given** no special preconditions

* **When** the user creates a repository with an invalid URL

* **Then** the system returns an error indicating the request is invalid
  * **And** the response includes an error message

---

### REPO-07: Get non-existent repository

Fetching a repository that does not exist returns an error.

* **Given** no repository exists with the specified ID

* **When** the user retrieves a repository with a random ID

* **Then** the system returns an error indicating the repository was not found
  * **And** the response includes an error message
