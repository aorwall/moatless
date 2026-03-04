# Tasks & Tags

Tasks are the primary unit of work in Moatless Vibe. Each task captures a user request, an optional agent conversation, and a link to a sandbox where code changes happen.

## What Users Can Do

- **Create a task** — with a repository and an optional initial message. If a message is included, the agent starts automatically; otherwise only the sandbox provisions.
- **View and list tasks** — retrieve a single task by ID or browse all tasks with filtering.
- **Update a task** — change the title or description after creation.
- **Tag a task** — attach labels for organization and filtering; tags can be set or cleared at any time.
- **Close a task** — mark the task as done, preventing new messages.
- **Delete a task** — permanently remove the task and its history.

## How It Works

1. A task is always linked to a **repository** that determines the sandbox image and system prompt.
2. Creating a task provisions a sandbox automatically. If an initial message is provided, the agent begins its first turn.
3. Tags are lightweight labels stored as simple strings — no separate tag management UI is required.
4. Closing a task is a soft transition: the task remains visible but rejects new messages.
5. Deleting a task is permanent and removes all associated data.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Task | A unit of work containing a user request and optional agent conversation |
| Tag | A text label attached to a task for filtering and organization |
| Open / Closed | The two lifecycle states — open accepts messages, closed does not |
