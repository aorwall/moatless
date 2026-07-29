# External reply

> **Status:** approved
> **Updated:** 2026-07-29
> **Vocabulary:** CONTEXT.md

## Current solution

Message-targeted external replies and task Binding replies take separate backend paths.

Message-targeted replies enter through the message reply route.
The route loads the Task message.
The route checks access to the owning Task.
The route branches to an internal assistant reply when the message has no external metadata.
The route resolves the Binding from the message's Task, Adapter, and external subject.
The route loads the Adapter connection.
The route loads the optional Subscription.
The route resolves outbound files.
The route chooses the task footer.
The route builds the outbound reply.
The route calls the Adapter.

Task Binding replies enter through the task reply route.
The route checks access to the Task.
The route calls `ReplyService`.
`ReplyService` selects one Binding.
`ReplyService` calls the outbound Binding helper.
The outbound Binding helper loads the Binding, Adapter connection, optional Subscription, and files.
The outbound Binding helper builds the outbound reply.
The outbound Binding helper persists a visible outbound stub.
The outbound Binding helper calls the Adapter.
The outbound Binding helper tags the stub with returned external metadata when available.

The affected state is:

| Entity | Current role |
|---|---|
| Task message | May carry external metadata naming the Adapter and external subject. |
| Binding | Ties the Task to an external subject through an Adapter. |
| Adapter connection | Holds the installed Adapter context needed to send externally. |
| Adapter | Owns provider-specific outbound behavior. |

## Planned solution

Add a backend `ExternalReplyService` module for external reply behavior.

The message reply route keeps HTTP concerns only.
The task Binding reply route keeps HTTP concerns only.
Both routes call the same service.

The service owns:

| Responsibility | Rule |
|---|---|
| Message-targeted replies | Resolve the original message into a Binding target and call the Adapter's message reply behavior. |
| Task Binding replies | Select the target Binding and call the Adapter's Binding reply behavior. |
| Internal message replies | Preserve the current internal assistant reply behavior for messages without external metadata. |
| Files | Resolve requested outbound files and reject unsupported file sends through the same behavior for both reply paths. |
| Footer | Apply the task footer for Adapters that use it. |
| Outbound persistence | Persist visible outbound stubs for external replies and tag them with returned external metadata when available. |

### Problem

The current message-targeted reply route contains domain behavior.

That makes the route the practical test surface for external reply rules.

It also leaves message-targeted replies and task Binding replies with parallel reply composition.

### Behaviour

#### Agent or CLI replies to an external message

Given a Task message came from an Adapter
When a caller replies to that message
Then the reply is sent to the external subject through the Adapter.

Given the Adapter returns an external id
When the reply is persisted in the Task timeline
Then the persisted row is tagged with that external id.

Given the original message is internal
When a caller replies to that message
Then the reply is persisted as an internal assistant reply.

#### Caller replies to a Task Binding

Given a Task has one target Binding
When a caller sends a Task reply
Then the reply is sent through the same external reply module.

Given a Task has more than one possible target Binding
When the caller does not choose one
Then the caller receives the current multiple-target response.

Given files are included
When the selected Adapter does not support files
Then the reply is not sent externally.

## Decisions

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-where-the-seam-sits) | Where the seam sits | Backend `ExternalReplyService` behind the route | Route-owned orchestration stays hard to test; Adapter-owned shared rules risk provider drift. |
| [2](#2-scope-of-the-module) | Scope | Cover both message-targeted replies and task Binding replies | Message-targeted only is smaller, but leaves two outbound reply implementations. |
| [3](#3-message-targeted-reply-persistence) | Message-targeted reply persistence | Persist a visible outbound stub and tag returned external metadata | Preserving current behavior avoids product change, but keeps message-targeted replies less observable. |

### 1. Where the seam sits

The seam sits in a backend `ExternalReplyService`.

The route remains the HTTP adapter.
The route extracts the request and serializes the response.
The service owns reply behavior.

Keeping orchestration in the route keeps domain behavior coupled to Axum tests.

Pushing shared rules into each Adapter would duplicate Binding, file, footer, and persistence rules across Adapters.

### 2. Scope of the module

The module covers both message-targeted replies and task Binding replies.

The current `ReplyService` can become a thin selector or fold into the new module.

Limiting the change to message-targeted replies would reduce implementation size.
It would leave the same outbound reply rules split across two modules.

### 3. Message-targeted reply persistence

Message-targeted external replies persist a visible outbound stub.

The stub is tagged with returned external metadata when the Adapter supplies an external id.

This matches task Binding replies.

Preserving the current behavior would avoid a visible product change.
It would leave message-targeted replies harder to audit in the Task timeline.

## Seams and testing

The primary module is `ExternalReplyService`.

Its interface is the test surface.

Tests should call the service through the same state and repository seams used by existing backend tests.

The Adapter trait remains the external-system seam.

No new trait is needed unless tests need a second Adapter implementation.
The existing fake or test Adapter setup should be reused where possible.

The route tests should only prove request wiring where needed.
Behavior tests should target the service interface.

## Out of scope

Inbound Adapter delivery is unchanged.

Sandbox outbox event publishing is unchanged.

Automatic mirroring of every Agent message to external systems is unchanged.

Provider-specific reply behavior is unchanged except where needed to preserve the unified service behavior.

## Open questions

None.

## Ticket breakdown

None.
