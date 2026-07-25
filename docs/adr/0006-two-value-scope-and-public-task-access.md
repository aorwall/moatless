# Access scope is two values; teams leave access control

Every scoped resource (repositories, tasks, workspaces, adapters, secrets,
credentials) has a **scope** of exactly `Global` or `User` and an owner
(`owner_user_id`, falling back to `created_by`). Access is a pure function of
scope, ownership, and the caller — no group membership, no per-request DB
resolution of a "team id":

- **Read** = admin ∨ scope is `Global` ∨ caller owns the resource ∨ (the
  resource is a Task ∧ its `visibility = 'public'`).
- **Control** (mutate, delete, run lifecycle/sandbox operations) = admin ∨
  (scope is `User` ∧ caller owns the resource).

`Global` resources are admin-created and admin-controlled; `User` resources are
read and controlled by their owner; admins bypass both. An invisible resource
reads as *not found*, never *forbidden* — mutating DAO methods embed the control
filter in their `WHERE` clause and map `rows_affected == 0` to a module-level
NotFound.

The **team** remains an entity (tables, admin CRUD, membership) but is no longer
an access boundary: the `Team` scope variant, `TeamRole`, team credentials,
personal-team auto-creation, and system-team bootstrapping are gone. Teams are
an org/directory grouping only.

A **public task** (`visibility = 'public'`) has exactly two grants: read+interact
(view, send messages, follow) for any authenticated user, and control (lifecycle,
sandbox operations, credentials) for owner-or-admin only. Public visibility never
grants control — including over the sandbox gRPC surface.

## Considered options

The prior model was a three-value scope (`Global | Team | User`) with a
`TeamRole`, per-request `resolve_team_id`, `ensure_team_manager`/`ensure_*`
service-layer guards, and credentials resolvable at a team level. It was rejected
because team-based access control added a whole membership-resolution axis to
every read and write for little real use, forced scattered imperative `ensure_*`
checks alongside the SQL filter (two places to keep in sync), and made "who can
see/do what" impossible to state as one pure predicate. Collapsing to two values
lets the policy live in one place — `can_read`/`can_control` over a `Scope` plus
owner — and pushes enforcement into the query filter itself.

## Consequences

- The policy is a small pure module (`backend-core::scope`) with a matrix
  snapshot test; every scoped read carries `access_filter` and every mutating
  DAO method carries `control_filter`, both binding `is_admin` + `user_id`.
- Team columns/tables (`team_id`, team-scoped rows) stay inert; the migration
  rewrites team-scoped rows to `scope='user', owner_user_id=COALESCE(owner_user_id, created_by)`,
  never making any resource *more* visible and never touching `created_by`.
- System/internal callers go through the same scoped methods with
  `ScopeContext::system()`, fenced by a Clippy `disallowed-methods` lint so every
  bypass is annotated with a `// reason:`.
- The frontend loses its team-scope selectors (secrets, repository config, admin
  GitHub integration); admin Teams CRUD stays.

Recorded from the "Access Scope by Design — Teams Leave Access Control" spec
(Decisions 1 and 5). Related: ADR-0004 (task creation service) for where task
scope/ownership is seeded.
