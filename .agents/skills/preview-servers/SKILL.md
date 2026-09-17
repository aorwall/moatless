---
name: preview-servers
description: >-
  Troubleshoot and reconfigure a task's preview servers — read their logs and
  process health, try a start command or env var live without a redeploy, then
  make the working config permanent. Use when a preview server is down, blank,
  crash-looping, serving the wrong thing, or missing an env var; when the start
  command or port needs changing; or when a live override needs promoting to
  the workspace config.
---

# Preview Servers

A preview server is a **sidecar container** next to the agent's sandbox
container, one per entry in the workspace's `servers[]`. It is not supervised by
the agent — you cannot see or signal its process. Everything you do to it goes
through the `moat` CLI.

Work in three beats. The override is a **scratchpad**; the workspace config is the
**record**. Never stop at a working scratchpad.

1. **Observe** — what is it doing, and what is it running?
2. **Override** — change it live, in seconds, and watch the logs.
3. **Promote** — write the proven config where it survives the task.

## 1. Observe

```bash
moat tasks status  $TASK_ID --json        # servers[] + containerDiagnostics[]
moat tasks servers $TASK_ID list          # status + the override in effect
moat tasks logs    $TASK_ID --server dev --no-follow   # `--server <name>`, not `<name>-server`
moat tasks logs    $TASK_ID --server dev --no-follow --tail 2000   # more history
moat tasks logs    $TASK_ID --server dev --previous --no-follow    # the instance that died
moat tasks logs    $TASK_ID --server dev               # follow live, until Ctrl+C
```

Use `--no-follow` unless you actually want to watch. The bare form streams
until something kills it; wrapping it in `timeout` truncates at an arbitrary
point and loses the *start* of a build, which is usually where the error is.
`--tail N` (default 500) sets how far back it starts.

`containerDiagnostics[]` is your process view: `restartCount`, `lastExitCode`,
`lastTerminatedReason`. A rising `restartCount` with `lastExitCode != 0` is a
crash loop; `OOMKilled` is a memory limit, not a bug in the start command.

**Server status alone lies about the app.** `started` only means the readiness
probe got a TCP connect on the port. Ask for real bytes:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' http://localhost:3000/   # from inside the sandbox
moat tasks preview $TASK_ID --server dev --path /                  # from anywhere
```

`localhost` works from inside the sandbox because pod containers share a network
namespace — that shared namespace is the *only* thing you share with the server.

**What you cannot do from inside the sandbox** — reach for logs and `curl`
instead of burning turns on these:

| Want | Why it fails | Instead |
|---|---|---|
| `ps` / `pgrep` the server process | separate PID namespace | `containerDiagnostics` + logs |
| `ss` / `netstat` for the listener | not installed, and same PID namespace limit | `curl localhost:$PORT` |
| `kill -HUP` the server to reload | can't signal another container | `moat tasks servers $TASK_ID restart <name>` |
| `kubectl` anything | usually not installed in the sandbox | `moat` CLI |

With cluster access from outside, the kubectl playbook is in
[sandbox-troubleshooting](../sandbox-troubleshooting/references/kubectl.md).

**Completion criterion:** you can name the server's status, its restart count and
last exit reason, the command it is actually running (`servers list` prints the
workspace's declared command and any override that replaces it), and whether the
port returns bytes.

## 2. Override

Task-scoped, applied in seconds, no pod rebuild, agent session untouched:

```bash
moat tasks servers $TASK_ID set dev --start-command "bun run dev -- --host 0.0.0.0 --port 3000"
moat tasks servers $TASK_ID set dev --env VITE_API_URL=http://localhost:8080 --env LOG_LEVEL=debug
moat tasks servers $TASK_ID set dev --unset LOG_LEVEL      # removes it even if the pod spec injected it
moat tasks servers $TASK_ID set dev --env FOO=bar --no-restart   # stage it, restart later
moat tasks servers $TASK_ID restart dev                    # re-run, unchanged config
moat tasks servers $TASK_ID reset dev                      # drop the override entirely
```

All of it works **from inside the sandbox** — unlike `tasks restart` /
`tasks redeploy`, none of it touches the agent container.

Every `set` and `restart` prints the resulting override and a generation number.
After each one, read the logs before changing anything else:

```bash
moat tasks servers $TASK_ID set dev --start-command "..." && sleep 5 && \
  moat tasks logs $TASK_ID --server dev --no-follow
```

Change one thing per cycle. Two changes and a still-broken server tells you
nothing about which one was wrong.

The override is **workspace state, not database state** — files under
`/opt/moatless/workspace/.moatless/servers/`. It wins over the workspace config
until you `reset`. Persistence depends on the workspace's independent PVC/S3
settings and the last successful snapshot; see [server config](references/server-config.md).

One practical consequence: reading an override needs a **running** sandbox.
`servers list` against a stopped one prints `override: unknown` rather than
`override: none`.

### Guardrails

- **`--env` is for configuration, not secrets.** Override values are stored as
  plaintext. Real secrets go through `moat secret set`, which lands them in
  `server.env` within 15 minutes; a `servers restart` then picks them up.
- **Reserved keys are rejected** (`PORT`, `WORKSPACE_PATH`, `DATA_DIR`, and the
  platform's credential keys). Nothing is written when a key is rejected.
- **`port` and `dockerImage` cannot change live** — the port is wired into the
  Service and IngressRoute, the image is the container's identity. Both need
  `moat tasks redeploy $TASK_ID` after a config change, and both return an
  explicit error rather than silently doing nothing. A redeploy of your own task
  replaces the pod you are running in; see Promote below.
- **You have a crash budget of 5.** A server that *exits on its own* five times
  is auto-disabled and forces a redeploy. Restarts you trigger do not count —
  only a self-exit does. So a start command that dies immediately burns the
  budget fast: fix it or `reset` before retrying blindly.

**Completion criterion:** the server reaches `started` and the port returns the
bytes you expect, with exactly one override you can read back from
`servers list`.

## 3. Promote

A live override is scoped to one task and invisible to everyone else's. Once the
config works, write it down. Two destinations, and the choice is not a
preference:

| Where the workspace came from | Promote to | How |
|---|---|---|
| `.moatless/workspaces.json` in the repo (`SOURCE=git` in `moat workspaces list`) | the repo config | edit the file, commit, push — git sync updates the DB |
| Created in the UI/CLI (`SOURCE=manual`) | the workspace row | `moat workspaces update <ws_id> --servers-json '[...]'` |

Writing to a git-synced workspace with `moat workspaces override` flips it to
`manual`, so the committed file silently stops applying. Prefer editing the file.
Use `override` only for a deliberate one-cluster deviation, and
`moat workspaces reset-git <ws_id>` to hand it back to git.

Field names differ between the two layers — the CLI/API speak camelCase
(`startCommands`), and the full `servers[]` schema plus the env-precedence rules
are in [references/server-config.md](references/server-config.md).

Then land it and clean up:

```bash
moat tasks servers $TASK_ID reset dev    # drop the scratchpad first
moat tasks redeploy $TASK_ID --yes       # rebuild on the new config (~1-2 min)
```

Reset before redeploying: the override survives a redeploy and would keep
shadowing the config you just promoted, leaving you unable to tell whether the
committed version actually works.

`$TASK_ID` here is your own task, so the redeploy replaces the pod you are
running in. `--yes` is what confirms that. This command and your current turn
die with the pod, so print nothing after it and check nothing after it. The new
pod resumes your session and sends you a `<sandbox-restart-note>`. Run
`moat tasks status $TASK_ID --json` in that turn, to confirm the server came
back on the promoted config.

**Completion criterion:** the override is gone, the config lives in the repo or
the workspace row, and a fresh sandbox reaches `started` without it.

## Making a preview inspectable

Element selection in the Moatless UI, and element picking from T3 Code, both
need `@moatless/inspector`'s client script to be *in* the previewed page. A
cross-origin frame cannot be injected into from outside, so the page has to load
it — nothing in the ingress adds it.

The package is baked into the sandbox image at `/opt/node_modules`, which is on
Node's resolution walk from any repo under `/opt/moatless/workspace`. So there
is nothing to install; a Vite project only adds the plugin:

```ts
// vite.config.ts
import inspectorPlugin from '@moatless/inspector/vite'
import componentTagger from '@moatless/inspector/vite-tagger'

export default defineConfig({
  plugins: [
    { ...componentTagger(), enforce: 'pre' as const },  // before the JSX compiler
    // ...the project's own plugins
    inspectorPlugin(),
  ],
})
```

`componentTagger()` stamps `data-component-id` onto every JSX element and must
run before `@vitejs/plugin-react`, or there is no JSX left to tag — that is what
maps a click back to a source location. `inspectorPlugin()` inlines the client
into the served HTML, and also the T3 annotation client, which resolves sources
by walking React fibers and so works without the tagger.

Restart the server after editing the config. Confirm it took with
`curl -s localhost:$PORT | grep -c INSPECTOR_SCRIPT_LOADED` — a page serving the
client contains that string. Non-Vite projects (Next.js, Remix) need the webpack
loader for tagging and have no injection hook; treat them as not yet supported.

## Symptoms

| Symptom | Most likely | Next move |
|---|---|---|
| `stopped`, never starts | server not in the workspace's `servers[]`, or auto-disabled after 5 crashes | `moat tasks servers $TASK_ID list`; check `moat events list $TASK_ID --types server` |
| `disabled`, and `logs` answers 404 | the task is headless, so provisioning dropped every server the workspace declares and created no sidecar | nothing to debug in the server; recreate the task with `--headless false` |
| `failed` right after provisioning | start command exits, or never binds the port within `startupTimeoutSeconds` | `logs --previous`; bind `0.0.0.0`, not `127.0.0.1` |
| `started` but the page is blank / 502 | bound the wrong interface or port | `curl localhost:$PORT`; compare the port in `servers list` with what the logs say it bound |
| Restart count climbing | crash loop | `logs --previous` for the exit reason; `OOMKilled` ⇒ raise the server's `resources.limits` (redeploy) |
| Env var missing in the app | workspace-level `envVars` never reach sidecars — a known gap | set it per-server, or live via `servers set --env` |
| Change to a `package.json` script had no effect | the repo mount is shared, but the running process isn't re-read | `moat tasks servers $TASK_ID restart <name>` |
| Wrong start command running | a stale override from earlier debugging | `servers list`, then `servers reset` |
| The command running isn't the one the repo's config declares | the workspace is `source=manual` and no longer follows the file, or the file used a key the loader ignores | `servers list` prints the workspace command beside the override; a repository sync names the keys it ignored |
| Preview loads, but element selection is unavailable | the project never added `inspectorPlugin()` — nothing injects it for you | [Making a preview inspectable](#making-a-preview-inspectable) |

For anything wider than the server container — pod evictions, OOM of the sandbox
itself, the agent unresponsive, the task stuck — switch to the
[sandbox-troubleshooting](../sandbox-troubleshooting/SKILL.md) skill.

Resolve `workspaceId` and `sandboxId` from task details before promotion or runtime lookup. Use returned preview URLs; shared tasks need not own a sandbox named after their task ID.
