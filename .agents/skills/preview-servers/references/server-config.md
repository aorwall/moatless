# Server config reference

## The `servers[]` schema

One entry per sidecar. Same shape in `.moatless/workspaces.json`, in
`--servers-json`, and in the API — camelCase everywhere.

| Field | Type | Default | Live-changeable? |
|---|---|---|---|
| `name` | string | required | no — identifies the container (`<name>-server`) |
| `label` | string | `name` | no — display only |
| `port` | number | required | **no** — wired into the Service and IngressRoute |
| `startCommands` | string[] | — | **yes** — `servers set --start-command` |
| `envVars` | object | — | **yes** — `servers set --env` / `--unset` |
| `startupTimeoutSeconds` | number | 120 | no |
| `autoReload` | bool | false | no |
| `enableInspector` | bool | false | no — drives the inspector + address bar in the UI |
| `default` | bool | false | no — which server the preview panel opens |
| `dockerImage` | string | the sandbox image | **no** — the container's identity |
| `resources` | `{requests,limits}` | repo-wide `resources` | no — `storage` here is ignored |

Two fields that do less than they look like they do:

- `startupTimeoutSeconds` is a **classification** timeout, not a probe setting.
  An unready container reports `starting` until it elapses, then `failed`. It
  does not give a slow server more time to boot.
- `autoReload` only adds a line to the agent's system prompt. The reloading is
  your dev server's own file watcher; the flag enables nothing.

`startCommands` is a list joined with ` && ` and run through `bash -c` from the
workspace root, so `["bun install", "bun run dev"]` is a valid sequence. Bind
`0.0.0.0` — a server on `127.0.0.1` fails the readiness probe and is unreachable
from the proxy.

## Where a server's env comes from

Later wins. All of it is re-read on every restart, so a `servers restart` is
enough to pick up a change at any layer.

1. **Container env** — `PORT`, `WORKSPACE_PATH`, `DATA_DIR`, and the server's own
   `envVars`. Baked into the pod spec; needs a redeploy to change.
2. **`server.env`** — user secrets from `moat secret set`, refreshed every 15
   minutes, plus the real `WORKSPACE_PATH`.
3. **The live override** — `servers set --env` / `--unset`. `--unset` is the only
   way to remove a variable layer 1 injected: sourcing a file can add but never
   remove.

**Workspace-level `envVars` do not reach preview servers at all** — they land in
the sandbox container only. This is a known gap, and it is silent: no error, no
effect. Put the variable in the server's own `envVars`, or set it live.

## Finding the workspace

`moat tasks get` does not report a workspace id. The default workspace for a
repository is `ws_<repositoryId>`:

```bash
REPO_ID=$(moat tasks get $TASK_ID --json | jq -r .repositoryId)
moat workspaces get ws_$REPO_ID
```

If the task was created with an explicit `--workspace`, that derivation is wrong.
Fall back to `moat workspaces list` and match the workspace whose `servers[]`
matches what `moat tasks status $TASK_ID --json` reports.

`moat workspaces list` also prints the `SOURCE` column that decides where you
promote: `git` ⇒ edit `.moatless/workspaces.json`; `manual` ⇒
`moat workspaces update`.

## Promoting: worked examples

Repo config — `.moatless/workspaces.json`, the record for `SOURCE=git`
workspaces. Commit and push; git sync updates the DB.

```json
{
  "key": "myapp-preview",
  "name": "MyApp Preview",
  "repos": [{ "url": "https://github.com/org/myapp.git", "primary": true }],
  "codeExecution": { "setupCommands": ["bun install"] },
  "servers": [
    {
      "name": "dev",
      "label": "Dev Server",
      "port": 3000,
      "startCommands": ["bun run dev -- --host 0.0.0.0"],
      "envVars": { "VITE_API_URL": "http://localhost:8080" },
      "startupTimeoutSeconds": 30,
      "autoReload": true,
      "enableInspector": true,
      "default": true,
      "resources": {
        "requests": { "cpu": "200m", "memory": "1Gi" },
        "limits": { "cpu": "1", "memory": "2Gi" }
      }
    }
  ]
}
```

CLI — for `SOURCE=manual` workspaces. `--servers-json` **replaces** the whole
array, so include every server, not just the one you changed:

```bash
moat workspaces update ws_<id> --servers-json '[
  {"name":"dev","port":3000,"startCommands":["bun run dev -- --host 0.0.0.0"],
   "envVars":{"VITE_API_URL":"http://localhost:8080"},"default":true}
]'
```

On a git-synced workspace the same flags exist as `moat workspaces override`,
which flips `source` to `manual` and makes the committed file stop applying.
`moat workspaces reset-git <ws_id>` hands it back to git and **replaces local
changes with the committed config**.

Either way, existing tasks keep their old pod until
`moat tasks redeploy $TASK_ID`. New tasks pick it up on creation.

## Where an override physically lives

There is one copy, on disk, and no database record:

```
/opt/moatless/workspace/.moatless/servers/<name>/
├── override.json   the override as authored — what `servers list` reports
├── command         derived; absent = use the baked start command
├── env             derived shell fragment, sourced after server.env
└── generation      monotonic counter; a bump means "restart the child"
```

`command` and `env` are derived for the sidecar's benefit — its supervisor is
bash in an arbitrary `dockerImage` and can't be assumed to have `jq`. You can
read all four from inside the sandbox; they're a sibling of the repo checkouts,
not inside one.

Because it's workspace state, an override lasts exactly as long as the workspace
does:

| Storage tier | Override survives a pod delete? |
|---|---|
| `ebs` (default) | yes — same PVC as the repos |
| `s3` | yes — rides the whole-workspace tar |
| Docker | yes — host bind mount |
| `node` | only if the replacement lands on the same node |
| `ephemeral` | no — nothing does, including your checkout |

The consequence for debugging: **`servers list` needs a running sandbox** to
report overrides. On a stopped one it prints `override: unknown`, not
`override: none` — don't read the former as "there isn't one".

The sidecar restarts in place only when `generation` advances. A server that
exits on its own still takes the container down — which is why self-crashes
count against the crash budget and your restarts do not.

Design notes and rationale: `docs/plans/live-server-config-reload.md`.
