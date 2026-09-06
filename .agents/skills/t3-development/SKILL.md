---
name: t3-development
description: "Develop and verify the T3 client against Moatless UI-RPC from a sandbox. Use for the two-repository shadow setup, browser/proxy authentication, T3 integration failures, and backend rebuilds."
---

# T3 client integration

The paired workspace checks out Moatless and T3. Its `web` server runs the T3
client on 5733 and proxies `/api` and `/ws` to `moatless-backend` on 8080. T3's
standalone Node backend is not part of this arrangement. Inspect the actual T3
checkout's scripts and proxy configuration before starting another dev server.

Backend protocol behavior belongs to [crates/t3code](../../../crates/t3code), with
composition in [server t3code](../../../crates/server/src/t3code). Client contracts
live in the separate T3 checkout; verify them there before changing wire shapes.
Use [shadow-dev](../shadow-dev/SKILL.md) for workspace setup and isolation.

## Establish backend readiness

```bash
curl --fail --max-time 10 http://localhost:8080/health
moat tasks logs <task-id> --server backend --no-follow --tail 2000
```

A web server can be ready while the backend is still compiling. Managed sidecars
share the Kubernetes pod network, but their processes are outside the agent's
PID namespace; control them through `moat tasks servers`.

Use [rpc-verification](../rpc-verification/SKILL.md) and its shared probe for
protocol reads, payload sizes, failures, and subscription transitions. A check
against the deployed backend does not verify an unreleased backend edit.

## Browser verification

Prefer the returned preview URL. A blank sidebar needs evidence from the app's
`/ws` handshake, console, and backend data; the absence of an empty-state label
is not proof that project data loaded.

The shadow sync script adds `http://localhost:5733` and
`http://127.0.0.1:5733` to its Origin allow-list. Confirm the effective backend
environment and browser authentication before using localhost. For a deployed
backend, discover the configured allowed origins instead of assuming localhost
is permitted. Secure cookies require an appropriate HTTPS origin.

The [browser checker](references/browser-check.mjs) requires all of the following:

- Vite already running on 5733 with its proxy pointing at the selected backend.
- A selected HTTPS hostname accepted by Vite and the backend Origin policy.
- A TLS front door already running, and Playwright installed in the scratch
  directory passed with `--scratch` (default `/tmp/pw`).
- `MOATLESS_API_KEY` and backend tasks with at least one repository name to assert.
- A compatible Chromium installation. The helper checks the baked browser path,
  then falls back to Playwright's own executable; install that browser if absent.

For a local TLS front door, create a certificate for the selected hostname and
start [tls-front.mjs](references/tls-front.mjs):

```bash
node .agents/skills/t3-development/references/tls-front.mjs \
  --listen 8444 --target 5733 --cert /tmp/pw/cert.pem --key /tmp/pw/key.pem
node .agents/skills/t3-development/references/browser-check.mjs \
  --backend http://localhost:8080 --host <allowed-host> --port 8444 --scratch /tmp/pw
```

The checker maps the selected hostname to the local TLS port, mints a session,
sets a host-specific cookie, and checks `/ws`, a real repository name, and console
errors. It does not start Vite or TLS. Track processes you start and clean up only
those processes; do not kill other workspace dev servers by a broad name match.

## Rebuild and diagnose

Confirm a backend server exists before rebuilding — `moat tasks servers <task-id> list`.
[shadow-dev](../shadow-dev/SKILL.md) owns `rebuild.sh` and states what the build costs
without one.

Set `SHADOW_CARGO_PROFILE=dev` in both the backend server env and rebuild shell,
then run `bash scripts/shadow/rebuild.sh`. Verify health and the changed behavior.
Use root Rust lint/test commands; do not add a `cargo check` loop.

| Symptom | Check |
|---|---|
| `/ws` 403 | Actual Origin, session/ticket auth, selected backend, allowed origins. |
| HTTP 404 | Proxy target and current backend route surface. |
| No projects | Successful app socket plus expected project data, not just first paint. |
| Contract field missing | T3 contracts and Rust projection tests. |
| Vite host rejected | Allowed-host configuration in the current T3 checkout. |
| Backend changes absent | Backend version, shadow profile, restart trigger, and build logs. |

Report which checkout changed and which backend the verification exercised.
