# Team Management Scenarios

Scenarios for team CRUD, membership, and team credentials.

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
│   ├── AUTH-06: Non-member cannot access team-scoped resources
│   └── AUTH-07: Remove member from team
│       └── AUTH-07.1: Removed member no longer in listing

Chain 3: Team Credentials
├── AUTH-08: Upsert team credential
│   ├── AUTH-08.1: Credential appears in listing
│   └── AUTH-08.2: Delete team credential
│       └── AUTH-08.2.1: Credential removed from listing

Standalone
├── AUTH-09: Create team with duplicate slug fails
└── AUTH-10: Non-member cannot manage team credentials
```

---

## Chain 1: Team CRUD

### AUTH-01: Create a team

Verifies that an authenticated user can create a new team.

* **Given** an authenticated user

* **When** the user creates a team with name "Engineering" and slug "engineering"

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

* **When** the team owner adds another user as a member

* **Then** the system confirms the membership was created

### AUTH-05.1: Member appears in team member listing

Verifies that a newly added member appears in the team's member list.

* **Given** AUTH-05 has completed

* **When** the team owner lists the team's members

* **Then** the listing includes the added user with role "member"

### AUTH-06: Non-member cannot access team-scoped resources

Verifies that a user who is not a team member cannot see team-scoped resources.

* **Given** AUTH-05 has completed
  * **And** a team-scoped repository exists under the team

* **When** a user who is not a member of the team lists repositories

* **Then** the team-scoped repository does not appear in their listing

### AUTH-07: Remove member from team

Verifies that a member can be removed from a team.

* **Given** AUTH-05 has completed

* **When** the team owner removes the added member

* **Then** the system confirms the removal

### AUTH-07.1: Removed member no longer in listing

Verifies that a removed member no longer appears in the team's member list.

* **Given** AUTH-07 has completed

* **When** the team owner lists the team's members

* **Then** the listing does not include the removed user

---

## Chain 3: Team Credentials

### AUTH-08: Upsert team credential

Verifies that a team member can store a credential at the team level.

* **Given** a team exists and the user is a member

* **When** the user stores a credential for provider "github" with an access token

* **Then** the system confirms the credential was stored

### AUTH-08.1: Credential appears in listing

Verifies that a stored team credential appears in the team's credential listing.

* **Given** AUTH-08 has completed

* **When** the user lists the team's credentials

* **Then** the listing includes a credential for provider "github"
  * **And** the access token value is not exposed in the listing

### AUTH-08.2: Delete team credential

Verifies that a team credential can be deleted.

* **Given** AUTH-08 has completed

* **When** the user deletes the "github" credential from the team

* **Then** the system confirms the deletion

### AUTH-08.2.1: Credential removed from listing

Verifies that a deleted team credential no longer appears in the listing.

* **Given** AUTH-08.2 has completed

* **When** the user lists the team's credentials

* **Then** the listing does not include a credential for provider "github"

---

## Standalone

### AUTH-09: Create team with duplicate slug fails

Verifies that creating a team with an already-used slug is rejected.

* **Given** a team with slug "engineering" exists

* **When** the user creates another team with slug "engineering"

* **Then** the system returns an error indicating the slug is already taken

### AUTH-10: Non-member cannot manage team credentials

Verifies that a user who is not a team member cannot store or list team credentials.

* **Given** a team exists
  * **And** the requesting user is not a member of the team

* **When** the user attempts to store a credential for the team

* **Then** the system returns an error indicating insufficient access
