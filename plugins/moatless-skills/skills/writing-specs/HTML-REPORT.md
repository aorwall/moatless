# The spec review page

One page, rendered once, at approval. Its whole job is to make a yes-or-no reachable in two minutes.

**Read [`/html-report`](../html-report/SKILL.md) first** — it owns where the file goes, the preview URL, the Write-tool rule, the scaffold and the style baseline. This file adds only what is specific to a spec.

Naming: `scratchpad/<slug>-review.html`, using the **stable** rule from that skill, so a re-render after a change keeps the same tab alive.

```
<origin>/tasks/<task-id>?file=scratchpad%2F<slug>-review.html&fileView=preview
```

**Link the spec file beside it, every time** — the page is a projection, the markdown is the artifact, and the reader should be one click from the thing that gets reviewed with the code:

```
<origin>/tasks/<task-id>?file=moatless%2Fspecs%2F<slug>.md&fileView=preview
```

## Not during the interview

No page is rendered while questions are being asked. The question lives in the chat message and everything else lives in `specs/<slug>.md` — see [GRILLING.md](GRILLING.md).

This used to be a live board that re-rendered after every answer, and the cost landed in the wrong place: several tool calls between the user answering and the next question arriving, spent updating a tab they weren't looking at. The chat progress line already reports position and settled state, and the file already holds the detail, so the board was paying latency for work two cheaper surfaces were doing anyway.

Skip the page at approval too when the lede pasted into the message is enough — it usually is for a spec with three decisions. Say you're skipping it, once.

## What the page renders

The **lede**, never the spec. The spec file is the source of truth and it is complete; this page exists so a decision can be reached without opening it. If a section needs more than 5–6 lines, condense it — the reader can open the file.

**1. What we're building** — one paragraph, plain language, no jargon. What will exist afterward that doesn't today?

**2. In scope / out of scope** — two short lists side by side. "In" is the committed surface; "out" sets expectations about what won't be touched.

**3. Decisions** — 3–6 rows, only choices the user might override. Render as a table and **keep the rejected column**: the alternatives are what the user is being asked to approve, so dropping them to save space removes the point of the section. Any decision still open gets its ranked options, marked as visibly unresolved.

**4. Work breakdown** — the Mermaid dependency graph, node colours by state, the same `classDef` set as the markdown (see [DESIGN.md](DESIGN.md)). The only technical detail on the page, and the user's chance to reshape the plan before work starts.

Footer: the spec's path, its status line, and the one next action.

**`## Today` does not go on this page.** By approval it has done its job — it was the interview's reference, and what is being approved is the decisions. Link it, don't re-render it. A page that opens with how the system currently works buries the choices under material the reader has already been through.

## Style notes on top of `/html-report`

- **The decision table is the centrepiece**, not a diagram. This is the one report type where a table carries more than the graphics — a spec review is a set of choices, and choices read better as rows than as boxes. Give it the weight the architecture report gives its before/after diagrams.
- **Status as a chip** in the header: amber for proposed, emerald for approved, slate for in progress with its `3/7` beside it.
- **Open questions get amber and sit above the fold.** An unresolved question the user scrolls past is the failure this page exists to prevent.
- Keep `/html-report`'s restraint: one accent colour, red for problems, amber for warnings, generous whitespace.
