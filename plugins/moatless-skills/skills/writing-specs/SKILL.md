---
name: writing-specs
description: Take a change from research to an agreed spec — research and decisions on a live HTML page the user works against, then one complete markdown spec written after agreement, for the agents that implement it. Use when writing or updating a spec, plan or change proposal, when recording what shipped, or when another skill needs the spec shape.
---

# Writing Specs

## The workflow

1. **Research the current state**, and ask until you and the user see the same thing. Read the code; ask the understanding questions; settle the vocabulary against `CONTEXT.md`.
2. **Render the page.** Its first content is that shared understanding, as `## Current solution`. Give the URL once.
3. **Research and grill toward what to build**, growing `## Planned solution` as you go. This phase mixes reading code, asking the user, and writing the page — the seams are one of the questions. Update the lede and the decision table at the top whenever an answer changes them.
4. **Get the plan approved, on the page.** Paste the decision table into the message so the choice is reachable without opening anything.
5. **Write `specs/<slug>.md`, once** — complete and uncapped. It is for the agent that implements.

Nothing goes into `specs/` before step 5. A half-written spec is a second place for an answer to live, and two places disagree within about three questions.

**There is no ticket step.** Work that needs a breakdown goes through `/orchestrate`, which runs `/to-tickets` once the spec exists.

**Recording and redrawing are different, and only one is a judgement call.** Recording is not optional: an answer lands, it goes on the page before the next question goes out — one `Edit` adding a row, because the page is the only record and an answer left in chat survives exactly as long as the session. Redrawing rewrites a section an answer made *wrong*, and the test is "wrong", not "out of date": a section the page already shows as open is incomplete, and incomplete costs nothing.

Everything else in this skill is *how* to do one of those steps:

| For | Read |
|---|---|
| Asking the questions in steps 1 and 3 | `/grilling` |
| Shaping the turns those questions are sent in | `/writing-messages` |
| Rendering and updating the page | [HTML-REPORT.md](HTML-REPORT.md), on top of `/html-report` |
| The elements that go in either surface | [DESIGN.md](DESIGN.md) |
| The shape of the file written in step 5 | Below |

## The two surfaces

- **The page** — `scratchpad/<slug>.html`. The only document the user ever works against, from step 2 to step 4. It carries the lede, the research, and every answer as it arrives.
- **The markdown** — `specs/<slug>.md`. Written at step 5, for the agent that implements, for `/code-review`'s spec axis, and for whoever asks "why is it like this" in six months.

**The page is capped by attention; the markdown is capped by nothing.** One screen is what gets approved. The implementing agent gets everything — flat, complete, anchored — because nothing is skimming it.

**Design against flatness, not against length.** A dense decision table gets read; a screen of uniform paragraphs does not, at any length. Every section carries a table, list, diagram or code block — a section that would be pure prose is either wrong or too long.

Neighbours that are not this:

- `specs/<domain>/NN-*.md` — Given/When/Then scenario chains with permanent IDs, tied to E2E tests. Own format, in `specs/AGENTS.md`.
- `docs/architecture/` — what the system **is**, as built.

## Researching the current state

A spec proposes a change to something. **Work out what that something does now, before asking anything about changing it.**

Two things go wrong without it. The user gets asked to choose between options whose consequences depend on current behaviour they are being told about *inside the question*, so the explanation and the decision compete for the same attention. And the facts scroll away: a table of current behaviour posted in chat is gone by question five, so the same ground gets covered twice.

Use the `/explaining-code` shape and its caps — the state slice first, then a trace of ≤ 7 hops with `file:line` at each. Two rules keep it from turning into a doc:

- **Only what the change touches.** A component the change won't affect doesn't appear. This is scoped by the diff you expect, not by the subsystem it lands in.
- **Say what is dead.** Paths that exist and never run are the most valuable thing here — they decide scope, and nobody finds them from the outside.

This is why the page exists so early. Research is the part with shape in it, and it is written once, so it costs exactly one render. It is also what lets the questions stay short: each one names the fact that decides it and points at the page, instead of carrying its own background.

## Growing the planned solution

Step 3 is where a spec becomes a specification rather than a list of choices, and it is the step that fails quietly. An interview that only asks ranked decisions produces a decision table and nothing else — five technology picks, no description of what gets built.

Three things keep it from going thin:

- **`## Planned solution` accumulates.** Every answer adds to it, not only to the decision table. If an answer changed nothing there, ask what it actually settled.
- **Ask about the seams.** Which seams the change is tested at, why the highest available one, and what prior art exists. Nothing else in the workflow asks this, and it is otherwise written from assumptions after approval.
- **Ask about behaviour, using `/grilling`'s selection shape.** "Here are the eighteen chains that exist — which survive, which are dropped, what's missing?" is a question the ranked-pick format cannot express.

**The stopping test is sufficiency, not slot-filling:** could someone build this without asking you anything? A page where every section has content but no statement of what the thing does is not finished.

## The spec markdown

Written at step 5, in one pass, with no interview left to run. Every heading, in this order, so a reader who has read one spec can navigate every spec. Sections may be empty; they may not be reordered or renamed.

```md
# <The change, not the component>

> **Status:** approved | in progress (3/7 tickets) | shipped v0.1.x | abandoned
> **Updated:** YYYY-MM-DD
> **Vocabulary:** CONTEXT.md

## Current solution   — how the affected area behaved before this change
## Planned solution   — what gets built
### Problem           — from the perspective of whoever feels it
### Behaviour         — user stories or Given/When/Then, grouped by actor
## Decisions          — the table, then one ### per row with the full reasoning
## Seams and testing
## Out of scope
## Open questions
## Ticket breakdown   — only when /to-tickets has run; "None." otherwise
```

### Rules for the spec

1. **It is written once, at step 5.** There is no draft state and no ideation state — those happened on the page. `approved` is the first status the file can carry.
2. **Nothing is capped.** No line budget, no five-item lists, no `<details>`. Every one of those bounds a human's reading, and no human is reading this. An agent reads the file whole or greps it; a collapsed section and a truncated list both cost it.
3. **It is self-contained.** The page is not in the repo and does not outlive the sandbox. Anything agreed there and not written here is gone — the research, the rejected alternatives, and the reasoning behind each one.
4. **Every decision carries what it cost.** The rejected alternatives are the most valuable content in the file: they are what stops the decision being re-litigated in six months. Never drop one for space.
5. **Status and Open questions are edited in place** every session that touches the spec. A stale status is worse than none: the reader can no longer trust any line in the file.
6. **No preamble.** The first heading after the title is `## Current solution`. Not "Background", not "Motivation", not "This document describes".

Two sections carry most of the weight. **Current solution** is the research, transcribed — written now, but describing the code as it was when the interview started. **Behaviour** covers every actor; past seven stories in a group, use a table rather than cutting any, because completeness is the whole point here.

For **Seams and testing**, use the `/codebase-design` vocabulary exactly: module, interface, depth, seam, adapter, leverage, locality.

## Writing style

Where the repo has a `specs/AGENTS.md`, its rules apply to every spec, not only scenario files: no endpoint paths, no field names, no status codes, no infrastructure terminology, no event strings. Describe what happens, in plain-English state names.

Two more:

- **Literal language.** No idiom, no figurative phrasing. "Circle back" and "get the ball rolling" force the reader to translate before they can act.
- **One instruction per line.** A sentence with two "and then"s is two lines.

Use the domain vocabulary in `CONTEXT.md` throughout, and respect existing ADRs — a spec that contradicts one must say so and argue for reopening it, in the Decisions table.

## What the user can change

Say these are adjustable the first time each one bites, once, in one line — a workflow nobody knows they can bend gets abandoned instead of tuned.

| Knob | Default | Say it when |
|---|---|---|
| Questions per turn | one | opening the first batch |
| The understanding questions | on | opening them — skippable if they wrote the code |
| Spec depth | full | the change is small enough that the page plus tickets would do |
| Where it stops | the approved page | the user wants tickets in the same sitting |
