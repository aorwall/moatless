---
name: rpc-verification
description: "Call and verify Moatless's UI-RPC surface (server.*, orchestration.*, sandbox.*, servers.* — crates/t3code/, mounted from crates/server/src/t3code/) from inside any sandbox: the WebSocket ticket exchange, the rpc.mjs probe script, and how to read Chunk/Exit/Fail/Die frames. Works against the deployed cluster backend with no local setup, or a local shadow backend for unreleased changes. Covers what a plain read-and-print cannot: proving a push/subscription feature actually pushes, by creating a throwaway task, subscribing, forcing a real state transition with `moat tasks stop`/`start`, and cleaning up with `moat tasks close`. Use when asked to test, verify, smoke-test, or probe an RPC method or subscription, to check whether a capability has actually deployed, or to confirm a push channel pushes rather than just answering a read."
---

# Verifying the UI-RPC surface

Prove an RPC method — read, command, or subscription — behaves correctly
against a live backend, from inside a sandbox, without a browser.

## Which backend to point at

| | `MOATLESS_BASE` | Sees | Setup |
| --- | --- | --- | --- |
| Deployed cluster backend | `$BACKEND_API_URL` | Whatever's currently released | None — `$MOATLESS_API_KEY` is already in every sandbox's env |
| Local shadow backend | `http://localhost:8080` | Your uncommitted/unmerged changes | [`shadow-dev`](../shadow-dev/SKILL.md) or [`t3-development`](../t3-development/SKILL.md) harness running |

Check `server.getConfig`'s `environment.serverVersion` and `.capabilities` before
concluding a method doesn't exist. An `UnsupportedMethodError` against the
deployed backend can just mean your change hasn't rolled out yet — that's a
different problem than a bug, and the fix is to wait for the deploy or switch
to the shadow backend, not to debug the RPC handler.

## The ticket exchange

Every HTTP route on this surface accepts `Authorization: Bearer $MOATLESS_API_KEY`
directly. The socket does not: `origin_checked` skips the Origin allow-list only
for `AuthMethod::WebSocketTicket` (`crates/t3code/src/lib.rs`), so a bearer-only
upgrade gets a **403**. Trade the key for a 60-second ticket first:

```bash
curl -s -X POST "$MOATLESS_BASE/api/auth/websocket-ticket" \
  -H "Authorization: Bearer $MOATLESS_API_KEY"
```

then open `${MOATLESS_BASE/http/ws}/ws?wsTicket=<ticket>`. That's what lets a
script connect from localhost where a browser without the ticket gets refused.

## Probing with rpc.mjs

`.agents/skills/t3-development/references/rpc.mjs` does the exchange, sends one
Effect-RPC `Request` envelope, answers `Ping` with `Pong`, acks every `Chunk`,
and prints each frame's tag and serialized byte size. It only needs
`MOATLESS_BASE` and `MOATLESS_API_KEY` — nothing else from the T3 harness — so
it works standalone against the deployed backend.

```bash
node .agents/skills/t3-development/references/rpc.mjs <tag> [payloadJson] [--stream] [--wait=ms]

# a read
MOATLESS_BASE=$BACKEND_API_URL node .agents/skills/t3-development/references/rpc.mjs \
  sandbox.status '{"threadId":"<task-id>"}'

# a subscription
MOATLESS_BASE=$BACKEND_API_URL node .agents/skills/t3-development/references/rpc.mjs \
  sandbox.subscribeStatus '{"threadId":"<task-id>"}' --stream --wait=15000
```

Without `--stream` the script closes on the first `Exit` — right for reads and
commands. A subscription needs `--stream` and a `--wait` window wide enough to
matter: its first frame is a seed, and every frame after that is optional, so a
window that's too short can't tell "nothing changed" from "didn't wait long
enough to see it change".

## Reading the frames

- **`Exit/Success`** — the RPC answered; the value is in `exit.value`.
- **`Exit/Failure` + `Die`** — an *untyped* defect, usually a bug or an
  unimplemented handler. The message is at `exit.cause[0].defect`.
- **`Exit/Failure` + `Fail`** — a *typed* refusal the schema declares
  (`UnsupportedMethodError`, a domain error). The tagged error is at
  `exit.cause[0].error`. `Fail` on an unrecognized method usually means version
  skew (capability not deployed yet); `Die` usually means an actual bug — check
  which one you got before deciding which.
- **`Chunk`** — one push on a subscription; the payload is `values[0]`. The
  script acks it automatically.
- A subscription's **first `Chunk` is the seed** — the same value a plain read
  would answer, sent immediately so a subscriber never waits out a poll
  interval before it knows anything. **No further `Chunk` means nothing
  changed** since the seed. That is the feature working, not a hang.

## Proving a push feature actually pushes

A seed `Chunk` only proves the subscribe path answers. It doesn't prove the
feature pushes on a real change — for that, the tracked state has to move
while you're listening. You can't force that on your own task's sandbox
without killing the session running the check, so use a disposable one:

1. `moat tasks create --repo <small-repo> --agent-type claude-code --headless true --name <throwaway-name>` — no `--message`, so only the sandbox starts, not an agent turn.
2. Poll `moat tasks status <id>` until `Sandbox: ready` (~20s for a small repo).
3. Start the subscription against that task's id in the background with a wide window: `... --stream --wait=25000 > /tmp/sub.log 2>&1 &`
4. A few seconds in, force the transition being tested — `moat tasks stop <id>` / `moat tasks start <id>` for sandbox lifecycle, or the equivalent action for whatever the subscription tracks.
5. `grep '^<-' /tmp/sub.log` should show the seed chunk **and** a second chunk carrying the new value. One chunk means only the seed path was exercised.
6. `moat tasks close <id>` — don't leave throwaway tasks running.

## Gotchas

- `moat tasks create` without `--message` starts the sandbox but not the
  agent — that's what makes it a cheap, fast throwaway target rather than a
  billed agent run.
- A background probe (`&`) survives a later `cd` in this harness — the shell's
  working directory persists across tool calls, the job doesn't care either
  way, but don't assume a relative script path in a *later* command still
  resolves the way it did when you launched the job.
- "Not on this commit yet" and "not deployed yet" are different failure
  conditions. A merged PR's capability can take several minutes to reach the
  cluster backend behind CI + image build + chart promotion — reread
  `server.getConfig` before concluding a merge didn't ship the change.

## Related skills

- [`t3-development`](../t3-development/SKILL.md) — stands up the local shadow
  backend this skill's probe can point at, and the T3-client-specific RPC
  surface.
- [`shadow-dev`](../shadow-dev/SKILL.md) — the underlying shadow-backend
  harness (`scripts/shadow/`), independent of T3.
- [`moat-cli`](../moat-cli/SKILL.md) — full reference for `moat tasks`
  subcommands used in the throwaway-task pattern above.
