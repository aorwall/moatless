# Access Control Scenarios

Scenarios for scope-based visibility across repositories, tasks, and channels.

## Overview

```
Chain 1: Repository Scope Filtering
├── AUTH-20: Create global-scoped repository
│   └── AUTH-20.1: Other user can see global repository
├── AUTH-21: Create team-scoped repository
│   ├── AUTH-21.1: Team member can see team repository
│   └── AUTH-21.2: Non-member cannot see team repository
├── AUTH-22: Create user-scoped repository
│   ├── AUTH-22.1: Creator can see user repository
│   └── AUTH-22.2: Other user cannot see user repository

Chain 2: Task Scope Inheritance
├── AUTH-23: Create task under team-scoped repository
│   ├── AUTH-23.1: Task inherits team scope from repository
│   ├── AUTH-23.2: Team member can see the task
│   └── AUTH-23.3: Non-member cannot see the task

Chain 3: Admin Bypass
├── AUTH-24: Admin can see all scoped resources
│   └── AUTH-24.1: Admin can see user-scoped repository of another user

Standalone
├── AUTH-25: Listing repositories filters by scope
└── AUTH-26: Listing tasks filters by scope
```

---

## Chain 1: Repository Scope Filtering

### AUTH-20: Create global-scoped repository

Verifies that a user can create a repository visible to all authenticated users.

* **Given** an authenticated user

* **When** the user creates a repository with scope "global"

* **Then** the system returns the new repository with scope "global"

### AUTH-20.1: Other user can see global repository

Verifies that any authenticated user can see a global-scoped repository.

* **Given** AUTH-20 has completed

* **When** a different user lists repositories

* **Then** the listing includes the global-scoped repository

### AUTH-21: Create team-scoped repository

Verifies that a user can create a repository visible only to team members.

* **Given** an authenticated user who belongs to a team

* **When** the user creates a repository with scope "team" and the team's ID

* **Then** the system returns the new repository with scope "team" and the team ID

### AUTH-21.1: Team member can see team repository

Verifies that a member of the owning team can see a team-scoped repository.

* **Given** AUTH-21 has completed

* **When** a user who is a member of the same team lists repositories

* **Then** the listing includes the team-scoped repository

### AUTH-21.2: Non-member cannot see team repository

Verifies that a user who is not a member of the owning team cannot see a team-scoped repository.

* **Given** AUTH-21 has completed

* **When** a user who is not a member of the team lists repositories

* **Then** the listing does not include the team-scoped repository

### AUTH-22: Create user-scoped repository

Verifies that a user can create a repository visible only to themselves.

* **Given** an authenticated user

* **When** the user creates a repository with scope "user"

* **Then** the system returns the new repository with scope "user"

### AUTH-22.1: Creator can see user repository

Verifies that the creator can see their own user-scoped repository.

* **Given** AUTH-22 has completed

* **When** the creator lists repositories

* **Then** the listing includes the user-scoped repository

### AUTH-22.2: Other user cannot see user repository

Verifies that a different user cannot see another user's user-scoped repository.

* **Given** AUTH-22 has completed

* **When** a different user lists repositories

* **Then** the listing does not include the user-scoped repository

---

## Chain 2: Task Scope Inheritance

### AUTH-23: Create task under team-scoped repository

Verifies that a task created under a team-scoped repository inherits the scope.

* **Given** a team-scoped repository exists

* **When** a team member creates a task under that repository

* **Then** the system creates the task successfully

### AUTH-23.1: Task inherits team scope from repository

Verifies that the task has the same scope and team as its parent repository.

* **Given** AUTH-23 has completed

* **When** the creator fetches the task details

* **Then** the task has scope "team" and the same team ID as the repository

### AUTH-23.2: Team member can see the task

Verifies that another member of the same team can see the inherited-scope task.

* **Given** AUTH-23 has completed

* **When** another team member lists tasks

* **Then** the listing includes the task

### AUTH-23.3: Non-member cannot see the task

Verifies that a user outside the team cannot see the inherited-scope task.

* **Given** AUTH-23 has completed

* **When** a non-member lists tasks

* **Then** the listing does not include the task

---

## Chain 3: Admin Bypass

### AUTH-24: Admin can see all scoped resources

Verifies that an admin user can see resources regardless of scope.

* **Given** a user-scoped repository exists created by a regular user
  * **And** a team-scoped repository exists for a team the admin does not belong to

* **When** an admin user lists repositories

* **Then** the listing includes both the user-scoped and team-scoped repositories

### AUTH-24.1: Admin can see user-scoped repository of another user

Verifies that an admin can fetch a specific user-scoped repository by ID.

* **Given** AUTH-24 has completed

* **When** the admin fetches the user-scoped repository by its ID

* **Then** the system returns the repository details

---

## Standalone

### AUTH-25: Listing repositories filters by scope

Verifies that a regular user's repository listing only includes resources matching their scope.

* **Given** global, team, and user-scoped repositories exist
  * **And** the user is a member of one team but not another

* **When** the user lists repositories

* **Then** the listing includes global repositories
  * **And** the listing includes team repositories for the user's team
  * **And** the listing includes repositories created by the user
  * **And** the listing excludes team repositories for teams the user does not belong to
  * **And** the listing excludes user-scoped repositories created by other users

### AUTH-26: Listing tasks filters by scope

Verifies that a regular user's task listing only includes tasks matching their scope.

* **Given** tasks exist under repositories with different scopes

* **When** the user lists tasks

* **Then** only tasks the user has access to (via global scope, team membership, or ownership) are returned
