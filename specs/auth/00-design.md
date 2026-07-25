# Authentication & Authorization

Auth controls who can access Moatless Vibe and what they can do. It supports local username/password login, OAuth login, session management, API keys for programmatic access, secure storage of provider tokens, and per-user access control.

Exactly one login provider is active per deployment. When no OAuth provider is configured the server uses local username/password auth, with an admin account seeded on first boot. Every API route requires a session cookie or an API key — there is no unauthenticated mode.

## What Users Can Do

- **Log in with a username and password** — the default when no OAuth provider is configured; credentials are argon2id-hashed in `user_credentials`.
- **Log in via OAuth** — authenticate through GitHub or Auth0; a session is created automatically.
- **Change and reset passwords** — users rotate their own; admins reset another user's without knowing the current one.
- **Manage sessions** — sessions are cookie-based JWTs; they expire after a configurable period.
- **Use API keys** — create long-lived keys for CI/CD or programmatic access.
- **Store provider tokens** — securely save third-party tokens (e.g., GitHub PATs) for use by the agent in sandboxes.
- **Create and manage teams** — admins organize users into teams. Teams are an org/directory concept for grouping people; they do **not** grant access to any resource.

## How It Works

1. The login page asks the server which mode it runs. Under local auth it posts credentials and receives a session cookie; under OAuth it redirects to the provider, and after consent the backend exchanges the code for tokens and creates a session.
2. Sessions are self-validating JWTs stored in cookies — no server-side session table lookup on every request.
3. API keys authenticate via a header and map to a user with a specific role.
4. Provider tokens are encrypted at rest and delivered to sandbox pods via gRPC when the agent needs them (e.g., for git push).
5. An admin role exists for system-level operations; regular users can only access their own resources.

## Access Control Model

Resources (repositories, tasks, channel configs, workspaces, secrets, credentials) have a two-value **scope** — `Global` or `User` — and an owner (`owner_user_id`, falling back to `created_by`). Access is a pure function of scope, ownership, and the caller:

- **Read** is allowed when the caller is an admin, **or** the scope is `Global`, **or** the caller owns the resource, **or** (for a Task) its `visibility` is `public`.
- **Control** (mutating, deleting, or running lifecycle/sandbox operations) is allowed when the caller is an admin, **or** the scope is `User` and the caller owns it.

So `Global` resources are readable by everyone but controlled only by admins; `User` resources are read and controlled by their owner. Admins bypass both checks.

### Public tasks

A task with `visibility = 'public'` is readable and interactable (viewing, sending messages, following) by **any** authenticated user. Public visibility never grants **control** — lifecycle and sandbox operations stay owner-or-admin. There are exactly two access grants for a public task: read+interact for everyone, control for owner-or-admin.

### Teams

Admins organize users into teams for directory/grouping purposes. Teams are created manually or auto-provisioned from Auth0 group claims on login. **Teams grant no resource access** — there is no team scope, and there are no team credentials. (Team-based access control was removed; see ADR 0006.)

### Credential Resolution

When a sandbox needs credentials (e.g., a GitHub token for git operations), the system resolves them using a priority chain:

1. **Environment variables** — admin-configured, highest priority.
2. **User credentials** — personal tokens linked to the individual user; a user-scoped secret shadows a global one of the same key.

The first match wins for each credential type.

### Scope Inheritance

- Tasks inherit scope and ownership from their parent repository.
- Tasks created via channel messages inherit scope and ownership from the channel config.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Auth provider | The active login mechanism: local password, GitHub, GitHub App, or Auth0 |
| Local credential | A username plus argon2id password hash in `user_credentials` |
| Session | A JWT cookie that authenticates browser requests |
| API key | A long-lived token for programmatic access |
| Provider token | A third-party credential (e.g., GitHub PAT) stored for sandbox use |
| Role | User permission level: regular user or admin |
| Team | An org/directory grouping of users; grants no resource access |
| Scope | Access level of a resource: global or user |
| Visibility | A task-only flag; `public` grants read/interact to any authenticated user (never control) |
