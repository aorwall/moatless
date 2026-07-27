# The spec page — layout

What goes on the page and in what order. `/html-report` owns the rest: where the file goes, the preview URL, the Write-tool rule, the scaffold and the style baseline. **Read it first.**

The page is `scratchpad/<slug>.html`, one file at one URL. `/writing-specs` says when it appears and when it changes.

## The sections, in this order

**1. Lede** — the top block, and the only part the user is guaranteed to read.

- **After this ships** — one sentence naming what someone can do that they cannot today.
- **In scope / out of scope** — two short lists side by side. "In" is the committed surface; "out" sets expectations about what won't be touched.
- **Decisions** — one row per settled answer, added as each lands. Four columns, and **keep the rejected one**: the alternatives are what the user is being asked to approve, so dropping them to save space removes the point of the section. A decision still open gets its ranked options, marked visibly unresolved.

**2. Current solution** — the state slice, the trace, the comparison tables. The page's first real content and the reason it exists this early: diagrams are exactly what a markdown preview cannot draw. It sits before the decisions because the reader needs the shape before the choices, and at approval it is what they check the decisions against.

**3. Planned solution** — what gets built, growing through the interview. Not a restatement of the decision table: the table says what was chosen, this says what the thing does.

**4. Open questions** — parked, nobody being asked. State the user needs to see before approving.

**5. Work** — the Mermaid dependency graph from `/to-tickets`, node colours by state, the `classDef` set from [DESIGN.md](DESIGN.md). Empty until the breakdown exists.

Footer: the status line. Nothing else — no path to a spec file, because there isn't one until the page is approved.

Sections 3 and 5 are empty for most of the interview and that is fine. **Leave them visible, with a "not settled yet" placeholder** — what is still open is information, and a section that silently isn't there reads as one nobody thought of.

## Style, on top of `/html-report`

- **The decision table is the centrepiece**, not a diagram. This is the one report type where a table carries more than the graphics — a spec review is a set of choices, and choices read better as rows than as boxes. Give it the weight the architecture report gives its before/after diagrams.
- **Status as a chip** in the header: amber while the interview runs, emerald once approved.
- **Open questions get amber and sit above the fold.** An unresolved question the user scrolls past is the failure this page exists to prevent.
- Keep `/html-report`'s restraint: one accent colour, red for problems, amber for warnings, generous whitespace.
- **The question being asked never appears here.** It is asked in chat. A question one turn stale is a page that is wrong, which would force a redraw every turn.
