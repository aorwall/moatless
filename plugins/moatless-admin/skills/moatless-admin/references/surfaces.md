# What an admin can configure

`moat <command> --help` is the authority on syntax; this file says what each
area controls, who may change it, and which owner wins.

- [Access model](#access-model)
- [API and CLI areas](#api-and-cli-areas)
- [Admin API routes with no CLI command](#admin-api-routes-with-no-cli-command)
- [Git-synced config](#git-synced-config)
- [Deployment config](#deployment-config)
- [Skills that own the neighbouring work](#skills-that-own-the-neighbouring-work)

## Access model

Resources are scoped `global` or `user`. Anyone reads a global one, and only an
admin writes or creates one; a user-scoped one belongs to its owner. An admin
acting through the API acts as themselves, not as the resource's owner. `moat secret set --scope`
takes `user` or `global`.

## API and CLI areas

| Area | Controls | Changing commands | Who |
|---|---|---|---|
| Users | Accounts, roles, provider id mappings | `user set-name`, `set-email`, `activate`, `deactivate`, `set-provider-id`, `delete-provider-id`; role through `moat api` | Admin for anyone else; see `users.md` |
| Bots | User accounts backed by a registered GitHub App | `bot create --app-key <key>` | Admin |
| Repositories | The repository registry | `repos create`, `update`, `import --org <org> [--dry-run]`, `sync-config <repo>`, `delete <id> --yes` | Admin for global repositories and `sync-config` |
| Workspaces | Listing and removing legacy manual rows only; configured in git | `workspaces delete <id> --yes` | Admin for global workspaces |
| Workspace credentials | A provider token pinned to a workspace, used instead of the run-as user's | `workspaces credentials set <ws> <provider> --value-file <file>`, `credentials remove` | Whoever may write the workspace; every attempt is audited |
| Loops | Scheduled and subscription automations | `loops create-schedule`, `create-subscription`, `update`, `activate --id <id> --run-as-user-id <user>`, `pause`, `resume`, `override`, `reset-git`, `delete --yes` | Admin, for every command |
| Plugins | Skill plugins and where each skill is on | `plugins create --name <n> --git-url <url>`, `delete`, `enable` and `disable <id> --reach everyone\|personal\|workspace [--skill --workspace --user]`, `clear`; `plugins effective` shows what someone gets | Admin for create, delete, `--reach everyone` and another user; workspace reach needs control of the workspace |
| Secrets | Env vars, provider tokens, runtime files delivered to agents or servers | `secret set <KEY> --value-file <file> [-s user\|global] [-t agent\|all\|backend\|server:<name>] [-k env\|provider_token\|runtime_file]`, `remove`, `disable`, `enable` | Owner for user scope, admin for global; values are write-only |
| Personal settings | The caller's own GitHub PAT | `settings github-pat` | Each user |
| Adapters | Inbound Slack, Linear, GitHub and webhook connections | `adapters connections create …` (an upsert), `adapters install --adapter linear` | Admin for global connections |
| Bindings | A task's link to an adapter subject | `bindings create …` | Whoever controls the task |
| Provider apps | Forgejo, Slack and Linear app credentials | `app set <provider> <key> --field n=v…`, `app remove` | Admin |
| GitHub Apps | Apps that back bot users | `github-app register <key> --app-id <n> --key-file <f>`, `rotate`, `remove` | Admin |

Area gotchas:

- **`app set` replaces every field.** An omitted field is cleared, except secrets. Nothing is stored when the provider rejects the credentials.
- **`github-app register` and `rotate` check the key with GitHub first.** `github-app remove` leaves the App's bot users in place, unable to get a token. The sign-in App is the deployment's `GITHUB_APP_*` and holds the reserved key `default`.
- **`repos sync-config` reclaims overridden workspaces** still declared in `workspaces.json`, replacing their local changes.
- **A deleted git loop leaves a tombstone.** `reset-git` restores it without its activation, so it needs `activate` again.
- **A loop's `--node-selector` outranks its workspace's.**

## Admin API routes with no CLI command

Call these with `moat api <METHOD> <path> --body '<json>'` and read the
`status` it prints.

| Route | Does |
|---|---|
| `PATCH /api/v1/admin/users/{id}` with `role` | Promotes or demotes a user |
| `POST /api/v1/users` | Creates a local password user |
| `PUT /api/v1/users/{login}/password` | Resets a password, local sign-in only |
| `POST`, `DELETE /api/v1/admin/settings/codex/config` | Sets or clears the deployment-wide Codex auth |
| `POST /api/v1/admin/users/{id}/provider-tokens/codex` | Seeds a Codex token for another user |
| `DELETE /api/v1/adapters/connections/{id}` | Removes an adapter connection |

## Git-synced config

A repository's default branch owns these. Change them by PR; the backend
checks each default branch every 2 minutes, from the webhook's `push` events
for an owner whose hook delivered one in the last 24 hours, else by polling
GitHub (every 30 minutes for a repository read without a credential).
`moat repos sync-config <repo>` syncs at once.

| File | Controls |
|---|---|
| `.moatless/workspaces.json` | `{version, workspaces: [...]}`. Per workspace: `key`, `name`, `repos`, `dockerImage`, `codeExecution.setupCommands`, `scripts`, `systemPrompt`, `resources`, `envVars` (no secrets), `languageServers`, `defaultModel`, `defaultEffort`, and the privileged `sandboxServiceAccount`, `networkProfile`, `nodeSelector`. A `"key": "default"` entry replaces the workspace every repository gets |
| `skills` in a `workspaces.json` entry | When present, git owns that workspace's skill activations and replaces them on every sync. `"skills": {}` clears them; leaving the key out leaves them to the API |
| `.moatless/loops/<name>/LOOP.md` | A loop: trigger, prompt, model. It lands inactive; `active` and the run-as user come only from `moat loops activate` |
| `.agents/skills/*`, `.claude/skills/*` | The repository's own skills, loaded for any work on it with no activation record |

Read the sync warnings after a change: an unknown key, a secret in `envVars`
and an ungranted privileged field are each dropped with a warning, not an
error.

## Deployment config

Every value is read at startup, so a change takes a PR to the values and a
redeploy. A Kubernetes deployment usually keeps its values in a gitops
repository, often as an umbrella chart over `k8s/helm/moatless` in this
repository; that chart's `values.yaml` holds the upstream defaults and is not
edited per deployment. The deployment skill your organisation's plugins
provide, if any, names where. On Docker Compose the values are `~/.moatless/compose/.env`, and
`docker compose up -d` applies them. `k8s-install`'s
`references/values-schema.md` documents each chart value.

| Concern | Chart values | Env vars |
|---|---|---|
| Sign-in and admins | `auth.provider`, `auth.oidc`, `auth.githubApp`, `auth.local`, `adminUserIds` | `AUTH_PROVIDER`, `OIDC_*`, `GITHUB_APP_*`, `ADMIN_USER_IDS` |
| Sandboxes | `kubernetes.sandboxResources`, `sandboxStorage`, `inactivityTimeoutMinutes`, `sandboxNodeSelector`, `sandboxEnv`, `allowedSandboxServiceAccounts` | `K8S_*`, `SANDBOX_*` |
| Network | `networkPolicy.sandbox`, `egressProxy`, `sandboxAccess.profiles`, `kubernetes.extraNetworkProfiles` | `SANDBOX_NETWORK_PROFILES` |
| Workspace sync | `workspaceSync.githubOrgs`, `orgSyncUserId`, `identityGrants` | `GITHUB_ORG_SYNC_ORGS`, `GITHUB_ORG_SYNC_USER_ID`, `WORKSPACE_IDENTITY_GRANTS` |
| Plugins | `plugins.repositories` | `PLUGIN_REPOSITORIES_JSON` |
| Defaults and flags | `extraEnv` | `FF_*`, `DEFAULT_TASK_VISIBILITY`, `DEFAULT_REPOSITORY_VISIBILITY`, `TASK_CLOSE_AFTER_SETTLED_DAYS` |
| Storage | `storage.backend`, `kubernetes.transcriptStorage` | `S3_*`, `*_S3_PREFIX` |
| Secrets | `secrets.*`, `externalSecrets`, `extraSecretEnv` | `SECRETS_ENCRYPTION_KEY`, `TASK_TOKEN_SECRET` |

Deployment gotchas:

- **A plugin in `plugins.repositories` is a `deployment` plugin.** Each start creates it if missing, and disables, without deleting, one that left the list. The API refuses to delete it while the values still declare it. Plugins an admin registered through the API are never touched.
- **`WORKSPACE_IDENTITY_GRANTS` entries name a repository and a workspace key exactly,** with the privileged values that workspace may declare, and optionally an `ownerUserId` that makes it private. Unknown fields fail the backend's start.
- **`GITHUB_ORG_SYNC_ORGS` needs `GITHUB_ORG_SYNC_USER_ID`.** Org sync then registers each org repository, and soft-deletes archived or missing ones, as soon as the org webhook reports a `repository` event, else hourly when the webhook is live and every 10 minutes when it is not.
- **Network profiles, service accounts and network policies are chart resources,** so changing one may need a chart change in this repository before the values can use it.

## Skills that own the neighbouring work

| Work | Skill |
|---|---|
| A first install, sign-in setup | `moatless-setup`, `k8s-install`, `docker-install` |
| Writing a `LOOP.md`, loop activation details | `moat-cli` (`references/loops.md`) |
| Sandbox CPU and memory | `workspace-resource-tuning` |
| A stuck or evicted sandbox | `sandbox-troubleshooting` |
| Editing Helm values in a gitops repository | `infra:helm-values` |
