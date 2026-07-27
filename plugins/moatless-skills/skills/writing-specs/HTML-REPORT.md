# The spec page

One page at one URL, live from the end of research to approval. It is the only thing the user is pointed at; the markdown is the implementing agent's copy.

**Read [`/html-report`](../html-report/SKILL.md) first** — it owns where the file goes, the preview URL, the Write-tool rule, the scaffold and the style baseline. This file adds only what is specific to a spec.

Naming: `scratchpad/<slug>.html`, using the **stable** rule from that skill, so a re-render keeps the same tab alive. Give the URL once, when `## Today` lands.

```
<origin>/tasks/<task-id>?file=scratchpad%2F<slug>.html&fileView=preview
```

**Never link the markdown alongside it.** It is written for the implementing agent, and it is the copy whose diagrams don't draw.

## When it renders

**First:** as soon as `## Today` is written, before the first question. Research is the part with shape in it — state diagrams, traces, comparison tables — and it is written once, so it costs exactly one render.

**After that: only when an answer made the page wrong.** Not on a cadence.

| An answer… | Re-render? |
|---|---|
| Contradicts something already drawn | **Yes** — the page now asserts something false |
| Changes scope, so part of `## Today` no longer applies | **Yes** — wrong by omission |
| Fills a slot the page already shows as open | No — incomplete, not wrong |
| Adds detail under a heading already rendered | No |

The distinction is *wrong* versus *incomplete*. A visibly unfinished page costs nothing; a page asserting something since overturned costs a decision.

This replaced a board that re-rendered after **every answer**, which put three or four tool calls between the user answering and the next question. That cost came from rendering the *question* — a question one turn stale is a page that is wrong by definition, so it forced a render every turn. Keeping questions in chat is what makes the rule above affordable.

## What the page renders

Everything the user needs while planning, and nothing they are being asked right now.

**1. What we're building** — one paragraph, plain language, no jargon. What will exist afterward that doesn't today?

**2. In scope / out of scope** — two short lists side by side. "In" is the committed surface; "out" sets expectations about what won't be touched.

**3. Decisions** — 3–6 rows, only choices the user might override. Render as a table and **keep the rejected column**: the alternatives are what the user is being asked to approve, so dropping them to save space removes the point of the section. Any decision still open gets its ranked options, marked as visibly unresolved.

**4. Work breakdown** — the Mermaid dependency graph, node colours by state, the same `classDef` set as the markdown (see [DESIGN.md](DESIGN.md)). The only technical detail on the page, and the user's chance to reshape the plan before work starts.

Footer: the status line and the one next action. Not the markdown's path — that is the agent's copy.

**`## Today` is rendered here, not linked.** It is the page's first content and the reason the page exists this early: state diagrams, traces and comparison tables are exactly what the markdown preview cannot draw. Put it after "what we're building" and before the decisions — the reader needs the shape before the choices, and by approval it is what they check the decisions against.

Order on the page: what we're building → scope → **the research** → decisions → open questions → work. The first two slots are empty for most of the interview and that is fine; empty is incomplete, and incomplete is not misleading.

## Style notes on top of `/html-report`

- **The decision table is the centrepiece**, not a diagram. This is the one report type where a table carries more than the graphics — a spec review is a set of choices, and choices read better as rows than as boxes. Give it the weight the architecture report gives its before/after diagrams.
- **Status as a chip** in the header: amber for proposed, emerald for approved, slate for in progress with its `3/7` beside it.
- **Open questions get amber and sit above the fold.** An unresolved question the user scrolls past is the failure this page exists to prevent.
- Keep `/html-report`'s restraint: one accent colour, red for problems, amber for warnings, generous whitespace.
