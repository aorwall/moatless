# Team Management Scenarios

Scenarios for team CRUD and membership. Teams are an org/directory grouping of
users; they grant **no** resource access.

> **Retired IDs** (team-based access control was removed — ADR 0006; IDs are
> permanent and never reused):
> - **AUTH-06** — "non-member cannot access team-scoped resources": there is no
>   team scope any more. Resource visibility is covered by the two-value model in
>   `02-access-control-scenarios.md`.
> - **AUTH-08, AUTH-08.1, AUTH-08.2, AUTH-08.2.1, AUTH-10** — team credentials:
>   credentials are resolved from env → user only (no team level).

## Overview

```
Chain 1: Team CRUD
├── AUTH-01: Create a team
│   ├── AUTH-01.1: Team appears in listing
│   ├── AUTH-02: Get team by ID
│   ├── AUTH-03: Update team
│   └── AUTH-04: Delete team
│       └── AUTH-04.1: Team removed from listing

Chain 2: Team Membership
├── AUTH-05: Add member to team
│   ├── AUTH-05.1: Member appears in team member listing
│   └── AUTH-07: Remove member from team
│       └── AUTH-07.1: Removed member no longer in listing

Standalone
└── AUTH-09: Create team with duplicate slug fails
```

---

## Chain 1: Team CRUD

### AUTH-01: Create a team

Verifies that an admin can create a new team.

* **Given** an authenticated admin

* **When** the admin creates a team with name "Engineering" and slug "engineering"

* **Then** the system returns the new team with the given name, slug, and a generated ID
  * **And** the team has timestamps for creation and last update

### AUTH-01.1: Team appears in listing

Verifies that a newly created team is visible in the team listing.

* **Given** AUTH-01 has completed

* **When** the user lists all teams

* **Then** the listing includes the "Engineering" team

### AUTH-02: Get team by ID

Verifies that a team can be retrieved by its ID.

* **Given** AUTH-01 has completed

* **When** the user fetches the team by its ID

* **Then** the system returns the team with name "Engineering" and slug "engineering"

### AUTH-03: Update team

Verifies that a team's details can be updated.

* **Given** AUTH-01 has completed

* **When** the user updates the team name to "Platform Engineering" and adds a description

* **Then** the system returns the updated team with the new name and description
  * **And** the slug remains "engineering"

### AUTH-04: Delete team

Verifies that a team can be deleted.

* **Given** AUTH-01 has completed

* **When** the user deletes the team

* **Then** the system confirms the deletion

### AUTH-04.1: Team removed from listing

Verifies that a deleted team no longer appears in the listing.

* **Given** AUTH-04 has completed

* **When** the user lists all teams

* **Then** the listing does not include the deleted team

---

## Chain 2: Team Membership

### AUTH-05: Add member to team

Verifies that a user can be added as a member of a team.

* **Given** a team exists

* **When** an admin adds another user as a member

* **Then** the system confirms the membership was created

### AUTH-05.1: Member appears in team member listing

Verifies that a newly added member appears in the team's member list.

* **Given** AUTH-05 has completed

* **When** an admin lists the team's members

* **Then** the listing includes the added user with role "member"

### AUTH-07: Remove member from team

Verifies that a member can be removed from a team.

* **Given** AUTH-05 has completed

* **When** an admin removes the added member

* **Then** the system confirms the removal

### AUTH-07.1: Removed member no longer in listing

Verifies that a removed member no longer appears in the team's member list.

* **Given** AUTH-07 has completed

* **When** an admin lists the team's members

* **Then** the listing does not include the removed user

---

## Standalone

### AUTH-09: Create team with duplicate slug fails

Verifies that creating a team with an already-used slug is rejected.

* **Given** a team with slug "engineering" exists

* **When** an admin creates another team with slug "engineering"

* **Then** the system returns an error indicating the slug is already taken
