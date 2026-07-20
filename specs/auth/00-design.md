# Authentication & Authorization

Auth controls who can access Moatless Vibe and what they can do. It supports local username/password login, OAuth login, session management, API keys for programmatic access, secure storage of provider tokens, and team-based access control.

Exactly one login provider is active per deployment. When no OAuth provider is configured the server uses local username/password auth, with an admin account seeded on first boot. Every API route requires a session cookie or an API key — there is no unauthenticated mode.

## What Users Can Do

- **Log in with a username and password** — the default when no OAuth provider is configured; credentials are argon2id-hashed in `user_credentials`.
- **Log in via OAuth** — authenticate through GitHub or Auth0; a session is created automatically.
- **Change and reset passwords** — users rotate their own; admins reset another user's without knowing the current one.
- **Manage sessions** — sessions are cookie-based JWTs; they expire after a configurable period.
- **Use API keys** — create long-lived keys for CI/CD or programmatic access.
- **Store provider tokens** — securely save third-party tokens (e.g., GitHub PATs) for use by the agent in sandboxes.
- **Create and manage teams** — organize users into teams for shared access to resources.
- **Store team credentials** — save provider tokens at the team level so all team members' sandboxes can use them.

## How It Works

1. The login page asks the server which mode it runs. Under local auth it posts credentials and receives a session cookie; under OAuth it redirects to the provider, and after consent the backend exchanges the code for tokens and creates a session.
2. Sessions are self-validating JWTs stored in cookies — no server-side session table lookup on every request.
3. API keys authenticate via a header and map to a user with a specific role.
4. Provider tokens are encrypted at rest and delivered to sandbox pods via gRPC when the agent needs them (e.g., for git push).
5. An admin role exists for system-level operations; regular users can only access their own resources.

## Access Control Model

Resources (repositories, tasks, channel configs) have a **scope** that controls who can see them:

- **Global** — visible to all authenticated users.
- **Team** — visible only to members of the owning team (and admins).
- **User** — visible only to the user who created the resource (and admins).

Admins bypass all scope restrictions and can see every resource.

### Teams

Users belong to one or more teams. Teams are created manually or auto-provisioned from Auth0 group claims on login. Each team can have its own credentials (provider tokens) that are shared across all team members' sandboxes.

### Credential Resolution

When a sandbox needs credentials (e.g., a GitHub token for git operations), the system resolves them using a priority chain:

1. **Environment variables** — admin-configured, highest priority.
2. **Team credentials** — shared tokens set at the team level.
3. **User credentials** — personal tokens linked to the individual user.

The first match wins for each credential type.

### Scope Inheritance

- Tasks inherit scope and team from their parent repository.
- Tasks created via channel messages inherit scope and team from the channel config.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Auth provider | The active login mechanism: local password, GitHub, GitHub App, or Auth0 |
| Local credential | A username plus argon2id password hash in `user_credentials` |
| Session | A JWT cookie that authenticates browser requests |
| API key | A long-lived token for programmatic access |
| Provider token | A third-party credential (e.g., GitHub PAT) stored for sandbox use |
| Role | User permission level: regular user or admin |
| Team | A group of users who share access to team-scoped resources |
| Scope | Access level of a resource: global, team, or user |
| Team credential | A provider token stored at the team level, shared by all members |
