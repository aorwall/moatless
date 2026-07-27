# The spec page

One page at one URL, live from the end of research to approval. While the interview runs it is **the only document** — the only thing the user is pointed at, and the only record of what has been settled. `specs/<slug>.md` does not exist yet; it is written in one pass after the user says yes.

**Read [`/html-report`](../html-report/SKILL.md) first** — it owns where the file goes, the preview URL, the Write-tool rule, the scaffold and the style baseline. This file adds only what is specific to a spec.

Naming: `scratchpad/<slug>.html`, using the **stable** rule from that skill, so an update keeps the same tab alive. Give the URL once, when the research lands.

```
<origin>/tasks/<task-id>?file=scratchpad%2F<slug>.html&fileView=preview
```

## Recording versus redrawing

Two operations, and they carry neither the same cost nor the same rule.

**Record — always, immediately.** An answer lands, it goes on the page before the next question is asked. The page is the only record there is, so an answer left in the chat survives exactly as long as the session does. This is one `Edit` adding a row or a bullet to a section that already exists, not a re-render.

**Redraw — only when the page is wrong.** Not on a cadence.

| An answer… | Redraw? |
|---|---|
| Contradicts something already drawn | **Yes** — the page now asserts something false |
| Changes scope, so part of the research no longer applies | **Yes** — wrong by omission |
| Fills a section the page already shows as open | No — recording it is enough |
| Adds detail under a heading already rendered | No |

The distinction is *wrong* versus *incomplete*. A visibly unfinished page costs nothing; a page asserting something since overturned costs a decision.

This replaced a board that re-rendered wholesale after **every answer**, which put three or four tool calls between the user answering and the next question. That cost came from rendering the *question* — a question one turn stale is a page that is wrong by definition, so it forced a full redraw every turn. Keeping questions in chat is what makes the rule above affordable.

**First render:** as soon as the research is done, before the first question. Research is the part with shape in it — state diagrams, traces, comparison tables — and it is written once, so it costs exactly one render.

## What the page renders

Everything the user needs while planning, and nothing they are being asked right now.

**1. What we're building** — one paragraph, plain language, no jargon. What will exist afterward that doesn't today?

**2. In scope / out of scope** — two short lists side by side. "In" is the committed surface; "out" sets expectations about what won't be touched.

**3. The research** — the state slice, the trace, the comparison tables. This is the page's first content and the reason it exists this early: diagrams are exactly what a markdown preview cannot draw. It sits before the decisions because the reader needs the shape before the choices, and by approval it is what they check the decisions against.

**4. Decisions** — one row per settled answer, added as each lands. Render as a table and **keep the rejected column**: the alternatives are what the user is being asked to approve, so dropping them to save space removes the point of the section. Any decision still open gets its ranked options, marked as visibly unresolved.

**5. Open questions** — parked, nobody being asked. These are state, and the user needs to see them before approving.

**6. Work breakdown** — the Mermaid dependency graph, node colours by state, the `classDef` set from [DESIGN.md](DESIGN.md). The only technical detail on the page, and the user's chance to reshape the plan before work starts.

Order on the page: what we're building → scope → **the research** → decisions → open questions → work. The first two are empty for most of the interview and that is fine; empty is incomplete, and incomplete is not misleading.

Footer: the status line and the one next action. No path to a spec file — there isn't one until the page is approved.

## Style notes on top of `/html-report`

- **The decision table is the centrepiece**, not a diagram. This is the one report type where a table carries more than the graphics — a spec review is a set of choices, and choices read better as rows than as boxes. Give it the weight the architecture report gives its before/after diagrams.
- **Leave the unfilled sections visible**, with a "not settled yet" placeholder. What is still open is information; a section that silently isn't there reads as one nobody thought of.
- **Status as a chip** in the header: amber while the interview runs, emerald once approved.
- **Open questions get amber and sit above the fold.** An unresolved question the user scrolls past is the failure this page exists to prevent.
- Keep `/html-report`'s restraint: one accent colour, red for problems, amber for warnings, generous whitespace.
