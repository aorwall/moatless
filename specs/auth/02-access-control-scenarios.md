# Access Control Scenarios

Scenarios for the two-value scope model (`Global` / `User`) across repositories,
tasks, and GitHub installation bindings.

> **Scope model.** Every resource is `Global` or `User` with an owner
> (`owner_user_id`, falling back to `created_by`). **Read** = admin ∨ global ∨
> owner ∨ (task ∧ `visibility = public`). **Control** = admin ∨ (user-scoped ∧
> owner). `Global` is admin-created and admin-controlled. An invisible resource
> reads as *not found*, never as *forbidden*.
>
> **Retired IDs** (team-based access control was removed — ADR 0006; IDs are
> permanent and never reused): **AUTH-21, AUTH-21.1–21.3** (team-scoped
> repositories), the team parts of **AUTH-23.x**, **AUTH-24.3** (ownerless →
> user scope), and the team-installation half of **AUTH-27**.

## Overview

```
Chain 1: Repository Scope Filtering
├── AUTH-20: Create global-scoped repository (admin)
│   ├── AUTH-20.1: Other user can see global repository
│   ├── AUTH-20.2: Non-admin cannot create global-scoped repositories
│   └── AUTH-20.3: Non-admin cannot mutate a global repository (not-found, not forbidden)
├── AUTH-22: Create user-scoped repository (the default)
│   ├── AUTH-22.1: Creator can see user repository
│   ├── AUTH-22.2: Other user cannot see user repository
│   └── AUTH-22.3: Other user gets not-found fetching it by id

Chain 2: Task Scope Inheritance
├── AUTH-23: Create task under a user-scoped repository
│   ├── AUTH-23.1: Task is user-scoped and owned by its creator
│   ├── AUTH-23.2: Owner can see the task in listings
│   ├── AUTH-23.3: Another user cannot see the task
│   ├── AUTH-23.4: Public visibility grants read to another user
│   └── AUTH-23.5: Public visibility does not grant control

Chain 3: Admin Bypass
├── AUTH-24: Admin can see all scoped resources
│   ├── AUTH-24.1: Admin can fetch a user-scoped repository by ID
│   └── AUTH-24.2: Admin re-submitting user scope cannot take ownership

Standalone
├── AUTH-25: Listing repositories filters by scope
├── AUTH-26: Listing tasks filters by scope
├── AUTH-27: Global GitHub installation binding is admin-only
└── AUTH-28: Racing local-user creation answers conflict
```

---

## Chain 1: Repository Scope Filtering

### AUTH-20: Create global-scoped repository

Verifies that an admin can create a repository visible to all authenticated users.

* **Given** an authenticated admin

* **When** the admin creates a repository with scope "global"

* **Then** the system returns the new repository with scope "global"

### AUTH-20.1: Other user can see global repository

Verifies that any authenticated user can see a global-scoped repository.

* **Given** AUTH-20 has completed

* **When** a different (non-admin) user lists repositories

* **Then** the listing includes the global-scoped repository

### AUTH-20.2: Non-admin cannot create global-scoped repositories

Verifies that global scope is reserved for admins.

* **Given** an authenticated non-admin user

* **When** the user tries to create a repository with scope "global"

* **Then** the system rejects the request

### AUTH-20.3: Non-admin cannot mutate a global repository

Verifies that global resources are admin-controlled, and that an unauthorized
mutation is indistinguishable from a missing resource.

* **Given** AUTH-20 has completed

* **When** a non-admin user tries to modify the global repository

* **Then** the system reports the repository as not found (not as forbidden)

### AUTH-22: Create user-scoped repository

Verifies that a user can create a repository visible only to themselves — the
default scope.

* **Given** an authenticated user

* **When** the user creates a repository with scope "user"

* **Then** the system returns the new repository with scope "user" owned by the creator

### AUTH-22.1: Creator can see user repository

Verifies that the creator can see their own user-scoped repository.

* **Given** AUTH-22 has completed

* **When** the creator lists repositories

* **Then** the listing includes the user-scoped repository

### AUTH-22.2: Other user cannot see user repository

Verifies that a different user cannot see another user's user-scoped repository in a listing.

* **Given** AUTH-22 has completed

* **When** a different user lists repositories

* **Then** the listing does not include the user-scoped repository

### AUTH-22.3: Other user gets not-found fetching it by id

Verifies that an invisible resource reads as missing, never as forbidden.

* **Given** AUTH-22 has completed

* **When** a different user fetches the user-scoped repository by its ID

* **Then** the system reports the repository as not found

---

## Chain 2: Task Scope Inheritance

### AUTH-23: Create task under a user-scoped repository

Verifies that a task created under a user-scoped repository inherits its scope and owner.

* **Given** a user-scoped repository exists

* **When** its owner creates a task under that repository

* **Then** the system creates the task successfully

### AUTH-23.1: Task is user-scoped and owned by its creator

Verifies that the task has the same scope and owner as its parent repository.

* **Given** AUTH-23 has completed

* **When** the creator fetches the task details

* **Then** the task has scope "user" and is owned by the creator

### AUTH-23.2: Owner can see the task in listings

Verifies that the owner sees their user-scoped task.

* **Given** AUTH-23 has completed

* **When** the owner lists tasks

* **Then** the listing includes the task

### AUTH-23.3: Another user cannot see the task

Verifies that a different user cannot see the owner's user-scoped task.

* **Given** AUTH-23 has completed

* **When** a different user lists tasks

* **Then** the listing does not include the task
  * **And** fetching it by id reads as not found

### AUTH-23.4: Public visibility grants read to another user

A task whose visibility is `public` is readable by any authenticated user, even
though its scope still names one owner.

* **Given** AUTH-23.3 has completed

* **When** the owner sets the task's visibility to public

* **Then** a different user can fetch it by id

### AUTH-23.5: Public visibility does not grant control

Public is a read grant, not a control grant: the owner is still the only
non-admin who can act on the task.

* **Given** AUTH-23.4 has completed

* **When** a different user tries to delete the task

* **Then** the system refuses

---

## Chain 3: Admin Bypass

### AUTH-24: Admin can see all scoped resources

Verifies that an admin user can see resources regardless of scope or owner.

* **Given** a user-scoped repository exists created by a regular user

* **When** an admin user lists repositories

* **Then** the listing includes the user-scoped repository

### AUTH-24.1: Admin can fetch a user-scoped repository by ID

Verifies that an admin can fetch a specific user-scoped repository by ID.

* **Given** AUTH-24 has completed

* **When** the admin fetches the user-scoped repository by its ID

* **Then** the system returns the repository details

### AUTH-24.2: Admin re-submitting user scope cannot take ownership

Verifies that editing scope does not silently transfer a repository to the viewing administrator.

* **Given** AUTH-24.1 has completed

* **When** the admin saves the repository with user scope again

* **Then** the repository keeps its original owning user

---

## Standalone

### AUTH-25: Listing repositories filters by scope

Verifies that a regular user's repository listing only includes resources they can read.

* **Given** global and user-scoped repositories exist, some owned by other users

* **When** the user lists repositories

* **Then** the listing includes global repositories
  * **And** the listing includes repositories owned by the user
  * **And** the listing excludes user-scoped repositories owned by other users

### AUTH-26: Listing tasks filters by scope

Verifies that a regular user's task listing only includes tasks they can read.

* **Given** tasks exist under repositories with different scopes and owners

* **When** the user lists tasks

* **Then** only tasks the user can read (global scope, ownership, or public visibility) are returned

### AUTH-27: Global GitHub installation binding is admin-only

Only administrators may inspect or mutate the Global GitHub App installation binding.

* **Given** an authenticated admin user and a regular user

* **When** the regular user requests the configured Apps or the global installation binding

* **Then** the system denies the request
  * **And** an admin may read both interfaces
  * **And** an unknown App key is rejected before persistence

### AUTH-28: Racing local-user creation answers conflict

Two sign-ups for the same login at the same instant are a conflict the platform
expects, not a server fault. The unique constraint decides which one wins; the
losers must be turned away as a conflict rather than surfacing as an internal
error.

* **Given** several simultaneous requests to create a local user with one login

* **When** they are submitted concurrently

* **Then** exactly one of them creates the user
  * **And** every other request is refused as a conflict
