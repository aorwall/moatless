---
name: spec
description: Spec a change from research to agreement — research and decisions on a live HTML page the user works against, then one complete markdown spec written after agreement, for the agents that implement it. Use when writing or updating a spec, plan or change proposal, when recording what shipped, or when another skill needs the spec shape.
---

# Spec

## The workflow

1. **Research the current state**, and ask until you and the user see the same thing. Read the code; ask the understanding questions; settle the vocabulary against `CONTEXT.md`.
2. **Render the page.** Its first content is that shared understanding, as `## Current solution`. Give the URL once.
3. **Research and grill toward what to build**, growing `## Planned solution` as you go. This phase mixes reading code, asking the user, and writing the page — the seams are one of the questions. Update the lede and the decision table at the top whenever an answer changes them.
4. **Get the plan approved, on the page.** Paste the decision table into the message so the choice is reachable without opening anything. This is an approval turn, not a ranked pick — see `/asking-questions`, "An approval is not a question".
5. **Write `.plans/<slug>.md`, once** — complete and uncapped. It is for the agent that implements.

Nothing goes into `.plans/` before step 5.

**Recording and redrawing are different, and only one is a judgement call.** Recording is not optional: an answer lands, it goes on the page before the next question goes out — one `Edit` adding a row. Redrawing rewrites a section an answer made *wrong*, and the test is "wrong", not "out of date" — a section the page already shows as open is incomplete, and incomplete costs nothing.

Everything else in this skill is *how* to do one of those steps:

| For | Read |
|---|---|
| Asking the questions in steps 1 and 3 | `/grilling` |
| Shaping the questions themselves | `/asking-questions` |
| Everything else true of a turn a person reads | `/writing-messages` |
| Rendering and updating the page | [HTML-REPORT.md](HTML-REPORT.md), on top of `/html-report` |
| The elements that go in either surface | [DESIGN.md](DESIGN.md) |
| The shape of the file written in step 5 | Below |

## The two surfaces

- **The page** — `scratchpad/<slug>.html`. The only document the user ever works against, from step 2 to step 4. It carries the lede, the research, and every answer as it arrives.
- **The markdown** — `.plans/<slug>.md`. Written at step 5, for the agent that implements and for `/code-review`'s spec axis.

**The page is capped by attention; the markdown is capped by nothing.** One screen is what gets approved; the implementing agent gets everything.

**Same sections on both surfaces, different resolution.** A section that earns a place in the markdown appears on the page too, at high level — the same model, drawn with the words a person uses instead of the types in the code. What the page never carries: module inventories, migrations, function signatures, error codes, test commands.

**Design against flatness, not against length.** Every section carries a table, list, diagram or code block — a section that would be pure prose is either wrong or too long.

Neighbours that are not this:

- `tests/api/specs/*.spec.ts` — executable scenario chains with permanent IDs. There is no parallel markdown specs tree.
- `docs/architecture/` — what the system **is**, as built.

## Researching the current state

A spec proposes a change to something. **Work out what that something does now, before asking anything about changing it.**

Use the `/explaining-code` shape and its caps — the state slice first, then a trace of ≤ 7 hops with `file:line` at each. Two rules keep it from turning into a doc:

- **Only what the change touches.** A component the change won't affect doesn't appear. This is scoped by the diff you expect, not by the subsystem it lands in.
- **Say what is dead.** Paths that exist and never run decide scope, and nobody finds them from the outside.

The research goes on the page, not into chat. Each question then names the one fact that decides it and points at the page, instead of carrying its own background.

## Growing the planned solution

An interview that only asks ranked decisions produces a decision table and nothing else — five technology picks, no description of what gets built. The decision table records *what was chosen*; `## Planned solution` has to say *what the thing is*.

**It grows as sections, not as a list of answers.** An answer usually sharpens a section that already exists. Appending a `###` per answer is what turns the planned solution back into a restatement of the decision table — the failure this section exists to prevent.

- **Ask about the seams.** Which seams the change is tested at, why the highest available one, and what prior art exists.
- **Ask about behaviour, using `/grilling`'s selection shape.** "Here are the eighteen chains that exist — which survive, which are dropped, what's missing?" is a question the ranked-pick format cannot express.
- **Ask about failure.** What can fail, what a partial write leaves behind, what stays available while a dependency is down. This is the material an interview loses most often, because nobody volunteers it.

**The stopping test is sufficiency, not slot-filling:** could someone build this without asking you anything? A page where every section has content but no statement of what the thing does is not finished.

### Sending one of those questions

**The page is updated before the question goes out, never after.** Whatever you found since the last answer — the precedent, the schema difference, the bug in the SDK, the committed decision that constrains it — goes onto the page as the open decision's evidence first. Then the message links to it. Research that reaches the user only as a chat message is lost twice over: the spec is written from the page, and the sandbox does not outlive the interview.

The turn itself is four things, in this order, and nothing else — `/asking-questions` owns the shape:

1. **What is being decided**, in one line.
2. **The options**, as a real numbered list, recommendation first. Never a fork in prose: "do you want X, or is there a reason for Y I'm not seeing" is answered by retyping X, which is the tell that no list was offered.
3. **At most one line of stake** — what it costs to pick wrong.
4. **The link**, naming what is on the page: "the protocol difference and the SDK bug are under Decision 5."

What is already settled goes in the footer line, not the opening. A turn that opens "Decision 2 is recorded, and reading the schema turned up…" has spent its first line on what you just did.

**If the question needs three paragraphs of background to be answerable, it is not one question.** Split it, or the background is not background — it is a decision you have not surfaced yet.

### What changes — the three lists the planned solution opens with

Immediately under `## Planned solution`, before `### Problem` and before any other heading. **It carries no heading of its own** — `Planned solution` is its heading, and a `### What changes` above it would read as a section rather than as the summary of the ones below.

- **New** — what does not exist today.
- **Changes materially** — what exists and behaves differently afterwards. Not "touched": a call site updated for a renamed argument is not a material change.
- **Untouched** — what a reader would reasonably expect to be affected and is not. **This is the list that earns the lead-in its place.** It is the only place in either surface that says what the change does not do, and it is what stops a reviewer assuming the blast radius is wider than it is.

All three are written every time. An empty **New** is a statement worth making; an empty **Untouched** means the boundary was not worked out.

**Named as things on the page, as paths in the markdown.** "Repository sync stops carrying authority" for a person; `backend/src/sync/mod.rs:161` for the agent. Same list, same order, both surfaces.

**`Untouched` is not `Out of scope`.** Out of scope is work someone might want and is not being done. Untouched is part of the system that keeps behaving exactly as it does today. A reader checking whether their own feature is at risk reads Untouched; a reader asking what else was considered reads Out of scope.

### The sections, and when each is included

Include a section when **any** of its triggers applies. A section whose triggers are all unmet is **left out of both surfaces** — not stubbed, not "n/a".

**Every section is a named heading**, carrying its own name from the table below. The triggers are how *you* decide whether to write it; they are not content. Neither surface carries a line justifying a section's presence — "included because", "always included", or a restatement of the trigger. A reader who has the section in front of them does not need to be told it qualified.

| Section | Include when |
|---|---|
| **Architecture and key flows** | More than one component is affected · a component's responsibility changes materially · a new dependency direction or architectural boundary appears · the change adds a background process, integration, cache or storage layer · the work could plausibly be placed in several parts of the system · the solution deviates from an established repository pattern |
| **Data model** | The change affects the shape, ownership, lifecycle, persistence, identity, validation, sensitivity or consistency of meaningful state — persistent records, cache keys and invalidation, session state, in-memory registries, message payloads, file formats, state machines, derived or replicated state |
| **Interfaces and contracts** | Another user, component or system depends on the behaviour |
| **Failure, safety and recovery** | A dependency can fail · a write can partially complete · operations may be retried · failure could leave inconsistent state · failure has a user-visible or operational consequence · a security-sensitive action can be rejected · recovery or rollback is required · existing behaviour must stay available in degraded mode |
| **Testing and verification** | Always |

"State that outlives a single call" is **not** the data-model test. An in-memory registry whose key changes qualifies, on identity and consistency.

### Architecture and key flows

**Name affected and newly introduced modules only.** Not an inventory of the subsystem, and never a path that has not been opened. For each one, keep four things apart:

- **Observed** — what the repository does today, with the symbol and line it was read at.
- **Proposed responsibility** — the one question that module answers after the change.
- **Proposed change** — what is edited or added.
- **Evidence** — file, symbol, test or configuration path, probed in this checkout.

**Placement follows directories that exist**, not a conventional source tree. If a route, table or job already has a home, that home is the answer — name the observed fact that puts it there. Where a fact could not be established, write that it was not established and what to probe; never a plausible value. Probing is cheap and an invented path reaches an implementer as fact.

**Key flows go here, not in a section of their own.** Include a flow when ordering matters, several components participate, a transaction or consistency boundary matters, work is asynchronous, data crosses a trust, process or network boundary, side effects happen, requests may be duplicated or reordered, or cache invalidation or propagation matters.

Do not document every flow. Document **the main successful flow, each material failure flow, and one flow per materially different consistency or security boundary** — each naming the rule that admitted it.

This section is per-module and detailed. The system-wide view is the three lists the planned solution opens with, and the two are written at different resolutions. The lead-in answers "which parts of the system does this touch at all". Architecture answers "what does this module become responsible for".

### Data model

Two levels, and the second is conditional:

- **Logical model** — entity meaning, ownership, identity, lifecycle, invariants, relationships.
- **Physical representation** — tables, fields, types, indexes, serialisation. **Only where it is constrained, or where repository evidence supports it.** A migration echoing an existing table's column names is constrained; a guessed index is not.

**Only changed or newly relevant fields.** Describing an existing table in full is noise and costs the implementing agent context it needs elsewhere. A field whose *meaning* changes is newly relevant even though the column is old — say which and why.

**Both surfaces take the same shape: a `classDiagram` first, then one sub-section per entity the change adds or modifies** — its real name as the heading, one line of what it holds and what identifies it, then a field table with the columns that entity's change turns on. See [DESIGN.md](DESIGN.md), "The data model". An entity the change only reads appears in the diagram and gets no sub-section. **Per-field marking is for modified entities**; a new one is new in every row, so it says so once above its table. The markdown is not capped, so it carries every changed entity; the page carries the same ones at one line of reasoning each.

### Interfaces and contracts

Separate the two, because they carry different weight:

- **External contract** — behaviour relied on by users, clients, services, plugins, stored data, scripts or operators. Trace each back to the requirement it serves. Changing one is a product decision.
- **Internal interface** — a proposed implementation boundary that may change without affecting observable behaviour. It belongs in the markdown so the implementer has a plan, and it is **not** a promise. Say that in the section, so changing one later needs no spec update.

**Name the consumer and name the interface, both as they exist in the code.** A contract written as "anyone reading a conversation can read messages and activities as one stream, on the conversation view" has said nothing an implementer can act on: it does not say which component holds the expectation, which type or route carries it, or what breaks. Write the component — `apps/frontend`'s conversation view, the paired T3 client, the sync worker — and the interface it holds: the payload type, the route, the event name, the RPC method, with `file:line`. A role belongs there only where there is genuinely no code on the other side, and then the interface is the command or documented procedure by name.

The test for whether something belongs at all is **whether anything on the other side breaks when it changes** — true of an in-repo payload the frontend renders, false of a helper one worker calls. That test admits in-repo boundaries; it does not admit module structure.

Where an internal change has a wide blast radius, list the call sites — as information, not as a contract.

### Failure, safety and recovery

For each failure worth naming: the condition, the state before, the state after, what the caller sees, whether retry is safe, the retry and timeout policy, the idempotency mechanism, cleanup or compensation, logging and alerting, and the recovery or rollback procedure. Routine failures fit one table; the ones that are not routine get a paragraph each.

**Fix the meaning, not the copy.** Exact user-visible wording is settled here only where it is genuinely a product or compliance decision — mark which. Otherwise state what the person must be able to work out and leave final copy to whoever owns it.

### Testing and verification

Unit, integration, contract and end-to-end cases; the exact commands, **including which tier actually builds them**; manual verification; acceptance criteria as Given/When/Then. Use the `/codebase-design` vocabulary for the seams tests attach to: module, interface, depth, seam, adapter, leverage, locality.

Separate **what only a person can judge** from what a test proves. That separation is what the page carries.

## The spec markdown

Written at step 5, in one pass, with no interview left to run. Headings in this order, so a reader who has read one spec can navigate every spec.

```md
# <The change, not the component>

> **Status:** approved | in progress | shipped v0.1.x | abandoned
> **Updated:** YYYY-MM-DD
> **Vocabulary:** CONTEXT.md

## Current solution   — the anchors, and what research turned up that the code does not say
## Planned solution   — what gets built
<the three lists — new · changes materially · untouched — with no heading of their own>
### Problem           — from the perspective of whoever feels it
### Behaviour         — user stories or Given/When/Then, grouped by actor
### Architecture and key flows    — conditional; triggers above
### Data model                    — conditional
### Interfaces and contracts      — conditional
### Failure, safety and recovery  — conditional
### Testing and verification      — always
## Out of scope
```

**The unconditional headings may not be reordered or renamed.** The five conditional ones are omitted when their triggers are unmet — an omitted section is a statement, an empty one is a gap.

### Rules for the spec

1. **It is written once, at step 5**, after the page is approved. `approved` is the first status the file can carry.
2. **Nothing is capped.** No line budget, no five-item lists, no `<details>`. An agent reads the file whole or greps it; a collapsed section and a truncated list both cost it.
3. **It is self-contained in what it asks for, not a copy of the page.** The page does not outlive the sandbox, so anything the implementation depends on and is only on the page is gone. That is a reason to write down every constraint, anchor and agreed behaviour — not a reason to transcribe the interview, which is what rules 4 to 6 exist to stop.
4. **There is no Decisions section.** The decision table is the page's, and it exists to get a person to approve a choice. This file is read by the agent that implements the choice, which does not re-open it — a table of what was picked over what, restating in a second place what the sections already state as fact, is one more thing to keep consistent and the first thing to go stale.
5. **Where a section states something that looks arbitrary, that section says why.** One line, in place: "no foreign key — a record must outlive what it names", "the closed set, because a client switches on it exhaustively". That is what an implementer needs to not undo the choice by accident, and it is the only part of the reasoning that survives the interview.
6. **There is no Open questions section and no Ticket breakdown.** A question that blocks implementation means the spec is not approved, and one that does not block belongs to the work item, not to the file an agent builds from — an agent that meets an open question either guesses or stalls, and both are worse than the question having been settled. The breakdown lives in the tracker `/to-tickets` writes to; a copy here says `3/7` until the day it says nothing true.
7. **Status is edited in place** every session that touches the spec.
8. **No preamble.** The first heading after the title is `## Current solution`. Not "Background", not "Motivation", not "This document describes".

### Current solution — anchors, not a transcription

**The agent has the repository.** Narrating how the current flow works is a slower, staler copy of code it is about to open, and it is the section that grows without limit because everything read during research feels worth keeping.

Two things earn a place, and nothing else does:

- **The anchors.** Every place the change lands, as `file:line` — the function that gets the new branch, the migration the next one follows, the type that gains a field. This is the section's main job: an agent that has to search for the seam can pick the wrong one.
- **What the code does not say.** The SDK bug, the ordering enforced in code and not in the schema, the constraint that only shows up under concurrency, the count that decided an approach, the precedent in another module. These took research; they do not survive the sandbox, and re-deriving one costs an hour.

**The test for a sentence is: would the implementer get this wrong, or spend a long time finding it?** If not, cut it — including anything true and interesting. A paragraph explaining what a function plainly does fails the test.

Where the *shape* of the current code is genuinely hard to hold — several components, an ordering that matters — draw it rather than narrate it, and keep the anchors on the diagram.

**Behaviour** covers every actor; past seven stories in a group, use a table rather than cutting any.

## Writing style

**The markdown is read by an agent, so it names things exactly** — fields, routes, events, status codes, commands, symbols, and file paths with `file:line` where the fact was read. Nothing in it is softened into plain-English state names: that costs the implementer the identifier it needs and buys nothing, because no person reads this file.

**The page is read by a person, so it names things as a person would** — "the App record", not `github_apps.app_key`. Same sections, same shapes, different resolution.

Two more:

- **Literal language.** No idiom, no figurative phrasing — no "circle back", no "get the ball rolling".
- **One instruction per line.** A sentence with two "and then"s is two lines.

Use the domain vocabulary in `CONTEXT.md` throughout, and respect local committed decisions from `AGENTS.md` files in the touched directory and its parents. A spec that contradicts one must say so and argue for changing it, in the section that contradicts it and on the page's decision table.

## What the user can change

Say these are adjustable the first time each one bites, once, in one line.

| Knob | Default | Say it when |
|---|---|---|
| Questions per turn | one | opening the first batch |
| The understanding questions | on | opening them — skippable if they wrote the code |
| Spec depth | full | the change is small enough that the page plus tickets would do |
| Where it stops | the approved page | the user wants tickets in the same sitting |
