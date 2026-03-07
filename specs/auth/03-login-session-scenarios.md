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
