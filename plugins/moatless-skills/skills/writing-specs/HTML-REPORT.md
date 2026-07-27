# The spec page — layout

What goes on the page and in what order. `/html-report` owns the rest: where the file goes, the preview URL, the Write-tool rule, the scaffold and the style baseline. **Read it first.**

The page is `scratchpad/<slug>.html`, one file at one URL. `/writing-specs` says when it appears and when it changes.

## The sections, in this order

**1. Lede** — the top block, and the only part the user is guaranteed to read.

- **After this ships** — one sentence naming what someone can do that they cannot today.
- **In scope / out of scope** — two short lists side by side. "In" is the committed surface; "out" sets expectations about what won't be touched.
- **Decisions** — one row per settled answer, added as each lands. Four columns, and **keep the rejected one**: the alternatives are what the user is being asked to approve. A decision still open gets its ranked options, marked visibly unresolved.

**2. Current solution** — the state slice, the trace, the comparison tables. It sits before the decisions: the reader needs the shape before the choices, and at approval it is what they check the decisions against.

**3. Planned solution** — what gets built, growing through the interview. Not a restatement of the decision table: the table says what was chosen, this says what the thing does.

**4. Open questions** — parked, nobody being asked. State the user needs to see before approving.

**5. Work** — the Mermaid dependency graph from `/to-tickets`, node colours by state, the `classDef` set from [DESIGN.md](DESIGN.md). **Only when a breakdown is being made.**

Footer: the status line. Nothing else — no path to a spec file, because there isn't one until the page is approved.

Section 3 is empty for most of the interview. **Leave it visible, with a "not settled yet" placeholder** — what is still open is information.

## Style, on top of `/html-report`

- **The decision table is the centrepiece**, not a diagram. Give it the weight the architecture report gives its before/after diagrams.
- **Status as a chip** in the header: amber while the interview runs, emerald once approved.
- **Open questions get amber and sit above the fold.**
- Keep `/html-report`'s restraint: one accent colour, red for problems, amber for warnings, generous whitespace.
- **The question being asked never appears here.** It is asked in chat; a page carrying it would need a redraw every turn.
