# Login & Session Scenarios

Scenarios for the ForwardAuth verify endpoint, session validation, and preview access control.

## Overview

```
Chain 1: ForwardAuth Verification
├── AUTH-30: Unauthenticated request is rejected
├── AUTH-31: Authenticated user passes verification
└── AUTH-32: API key passes verification

Chain 2: Preview Access Control
├── AUTH-33: Authenticated user can access own task's preview
├── AUTH-34: Non-member cannot access team-scoped task preview
└── AUTH-35: Admin can access any task's preview

Standalone
├── AUTH-36: Expired session is rejected
└── AUTH-37: Invalid API key is rejected

Standalone: Setup Requirements
├── AUTH-40: A fresh user's GitHub requirement reports missing
├── AUTH-41: A non-admin is never offered the org-wide route
├── AUTH-42: An admin is offered the org-wide route first
├── AUTH-43: A user's own PAT satisfies the requirement
├── AUTH-44: A PAT-backed user can create a task for a GitHub repository
└── AUTH-45: A non-admin cannot manage global GitHub installations
```

---

## Chain 1: ForwardAuth Verification

### AUTH-30: Unauthenticated request is rejected

Verifies that a request without credentials is rejected by the verify endpoint.

* **Given** authentication is enabled

* **When** an unauthenticated request is sent to the verify endpoint

* **Then** the system returns an error indicating authentication is required

### AUTH-31: Authenticated user passes verification

Verifies that a request with a valid session passes the verify endpoint.

* **Given** authentication is enabled
  * **And** the user has a valid session

* **When** the user's request is sent to the verify endpoint

* **Then** the system returns a success response

### AUTH-32: API key passes verification

Verifies that a request with a valid API key passes the verify endpoint.

* **Given** authentication is enabled
  * **And** the user has a valid API key

* **When** a request with the API key is sent to the verify endpoint

* **Then** the system returns a success response

---

## Chain 2: Preview Access Control

### AUTH-33: Authenticated user can access own task's preview

Verifies that the verify endpoint grants access when the forwarded host matches a task the user owns.

* **Given** authentication is enabled
  * **And** a task exists created by the user

* **When** a request is sent to the verify endpoint with the task's preview hostname

* **Then** the system returns a success response

### AUTH-34: Non-member cannot access team-scoped task preview

Verifies that the verify endpoint denies access when the forwarded host matches a team-scoped task the user cannot see.

* **Given** authentication is enabled
  * **And** a team-scoped task exists
  * **And** the requesting user is not a member of the team

* **When** a request is sent to the verify endpoint with the task's preview hostname

* **Then** the system returns an error indicating access is denied

### AUTH-35: Admin can access any task's preview

Verifies that an admin user can access any task's preview regardless of scope.

* **Given** authentication is enabled
  * **And** a user-scoped task exists created by another user

* **When** an admin request is sent to the verify endpoint with the task's preview hostname

* **Then** the system returns a success response

---

## Standalone

### AUTH-36: Expired session is rejected

Verifies that a request with an expired session token is rejected.

* **Given** authentication is enabled
  * **And** the user's session has expired

* **When** the user's request is sent to the verify endpoint

* **Then** the system returns an error indicating authentication is required

### AUTH-37: Invalid API key is rejected

Verifies that a request with an invalid API key is rejected.

* **Given** authentication is enabled

* **When** a request with a non-existent API key is sent to the verify endpoint

* **Then** the system returns an error indicating authentication is required

---

## Standalone: Setup Requirements

`GET /api/v1/auth/me` carries a setup block: what the environment still needs
from this user before it will run their work, and what they can do about it.
The remediation offered is tailored to who is asking.

---

### AUTH-40: A fresh user's GitHub requirement reports missing

* **Given** a user with no GitHub token of any kind

* **When** the user reads their own session

* **Then** setup is reported incomplete
  * **And** the GitHub requirement is missing, at required severity

---

### AUTH-41: A non-admin is never offered the org-wide route

Offering an action the caller cannot take is worse than offering none.

* **Given** a non-admin user with an unmet GitHub requirement

* **When** the user reads their own session

* **Then** the requirement carries at least one action
  * **And** none of them is the admin-only org-wide route
  * **And** one of them opens their own settings

---

### AUTH-42: An admin is offered the org-wide route first

Actions are ordered most-preferred first, and an admin should connect the
organisation rather than only themselves.

* **Given** an admin user with an unmet GitHub requirement

* **When** the admin reads their own session

* **Then** the first action is the admin route to the org-wide GitHub settings

---

### AUTH-43: A user's own PAT satisfies the requirement

* **Given** a user with no GitHub token
  * **And** the environment can stand in for the GitHub API

* **When** the user saves a personal access token

* **Then** setup is reported complete
  * **And** the GitHub requirement is satisfied

---

### AUTH-44: A PAT-backed user can create a task for a GitHub repository

A stored token lives encrypted in the secret store; its provider-token row
carries only a reference. Preflight has to follow that reference — otherwise
every token-backed user reads as disconnected and their work is refused.

* **Given** a user who has saved a personal access token
  * **And** the stored provider token is a reference rather than a value

* **When** the user creates a task for a GitHub-provider repository

* **Then** the request is not refused as a missing GitHub account

---

### AUTH-45: A non-admin cannot manage global GitHub installations

* **Given** a non-admin user

* **When** the user reads or writes the global GitHub installation settings

* **Then** every one of those routes answers 403 — forbidden, not malformed
