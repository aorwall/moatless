# Spec — The T3 sidebar shows real Moatless work

> **Status:** approved
> **Updated:** 2026-07-27
> **Vocabulary:** CONTEXT.md

## Current solution

T3 Code is a separate client being adapted to run against this backend. The compatibility surface it talks to is `backend/src/ui_rpc`, an opt-in module that speaks T3's contract and translates it to Moatless concepts. Phase one of that module is complete and deliberately empty: every surface T3 needs in order to start up exists and answers, and none of them reads the database.

The full domain mapping is in [docs/t3-code-domain-mapping.md](../docs/t3-code-domain-mapping.md). Orientation for the whole integration is in [docs/t3-code-integration-start-here.md](../docs/t3-code-integration-start-here.md).

### What happens today when T3 starts up

| Step | What it does |
|---|---|
| The module mounts its routes | Four surfaces: public discovery, authenticated session bootstrap, the shell listing, and the socket. All built from environment configuration alone. |
| A client asks who this server is | Answered before any login. Declares the environment's identity, its label, and which optional features it supports. |
| A client establishes a session | Cookie-authenticated. Grants permission to read orchestration data. |
| A client asks for the shell | Logs the caller, discards their identity, and returns an empty listing. |
| A client opens the socket | The origin is checked by exact match, then the connection is upgraded. |
| A client subscribes to the shell | Sends one empty listing, plus a synchronization marker when asked for one, then holds a subscription that never sends anything again. |
| A client asks to replay missed changes | Always answers with nothing. |

The module holds no database handle at all. Its state is the environment configuration and the session cookie name. So this change is not a matter of replacing an empty listing with a full one — it is the first time this module reads anything.

The replay path is dead in the strict sense: it exists, it is reachable, and nothing can ever come back from it.

### What a conversation row has to carry

T3's sidebar row carries twenty pieces of information. Roughly half come straight from a Task; the rest are computed, and six have no Moatless equivalent at all.

| What the row needs | Where it comes from |
|---|---|
| Its identity | The Task's identity |
| Which project it belongs to | The Task's Workspace. New Tasks always get one — creation makes a shadow Workspace when none is given — but older and hand-written rows can have none, and provisioning still falls back to Repository-only behaviour for those. A conversation with no resolvable Workspace must be left out of the listing entirely, not listed against a project that is absent: T3 drops such a row without telling anyone, so it would vanish with no diagnosis. |
| Its title | The Task's name. Optional in Moatless and required by T3, so it needs a fallback. |
| Its branch | The Task's branch |
| When it was created and last changed | The Task's timestamps |
| When a person last wrote to it | Computed. T3 counts only messages authored by a person. Moatless's nearest stored value also moves when the agent writes, so it is the wrong source and the value has to be derived. |
| Its most recent turn, and that turn's state | The Task's newest Turn. The two state vocabularies differ by two values and need an explicit mapping. |
| Its session | Sandbox lifecycle together with the agent Session. Absent when no Sandbox was ever requested. |
| Whether it is archived | A closed Task is an archived conversation. Closing records no timestamp of its own, so one has to be chosen. |
| Whether the agent is waiting on an answer | Computed from an unanswered question raised by the agent. |
| Whether a proposed plan is waiting | Computed from an unanswered plan the agent proposed. |
| Its working copy location | The Sandbox's workspace directory. |
| Whether it is settled, and since when | No equivalent. Moatless conversations are open, closed, or errored. The contract supplies its own empty value when the field is left out, so omitting it is safe. |
| Whether it is snoozed, and until when | No equivalent. Optional in the contract, so it can be omitted. |
| Whether approvals are pending | No equivalent. The contract requires a definite yes or no with no empty value available, so the answer is always no. |
| Its runtime mode | No equivalent, and the contract requires one of four named modes with no empty value. The permissive mode is both the contract's own default and an honest description of a Moatless Sandbox. |
| Its model | No equivalent, and this one is not answerable by convention. The contract requires a resolvable model identity, refuses an empty value, and offers no default. Model choice belongs to a Turn in Moatless, not to the conversation. See the open questions. |

### How T3 solves the same problem on its own side

Worth recording, because two of its four answers transfer and two do not.

| What T3 does | Transfers |
|---|---|
| One writer and one position counter for everything. Contention is not a tuning choice; it is the architecture. | No. Free for a single-user local server, a throughput ceiling for a shared deployment. |
| The position advances on every recorded change, including agent output. | No, for the same reason. |
| The sidebar row is recomputed on a bounded set of change kinds — eight of them, not all changes. | Yes. The feed does not have to carry everything that happens. |
| The activity timestamp counts only messages authored by a person. | Yes, and it removes the highest-rate change in the system from the feed entirely. |
| The live channel coalesces a burst per conversation, then re-reads the row and sends it whole. | Yes. Absorbs bursts without per-change delivery. |

### Two properties of the current system that this change depends on

**The history log is written off the live channel, not with the change.** Only three code paths write a durable record in the same transaction as the change it describes: Sandbox lifecycle, Turn transitions, and the ingress from a Sandbox. Everything else — renaming a conversation among them — publishes to the live channel and returns, and a downstream relay copies what it reads there into the log.

That relay is best-effort. If the copy fails it logs a warning and moves on, and the change is then durable nowhere. If it succeeds, the record still lands after the change it describes, at a position the change did not choose and no reader of the change can observe.

So the log is unsafe as a resume position for two independent reasons, not one. Ordering can be fixed by choosing the position carefully. A record that was dropped cannot be replayed by any position — and a client cannot distinguish a dropped record from one that has not arrived yet, so the loss is silent in the only direction that matters.

**Reading is not controlling.** Moatless admits a conversation for reading when the viewer is an admin, or the conversation is public, or the row is global, or the viewer owns it. It admits a conversation for controlling only when the viewer is an admin or owns it. Being public or global grants no control.

T3's sidebar row has no place to record that difference, and its feature negotiation is per environment rather than per conversation. So the set a viewer may read and the set a viewer may act on differ, and the contract cannot currently express which is which.

**Following already exists, and already drives the Moatless sidebar.** A person follows a conversation by creating it, by writing to it, or by asking to explicitly. The Moatless listing switched to a follow-based filter some time ago, and creators were backfilled as followers of their own conversations so nobody's list emptied. Following is recorded durably; it narrows a listing rather than widening one, so it has never granted access to anything.

**A conversation whose project is missing disappears.** T3's sidebar groups conversations by project and silently skips any conversation whose project is absent from the listing — a bare skip, with no error and nothing logged. The two lists therefore cannot be filtered independently.

## Planned solution

The compatibility module gains data access and returns real content: the conversations the viewer follows, the projects those conversations run against, and a position that a disconnected client can resume from. A durable record is written whenever a conversation's sidebar-visible state changes, in the same transaction as the change itself.

### Problem

Someone running T3 Code against a Moatless deployment today sees a working, connected, permanently empty sidebar. The connection succeeds, authentication succeeds, and there is nothing to look at. Every piece of work they have in the deployment is invisible in the client they want to use.

For the people building the integration, the second problem is that the surfaces which look easiest to fill are the ones that will be wrong in ways nobody notices. A listing assembled without care returns work the viewer cannot act on. A position derived from the existing durable log misses renames entirely, and skips changes whenever two writers commit out of order. Both failures are silent, and both look correct in any test that does not use a real database.

### Behaviour

**As someone using T3 Code against a Moatless deployment:**

When I open the client, I see the conversations I follow, grouped under the project each one runs against.
I follow a conversation by starting it, by writing to it, or by saying so explicitly. I stop following it by saying so.
When I follow a conversation that runs against a project someone else owns, that conversation still appears, under that project's name.
When I look at a project I do not own, I see its name and the conversations in it that I follow — usually none.
When a conversation changes while I am watching — a turn starts, an agent finishes, a sandbox stops — the sidebar updates without my reloading.
When I close the client and reopen it shortly after, I see the changes I missed rather than a rebuilt list.
When I have been disconnected long enough that the missed changes cannot be listed, I get a freshly built list instead, and nothing is silently absent.
When the agent writes at length, my sidebar does not reorder itself continuously; ordering moves when a person writes, and when a turn starts or ends.
Every action the sidebar offers me on a conversation is one I am permitted to take.

**As an administrator:**

When I open the client, I see the conversations I follow, exactly like everyone else. Being an administrator does not fill my sidebar with the deployment.
I see every project in the deployment, because administrators may read them all. Most of them are empty groups, and that is the deployment's shape, which is useful to me.
When I need to look at a conversation I do not follow, I follow it, or I use the Moatless UI.

**As someone who works with other people:**

A conversation someone else started appears in my sidebar as soon as I write to it, and stays until I stop following it.
Work I have nothing to do with does not appear, whoever owns it and whether or not it is public.

**As an operator:**

When I replace or restore the database, connected clients do not silently stop updating.
When a client asks to resume from a position this deployment has never reached, it is given a fresh listing rather than an empty answer.

## Decisions

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-a-t3-project-is-a-workspace) | What a T3 project is | Workspace | A Repository-shaped project must still resolve to a Workspace on creation, and multi-repository Workspaces would be silently flattened |
| [2](#2-the-change-feed-is-a-dedicated-record) | What the change feed is | A dedicated record, written in the mutation's own transaction | Completing the existing history log re-entangles history with state, which was deliberately undone, and still requires finding every mutation path |
| [3](#3-the-whole-deployment-is-one-environment) | Environment granularity | The whole deployment | A Sandbox is a worktree, not an environment; making each one an environment leaves the list of all conversations with nowhere to live |
| [4](#4-conversations-are-filtered-by-what-the-viewer-follows) | Which conversations are listed | Those the viewer follows, and may still read | Filtering by control fills an administrator's sidebar with the deployment and hides colleagues' work from everyone else |
| [5](#5-projects-are-filtered-by-what-the-viewer-may-read) | Which projects are listed | Those the viewer may read, plus any referenced by a listed conversation | Read alone no longer guarantees the referenced project is present, and a conversation whose project is absent disappears silently |
| [6](#6-what-enters-the-feed) | What enters the feed | Eight change kinds; agent output excluded | Including agent output puts the highest-rate path in the system behind a deployment-wide lock, to move a value defined as counting people |
| [7](#7-two-guards-on-resuming) | Resuming from an impossible position | Reject the position and tie identity to the database | Either guard alone leaves a case the other covers |
| [8](#8-tested-against-a-real-database) | Testing seam | Against a real database | Shape-only tests cannot see access filtering or commit ordering, which are the two things most likely to be wrong |

### 1. A T3 project is a Workspace

A conversation already references a Workspace. Mapping a project to a Workspace is therefore the direct route: no resolution step is needed at creation time, and Workspaces composed of several repositories stay visible instead of being reduced to one.

No repository becomes unreachable, because every Repository already has a default Workspace derived for it.

**Rejected: mapping a project to a Repository.** This was the earlier reading, recorded in T3 Code's own concept map. It requires resolving a Repository to a Workspace on every conversation creation, and it presents a multi-repository Workspace as though it were a single repository, which is a lie the sidebar cannot recover from.

**Consequence that must be handled.** T3 treats a project's root location as an identity, unique across active projects, and derives its own grouping key from that location. Moatless has no per-Workspace location — repositories are placed inside a Sandbox's workspace directory, and that directory is the same for every Workspace. Reporting it as-is would give every project the same identity and collapse them into a single group in the sidebar, and would be refused outright by T3 on the write path. The module must therefore produce a distinct, stable location per Workspace. It is a nominal value in any case: the real location only exists once a conversation has a Sandbox.

**Related consequence, deferred.** T3's project-scoped file operations have no target until a conversation exists, because files live inside a per-conversation Sandbox. That is the same gap as the machine-scoped methods described in Decision 3, and it is out of scope here.

### 2. The change feed is a dedicated record

Every change to a sidebar-visible value writes a durable record, in the same transaction as the change itself. That record carries the position a resuming client asks for. Nothing is published without being recorded first, which is precisely the guarantee the current relay-written paths cannot make.

**The position is one number for the whole environment.** T3 asks for the shell with a single position and discards anything at or below the position it already holds. A counter kept per conversation cannot answer that: it produces many positions where the contract has room for one. The counter is therefore per deployment.

**Taking the position inside the transaction is what makes it correct.** The allocating statement holds its row until the transaction commits, so a later position cannot become visible before an earlier one. Commit order and position order are the same order. The entire resume mechanism rests on that property.

The cost is that changes which advance the position serialize against one another. Decision 6 is what keeps that cost small.

**Rejected: completing the existing history log and taking the position from it.** This looks cheaper and is not. It makes a log whose purpose is history into a load-bearing part of correctness, which is exactly the entanglement that was deliberately undone when turn state became a row written transactionally. It also still requires finding and changing every path that mutates a sidebar-visible value, so it saves none of the work that actually carries risk.

**Rejected: no replay at all, rebuild the listing on every reconnection.** T3 already rebuilds past a large enough gap, so this is within its contract and is by far the cheapest option. It was rejected because the listing is not paginated and an administrator's listing is the entire deployment, which turns every brief disconnection into a full scan.

**Why a naive position from the existing log would have been wrong.** A position assigned when a row is inserted but made visible when its transaction commits can be read out of order. Two writers take consecutive positions, the second commits first, a reader polling for anything past the first position sees the second, advances, and never sees the first. This is the hazard recorded in T3 Code's convergence analysis, and a position taken as the current maximum of the existing log has exactly this bug. It fails silently: the client believes it is up to date.

### 3. The whole deployment is one environment

A Sandbox looks like an environment because it is the thing with a filesystem, a checkout, terminals and preview servers — everything T3's machine-scoped methods assume is present. It is not an environment. It is a worktree.

| T3 | Moatless | Lifetime |
|---|---|---|
| Environment — the server you connect to | The deployment | Permanent |
| Project — a configured checkout root | Workspace | Long-lived |
| Worktree — per-conversation working copy | Sandbox | Disposable, often absent |

A worktree is per-conversation, disposable, and may be absent while the conversation still exists. That is a Sandbox exactly: absence of the record means no Sandbox was ever requested. T3 already has a slot for this concept and it is not the environment slot.

**Rejected: each Sandbox is an environment.** Once the levels are lined up this is not merely expensive, it is incoherent. The shell is the list of all conversations; if each conversation is its own environment, the list has nowhere to live. T3's list of connections is registered by the user rather than enumerated from a server, so populating it would require something that already knows the conversation list — which is the thing the shell was supposed to provide. Beyond that: a deployment with a hundred open conversations would need a hundred sockets to draw one sidebar, a stopped Sandbox would be a disconnected environment whose conversation must still be listed, and discovery would have to answer per conversation before any conversation exists.

**Deferred cost, stated plainly.** T3's machine-scoped surface — terminal, preview, version control, file operations — carries no conversation identifier, because in T3 the environment is the machine. Under this mapping every one of those needs one. That is a genuine contract change on the T3 side. It does not block the shell, which is why the shell is the right thing to build first.

### 4. Conversations are filtered by what the viewer follows

The listing is what the viewer follows. Not what they own, and not what they may act on.

A person follows a conversation by starting it, by writing to it, or by asking to. They stop by asking to. So the sidebar is the set of work someone has actually engaged with, and it is theirs to curate — which is what a sidebar is for, and what neither ownership nor permission describes.

**This is already how the Moatless UI works.** Following is not being introduced here. The listing switched to a follow-based filter some time ago, and creators were backfilled as followers of their own conversations so nobody's list emptied. T3 adopting the same rule makes the two clients agree about what a person's list is, rather than presenting the same deployment two different ways.

**Administrators are not a special case.** An administrator's sidebar is the conversations they follow, like everyone else's. Being able to see everything is not a reason to be shown everything, and a sidebar containing the whole deployment is not usable as a sidebar. When an administrator needs a conversation they do not follow, they follow it, or they use the Moatless UI — which is the surface built for looking across the deployment.

**Following does not grant access.** The read rule still applies underneath: the listing is what the viewer follows *and* may still read. A follow is a durable row and access is not — a conversation can be made private, or change hands, long after someone wrote to it. Without the read rule underneath, a stale follow would keep showing work the viewer is no longer allowed to see.

**Rejected: filter by what the viewer may control.** This was the earlier decision. It listed an administrator the entire deployment while hiding a colleague's conversation from someone actively working on it — wrong in both directions at once, and for the same reason: control describes authority over work, and a sidebar is about involvement in it.

**Rejected: list everything readable.** Nothing would disappear, and T3 would offer actions that fail when used, with no way to explain the failure in the sidebar. It also makes the listing larger the more open a deployment is, which is backwards.

**Rejected: follow, without the read rule underneath.** Simpler, and it turns every historical follow into a permanent access grant that no permission change can revoke.

**What this fixes.** The unbounded listing is no longer a deployment-sized problem — the biggest sidebar is now the most-involved person's, not the administrator's. And public and global work is no longer invisible on a technicality: if someone is involved with it, they follow it, and it appears.

**Still deferred: a per-conversation permission marker in the contract.** T3's row has nowhere to record that a viewer may read a conversation but not act on it, so the sidebar can still offer an action that fails — now for a followed conversation the viewer does not own. Rarer than before, because people mostly follow what they work on, and unchanged as the durable answer.

### 5. Projects are filtered by what the viewer may read

Projects use the read rule while conversations use the follow rule. The asymmetry is deliberate and load-bearing.

T3's sidebar groups conversations by project and silently skips any conversation whose project is missing from the listing. So the project list has one hard obligation: **every project referenced by a listed conversation must be present.** Anything narrower deletes a person's work from their own sidebar with no error and nothing to notice.

The read rule alone does not discharge that obligation, and Decision 4 is what breaks it. People follow each other's conversations, and those can run against a Workspace that is private to someone else. Read admits public, global and owned Workspaces — not that one. So the list is the union: every Workspace the viewer may read, **plus** every Workspace referenced by a conversation in the listing, whether or not they may read it.

Administrators may read every Workspace, so the union adds nothing for them. They see every project in the deployment, most of them empty.

A project with no listed conversations renders as an empty group. That is intended: seeing the names gives a person the shape of the deployment they work in.

**Rejected: list only projects referenced by a listed conversation.** The tightest correct set, and it leaks nothing. Rejected because the empty groups are worth more than the small disclosure — though note this is now the larger half of the union rather than an alternative to it.

**Rejected: the read rule alone.** What this decision said before Decision 4 changed, and it is now unsound in exactly the case following was introduced to support: a colleague's conversation, in their private Workspace, that you were asked to look at.

**Rejected: filter both lists the same way.** Simplest to write, and wrong in a way that is invisible from the outside.

### 6. What enters the feed

A bounded set, mirroring the eight change kinds T3 recomputes its own sidebar row on, plus one T3 has no equivalent for.

| Change | Rate |
|---|---|
| A conversation is created, closed, reopened, or deleted | Low |
| Its name or description changes | Low |
| Its tags, visibility, or expiry change | Low |
| A person sends a message | Low |
| A turn starts, completes, errors, is stopped, or is interrupted | Moderate |
| Sandbox lifecycle moves | Moderate |
| A question or proposed plan becomes pending, or is answered | Low |
| A workspace is created, renamed, or deleted | Low |
| Someone starts or stops following a conversation | Low |
| **Agent output** | **Excluded** |

Agent output is excluded because the only sidebar value it could touch is the activity timestamp, and T3 defines that as counting messages authored by a person. Including it would place the highest-rate path in the system behind a deployment-wide lock in order to move a value it is not supposed to move.

Following is in the feed because it is the one change that adds or removes a whole row. It is also the change with the narrowest audience: it alters one person's listing and nobody else's. The same is true, more weakly, of everything else in the table — a change to a conversation matters only to the people following it — so what a viewer is told is their own slice of the feed, not all of it. The position stays deployment-wide regardless; it is a position in the record of changes, not a count of what any one person received.

Note that writing to a conversation now does two things at once for the author: it advances the activity timestamp, and if they were not already following, it adds the row. Both are already in the table, but a listening client has to handle the second before it can render the first.

On delivery, a burst affecting one conversation is coalesced, and the row is then re-read and sent whole. Never a per-change difference. This is the same shape as the assembled message upsert already used elsewhere in this backend, and it is what makes T3's own burst behaviour hold up.

**Rejected: include agent output.** Sidebar ordering would be exact and live. Every agent message would serialize on one row.

**Rejected: include agent output, collapsed to at most one record per conversation per interval.** A middle road that adds a timer and its failure modes in order to buy ordering precision that nobody observes.

**Note on how this decision was reached.** The rate of the activity timestamp was originally assessed as high, on the assumption that it tracked messages in both directions. That was a mistake: it was measured against Moatless's stored value, which does move for agent messages, rather than against T3's definition, which counts only people. With the correct definition the highest-rate item leaves the feed entirely and the contention concern largely dissolves.

### 7. Two guards on resuming

Two guards, because each covers the other's blind spot.

A position higher than any this deployment has reached is impossible, and is answered with a freshly built listing rather than an empty reply.

The environment's identity is derived from the database, so a replaced database presents as a different environment and a client discards its cached listing without being told to.

**Why neither alone is enough.** Rejecting impossible positions handles a counter reset to zero. It does not handle a restored backup: positions between the restore point and the client's position get handed out a second time to different changes, after which the client's position looks valid again and it skips everything in between. Only a changed identity catches reused positions. Conversely, deriving identity from the database does not catch a counter reset that leaves the database identity intact.

**Why this matters more than it looks.** The failure is silent in the one direction nothing watches. T3 guards against the client falling behind the server. Neither side guards against the server falling behind the client. A client asking for everything after a position the server has never reached receives nothing, forever, while reporting itself as live.

### 8. Tested against a real database

The primary seam is the module's data access, exercised against a real database using the existing integration-test harness and its row factories. Workspaces and conversations are seeded, a listing is assembled for a given viewer, and the assertions are about what that viewer sees.

A dedicated test commits two positions out of order and asserts that no client can observe the later one first.

A small number of shape-level tests are retained for the message envelope encoding, where the subject genuinely is the encoding rather than the data.

**Rejected: shape-level tests only.** This is what phase one has, and it is fast. It is blind to both of the things most likely to be wrong here: a filter that admits the wrong rows, and two transactions committing positions out of order. Neither is observable without a real database.

**Rejected: end-to-end through the T3 client.** Highest confidence, and it makes the T3 web application a dependency of this backend's test suite. A failure would report that something broke without indicating where. Manual verification against a real client is already covered by the local development runbook, which is the right form for that question at this stage.

## Seams and testing

The compatibility module is the adapter. Its entire purpose is to present Moatless state in T3's vocabulary, and it is the only place in this backend permitted to know T3's vocabulary. T3 names appear on the wire and nowhere behind it. Keeping that boundary sharp is what makes the module deep: a wide and detailed external contract, resting on a narrow internal interface — read the conversations this viewer follows, read the Workspaces they need in order to be grouped, read the position.

The leverage in this change is concentrated in two places, and they are not the places with the most code.

The first is the position allocation. It is a handful of lines, it is the foundation of every resume, and it fails silently when wrong. It gets a test of its own that commits out of order deliberately.

The second is the pair of listing rules. They are the difference between a correct listing and one that discloses work or hides it, and they are not the same rule, which is the part most likely to be “tidied” into a bug. They are tested by seeding conversations and Workspaces across the ownership, visibility and following combinations and asserting per viewer.

Locality argues for the change feed record being written by the same code that performs the mutation, rather than by a listener that observes it afterwards. A listener would be a second place for the truth to live, and the gap between the two would be exactly the class of bug that made the existing history log unusable for this purpose.

The testing seam is the module's data access, against a real database. This is the highest seam that is both fast and diagnostic: it exercises the real access filters and the real ordering, and a failure names the query that broke. The seam above it — the T3 client itself — is covered manually through the local development runbook rather than in the automated suite.

## Out of scope

Conversation detail: messages, turns, and differences.

Command dispatch. The write path stays unimplemented, so this phase is read-only.

Terminal, preview, version control, and file operations, all of which need a conversation identifier added to their contract first.

Renaming the conversation concept in T3 Code to match this backend's vocabulary. That is recorded in the domain mapping document and is deliberately sequenced after the mapping has been exercised.

Pagination of the listing. T3's contract has no place for it, and adding one is a change on the T3 side.

Any change to what this backend treats as its source of truth.

Adding a per-conversation permission marker to T3's contract, which Decision 4 defers.

## Open questions

**The listing has no bound.** T3's shell holds every unarchived conversation at once, and the contract has no place for pagination. Decision 4 makes this much smaller — the largest sidebar is now the most-involved person's rather than the deployment — but it is a bound nobody enforces, and someone who has written to a thousand conversations over a year has a thousand-row sidebar. Whether a follow should age out, and after how long, is a product question. The pagination question underneath it has to be settled with T3 rather than inside this backend.

**Whether an administrator will accept a sidebar that is not the deployment.** Decision 4 deliberately removes the administrator's privileged view, on the grounds that a sidebar of everything is not a sidebar. If administrators are in practice using T3 as an operations console rather than a place to do their own work, this is the wrong trade and the answer is a filter or a second view, not a wider default.

**Which model a conversation reports.** This is the one required value in the row with no source and no available fallback: the contract refuses an empty model and supplies no default of its own. The two candidates are the newest Turn's model, which is accurate but changes under the sidebar as work proceeds, and a single deployment-wide default, which is stable and sometimes wrong. This has to be settled before the row can be built at all.

**Which timestamp represents archiving.** Closing a conversation records no timestamp of its own, and T3 wants one. The most recent update is the obvious substitute and is not the same thing. The contract does supply an empty value here, so this does not block the build — it only decides whether archived conversations sort correctly.

**How the two turn-state vocabularies map.** They differ by two values. The mapping is mechanical but has not been written down, and the turn state is a required part of the row, so it has to be written down before the row can be built.

**Whether the live channel's existing silent frame-dropping should be addressed here.** It currently discards frames when a consumer falls behind and reports this to nobody. Introducing a position makes the loss detectable for the first time, which is a benefit available independently of this work.

## Ticket breakdown

Seven tickets in `plans/t3-shell-projection/`, each a vertical slice that is demoable on its own. Awaiting approval of the shape.

```
01 settle the two unsourced row fields ─┐
                                        ├─→ 03 sidebar shows real conversations
02 sidebar shows real projects ─────────┘            │
                                                     ▼
                                     04 sidebar updates when a turn changes
                                                  │        │
                        05 every change kind ◄────┘        └───► 06 reconnect sees what it missed
                                                                            │
                                                                            ▼
                                                          07 never silently stranded
```

| # | Delivers | Blocked by |
|---|---|---|
| 01 | The model and turn-state answers, written down | — |
| 02 | Real, distinct project groups in the sidebar, still empty | — |
| 03 | Those groups filled with the conversations the viewer follows | 01, 02 |
| 04 | Rows that move when a turn changes, without reloading | 03 |
| 05 | The remaining seven change kinds live | 04 |
| 06 | Reconnection that resumes instead of rebuilding | 04 |
| 07 | No client silently stalled after a database is replaced | 06 |

Two open questions are deliberately not ticketed. The unbounded listing needs a decision made with T3 rather than inside this backend. Whether an administrator will accept a sidebar that is not the deployment is a question about how T3 is actually used, and Decision 4 records what to do if the answer is no.

The live channel's existing silent frame-dropping is also not ticketed here. Introducing a position makes that loss detectable for the first time, but the fix is worth doing whether or not this work happens, so it belongs on its own.
