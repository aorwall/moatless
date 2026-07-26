---
name: writing-specs
description: Shape a spec or plan so it can be approved without reading all of it — a capped lede carrying the decisions, open questions and next action, over uncapped detail. Use when writing or updating a spec, plan or change proposal, when recording what shipped, or when another skill needs the spec shape.
---

# Writing Specs

A spec has **two audiences, not two altitudes**, and they live in one file:

- **The lede** — for the human who approves. Assume this is the only part they ever read. It is not a summary of the detail; it is the entire decision surface, and a decision that lives only below the `---` is a decision that never gets made.
- **The detail** — for the agent that implements, for `/code-review`'s spec axis, and for whoever asks "why is it like this" in six months. Written to be **located in**, not read through.

**Cap the surface, never the substance.** Nothing gets cut to be brief; the entry point gets capped so the reader always knows where to start.

**Design against flatness, not against length.** A dense decision table gets read; a screen of uniform paragraphs does not, at any length. Every section carries a table, list, diagram or code block — a section that would be pure prose is either wrong or too long.

## Where specs live

`specs/<slug>.md` — one file per proposed change, in the repo so it survives the sandbox and gets reviewed with the code.

Neighbours that are not this:

- `specs/<domain>/NN-*.md` — Given/When/Then scenario chains with permanent IDs, tied to E2E tests. Own format, in [`specs/AGENTS.md`](../../../specs/AGENTS.md).
- `docs/architecture/` — what the system **is**, as built.
- `docs/adr/` — **why** a hard-to-reverse decision was made. Offer one only when it passes all three conditions in the `/domain-modeling` skill's ADR format.

## The lede

Every `specs/<slug>.md` opens with this block. Everything below the `---` is detail.

```md
# <The change, not the component>

> **Status:** ideation | discussion | proposed | approved | in progress (3/7 tickets) | shipped v0.1.x | abandoned
> **Updated:** YYYY-MM-DD
> **Vocabulary:** CONTEXT.md · **Decisions:** ADR-NNNN

## Lede

**After this ships:** one sentence naming what someone can do that they cannot do today.

**In scope:** 3–5 bullets, ≤ 10 words each.
**Out of scope:** 3–5 bullets.

**Decisions** — the ones a reader could reasonably disagree with:

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| 1 | Where the seam sits | … | … |

**Open questions:** ≤ 3, each marked blocking or not. Delete as they resolve.

**Work:** a Mermaid graph — ticket names, arrows for blocking edges.

**Next action:** one thing, doable in under two minutes.

---
```

### Rules for the lede

1. **≤ 40 lines, and count them — don't estimate.** The Work graph's fenced block is the one thing that doesn't count against it: the budget bounds reading, and a diagram is looked at rather than read. Everything else counts, including table headers and separator rows, which eat about four lines before any content. If it doesn't fit, it belongs below the `---`.
2. **Present the decision space, not one path.** Each decision row names what was rejected and why. Collapsing a choice to a single answer doesn't reduce the reader's load — it hides the thing they have to hold. Where a decision is still live, give 2–4 ranked options with a one-line trade-off each, recommendation first.
3. **Status and Open questions are edited in place** every session that touches the spec. A stale status is worse than none: the reader can no longer trust any line in the file. The first three values say what is being asked of the reader, so it is never inferred: **ideation** — half-formed, no response owed; **discussion** — input wanted now, nothing settled; **proposed** — complete, awaiting a yes or a no.
4. **No preamble.** The first heading after the title is `## Lede`. Not "Background", not "Motivation", not "This document describes".
5. **One next action, and it is small.** "Open `backend/src/agent/mod.rs`" counts. Two next actions means the reader picks, and picking is where work stalls.
6. **Cap every list at 5 and every bullet at ~10 words.** Five ranked beats ten unranked.

## The detail

Written for the implementer and the archive, and optimised for someone arriving with a specific question — never for someone starting at the top. Completeness lives here precisely because brevity is guaranteed above it.

Fixed heading order, so a reader who has read one spec can navigate every spec. Sections may be empty; they may not be reordered or renamed. The order never varies by author or by document — predictable structure is what makes "one click away" cheap enough to use.

```md
## Problem
## Solution
## Decisions          — one ### per lede table row, with the full reasoning
## Behaviour          — user stories or Given/When/Then, grouped by actor
## Seams and testing
## Out of scope
## Open questions
## Ticket breakdown
```

- **Problem / Solution** — from the perspective of whoever feels it. Name the user-visible risk, not the code smell.
- **Decisions** — the rejected alternatives are the most valuable content in the file: they are what stops the decision being re-litigated in six months. Never delete one to save space; push it down the page.
- **Behaviour** — group user stories by actor and cap each group at 7. Past that, use a table. A flat list of thirty stories is a list nobody reads; the same thirty grouped and tabled is one screen. Cover every actor — completeness lives here, brevity lives in the lede.
- **Seams and testing** — which seams, why the highest available one, and prior art for the tests. Use the `/codebase-design` vocabulary exactly: module, interface, depth, seam, adapter, leverage, locality.
- **Ticket breakdown** — vertical slices, each sized to one context window, each declaring its blocking edges. `/to-tickets` produces these; paste its graph back into the lede's Work section so the two never disagree.

## Design elements

The spec is read in plain markdown far more often than as a rendered page, so the design goes in the markdown. Every element must **locate**, **chunk**, **signal state**, **show shape**, or **hide** — anything else is decoration, and decoration competes for the attention the real signals need. Four are not optional:

- **The decision table**, four columns, rejected alternative beside the chosen one. The layout is the point: you cannot read the decision without reading what it cost.
- **The work graph**, Mermaid, node colours by state (done / in progress / todo / blocked). It shows the dependency shape and doubles as the progress display.
- **`<details>` for the long tail** — story groups past the first, prior art, full rejected reasoning. Substance stays in the file; the scroll stays short. Always put the count in the summary. Never inside the lede.
- **Deep links** from each decision row down to its `###` section, which is what makes "one level down is one click away" true rather than aspirational.

These are also what keeps the page from going flat. Cutting an element to "keep it clean" removes something for the eye to land on, and a stretch with nothing to land on is where reading stops — the cost is paid in attention, not in lines.

Full catalogue, the state-coloured `classDef` block, and the ban list: [DESIGN.md](DESIGN.md).

For the rendered page, `/html-report` supplies the mechanics and [HTML-REPORT.md](HTML-REPORT.md) the spec layer. One file at one stable URL, in two states: a live **board** during the interview, the **review page** at approval. It renders the lede, never the spec, and it is a projection — write to the markdown first, re-render second, so nothing exists only there.

## Writing style

The rules in [`specs/AGENTS.md`](../../../specs/AGENTS.md) apply to every spec, not only scenario files: no endpoint paths, no field names, no status codes, no infrastructure terminology, no event strings. Describe what happens, in plain-English state names.

Two more:

- **Literal language.** No idiom, no figurative phrasing. "Circle back" and "get the ball rolling" force the reader to translate before they can act.
- **One instruction per line.** A sentence with two "and then"s is two lines.

Use the domain vocabulary in `CONTEXT.md` throughout, and respect existing ADRs — a spec that contradicts one must say so and argue for reopening it, in the Decisions table.

## The sequence

Each step ends on a checkable criterion, so you can stop cleanly at any of them.

1. **Grill.** Interview the user toward the lede's slots — **read [GRILLING.md](GRILLING.md) before asking the first question.** The four rules that change the outcome:
   - **One question per turn**, never two. Two questions means the user picks which to answer, and picking is where work stalls.
   - **Batch by lede slot, and finish a batch before starting the next.** All the scope questions, then all the decisions. Name the batch and its size when it opens. Switching subject between consecutive questions is the expensive move, not answering another question.
   - **Every question carries your recommended answer** plus 2–4 ranked alternatives with one-line trade-offs. Reacting to a proposal is cheap; answering a blank prompt is not.
   - **Every question restates position and settled state** — "Q3 of ~6 · filling: Decisions row 2 · settled: in-process seam". Never ask the user to remember an earlier answer.
   - **Look facts up; only ask decisions.** Being asked something the agent could have read is the fastest way to lose the room.

   Bind the questions to the lede's slots so the interview has a visible end, and say the estimated count up front. Park anything you can't resolve into Open questions rather than chasing it. ADRs and `CONTEXT.md` entries land inline as decisions crystallize (`/domain-modeling`).

   **Write each answer into `specs/<slug>.md` as it lands** — the lede is built during the interview, not reconstructed after it. Past about four questions, render a live **board** beside it (see [HTML-REPORT.md](HTML-REPORT.md)): the lede drawn as a skeleton with its empty slots visible, re-rendered from the file each time an answer lands. It holds the settled state so the chat doesn't have to, and the empty slots show how much is left without anyone counting.
   *Done when:* every lede slot is filled or explicitly parked.
2. **Approve at the lede.** ≤ 40 lines, nothing below the `---` yet. Paste it into the message *and* link the file — `<origin>/tasks/<task-id>?file=moatless%2Fspecs%2F<slug>.md&fileView=preview` — then wait. This is the approval, not a checkpoint on the way to one, so before you show it, check that nothing needed to say yes is waiting to be written below.
   *Done when:* the user agrees with every row of the decision table. Approving 40 lines takes two minutes; approving 500 never happens.
3. **Fill the detail, after approval.** Expand below the `---` without re-interviewing. It is written for the implementer, so don't send the user back through it; the lede changes only if a decision changes.
   *Done when:* every lede decision row has a `###` section under `## Decisions`.
4. **Tickets.** Run `/to-tickets`, then paste the dependency graph into the lede.
   *Done when:* the lede's Work graph matches the published tickets.
5. **Track.** As tickets land, edit the Status line and the Work graph. Nothing else moves.
   *Done when:* Status reads `shipped <version>` and Open questions is empty.

Steps 2 and 3 are the load-bearing split: do the full thinking, then give the reader a filtered pass over what survived, with every level below it one click away.

## What the user can change

Say these are adjustable the first time each one bites, once, in one line — a workflow nobody knows they can bend gets abandoned instead of tuned.

| Knob | Default | Say it when |
|---|---|---|
| Questions per turn | one | opening the first batch — "one at a time, or the whole batch at once?" |
| The board | on past ~4 questions | giving the URL |
| Detail depth | full | the spec is small enough that the lede plus tickets would do |
| Where it stops | the lede | the user wants tickets in the same sitting |
