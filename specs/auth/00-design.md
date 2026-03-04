# Authentication & Authorization

Auth controls who can access Moatless Vibe and what they can do. It supports OAuth login, session management, API keys for programmatic access, and secure storage of provider tokens.

## What Users Can Do

- **Log in via OAuth** — authenticate through GitHub or Auth0; a session is created automatically.
- **Manage sessions** — sessions are cookie-based JWTs; they expire after a configurable period.
- **Use API keys** — create long-lived keys for CI/CD or programmatic access.
- **Store provider tokens** — securely save third-party tokens (e.g., GitHub PATs) for use by the agent in sandboxes.

## How It Works

1. The frontend redirects to the OAuth provider. After consent, the backend exchanges the code for tokens and creates a session.
2. Sessions are self-validating JWTs stored in cookies — no server-side session table lookup on every request.
3. API keys authenticate via a header and map to a user with a specific role.
4. Provider tokens are encrypted at rest and delivered to sandbox pods via gRPC when the agent needs them (e.g., for git push).
5. An admin role exists for system-level operations; regular users can only access their own resources.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| OAuth provider | GitHub or Auth0, used for login |
| Session | A JWT cookie that authenticates browser requests |
| API key | A long-lived token for programmatic access |
| Provider token | A third-party credential (e.g., GitHub PAT) stored for sandbox use |
| Role | User permission level: regular user or admin |
