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

It carries the same sections as the spec — see `/writing-specs`, "The sections, and when each is included" — at **high level**, each one naming the trigger that admitted it:

| Section | On the page |
|---|---|
| Architecture and key flows | One after-picture. What is new, what changes materially, what is untouched — named as things, not files. **One** key flow, the one that decides whether the design works, with the rest named in a sentence |
| Data model | The model as a diagram, then why each field exists, in sentences |
| Interfaces and contracts | What each actor can do and where, and what is promised. Internal boundaries appear only as *not* promises, or not at all |
| Failure, safety and recovery | What the person must be able to work out in each failure, and which rows are fixed wording rather than meaning. Plus what still works while a dependency is down |
| Testing and verification | Only what a person has to judge, and the acceptance criteria they are approving |

**One element per section.** Five sections with a table *and* a diagram each is three screens of scroll; the page is capped by attention and the markdown is where the rest goes.

**4. Open questions** — parked, nobody being asked. State the user needs to see before approving.

**5. Work** — the Mermaid dependency graph from `/to-tickets`, node colours by state, the `classDef` set from [DESIGN.md](DESIGN.md). **Only when a breakdown is being made.**

Footer: the status line. Nothing else — no path to a spec file, because there isn't one until the page is approved.

Section 3 is empty for most of the interview. **Leave it visible, with a "not settled yet" placeholder** — what is still open is information.

## Style, on top of `/html-report`

- **The decision table is the centrepiece**, not a diagram. Give it the weight the architecture report gives its before/after diagrams.
- **Status as a chip** in the header: amber while the interview runs, emerald once approved.
- **Open questions get amber and sit above the fold.**
- Keep `/html-report`'s restraint: one accent colour, red for problems, amber for warnings, generous whitespace.
- **Never name a page class `label`, `node`, `edge`, `cluster` or `title`.** Those are Mermaid's own SVG class names and it inherits the rule — see [DESIGN.md](DESIGN.md), "Diagram hazards". This collides with `/html-report`'s style baseline, which suggests `text-xs uppercase tracking-wider` for labels inside diagrams: give that utility a namespaced name such as `.kicker`.
- **The question being asked never appears here.** It is asked in chat; a page carrying it would need a redraw every turn.
