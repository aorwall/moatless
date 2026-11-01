# Secret requests: an agent asks, a person answers in T3

> **Status:** approved
> **Updated:** 2026-11-01
> **Vocabulary:** CONTEXT.md

## Problem

An agent that needs a credential can only tell the person to run
`moat secret set`. An Artifact page cannot collect one safely: the agent wrote
every line of its script, so whatever a reader types is the agent's to read
and save as a record, and `allow-popups` lets it leave the deployment
(`docs/deferred-changes.md`, "An Artifact page can still send what it reads").

## Solution

The agent runs `moat secret request <KEY>`. The thread shows T3's existing
secret card (`t3code/apps/web/src/components/chat/SecretRequestCard.tsx`), drawn
by T3 itself rather than by agent code. The answer goes over the session socket to
`secrets.answerRequest`, which Moatless serves by calling `SecretService::put_secret`
as the person answering. The Task gets a message saying the key was saved, never
the value, and the turn that message starts fetches the credential like any other.

An Artifact that needs a secret links to the thread instead of rendering an
input: the page may point at a request, never draw one.

T3 already has every client piece: the `secret_request` turn item
(`packages/contracts/src/orchestrationV2.ts`), the card on web and mobile, the
`secrets.answerRequest` RPC and its `SecretRequestError` reasons. Moatless has
no `secret_request` item and does not declare the method
(`t3code/docs/fork/gaps.md`, the `secrets.answerRequest` union entry).

## Flow

1. `moat secret request KEY --scope user|global --kind env|runtime_file
   --reason "…" [--label "…"]` posts to
   `POST /api/v1/tasks/{task_id}/secret-requests` as the sandbox's user. The
   backend writes a `secret_requested` task event and projects it, in one
   transaction, as a `SecretRequested` activity. The response is the request id.
2. `crates/t3code` projects that activity as a `secret_request` turn item with
   `secretStatus` `pending`, or the outcome of its `SecretRequestAnswered`
   activity.
3. `secrets.answerRequest` maps `turnItemId` (`item:activity:<id>`) back to the
   activity and calls `moatless_tasks::secret_request::service::answer`.
   - **Save:** `put_secret` with the viewer's `ScopeContext`, then the
     `secret_request_answered` event. Its id is derived from the request id,
     so a second answer conflicts and is refused as `already_answered`.
   - **Decline:** the answered event only.
4. The Task gets a message naming the key, the scope and who answered, so a
   stopped Sandbox wakes and the agent continues.

## Rules

- **A `user` request is answered only by the user who made it.** The Sandbox
  resolves user-scope secrets for its actor (`sandbox-control/src/secrets.rs`),
  so a value saved under another viewer's user scope never reaches it. Others see
  the card's error. A Task whose actor differs from its token user is a deferred
  change, not handled here.
- **A `global` request is answered only by an admin.** `put_secret` refuses
  anyone else; the service refuses earlier with a message the card shows.
- **Reserved keys cannot be requested.** `is_reserved_key`
  (`core/src/secret_keys.rs`) names keys the platform owns; asking a person to
  replace `GITHUB_TOKEN` is how a misled agent swaps in an account it controls.
- **Kinds are `env` and `runtime_file` only.** `provider_token`, `deployment`
  and `internal` are set by an admin through `moat secret set`.
- **The value never enters a task event, activity, message, record or log.**
  The request and its answer carry the key, scope, kind and outcome.
- **A request stays pending until answered.** It is not tied to the turn that
  asked, unlike `AskUserQuestion`: the agent ends its turn and is woken by the
  answer.

## Not in this change

- The privacy note on T3's card ("Stored securely, never shown to the agent")
  is upstream's and is wrong for an `env` secret, which the agent can read. The
  fix is a fork change in `t3code` (`secretRequestPrivacyNote`).
- A standalone page for readers of an Artifact who are not on the thread.
- Cancelling a request from the agent's side.
