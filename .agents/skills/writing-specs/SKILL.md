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

## Today, before the first question

A spec proposes a change to something. **Write down what that something does now, before asking anything about changing it** — into the spec, as `## Today`, finished before the interview opens.

Two things go wrong without it. The user gets asked to choose between options whose consequences depend on current behaviour they are being told about *inside the question*, so the explanation and the decision compete for the same attention. And the facts scroll away: a table of current behaviour posted in chat is gone by question five, so the same ground gets covered twice.

Use the `/explaining-code` shape and its caps — the state slice first, then a trace of ≤ 7 hops with `file:line` at each. Two rules keep it from turning into a doc:

- **Only what the change touches.** A component the change won't affect doesn't appear. This section is scoped by the diff you expect, not by the subsystem it lands in.
- **Say what is dead.** Paths that exist and never run are the most valuable thing here — they decide scope, and nobody finds them from the outside.

While the interview is running this is the only complete section in the file, so **it is what you link to, and what questions point at instead of restating.**

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
## Today              — how the affected area works now, written before the interview
## Problem
## Solution
## Decisions          — one ### per lede table row, with the full reasoning
## Behaviour          — user stories or Given/When/Then, grouped by actor
## Seams and testing
## Out of scope
## Open questions
## Ticket breakdown
```

- **Today** — the exception to everything in this section: written *before* the interview, not after approval, and the one part the user reads while being grilled. Everything below it is written after the lede is agreed.
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

For the rendered page, `/html-report` supplies the mechanics and [HTML-REPORT.md](HTML-REPORT.md) the spec layer. There is **one page and it appears once**, at approval: the review page. Nothing is rendered during the interview — the questions are the chat's job and the detail is the file's, and a page that has to be re-rendered between every question puts a build step between the user's answer and the next question. The page renders the lede, never the spec, and it is a projection: write to the markdown first, render second, so nothing exists only there.

## Writing style

The rules in [`specs/AGENTS.md`](../../../specs/AGENTS.md) apply to every spec, not only scenario files: no endpoint paths, no field names, no status codes, no infrastructure terminology, no event strings. Describe what happens, in plain-English state names.

Two more:

- **Literal language.** No idiom, no figurative phrasing. "Circle back" and "get the ball rolling" force the reader to translate before they can act.
- **One instruction per line.** A sentence with two "and then"s is two lines.

Use the domain vocabulary in `CONTEXT.md` throughout, and respect existing ADRs — a spec that contradicts one must say so and argue for reopening it, in the Decisions table.

## The sequence

Each step ends on a checkable criterion, so you can stop cleanly at any of them.

1. **Map today.** Read the code, then write `## Today` into `specs/<slug>.md` — the affected area only, in the `/explaining-code` shape. No questions yet.
   *Done when:* the file exists with a complete `## Today` and an empty lede skeleton above it.
2. **Grill.** Interview the user toward the lede's slots — **read [GRILLING.md](GRILLING.md) before asking the first question.** The six rules that change the outcome:
   - **Open on shared understanding.** The first batch presents `## Today` and settles the vocabulary against `CONTEXT.md`. Decisions phrased in terms the two of you don't hold identically get re-litigated later.
   - **One question per turn**, never two. Two questions means the user picks which to answer, and picking is where work stalls.
   - **Batch by lede slot, and finish a batch before starting the next.** Understanding, then scope, then decisions. Name the batch and its size when it opens. Switching subject between consecutive questions is the expensive move, not answering another question.
   - **Every question carries your recommended answer** plus 2–4 ranked alternatives with one-line trade-offs. Reacting to a proposal is cheap; answering a blank prompt is not.
   - **Every question restates position and settled state** — "Q3 of ~6 · filling: Decisions row 2 · settled: in-process seam". Never ask the user to remember an earlier answer.
   - **Look facts up; only ask decisions.** Being asked something the agent could have read is the fastest way to lose the room.

   Bind the questions to the lede's slots so the interview has a visible end, and say the estimated count up front. Park anything you can't resolve into Open questions rather than chasing it. ADRs and `CONTEXT.md` entries land inline as decisions crystallize (`/domain-modeling`).

   **The interview happens in chat, against the spec file.** Write each answer into `specs/<slug>.md` as it lands — the lede is built during the interview, not reconstructed after it — and link the file rather than restating what's already in it. No page is rendered yet; the question lives in the message and the detail lives in the file.
   *Done when:* every lede slot is filled or explicitly parked.
3. **Approve at the lede.** ≤ 40 lines, nothing below the `---` but `## Today`. Paste it into the message *and* link the file — `<origin>/tasks/<task-id>?file=moatless%2Fspecs%2F<slug>.md&fileView=preview` — then wait. This is where a **review page** earns its place, if anywhere does (see [HTML-REPORT.md](HTML-REPORT.md)): one decision, one screen, once. Before you show it, check that nothing needed to say yes is waiting to be written below.
   *Done when:* the user agrees with every row of the decision table. Approving 40 lines takes two minutes; approving 500 never happens.
4. **Fill the detail, after approval.** Expand below `## Today` without re-interviewing. It is written for the implementer, so don't send the user back through it; the lede changes only if a decision changes.
   *Done when:* every lede decision row has a `###` section under `## Decisions`.
5. **Tickets.** Run `/to-tickets`, then paste the dependency graph into the lede.
   *Done when:* the lede's Work graph matches the published tickets.
6. **Track.** As tickets land, edit the Status line and the Work graph. Nothing else moves.
   *Done when:* Status reads `shipped <version>` and Open questions is empty.

Steps 3 and 4 are the load-bearing split: do the full thinking, then give the reader a filtered pass over what survived, with every level below it one click away.

## What the user can change

Say these are adjustable the first time each one bites, once, in one line — a workflow nobody knows they can bend gets abandoned instead of tuned.

| Knob | Default | Say it when |
|---|---|---|
| Questions per turn | one | opening the first batch — "one at a time, or the whole batch at once?" |
| The understanding batch | on | opening it — it's skippable if they wrote the code |
| The review page | on | reaching approval — the lede in the message may be all they want |
| Detail depth | full | the spec is small enough that the lede plus tickets would do |
| Where it stops | the lede | the user wants tickets in the same sitting |
