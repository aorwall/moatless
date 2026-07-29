# Spec — The T3 sidebar shows real Moatless work

> **Status:** approved
> **Updated:** 2026-07-28
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
| Its most recent turn, and that turn's state | The Task's newest Turn. The vocabularies are not the same size — Moatless has two states T3 has no word for — and one word they share means different things on the two sides; [Decision 10](#10-the-turn-state-mapping-in-full) maps all five. A conversation that has never run reports no turn, which the contract allows. |
| Its session | Sandbox lifecycle together with the agent Session. Absent when no Sandbox was ever requested. |
| Whether it is archived | A closed Task is an archived conversation. Closing records no timestamp of its own, so one has to be chosen. |
| Whether the agent is waiting on an answer | Computed from an unanswered question raised by the agent. |
| Whether a proposed plan is waiting | Computed from an unanswered plan the agent proposed. |
| Its working copy location | The Sandbox's workspace directory. |
| Whether it is settled, and since when | No equivalent. Moatless conversations are open, closed, or errored. The contract supplies its own empty value when the field is left out, so omitting it is safe. |
| Whether it is snoozed, and until when | No equivalent. Optional in the contract, so it can be omitted. |
| Whether approvals are pending | No equivalent. The contract requires a definite yes or no with no empty value available, so the answer is always no. |
| Its runtime mode | No equivalent, and the contract requires one of four named modes with no empty value. The permissive mode is both the contract's own default and an honest description of a Moatless Sandbox. |
| Its model | No conversation-level equivalent — model choice belongs to a Turn — and the contract requires a resolvable identity with no empty value and no default of its own. [Decision 9](#9-a-conversation-reports-its-newest-turns-model) takes the newest Turn's model, through a chain that always ends in an answer. |

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
| [9](#9-a-conversation-reports-its-newest-turns-model) | Which model a conversation reports | The newest Turn's model, resolved through a chain that always ends in an answer, on the provider instance named by the agent kind | A deployment-wide default is stable, is wrong for every conversation not on it, and gives the sidebar no sign that it is guessing |
| [10](#10-the-turn-state-mapping-in-full) | How the two turn-state vocabularies map | A named counterpart for all five Moatless states; a stopped turn is reported as interrupted, and an interrupted one as running | Matching the two words spelled *interrupted* to each other reports a turn that resumes as a turn that ended |
| [11](#11-a-resuming-clients-held-set-is-two-questions-not-one) | What a resuming client is assumed to hold | Nothing removable, and the viewer's readable projects for grouping | One set for both questions is either a full listing per reconnection or a removal naming an id the client was never shown |

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

**Holding one row for the whole deployment imposes a lock order, and callers have to honour it.** Every writer that records a change contends on the counter row. A transaction that takes the counter and *then* locks a `tasks` row is ordered counter-before-tasks; one that updates a `tasks` row and then records is ordered tasks-before-counter. Both are natural to write, and together they are a deadlock: two concurrent writers, opposite order, the same two locks. The rule is therefore **allocate last** — take every other lock the transaction needs first, and none afterwards — which makes the counter the innermost lock of everything that touches it, a total order by construction that needs no agreement between callers. It also shortens the hold on the row the whole deployment serializes on; the same transaction priced both ways runs about half again as fast when it allocates last. The invariant is stated canonically on `record_change` in `backend/src/task_listing/dao.rs`, and call sites carry a pointer rather than a re-derivation.

**A record names exactly one subject, and it is not always a conversation.** This decision says only that every change writes a record, and everything downstream of it assumed the subject was a Task — which is true of eight of the nine kinds in Decision 6 and false of the ninth. A Workspace change cannot be expressed as a Task change in either direction: a Workspace being *created* has no Tasks to name, and the title a sidebar groups by lives on the project row, which the conversation event does not carry. So the record names a Task **or** a Workspace, enforced structurally by a CHECK that exactly one of the two columns is set (`task_listing_changes`, migration `0105`) rather than by the discipline of every caller. Neither column carries a foreign key, for the reason a deletion is a change worth delivering: a record has to outlive the row it names.

**Rejected: completing the existing history log and taking the position from it.** This looks cheaper and is not. It makes a log whose purpose is history into a load-bearing part of correctness, which is exactly the entanglement that was deliberately undone when turn state became a row written transactionally. It also still requires finding and changing every path that mutates a sidebar-visible value, so it saves none of the work that actually carries risk.

**Rejected: no replay at all, rebuild the listing on every reconnection.** T3 already rebuilds past a large enough gap, so this is within its contract and is by far the cheapest option. It was rejected because rebuilding re-sends the entire projects and threads list **over the socket** on every reconnection, and replay is what removes that. This is what the contract asks replay for in its own words — `afterSequence` exists "so the subscription resumes without re-sending the entire projects/threads list" — and what T3's reference server implements it for.

**Correction to the reason originally given.** This rejection first read "the listing is not paginated and an administrator's listing is the entire deployment, which turns every brief disconnection into a full scan." That argument does not hold, and it is recorded here rather than deleted because a rejected alternative whose stated reason is false invites the decision to be reopened along with the reason. The client loads the shell snapshot over HTTP on every subscribe *and resubscribe* — `packages/client-runtime/src/state/shell.ts:204-212`, inside the `makeSubscribeInput` that `subscribeDynamic` calls each time — applies it, and only then passes *that* snapshot's sequence as `afterSequence`. So the full scan happens on every reconnection whether or not replay works, and the position a client resumes from is normally seconds old rather than days. Replay saves the socket frame, not the scan.

The scan itself is a real cost and it lands somewhere else: on the client's HTTP snapshot, or on pagination, which is out of scope here and is already an [open question](#open-questions) below.

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

**The two halves are not projected the same way, and that narrowing is what makes the paragraph above true.** It was not always true. This decision has always priced the widening in names — *"seeing the names gives a person the shape of the deployment they work in"*, *"the small disclosure"* — while the code projected both halves through one function, and that function emits a project's **scripts**: every server's name, label, and start commands joined as one string. So following one colleague's conversation disclosed the start commands of the private Workspace it runs in. A start command carries paths, flags, and whatever else someone put in a server config; it is not a name, and no argument here ever justified it.

A Workspace the viewer may **not** read is therefore projected to what the contract needs in order to render a group and no more:

| Field | Widened half |
|---|---|
| `id`, `title`, `workspaceRoot` | Present. The id is the grouping key, the title is the name this decision is argued on, and the root is synthesized from the id, so it discloses nothing the id does not. |
| `repositoryIdentity`, `defaultModelSelection` | `null`, as they are for every project. |
| `scripts` | **Empty.** The narrowing. |
| `createdAt`, `updatedAt` | Present. Required by the contract with no empty value and no decoding default, and an invented timestamp is the kind of plausible stand-in this spec refuses everywhere else. So they are disclosed, and that is stated here rather than left to be discovered. |

Workspaces the viewer *may* read are unchanged. This narrows one half of the union, not the listing.

**The conversation rows need no equivalent narrowing, and the reason is not symmetry.** A conversation is listed because the viewer **follows** it, which is a different permission from reading the Workspace it runs in — so the question has to be asked separately. It is answered by Decision 4: the conversation filter is *follows* **and** *may still read*, so every conversation row is one the read rule admits in its own right, and every value in it is a fact about that Task. The only Workspace-derived value in the row is `projectId`, which is the id the grouping needs and which the widened project already carries.

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

**"Becomes pending, or is answered" reads as one mechanism and is two.** The table's row for a pending question describes a single change kind. In the code the two halves come from different places, and only one of them is a write anybody makes deliberately. A question or plan *becomes* pending when the agent writes an `AskUserQuestion` or `ExitPlanMode` tool call, which is a message append and is recorded. It stops being pending when a `tool_result` block with the matching id exists — and the person's answer arrives as exactly one row, a `user` message whose content is that block (`message/send.rs::send_tool_response_flow`), so answering is recorded by the same rule that records a person writing. That is the path that matters and it is covered end to end.

**The gap this leaves, and why it is left.** The flag is read off the *newest* assistant message, so it also drops when the agent supersedes its own unanswered question with a later assistant message — and that message records nothing, because it is agent output. A sidebar can therefore keep showing *waiting on you* until something else moves the row. Closing it would mean recording on ordinary agent output, which is the one thing this decision exists to keep out of the feed; the trade is not close. In practice the turn ends immediately after, and a turn ending is in the table. Recorded here rather than left implicit: the alternative to knowing about this gap is rediscovering it as a stale flag with no explanation.

**The workspace row is low-rate because of a gate, not because workspaces are rare.** Read from the table alone, "a workspace is created, renamed, or deleted" sizes as an administrative action a few times a week, and that is right for the explicit paths. It is not the whole picture: creating a conversation with no workspace named runs `ensure_shadow_workspace` for its repository, which is a creation path on the busiest write in the system. What keeps it low-rate is that the creation is an `ON CONFLICT` upsert and the record is gated on it having actually inserted — so it fires once per repository for the life of the deployment, not once per conversation. The gate is load-bearing and is invisible from this table; anyone sizing the counter's load from the table should know the gate is what makes the number small.

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

**The identity comes from outside the data, and both halves are load-bearing.** It is the cluster's `system_identifier` and the database's OID — `pg_control_system()` and `pg_database.oid`, neither of which is a row anything writes. A seeded identifier row would be the wrong choice for the failure this is built for: a restore copies the data, so it would copy the identifier with it and the restored database would present as the original. Each half alone is also insufficient. Database OIDs begin at 16,384 in every freshly-`initdb`'d cluster, so the OID alone would let a restore into a new server present as the database it replaced; the cluster identifier alone would miss a restore into a second database in the same cluster, which is what a restore-and-cut-over normally is.

**Reading the identity needs no privilege, and that was checked rather than assumed.** `pg_control_system()` carries no ACL of its own, so `EXECUTE` is `PUBLIC`'s by default and an ordinary application role can call it: `pg_proc.proacl` for it is null on PostgreSQL 13, 16 and 17, and the function appears nowhere in the `system_functions.sql`/`system_views.sql` that `initdb` runs. The functions people are remembering — `pg_ls_logdir`, `pg_read_file`, `pg_stat_file`, `pg_ls_dir`, `pg_switch_wal` — each carry an explicit `{postgres=X/postgres}` and a matching `REVOKE ... FROM public` in those files. This matters because the identity is read at startup and the router returns an error rather than inventing one, so a read a least-privilege application role could not make would take the T3 router down on precisely the deployments configured correctly. A superuser can still revoke it by hand per database; that breaks loudly at startup rather than quietly at runtime. `the_identity_is_readable_by_a_least_privilege_application_role` runs the statement as a role holding one grant in the database — every other test connects as the database's creating superuser and cannot see this at all.

**The client is never told, and does not have to be.** The identity is published as part of the `environmentId`, which is the key T3 caches its listing under (`client-runtime/src/platform/persistence.ts:59-63`). This is the point: an identity mismatch after a restore looks, from the client's side, exactly like a legitimate first connection, so the guard cannot rest on the client reporting honestly what it holds. It rests on the server's answer.

**What a changed id does to a client depends on how it connected, and one of the two is not silent.** A same-origin client — how Moatless is reached — builds its connection *from* the descriptor rather than checking ours against one it stored (`apps/web/src/connection/platform.ts:286-300`), so the new id simply has no cached listing under it and the shell starts empty. A registered bearer, relay or SSH connection stored the id at onboarding and compares it on every attempt, refusing to connect on a mismatch: *"Connected environment X does not match Y"* (`client-runtime/src/connection/errors.ts:25-33`). Replacing the database therefore means re-onboarding those connections. That is the correct answer — it *is* a different environment — but it is a real operational cost and is recorded rather than discovered.

The other cost is smaller and easier to miss: client-side state keyed per project is keyed `{environmentId}:{projectId}` (`client-runtime/src/environment/scoped.ts:24-31`), and unsent composer drafts are among it (`apps/web/src/composerDraftStore.ts:1193`). A changed id orphans them even though the Workspaces they belong to are unchanged. Sidebar grouping recovers, because it is recomputed; a draft does not.

**The identity is the outer layer, not the mechanism.** The id is read once per page load and held for the session, so it moves at reload rather than at reconnect. Everything above about a mid-session replacement is carried by the two server-side checks; the published id is what stops a client from carrying a stale listing across a reload.

**A third place the check has to run.** "Two guards on resuming" names where they were first needed, not the whole of it. A subscription that stays open across the replacement never resumes: the socket is up, the cursor is where it was, and the poll goes on asking for changes past a position the new database has not reached — answered with silence, then, once the replacement counts past it, with changes, having skipped everything in between. So the identity is compared on every poll as well, and a mismatch rebuilds the listing in place. The cost is one extra round trip per poll per subscription, on a path that already makes one.

**A restore has two shapes and only one of them is covered.** Restoring into a new database and repointing the deployment changes the OID, so the identity moves and every client rebuilds. Running `pg_restore` into the existing database does not: the cluster is the same cluster and the OID is the same OID, by construction, because it is the same database. A client holding a position above the restored head is still caught — that position is impossible until the counter climbs back — but once the counter has climbed past it, a reconnection is admitted and the changes issued in between are skipped in silence. **So the supported procedure is to restore into a new database and cut over**, and that is not a preference: it is the only one of the two the guards can see.

Making the identity sensitive to an in-place restore is possible — the counter table's `relfilenode` changes when the table is rewritten — and it is deliberately not done. It would also change on `VACUUM FULL`, `CLUSTER`, and rewriting `ALTER TABLE`, and a changed identity does not merely cost a rebuild: a registered bearer, relay or SSH connection *refuses to connect* until it is re-onboarded. That turns routine maintenance into a client lockout. The identity is tied to things that change only when the database genuinely is a different database, and the in-place restore is left to the procedure instead.

**One more case sits outside all of this, and no guard is proposed for it either.** Emptying `task_listing_changes` by hand while leaving the counter alone changes neither the position nor the identity: the cursor is below the head, the database is the same database, and the replay is answered correctly with the records that survive — which is to say, with nothing. Catching it would need a third guard on the retained range itself, comparing a resuming cursor against the oldest surviving record. That is not built, because the operation it defends against is not one the system performs; pruning is bounded so that it cannot delete a record `resume` will admit, and that bound is asserted at compile time. It is recorded here so that a future change to pruning is read as touching a guarantee rather than a constant.

### 8. Tested against a real database

The primary seam is the module's data access, exercised against a real database using the existing integration-test harness and its row factories. Workspaces and conversations are seeded, a listing is assembled for a given viewer, and the assertions are about what that viewer sees.

A dedicated test commits two positions out of order and asserts that no client can observe the later one first.

A small number of shape-level tests are retained for the message envelope encoding, where the subject genuinely is the encoding rather than the data.

**Rejected: shape-level tests only.** This is what phase one has, and it is fast. It is blind to both of the things most likely to be wrong here: a filter that admits the wrong rows, and two transactions committing positions out of order. Neither is observable without a real database.

**Rejected: end-to-end through the T3 client.** Highest confidence, and it makes the T3 web application a dependency of this backend's test suite. A failure would report that something broke without indicating where. Manual verification against a real client is already covered by the local development runbook, which is the right form for that question at this stage.

### 9. A conversation reports its newest Turn's model

The row reports the model the conversation is actually running: the newest Turn's, not a fixed one.

**Moatless already answers this question, and answers it this way.** The conversation view computes a single model for a conversation by taking the most recent model an agent reply reported it was produced by, falling back to the model that was asked for. Reusing that rule is what makes the two clients say the same thing about the same conversation, exactly as Decision 4 does for what a person's list is. Inventing a second rule here would let the Moatless UI and T3 disagree about a conversation while both are correct by their own lights.

**The churn the other candidate avoids is not a cost here.** Decision 6 re-reads the whole row and sends it whole whenever anything in the bounded set changes, and a turn starting is already in that set. So a conversation that changes model costs no extra delivery — and it is a real change that the person watching asked for, not noise. The model moves when a new turn runs on a different model, and at no other time.

**The chain has to end in an answer, because the contract refuses an empty one.** Every step above the last can be absent, and the absences are ordinary rather than exotic:

| Step | Where it comes from | When it is absent |
|---|---|---|
| 1 | The concrete model the newest agent reply in the newest Turn reported | The conversation has never run, or the turn has started and nothing has come back yet |
| 2 | The model asked for when that turn was started | Nobody chose one — the common case, since a conversation started without a choice carries none |
| 3 | The deployment-wide default for that conversation's kind of agent | The deployment sets none for that kind — the defaults it forwards to Sandboxes are optional configuration |
| 4 | A single last-resort identity, named once in the module | Never |

**A choice that names no model is not an answer.** Moatless model choices are catalog names, and one of them means *whatever this agent picks* — it is resolved inside the Sandbox at run time and is never written back. Reporting it as though it were a model identity would put a word in the row that names nothing. Step 2 therefore skips it and falls through to step 3, which is what it actually resolves to. This is also why step 1 sits above step 2: only the agent's own reply carries the identity of the model that ran, rather than the name of a choice.

Step 4 is the only invented value in the row. The module records when it reaches it, so the invention is visible rather than silent, and a deployment that configures a default model never gets there.

**The identity has two parts, and the chain above fills one of them.** What the contract calls a model selection is a model name together with the provider instance it runs on, and the instance is the routing key — the thing T3 resolves the model against. It is required and non-empty exactly as the model name is, so it needs an answer too. Moatless has no provider-instance concept, and does not need to grow one: T3's default instance for a driver is named after the driver itself, and a conversation's agent kind already names the driver — the Claude-family agents run on the Claude driver, Codex agents on the Codex one. So the instance is derived from the agent kind, and the four steps above fill only the model name. This is the safer half of the pair in any case: T3 treats an instance a build does not recognize as unavailable rather than refusing to decode it, so a name it has never seen cannot break the listing.

**Rejected: a single deployment-wide default for every conversation.** Stable, one value to configure, and never late. It is also wrong for every conversation not running the default, and wrong invisibly: a sidebar showing a plausible model gives no sign that it is a stand-in, so nobody ever discovers it. That is the same failure shape this spec rejects everywhere else — an answer that looks right and cannot be checked. Stability was its only argument, and Decision 6 already pays the delivery cost that stability was protecting.

**Rejected: leaving the field out and letting T3 choose.** The contract refuses it. There is no empty value and no decoding default, so this is not an available option — it is stated here only because it is the first thing anyone reaches for.

### 10. The turn-state mapping in full

Five Moatless states, four T3 states. Three words appear in both vocabularies — *completed*, *error*, *interrupted* — and one of those three means different things on the two sides, which is the whole of the difficulty. Beyond them, Moatless has two words T3 has no equivalent for, *active* and *stopped*; T3 has one Moatless lacks, *running*. Of the two Moatless surplus words, *active* is a rename of T3's *running* and *stopped* is a real gap, decided below. Neither is defaulted, and the shared word that lies is handled explicitly.

**Moatless to T3 — every state has a counterpart:**

| Moatless | T3 | Why |
|---|---|---|
| active | running | The same state under a different word. This is the rename half of the difference. |
| completed | completed | Identical. |
| error | error | Identical. |
| stopped | interrupted | A person cut the turn short. T3 has no *stopped*, and *interrupted* is what T3 calls that event: ended, and ended by a person rather than by the agent. |
| interrupted | running | Not a name match — a state match. See below. |

**An interrupted turn is not a finished turn.** In Moatless, *interrupted* means the conversation's Sandbox went away mid-turn. The turn stays open: it resumes when the work is rescheduled, and the backend's own backstop reconciles it into a terminal state if it never does. It is grouped with *active* everywhere this backend asks which turns are still open. Mapping it onto T3's *interrupted* because the two words match would report a turn that is about to carry on as a turn that ended, and the row would stay wrong until something else overwrote it — the sidebar quietly stops showing work that is still happening. Reporting it as *running* is right for both outcomes: if the work resumes, the row was already correct; if it never does, the backstop writes a terminal state and the row follows.

The cost is that a live turn and a turn waiting to be rescheduled look the same in the sidebar. That is the correct amount of detail for a sidebar — both mean *not finished*, both render the same way, and the difference is Moatless's to resolve, not the viewer's to watch.

**Nothing is left over on the T3 side.** Its one word Moatless lacks is *running*, and both Moatless states meaning a turn is still open land on it. Every T3 turn state is reachable, so the mapping is onto as well as total.

**There is no not-yet-started state in either vocabulary, and none is needed.** A Moatless turn comes into existence already running — there is no queued turn to represent — and T3's turn state has no value for one that has not started either. So a message waiting for a Sandbox has no turn attached to it at all. The contract already has the place for that: a conversation's latest turn may be absent, and a conversation that has never run reports no turn rather than an invented one. This is the one place where a gap on both sides cancels instead of needing a decision.

The write path is out of scope, so this mapping is only ever applied in the Moatless-to-T3 direction.

### 11. A resuming client's held set is two questions, not one

Decision 6's removal delivery rests on a set of ids the feed has already sent this client: only those may come back as a `thread-removed` or a `project-removed`, which is what makes a removal safe on a shared deployment. Decision 7's resume hands a reconnecting client the changes it missed instead of the listing. The two were built on separate branches and meet badly, because a resuming subscription has no such set — nothing on this side records what was sent before the socket dropped — and the feed also consults that same set to decide whether a changed conversation can go out as a row at all. Seeded empty, every reconnection paid for a full listing on its first change, which is the cost Decision 7 exists to remove.

They are two questions and they want different answers.

**May this id be taken back?** Only ids this feed sent. The set has to be a *subset* of what the client was shown, and there is no honest way to rebuild it: the listing the client holds came from a snapshot served over a separate HTTP request that this process kept nothing of. Reconstructing it from what the viewer may currently read is the obvious substitute and is wrong in the direction that matters — the ids missing from it are exactly the rows that left the listing while the client was away, which are exactly the rows it is owed a removal for. So a resumed feed starts with nothing removable and grows the set by delivery alone. The cost is real and is accepted: **a row that leaves a listing across a disconnection is not taken back until something rebuilds that listing.** It is a delivery not made rather than one made wrongly.

**Will the client be able to group this row?** A conversation whose project the client has no group for is dropped on that side in silence, which is why Decision 5's case sends a whole listing instead. This question has a sound answer that the first one does not, and it comes from the client's own behaviour: a reconnecting client re-fetches the whole shell snapshot over HTTP and applies it *before* it subscribes, and the `afterSequence` it sends is that snapshot's sequence (`client-runtime/src/state/shell.ts:204-212`). So it is holding a listing this server built moments ago — and that listing contains **every project the viewer may read**, unconditionally (Decision 5). A project the viewer may read now was therefore in it, unless it became readable in between, and becoming readable is a Workspace mutation that records its own change (Decision 2) and is replayed as a `project-upserted` before anything can be delivered under it.

So the resume seeds the grouping answer with the viewer's readable projects and nothing else. Projects held only because a listed conversation *references* them — Decision 5's second clause, a colleague's private Workspace — are deliberately left out, because that is precisely the case where a conversation can arrive under a project the client has never been sent. Those still cost one listing, once.

**Rejected: one set for both questions.** It is what the merge produces and either setting is a defect. Empty means every reconnection buys a full listing on its first change. Seeded from what the viewer may read means a removal can name an id this client was never shown — including one that never survived projection, since a conversation whose Workspace is gone is returned by the read and dropped from the listing.

**What this does not cover.** The viewer's own scope widening between their snapshot request and their subscribe: no Workspace changed, so no record exists, and a project that became readable in that window is assumed held when it may not be. The window is one round trip and the consequence is one row not rendered until the next rebuild, not anything disclosed.

**Why the pair cannot simply be delivered together.** A conversation arriving under a project the client has no group for needs two events, and there is no position pair that delivers both. The client advances its position on the first and discards anything at or below it, so two events at one position deliver one. Sending the project at `P - 1` fails exactly when `P == cursor + 1` — the quiet single-user case, where there is no integer strictly between the position the client holds and the one the change took. Sending the conversation at `P + 1` looks like the way around that and is worse: `P + 1` is not allocated, so when a real change later takes it the client has already moved past and drops it in silence. Inventing a position corrupts the cursor, so the answer is a fresh snapshot, which the contract declares as a stream item and the client applies whole.

**`rebuild` and `resynchronize` reseed both.** Both replace the client's listing wholesale, so both replace everything the feed believes about it — the id sets included, from the snapshot they just sent. Leaving them is silent in both directions: an id from the replaced listing comes back as a removal naming a row the client does not have, and a row the snapshot introduced is in no set, so its own removal is never sent.

**They do not treat the cursor the same way, and the difference is not incidental.** `resynchronize` *advances* it — `max(cursor, snapshot.sequence)` — because the counter is the same counter, so the old cursor is still meaningful and rewinding it would redeliver. `rebuild` *sets* it, because the old cursor was allocated by a counter that is now gone and can sit anywhere relative to the new database's head; a `max()` there would park the feed above a position the replacement has not reached and answer every poll with silence until it got there. Collapsing the two into one `max()` reads as a simplification and breaks the replacement case silently.

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

**Which timestamp represents archiving.** Closing a conversation records no timestamp of its own, and T3 wants one. The most recent update is the obvious substitute and is not the same thing. The contract does supply an empty value here, so this does not block the build — it only decides whether archived conversations sort correctly.

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
