# The spec page

One file, one URL, two states. During the interview it is a **board** that fills as answers land; at approval it is the **review page**. Same path throughout — the user opens one tab at question one and never hunts for the current version.

**Read [`/html-report`](../html-report/SKILL.md) first** — it owns where the file goes, the preview URL, the Write-tool rule that keeps the open tab live, the scaffold and the style baseline. This file adds only what is specific to a spec.

Use the **stable** naming rule from that skill: `scratchpad/<slug>-board.html`, overwritten in place. This page is updated on every answer, so a timestamped name would break the tab the user opened at question one.

Give the URL once, at the first question:

```
<origin>/tasks/<task-id>?file=scratchpad%2F<slug>-board.html&fileView=preview
```

**Link the spec file alongside it, every time you present the lede** — the board is a projection, the markdown is the artifact, and the reader should be one click from the thing that gets reviewed with the code:

```
<origin>/tasks/<task-id>?file=moatless%2Fspecs%2F<slug>.md&fileView=preview
```

Skip the board entirely for interviews under about four questions — the setup costs more attention than it saves.

## The board — the lede, drawn as it fills

Render the lede's slots as a skeleton from question one, **including the empty ones**. The reader sees the shape of the finished thing immediately and watches it fill, so "how much is left" is answered by the drawing rather than by a number they have to trust. It also puts every settled answer permanently on screen, which is the working-memory problem this whole workflow exists to solve.

Three slot states, visually distinct at a glance:

| State | Treatment |
|---|---|
| **Settled** | Solid border, white card, content rendered |
| **Current** | Amber ring (`ring-2 ring-amber-400`), carrying the question and its 2–4 ranked options in full |
| **Pending** | Dashed border (`border-dashed border-slate-300`), label only, no content |

The header carries the title, the status chip, and the progress bar — `Q3 of ~6 · ▓▓▓░░░`. The footer carries nothing; there is no next action yet.

The board is read-only, per `/html-report`'s baseline: the user answers in chat, and the page's only job is to hold the state they would otherwise have to.

## File first, board second

Never let an answer exist only on the board. When one lands, write it into `specs/<slug>.md`, then re-render the board from the file. The board is a projection, never a source.

That ordering is what makes the interview survivable: a sandbox restart or a closed tab costs nothing, and a session can stop after any question and resume from the markdown.

Re-render when an answer lands — not on every message.

## The review page — at approval

When the last slot fills, the board becomes the review page — same file, same URL, same tab: drop the pending outlines, add the four sections below. It renders the lede, not the spec. The spec file is the source of truth and it is complete; this page exists so a decision can be reached in two minutes. If a section needs more than 5–6 lines, condense it — the reader can open the file.

**1. What we're building** — one paragraph, plain language, no jargon. What will exist afterward that doesn't today?

**2. In scope / out of scope** — two short lists side by side. "In" is the committed surface; "out" sets expectations about what won't be touched.

**3. Decisions** — 3–6 rows, only choices the user might override. Render as a table and **keep the rejected column**: the alternatives are what the user is being asked to approve, so dropping them to save space removes the point of the section. Any decision still open gets its ranked options, marked as visibly unresolved.

**4. Work breakdown** — the Mermaid dependency graph, node colours by state, the same `classDef` set as the markdown (see [DESIGN.md](DESIGN.md)). The only technical detail on the page, and the user's chance to reshape the plan before work starts.

Footer: the spec's path, its status line, and the one next action.

## Style notes on top of `/html-report`

- **The decision table is the centrepiece**, not a diagram. This is the one report type where a table carries more than the graphics — a spec review is a set of choices, and choices read better as rows than as boxes. Give it the weight the architecture report gives its before/after diagrams.
- **Status as a chip** in the header: amber for proposed, emerald for approved, slate for in progress with its `3/7` beside it.
- **Open questions get amber and sit above the fold.** An unresolved question the user scrolls past is the failure this page exists to prevent.
- Keep `/html-report`'s restraint: one accent colour, red for problems, amber for warnings, generous whitespace.
