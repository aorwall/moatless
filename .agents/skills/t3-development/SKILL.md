---
name: t3-development
description: "Develop and test the T3 Code client against this backend's t3code surface from inside a sandbox — the two-repo workspace where T3's Vite server on :5733 proxies /api and /ws to a moatless-vibe-backend on :8080. Covers the shadow rebuild loop, headless UI-RPC probing with an API key and a WebSocket ticket, and end-to-end browser verification with Playwright — against the preview URL when a shadow backend exists, or against the deployed backend through a local Vite on an allow-listed https origin when it does not. Use when working on crates/t3code/** or backend/src/t3code/**, on the T3 side of the Moatless adapter, when verifying a client change in a real browser, or when the T3 sidebar renders empty, the socket 403s, or messages never load."
---

# T3 Code against this backend

The `t3code (Moatless-backed)` workspace checks out **both repos side by side**
under `/opt/moatless/workspace/` and runs each side as a sidecar container:

| Server | Port | Serves | Reload |
| --- | --- | --- | --- |
| `web` | 5733 | Vite dev server for `t3code/apps/web`, `T3CODE_PROXY_TARGET=http://127.0.0.1:8080` | HMR, automatic |
| `backend` | 8080 | `scripts/shadow/up.sh` → `moatless-vibe-backend` | manual, see below |

T3's own Node server (`t3code/apps/server`) is **not** running and never will be.
Every backend path the client calls is proxied to this backend's t3code
compatibility surface, `crates/t3code/`, with root adapters in
`backend/src/t3code/`. This is the same harness as the
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
and no Origin gets a **403** (`crates/t3code/src/lib.rs`). Trade the key
for a 60-second ticket at `POST /api/auth/websocket-ticket` and pass it as
`wsTicket` (`crates/t3code/src/oauth.rs`) — *that* is what lets a script connect
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

**Which harness you have decides everything below.** Check before you plan:

```bash
curl -s --max-time 5 http://localhost:8080/health   # shadow backend, or silence
kubectl auth can-i get secrets -n moatless          # what the shadow needs to start
```

| | Shadow backend | Deployed backend only |
| --- | --- | --- |
| Backend | `scripts/shadow/up.sh` on `:8080` | `moatless-vibe-backend.moatless.svc.cluster.local:8080` |
| Browse from | the preview URL | local Vite behind a TLS front door |
| Backend edits | `rebuild.sh` | not yours — hand them over |

`sync-env.sh` needs `get secrets`. When that is **no** and nothing answers on
`:8080`, you have the deployed backend only: client-side changes are fully
verifiable against it, backend changes are not testable here at all. Say so
rather than reporting a client-side pass as if it covered both.

### Against the shadow: use the preview URL

**Never `localhost:5733`.** `T3_UI_RPC_ALLOWED_ORIGINS` is synced from the live
Deployment, so a browser on `http://localhost:5733` gets **403 on the WebSocket
upgrade** and the app renders "No projects yet" — a convincing and entirely false
empty state. HTTP through the Vite proxy still works, which is what makes the
trap hard to read.

```
https://$TASK_ID--5733.preview.moatless.xyz/
```

The user is already authenticated there; hand them the URL rather than
describing what you saw. Ask before driving a browser yourself.

### Against the deployed backend: a local Vite on an allow-listed origin

The Origin allow-list is the whole problem, and it is readable — do not guess:

```bash
kubectl get deploy -n moatless moatless-vibe-backend \
  -o jsonpath='{.spec.template.spec.containers[0].env[?(@.name=="T3_UI_RPC_ALLOWED_ORIGINS")].value}'
# https://moatless-t3.staging.moatless.xyz,https://*.preview.moatless.xyz
```

Every entry is **https**, and the Moatless session cookie is `Secure`. So the
browser has to believe it is on one of those names, over TLS, while talking to
your Vite. Three pieces, no repo edits.

Both servers run in the background. Capture each PID as you spawn it and stop
only those (`t3code/AGENTS.md` rule 1 — this machine runs other dev servers, and
one of them may already be a `vp dev` you did not start).

```bash
HOST_NAME=moatless-t3.staging.moatless.xyz
SKILLS=/opt/moatless/workspace/moatless/.agents/skills/t3-development
mkdir -p /tmp/pw

# 1. Vite, proxying to the deployed backend, with the name allowed through
#    Vite's own DNS-rebinding check. `vp dev` lives at the t3code repo root.
(cd /opt/moatless/workspace/t3code/apps/web && \
  PORT=5733 HOST=127.0.0.1 \
  T3CODE_PROXY_TARGET=http://moatless-vibe-backend.moatless.svc.cluster.local:8080 \
  T3CODE_DEV_ALLOWED_HOSTS=$HOST_NAME \
  ../../node_modules/.bin/vp dev > /tmp/pw/vite.log 2>&1) &
VITE_PID=$!

# 2. TLS in front of it — a self-signed cert for that exact name.
openssl req -x509 -newkey rsa:2048 -nodes -days 5 \
  -keyout /tmp/pw/key.pem -out /tmp/pw/cert.pem \
  -subj "/CN=$HOST_NAME" -addext "subjectAltName=DNS:$HOST_NAME"
node $SKILLS/references/tls-front.mjs --listen 8444 --target 5733 &
TLS_PID=$!

# 3. Chromium, resolving that name to the front door.
#    --host-resolver-rules=MAP <host> 127.0.0.1:8444 --ignore-certificate-errors

# …and afterwards:
kill $TLS_PID $VITE_PID
```

`vp dev` spawns a worker that **outlives the parent you killed**, so confirm the
port actually came back before walking away:

```bash
(exec 3<>/dev/tcp/127.0.0.1/5733) 2>/dev/null && echo "still up" || echo "released"
ss -H -ltnp 2>/dev/null | grep :5733    # the owner — often blank in a sandbox
```

If it is still up, rule 1 is the whole game: kill the port's owner from `ss`
after confirming `/proc/<pid>/cwd` is your `apps/web`. When `ss` cannot see the
owner — common here, since it only reports processes in your own namespace —
read `/proc/*/cwd` and `/proc/*/cmdline` directly and confirm the PID is one
**you** started. Do not `pgrep -f … | kill`: your own agent process carries the
pattern in its argv and matches, and so does the pre-existing `vp dev` this
workspace may already be running.

**Do not add `server.https` to `t3code/apps/web/vite.config.ts`.** It is an
upstream file, so it is a fork delta paid at every merge — for something only a
test needs. Terminating TLS outside the repo costs nothing and leaves no diff.

`references/browser-check.mjs` does all of it and asserts the result:

```bash
node $SKILLS/references/browser-check.mjs \
  --backend http://moatless-vibe-backend.moatless.svc.cluster.local:8080 \
  --host $HOST_NAME --port 8444
# app /ws sockets: 1
# project shown:   soap-tm (backend lists 7)
# console errors:  0
# PASS
```

The first request to a cold Vite takes ~13 s while it transforms the module
graph. That is not a hang, and it is why the checks below poll rather than sleep.

### Playwright in this image

Install into a scratch directory and run from there. Neither repo uses npm —
t3code is pnpm, this one is bun — so an `npm install` inside either checkout
leaves an `node_modules` tree and lockfile churn unrelated to your change.

```bash
mkdir -p /tmp/pw && cd /tmp/pw
npm install --no-save --no-package-lock playwright@latest
ls "${PLAYWRIGHT_BROWSERS_PATH:-/opt/playwright-browsers}"   # chromium-<build>
```

Chromium is already baked into the image (`docker/Dockerfile.sandbox-base`), and
`PLAYWRIGHT_BROWSERS_PATH` is often already pointing at it — check before
setting it, and only set it if a run starts re-downloading 265 MB.

**Do not try to match Playwright's version to the baked build.** They drift: at
the time of writing `@latest` is 1.62.1 and wants `chromium_headless_shell-1234`,
while the image bakes `chromium-1194`, so a default launch dies with *"Executable
doesn't exist"*. Name the binary instead and the question disappears — any recent
Playwright drives an older chrome:

```js
executablePath: "/opt/playwright-browsers/chromium-1194/chrome-linux/chrome"
```

`browser-check.mjs` discovers that path itself. Launch with
`args: ["--no-sandbox"]`; for headed mode there is a `playwright-cli` wrapper on
`PATH` that runs under xvfb.

One more trap for a script kept in this repo: **ESM resolves imports from the
script's own directory, not the cwd**, so `import { chromium } from "playwright"`
in `.agents/skills/...` cannot see `/tmp/pw/node_modules` however you invoke it.
Reach it with `createRequire("/tmp/pw/")("playwright")`.

### What a healthy load looks like

- Exactly **one** app socket, `wss://<host>/ws?...`. Vite's HMR socket is a
  second, separate `ws://127.0.0.1:5733/?token=…` on the dev origin — match the
  path so you do not count it as the app's.
- A project name **the backend actually lists** rendered in the sidebar.
- Zero console errors.

**"No projects yet" clears on first paint, before any data arrives.** So a check
that waits for that string to disappear passes against a blank page — which is
exactly the failure it was written to catch. Assert a real name instead:
`GET /api/v1/tasks?limit=20`, take the `repositoryName`s, and wait for one of
them to appear.

## The rebuild loop

T3 edits under `t3code/apps/` and `t3code/packages/` hot-reload — do nothing.

Backend edits need a compile and a restart. `up.sh` watches a trigger file on the
shared volume; `rebuild.sh` compiles in your container and bumps it:

```bash
cd /opt/moatless/workspace/moatless
bash scripts/shadow/rebuild.sh        # compile + signal; the restart lands ~5s later
curl -s http://localhost:8080/health  # confirm it came back
```

Budget for it. Measured in this pod, for one edit under `crates/t3code/`:

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

Wire-shape changes belong in `crates/t3code/src/projection.rs`'s test module,
asserted field-for-field against `t3code/packages/contracts/src/orchestration.ts`.
A projection that compiles and renders can still be wrong in a field no current
client reads.

## Which repo owns the change

| Symptom | Usually |
| --- | --- |
| Route 404s, an RPC answers a `Die` defect, a field is missing from a payload | `crates/t3code/` or `backend/src/t3code/` |
| The payload is right but the UI misreads it | `t3code/apps/web`, `t3code/packages/client-runtime` |
| Client and server disagree about a shape | `t3code/packages/contracts` is the contract; the Rust side transcribes it by hand |

`moat gh pr create` infers `-R` from the repository you run it in, so it opens
PRs against **either** checkout — run it from the one the change lives in. Say
which repo each PR is against, so a reviewer is not hunting in the wrong one.

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
| Playwright says "Executable doesn't exist" | Browser-build mismatch, and version-chasing will not fix it. Pass `executablePath` naming the baked `chromium-<build>` binary. |
| `ERR_MODULE_NOT_FOUND: playwright` from a script in this repo | ESM resolves from the script's directory, not the cwd. `createRequire("/tmp/pw/")("playwright")`. |
| Browser check passes but the page is blank | It asserted on "No projects yet" being absent, which is true on first paint. Assert a project name the backend lists. |
| Vite answers "Blocked request. This host is not allowed" | `T3CODE_DEV_ALLOWED_HOSTS` does not include the name you browsed. |
| WS 403 against the deployed backend | Origin is not on `T3_UI_RPC_ALLOWED_ORIGINS`, or you reached Vite over http. Both entries are https. |
| `sync-env.sh` fails | Needs `get deployments` / `get secrets`: `kubectl auth can-i get secrets -n moatless`. Without `get secrets` there is no shadow — see the harness table. |
| T3 edit not showing | HMR runs in the sidecar. `curl -s localhost:5733/ \| head`, then `moat tasks servers $TASK_ID restart web`. |
