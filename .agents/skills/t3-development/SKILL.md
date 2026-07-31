---
name: t3-development
description: "Develop and test the T3 Code client against this backend's UI-RPC surface from inside a sandbox — the two-repo workspace where T3's Vite server on :5733 proxies /api and /ws to a shadow moatless-vibe-backend on :8080. Covers the shadow rebuild loop, headless UI-RPC probing with an API key and a WebSocket ticket, and browser verification against the preview URL. Use when working on backend/src/ui_rpc/**, on the T3 side of the Moatless adapter, or when the T3 sidebar renders empty, the socket 403s, or messages never load."
---

# T3 Code against this backend

The `t3code (Moatless-backed)` workspace checks out **both repos side by side**
under `/opt/moatless/workspace/` and runs each side as a sidecar container:

| Server | Port | Serves | Reload |
| --- | --- | --- | --- |
| `web` | 5733 | Vite dev server for `t3code/apps/web`, `T3CODE_PROXY_TARGET=http://127.0.0.1:8080` | HMR, automatic |
| `backend` | 8080 | `scripts/shadow/up.sh` → `moatless-vibe-backend` | manual, see below |

T3's own Node server (`t3code/apps/server`) is **not** running and never will be.
Every backend path the client calls is proxied to this backend's UI-RPC
compatibility surface, `backend/src/ui_rpc/`. This is the same harness as the
[`shadow-dev`](../shadow-dev/SKILL.md) skill — the shadow backend, the same
`scripts/shadow/` scripts, the same shared dev Postgres/Redis/S3 — with T3's
client in front of it instead of `apps/frontend`. Read that skill for how the
shadow itself works; this one is about the pair.

Because a proxy target is configured, the client runs in **Moatless cookie-auth
mode** (`t3code/apps/web/src/environments/primary/auth.ts`,
`VITE_MOATLESS_PROXY_AUTH`). There is no pairing step and no one-time token.

Neither server is your child process — `ps`, `pgrep`, `kill` and `ss` cannot see
across the container boundary. Everything you do to them goes through `moat`;
see [`preview-servers`](../preview-servers/SKILL.md).

**What t3code's own instructions get wrong here.** `t3code/AGENTS.md` under "Dev
servers" and "Test data" — `vp run dev`, pairing URLs, worktree `.t3`, seeding
`state.sqlite` — describes the upstream standalone layout, and so does the
`test-t3-app` skill that workspace installs. None of it applies. State lives in
Postgres, and the dev servers are already running.

## First: is the backend actually up?

A fresh sandbox compiles the backend before it can serve anything — **about 8
minutes** from provision. `moat tasks status` reporting `started` only means a
TCP probe succeeded, and `web` comes up long before `backend` does. A T3 page
loaded in that window looks like a broken integration rather than a backend that
is still linking.

```bash
curl -s http://localhost:8080/health                        # {"status":"ok"} or nothing
timeout 20 moat tasks logs $TASK_ID --server backend        # streams forever; always bound it
```

## Testing the UI-RPC surface without a browser

Every HTTP route accepts `Authorization: Bearer $MOATLESS_API_KEY` directly — no
cookie, no session, no Origin header. Prefer this for anything you can check
without pixels.

```bash
curl -s http://localhost:8080/.well-known/t3/environment                      # public
curl -s -H "Authorization: Bearer $MOATLESS_API_KEY" \
     http://localhost:8080/api/orchestration/shell | head -c 400
curl -s -H "Authorization: Bearer $MOATLESS_API_KEY" \
     http://localhost:8080/api/orchestration/threads/$THREAD_ID | head -c 400
```

**The socket is stricter than the routes.** A bearer key alone is not enough
there: `origin_checked` skips the Origin allow-list only for
`AuthMethod::WebSocketTicket`, so an upgrade carrying `Authorization: Bearer …`
and no Origin gets a **403** (`backend/src/ui_rpc/mod.rs:860,896`). Trade the key
for a 60-second ticket at `POST /api/auth/websocket-ticket` and pass it as
`wsTicket` (`backend/src/ui_rpc/auth.rs`) — *that* is what lets a script connect
from localhost when a browser cannot.

`references/rpc.mjs` does the exchange, sends one Effect-RPC envelope,
answers keepalives, acks chunks, and prints each frame's serialized size. From
the repo root:

```bash
node .agents/skills/t3-development/references/rpc.mjs server.getConfig
node .agents/skills/t3-development/references/rpc.mjs \
  orchestration.subscribeShell '{}' --stream --wait=6000
node .agents/skills/t3-development/references/rpc.mjs \
  orchestration.subscribeThread "{\"threadId\":\"$THREAD_ID\"}" --stream
```

It prints `<- Chunk 835707B`, so it doubles as the payload-size check. Snapshots
are large — the shell snapshot is ~800 KB, a busy thread ~280 KB — and a
regression that re-sends a whole snapshot per change is invisible in a browser
and obvious here.

An unimplemented method answers `Exit/Failure` with a `Die` defect naming the
phase-one server; a *typed* refusal is a `Fail`. Both are normal — check which
one you got before concluding anything.

## Testing the T3 UI in a browser

**Use the preview URL, never `localhost:5733`.** `T3_UI_RPC_ALLOWED_ORIGINS` is
synced from the live Deployment, where it is `https://*.preview.moatless.xyz`
only, so a browser on `http://localhost:5733` gets **403 on the WebSocket
upgrade** and the app renders "No projects yet" — a convincing and entirely false
empty state. HTTP through the Vite proxy still works, which is what makes the
trap hard to read.

```
https://$TASK_ID--5733.preview.moatless.xyz/
```

The user is already authenticated there; hand them the URL rather than
describing what you saw. Ask before driving a browser yourself.

For a headless check, mint a session cookie and set it on
`.preview.moatless.xyz`:

```bash
curl -si -X POST http://localhost:8080/api/v1/auth/api-key/session \
  -H "Authorization: Bearer $MOATLESS_API_KEY" -H 'Content-Type: application/json' \
  -d '{}' | grep -oP 'moatless_session=\K[^;]+'
```

Chromium **is** already in the image (`docker/Dockerfile.sandbox-base`), but the
cluster's sandbox cache volume overrides `PLAYWRIGHT_BROWSERS_PATH` to an empty
node-local mount, so a naive run re-downloads 265 MB and a pinned old Playwright
then fails on a browser-build mismatch. Point it back at the image and match its
build:

Install it in a scratch directory and run the script from there. Neither repo
uses npm — t3code is pnpm, this one is bun — so an `npm install` inside either
checkout leaves an `node_modules` tree and lockfile churn that has nothing to do
with the change you are verifying.

```bash
mkdir -p /tmp/pw && cd /tmp/pw
npm install --no-save --no-package-lock playwright@latest   # matches the build in the image
export PLAYWRIGHT_BROWSERS_PATH=/opt/playwright-browsers
```

The version has to match the **browser build** the image baked, which
`ls /opt/playwright-browsers` shows as `chromium-<build>`. `@latest` is right
while the image installs Playwright unpinned; if it errors with *"Executable
doesn't exist"*, the image was pinned — install the Playwright release that
ships that build instead. Do not copy a version from a `package.json`:
`t3code/apps/desktop` pins 1.60.0, which wants a build the image does not have.

Launch with `args: ["--no-sandbox"]`. For headed mode there is a
`playwright-cli` wrapper on `PATH` that runs under xvfb.

A healthy load shows exactly **one** `/ws` socket, zero console errors, and real
Workspace names in the sidebar.

## The rebuild loop

T3 edits under `t3code/apps/` and `t3code/packages/` hot-reload — do nothing.

Backend edits need a compile and a restart. `up.sh` watches a trigger file on the
shared volume; `rebuild.sh` compiles in your container and bumps it:

```bash
cd /opt/moatless/workspace/moatless
bash scripts/shadow/rebuild.sh        # compile + signal; the restart lands ~5s later
curl -s http://localhost:8080/health  # confirm it came back
```

Budget for it. Measured in this pod, for one edit under `backend/src/ui_rpc/`:

| | time |
| --- | --- |
| `cargo build --release -p moatless-vibe-backend` (what `rebuild.sh` runs) | **4 m 34 s** |
| `cargo build -p moatless-vibe-backend` (dev profile) | 21 s |
| `cargo check -p moatless-vibe-backend`, first run in a sandbox | 2 m 19 s |
| backend restart after the trigger | ~5 s |

The release profile is `lto = "thin"` and shares no artifacts with the dev
profile your tests and clippy build, so an edit-rebuild-per-file rhythm costs you
twice. Iterate on `cargo check`, batch the backend edits, and pay the release
build once when the change is finished.

Other levers, all from [`shadow-dev`](../shadow-dev/SKILL.md):

- **Env changed on the real Deployment** — `bash scripts/shadow/sync-env.sh`, then `rebuild.sh`.
- **Extra env for the shadow** — `KEY=value` lines in `/opt/moatless/data/shadow/overrides.env`, then `rebuild.sh`. These win over the synced values; it is how you'd set `RUST_LOG=debug` or widen `T3_UI_RPC_ALLOWED_ORIGINS`.
- **Backend wedged** — `moat tasks servers $TASK_ID restart backend` re-runs `up.sh` against the warm target.

## Before calling a backend change done

```bash
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test  -p moatless-vibe-backend
```

Both build the dev profile. The first `cargo test` in a fresh sandbox is a cold
codegen of the whole dependency graph — several minutes, not a hang.

Wire-shape changes belong in `backend/src/ui_rpc/projection.rs`'s test module,
asserted field-for-field against `t3code/packages/contracts/src/orchestration.ts`.
A projection that compiles and renders can still be wrong in a field no current
client reads.

## Which repo owns the change

| Symptom | Usually |
| --- | --- |
| Route 404s, an RPC answers a `Die` defect, a field is missing from a payload | `backend/src/ui_rpc/` |
| The payload is right but the UI misreads it | `t3code/apps/web`, `t3code/packages/client-runtime` |
| Client and server disagree about a shape | `t3code/packages/contracts` is the contract; the Rust side transcribes it by hand |

A task in this workspace is bound to the **t3code** repository, so
`moat gh pr create` refuses a PR against this repo (*"does not match task
repository"*). Open moatless-side PRs with the plain `gh` CLI instead, and say
which repo each PR is against so a reviewer is not hunting in the wrong one.

Background reading, in order: `CONTEXT.md` (the four colliding terms),
`t3code/docs/reference/moatless-concept-map.md` §6,
`docs/t3-code-current-state.md`. `docs/runbooks/t3-code-local-development.md`
documents the Docker Compose path, not this one.

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| Sidebar empty, "No projects yet" | Browsing `localhost:5733`. The WS upgrade is 403 on Origin. Use the preview URL. |
| Connection refused on :8080 | Backend still compiling. `timeout 20 moat tasks logs $TASK_ID --server backend`. |
| "Loading messages…" pill never clears | Thread detail is served but `subscribeThread` never emitted `{kind:"synchronized"}`, which `server.getConfig`'s `threadResumeCompletionMarker` promises. |
| Socket opens, snapshot arrives, then nothing | The feed is polling a fingerprint that did not move — in-place block merges keep the same row id. |
| Playwright re-downloads browsers | `PLAYWRIGHT_BROWSERS_PATH` override. Set it to `/opt/playwright-browsers`. |
| Playwright says "Executable doesn't exist" | Browser-build mismatch. Install the Playwright release that ships the `chromium-<build>` in `/opt/playwright-browsers`. |
| `sync-env.sh` fails | Needs `get deployments` / `get secrets`: `kubectl auth can-i get secrets -n moatless`. |
| T3 edit not showing | HMR runs in the sidecar. `curl -s localhost:5733/ \| head`, then `moat tasks servers $TASK_ID restart web`. |
