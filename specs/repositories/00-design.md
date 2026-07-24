# Repository Management

Repositories define the codebase and configuration that tasks work against. Each repository points to a Git remote and configures how sandboxes are built and what tools the agent has access to.

## What Users Can Do

- **Create a repository** — from a GitHub URL; the system clones and registers it.
- **View and list repositories** — retrieve a single repository by ID or browse all.
- **Update a repository** — change the name, description, or configuration after creation.
- **Manage repository access** — choose global, team, or immutable user scope and manually verify provider reachability.
- **Configure sandbox settings** — set the Docker image, system prompt, server definitions, and plugin list.
- **Delete a repository** — permanently remove the repository and its configuration.

## How It Works

1. Creating a repository triggers a clone of the remote URL. The repository record stores metadata and configuration.
2. Each repository carries configuration that controls sandbox provisioning: the Docker image to use, the system prompt for the agent, server definitions for preview, and optional plugins.
3. Tasks reference a repository — changing the repository configuration affects future tasks but not existing ones.
4. Branches can be specified when creating a task to work on a specific branch of the repository.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Repository | A registered Git remote with sandbox and agent configuration |
| Docker image | The container image used to build the sandbox for tasks under this repository |
| System prompt | Instructions given to the agent when working on tasks for this repository |
| Server definition | Configuration for preview servers that run inside the sandbox |
| Access scope | Who can discover and use a repository: global, one team, or its owning user |
| Access status | The latest persisted provider reachability check and its completion time |

User ownership is immutable on the repository edit surface. Saving a
user-scoped repository keeps its existing owner; an ownerless repository cannot
be moved to user scope until a future explicit ownership workflow exists.

GitHub operations on a user-scoped repository use the owning user's credential
resolution even when an administrator is viewing it. Team and global
repositories continue to resolve from the caller. GitHub App installation
tokens are minted for the owner parsed from the repository URL.
