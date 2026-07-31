# The spec page — layout

What goes on the page and in what order. `/html-report` owns the rest: where the file goes, the preview URL, the Write-tool rule, the scaffold and the style baseline. **Read it first.**

The page is `scratchpad/<slug>.html`, one file at one URL. `/writing-specs` says when it appears and when it changes.

## The sections, in this order

**1. Lede** — the top block, and the only part the user is guaranteed to read.

- **After this ships** — one sentence naming what someone can do that they cannot today.
- **In scope / out of scope** — two short lists side by side. "In" is the committed surface; "out" sets expectations about what won't be touched.
- **Decisions** — one row per settled answer, added as each lands. Four columns, and **keep the rejected one**: the alternatives are what the user is being asked to approve. A decision still open gets its ranked options, marked visibly unresolved.
- **The open decision carries its own evidence**, directly under those options: the facts that decide it, one line each with `file:line`. **This is what the question turn links to instead of explaining itself.** Without a home here, everything found between two questions ends up in a chat message, where it is both too long to read and absent from the artefact the spec is written from. Write it here first, then ask.

**2. Current solution** — the state slice, the trace, the comparison tables. It sits before the decisions: the reader needs the shape before the choices, and at approval it is what they check the decisions against.

**3. Planned solution** — what gets built, growing through the interview. Not a restatement of the decision table: the table says what was chosen, this says what the thing does.

**It opens with the three lists** — new, changes materially, untouched — side by side, named as things rather than files, with **no heading of their own**: the `Planned solution` heading is theirs. See `/writing-specs`, "What changes". They are the summary of everything below them, so they come before the first `<h3>`, and **untouched** is the list the reviewer checks their own assumptions against. It does not repeat the lede's out-of-scope list: that one is about work nobody is doing, this one is about behaviour nobody should worry about.

Then the same sections as the spec — see `/writing-specs`, "The sections, and when each is included" — at **high level**. Each one is a heading with its own name, then **one** element in the form below. **No line justifying the section**: not "included because", not "always included".

| Section | The form it takes on the page |
|---|---|
| Architecture and key flows | The affected components, named as things — what each one becomes responsible for, one line each. Then **one** key flow as a `sequenceDiagram`: the one that decides whether the design works. The other flows are named in a sentence, not drawn |
| Data model | **A field table** — one row per field of each **stored entity**, name and type in their own columns, **coloured by what the change does to that field**: new, meaning changes, or read-and-unchanged. A `classDiagram` instead when the relations are the point — [DESIGN.md](DESIGN.md), "The data model". Then one short block per entity in the logical vocabulary. A model is a marked-up table or a diagram; a list of five labelled bullets is neither |
| Interfaces and contracts | One table — **actor · what they can do · on which surface · when it takes effect** — then the promises as a list. **Internal interfaces do not appear**, beyond at most one line saying module names and signatures are in the spec and are not promises. A bullet about how a worker checks a flag is internal and belongs in the markdown |
| Failure, safety and recovery | One table — **when this happens · what the person must be able to work out · is the wording fixed** — then what still works while a dependency is down, and what rolling the change back would do |
| Testing and verification | Two lists side by side: what only a person can judge, and the acceptance criteria being approved. No commands, no test names |

**Only what is stored gets a box** — states and flags are fields on an entity, and a file that is read but never persisted is not in the model at all. See [DESIGN.md](DESIGN.md), "Choosing a diagram form".

**The marking is the reason this section exists.** A person approving a data model is asking which fields move; three states answer it before they read a word, and the entity blocks underneath explain why. An unmarked field list is the schema retyped.

**The data model is the one place the page uses the real names.** Elsewhere it says "the App record"; here it says `github_apps.app_key`, because a person approving a data model is approving the entity and the column, and "the record" does not say what changes.

**Use the same labels the spec uses.** A data model on the page is described with the logical-model vocabulary — entity meaning, ownership, identity, lifecycle, invariants, relationships — not with labels invented for one spec.

**No two sections take the same form.** Five cards each holding five bullets are indistinguishable, and the reader loses which section they are in. Every section above has a different shape for that reason.

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
