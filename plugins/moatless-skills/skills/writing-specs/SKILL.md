---
name: writing-specs
description: Take a change from research to an agreed spec — research rendered to a live HTML page, the interview in chat against it, and one complete markdown spec written after agreement for the agents that implement it. Use when writing or updating a spec, plan or change proposal, when recording what shipped, or when another skill needs the spec shape.
---

# Writing Specs

A spec has **two surfaces, and they do not exist at the same time**:

- **The page** — `scratchpad/<slug>.html`. The only document the user ever works against. It appears when the research lands, carries every answer as it arrives, and is what gets approved.
- **The markdown** — `specs/<slug>.md`. Written **once, after agreement**, for the agent that implements, for `/code-review`'s spec axis, and for whoever asks "why is it like this" in six months.

**Nothing is drafted in markdown while the user is still deciding.** A half-written spec is a second place for an answer to live, and two places disagree within about three questions. While the interview runs there is exactly one record of it, and the user can see it.

**The page is capped by attention; the markdown is capped by nothing.** One screen is what gets approved. The implementing agent gets everything — flat, complete, anchored — because nothing is skimming it.

**Design against flatness, not against length.** A dense decision table gets read; a screen of uniform paragraphs does not, at any length. Every section carries a table, list, diagram or code block — a section that would be pure prose is either wrong or too long.

## Where things live

| | Path | Lifetime |
|---|---|---|
| The page | `/opt/moatless/workspace/scratchpad/<slug>.html` | Research → approval. Not in the repo |
| The spec | `specs/<slug>.md` in the repo | Written at approval, kept forever |

Neighbours that are not this:

- `specs/<domain>/NN-*.md` — Given/When/Then scenario chains with permanent IDs, tied to E2E tests. Own format, in `specs/AGENTS.md`.
- `docs/architecture/` — what the system **is**, as built.
- `docs/adr/` — **why** a hard-to-reverse decision was made. Offer one only when it passes all three conditions in the `/domain-modeling` skill's ADR format.

## Research, before the first question

A spec proposes a change to something. **Work out what that something does now, before asking anything about changing it** — and render it as the page's first section, finished before the interview opens.

Two things go wrong without it. The user gets asked to choose between options whose consequences depend on current behaviour they are being told about *inside the question*, so the explanation and the decision compete for the same attention. And the facts scroll away: a table of current behaviour posted in chat is gone by question five, so the same ground gets covered twice.

Use the `/explaining-code` shape and its caps — the state slice first, then a trace of ≤ 7 hops with `file:line` at each. Two rules keep it from turning into a doc:

- **Only what the change touches.** A component the change won't affect doesn't appear. This section is scoped by the diff you expect, not by the subsystem it lands in.
- **Say what is dead.** Paths that exist and never run are the most valuable thing here — they decide scope, and nobody finds them from the outside.

This is why the page exists this early. Research is the part with shape in it — state diagrams, traces, comparison tables — and it is written once, so it costs exactly one render. It is also what lets the questions stay short: each one names the fact that decides it and points at the page, instead of carrying its own background.

When the spec is written at the end, this becomes `## Today`.

## The page

`/html-report` supplies the mechanics; [HTML-REPORT.md](HTML-REPORT.md) supplies the spec layer — the section order, what each one holds, and when the page is redrawn. Two rules are load-bearing enough to state here:

- **Every answer is recorded on the page before the next question is asked.** It is the only record there is, so an answer that isn't on it exists nowhere. That is one `Edit` adding a row, not a re-render.
- **The page never carries the question being asked.** Questions are asked and answered in chat. A question one turn stale is a page that is wrong, so rendering questions would force a redraw every single turn. *Open* questions — parked, nobody being asked — are state, and they belong on the page.

## The spec markdown

Written after the page is approved, in one pass, with no interview left to run. Every heading, in this order, so a reader who has read one spec can navigate every spec. Sections may be empty; they may not be reordered or renamed.

```md
# <The change, not the component>

> **Status:** approved | in progress (3/7 tickets) | shipped v0.1.x | abandoned
> **Updated:** YYYY-MM-DD
> **Vocabulary:** CONTEXT.md · **Decisions:** ADR-NNNN

## Today              — how the affected area behaved before this change
## Problem
## Solution           — opens with what someone can do afterward, then what is in scope
## Decisions          — the table, then one ### per row with the full reasoning
## Behaviour          — user stories or Given/When/Then, grouped by actor
## Seams and testing
## Out of scope
## Open questions
## Ticket breakdown
```

### Rules for the spec

1. **It is written once, at approval.** There is no draft state and no ideation state — those happened on the page, in chat. `approved` is the first status the file can carry.
2. **Nothing is capped.** No line budget, no five-item lists, no `<details>`. Every one of those bounds a human's reading, and no human is reading this. An agent reads the file whole or greps it; a collapsed section and a truncated list both cost it.
3. **It is self-contained.** The page is not in the repo and does not outlive the sandbox. Anything agreed on the page and not written here is gone — the research, the rejected alternatives, and the reasoning behind each one.
4. **Every decision carries what it cost.** The rejected alternatives are the most valuable content in the file: they are what stops the decision being re-litigated in six months. Never drop one for space.
5. **Status and Open questions are edited in place** every session that touches the spec. A stale status is worse than none: the reader can no longer trust any line in the file.
6. **No preamble.** The first heading after the title is `## Today`. Not "Background", not "Motivation", not "This document describes".

### Notes per section

- **Today** — the research, transcribed from what you found. It is written now, but it describes the code as it was when the interview started.
- **Problem / Solution** — from the perspective of whoever feels it. Name the user-visible risk, not the code smell.
- **Decisions** — open with the four-column table, one row per decision, each `#` linking down to its `###` section. The table is the index; the sections are the reasoning.
- **Behaviour** — group user stories by actor. Past seven in a group, use a table — thirty stories grouped and tabled is one screen, and completeness is the whole point here.
- **Seams and testing** — which seams, why the highest available one, and prior art for the tests. Use the `/codebase-design` vocabulary exactly: module, interface, depth, seam, adapter, leverage, locality.
- **Ticket breakdown** — vertical slices, each sized to one context window, each declaring its blocking edges, plus the Mermaid work graph. `/to-tickets` produces these.

## Design elements

Every element must **locate**, **chunk**, **signal state**, **show shape**, or **hide** — anything else is decoration, and decoration competes for the attention the real signals need. Which ones apply depends on which surface you are writing; [DESIGN.md](DESIGN.md) has the full catalogue, the state-coloured `classDef` block, and the ban list.

Three carry the spec markdown:

- **The decision table**, four columns, rejected alternative beside the chosen one. The layout is the point: you cannot read the decision without reading what it cost.
- **The work graph**, Mermaid, node colours by state (done / in progress / todo / blocked). It shows the dependency shape and doubles as the progress display.
- **Deep links** from each decision row down to its `###` section — the cheapest way for an agent to land on the right paragraph instead of the right file.

## Writing style

Where the repo has a `specs/AGENTS.md`, its rules apply to every spec, not only scenario files: no endpoint paths, no field names, no status codes, no infrastructure terminology, no event strings. Describe what happens, in plain-English state names.

Two more:

- **Literal language.** No idiom, no figurative phrasing. "Circle back" and "get the ball rolling" force the reader to translate before they can act.
- **One instruction per line.** A sentence with two "and then"s is two lines.

Use the domain vocabulary in `CONTEXT.md` throughout, and respect existing ADRs — a spec that contradicts one must say so and argue for reopening it, in the Decisions table.

## The sequence

Each step ends on a checkable criterion, so you can stop cleanly at any of them.

1. **Research.** Read the code, work out how the affected area behaves today, and render the page from it before asking anything.
   *Done when:* the page exists, carries the research, and its URL has been given.
2. **Grill.** Interview the user against that page — **read [GRILLING.md](GRILLING.md) before asking the first question.** The six rules that change the outcome:
   - **Open on shared understanding.** The first batch presents the research and settles the vocabulary against `CONTEXT.md`. Decisions phrased in terms the two of you don't hold identically get re-litigated later.
   - **One question per turn**, never two. Two questions means the user picks which to answer, and picking is where work stalls.
   - **Batch by page section, and finish a batch before starting the next.** Understanding, then scope, then decisions. Name the batch and its size when it opens. Switching subject between consecutive questions is the expensive move, not answering another question.
   - **Every question carries your recommended answer** plus 2–4 ranked alternatives with one-line trade-offs. Reacting to a proposal is cheap; answering a blank prompt is not.
   - **Every question restates position and settled state** — "Q3 of ~6 · filling: Decisions row 2 · settled: in-process seam". Never ask the user to remember an earlier answer.
   - **Look facts up; only ask decisions.** Being asked something the agent could have read is the fastest way to lose the room.

   Bind the questions to the page's sections so the interview has a visible end, and say the estimated count up front. Every answer lands on the page before the next question goes out; nothing is written to `specs/` yet. Park what you can't resolve into the page's open questions rather than chasing it. ADRs and `CONTEXT.md` entries land inline as decisions crystallize (`/domain-modeling`).
   *Done when:* every section of the page is filled or explicitly parked.
3. **Approve at the page.** Paste the decision table into the message so the choice is reachable without opening anything, link the page, and wait. Before you show it, check that nothing needed to say yes is still unwritten.
   *Done when:* the user agrees with every row of the decision table. Approving one screen takes two minutes; approving 500 lines never happens.
4. **Write the spec, once.** Now `specs/<slug>.md` gets written, complete, in the heading order above. It is for the implementing agent, so don't send the user back through it.
   *Done when:* every decision on the page has a `###` section under `## Decisions`, and nothing agreed exists only on the page.
5. **Tickets.** Run `/to-tickets`, then put the dependency graph in `## Ticket breakdown`.
   *Done when:* the work graph matches the published tickets.
6. **Track.** As tickets land, edit the Status line and the work graph. Nothing else moves.
   *Done when:* Status reads `shipped <version>` and Open questions is empty.

Steps 3 and 4 are the load-bearing split: the user agrees to one screen, and only then is the full record written.

## What the user can change

Say these are adjustable the first time each one bites, once, in one line — a workflow nobody knows they can bend gets abandoned instead of tuned.

| Knob | Default | Say it when |
|---|---|---|
| Questions per turn | one | opening the first batch — "one at a time, or the whole batch at once?" |
| The understanding batch | on | opening it — it's skippable if they wrote the code |
| Spec depth | full | the change is small enough that the page plus tickets would do |
| Where it stops | the approved page | the user wants tickets in the same sitting |
