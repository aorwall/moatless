# Repository Scenarios

This spec defines all testable scenarios for repository management: creation from a GitHub URL, retrieval, updates, listing, and deletion. These scenarios test repository management using real GitHub repos.

---

## Overview

### Chain: Repository CRUD

```
├── REPO-01: Create repository from GitHub URL
│   ├── REPO-01.1: Repository appears in listing
│   ├── REPO-02: Get repository by ID
│   ├── REPO-03: Update repository metadata
│   ├── REPO-04: List repositories
│   ├── REPO-05: Delete repository
│   │   └── REPO-05.1: Repository removed from listing
```

### Standalone

```
├── REPO-06: Create repository with missing required fields
├── REPO-07: Get non-existent repository
├── REPO-08: Reject an unverifiable repository target
└── REPO-09: Manually verify repository access
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

---

### REPO-01.1: Repository appears in listing

A newly created repository is visible in the listing.

* **Given** REPO-01 has completed

* **When** the user retrieves the repository listing

* **Then** the created repository appears in the listing

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

### REPO-04: List repositories

The repository listing returns all available repositories.

* **Given** REPO-03 has completed

* **When** the user lists all repositories

* **Then** the listing contains at least one repository
  * **And** the previously created repository is present with its updated name

---

### REPO-05: Delete repository

A repository can be deleted.

* **Given** REPO-01 has completed

* **When** the user deletes the repository

* **Then** the system confirms the deletion

---

### REPO-05.1: Repository removed from listing

A deleted repository no longer appears in the listing.

* **Given** REPO-05 has completed

* **When** the user retrieves the repository listing

* **Then** the deleted repository is not in the listing

---

## Standalone Scenarios

These scenarios are independent — each creates and tears down its own resources.

---

### REPO-06: Create repository with missing required fields

Creating a repository without the required fields returns a validation error.

* **Given** no special preconditions

* **When** the user creates a repository without the required fields

* **Then** the system returns a validation error
  * **And** the response includes an error message

---

### REPO-07: Get non-existent repository

Fetching a repository that does not exist returns an error.

* **Given** no repository exists with the specified ID

* **When** the user retrieves a repository with a random ID

* **Then** the system returns an error indicating the repository was not found
  * **And** the response includes an error message

---

### REPO-08: Reject an unverifiable repository target

Manual verification only runs against a persisted URL and a provider with a supported check.

* **Given** a repository has no URL, an invalid URL, or uses an unsupported provider

* **When** a user with write permission asks the system to verify access

* **Then** the system rejects the request without contacting the provider
  * **And** the repository's previous access status is unchanged

### REPO-09: Manually verify repository access

The stored access state reflects the provider's current response using credentials for the repository's scope.

* **Given** a repository has a valid persisted remote URL
  * **And** the user has write permission
  * **And** User scope has an owner PAT or OAuth token, or Team/Global scope has
    an App installation bound to the repository URL owner

* **When** the user asks the system to verify repository access

* **Then** the response records whether the repository is reachable
  * **And** the response records when the check completed
  * **And** a reachability failure is returned as repository status rather than a request failure
  * **And** credentials from any other repository scope are not used as fallback
