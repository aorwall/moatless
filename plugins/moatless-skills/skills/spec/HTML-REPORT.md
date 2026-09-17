# The spec page — layout

What goes on the page and in what order. `/html-report` owns the rest: where the file goes, the preview URL, the Write-tool rule, the scaffold and the style baseline. **Read it first.**

The page is `scratchpad/<slug>.html`, one file at one URL. `/spec` says when it appears and when it changes.

## The sections, in this order

**1. Lede** — the top block, and the only part the user is guaranteed to read.

- **After this ships** — one sentence naming what someone can do that they cannot today.
- **In scope / out of scope** — two short lists side by side. "In" is the committed surface; "out" sets expectations about what won't be touched.
- **Decisions** — one row per settled answer, added as each lands. Four columns, and **keep the rejected one**: the alternatives are what the user is being asked to approve. A decision still open gets its ranked options, marked visibly unresolved.
- **The open decision carries its own evidence**, directly under those options: the facts that decide it, one line each with `file:line`. **This is what the question turn links to instead of explaining itself.** Without a home here, everything found between two questions ends up in a chat message, where it is both too long to read and absent from the artefact the spec is written from. Write it here first, then ask.

**2. Current solution** — the state slice, the trace, the comparison tables. It sits before the decisions: the reader needs the shape before the choices, and at approval it is what they check the decisions against.

**3. Planned solution** — what gets built, growing through the interview. Not a restatement of the decision table: the table says what was chosen, this says what the thing does.

**It opens with the three lists** — new, changes materially, untouched — side by side, named as things rather than files, with **no heading of their own**: the `Planned solution` heading is theirs. See `/spec`, "What changes". They are the summary of everything below them, so they come before the first `<h3>`, and **untouched** is the list the reviewer checks their own assumptions against. It does not repeat the lede's out-of-scope list: that one is about work nobody is doing, this one is about behaviour nobody should worry about.

Then the same sections as the spec — see `/spec`, "The sections, and when each is included" — at **high level**. Each one is a heading with its own name, then **one** element in the form below. **No line justifying the section**: not "included because", not "always included".

| Section | The form it takes on the page |
|---|---|
| Architecture and key flows | The affected components, named as things — what each one becomes responsible for, one line each. Then **one** key flow as a `sequenceDiagram`: the one that decides whether the design works. The other flows are named in a sentence, not drawn |
| Data model | **A `classDiagram` first**, then **one block per entity the change adds or modifies** — the entity's real name as the heading, one line of what it holds and whether it is new, and **its own field table**. A modified entity colours its rows by what happens to each field; a new one says "new" once in that line and drops the marking — [DESIGN.md](DESIGN.md), "The data model". The diagram is the map; the tables are the detail |
| Interfaces and contracts | One table — **consumer · the interface it depends on · what it can rely on · status** — then the promises as a list. **Both of the first two columns name something in the code**: the component, and the type, route, event or RPC method with `file:line`. The third is the promise, phrased the same whether the interface is new or old; new / changed / breaking / unchanged is the fourth — [DESIGN.md](DESIGN.md), "The interfaces table" |
| Failure, safety and recovery | One table — **when this happens · what the person must be able to work out · is the wording fixed** — then what still works while a dependency is down, and what rolling the change back would do |
| Testing and verification | Two lists side by side: what only a person can judge, and the acceptance criteria being approved. No commands, no test names |

**Only what is stored gets a box** — states and flags are fields on an entity, and a file that is read but never persisted is not in the model at all. See [DESIGN.md](DESIGN.md), "Choosing a diagram form".

**Marking earns its place only where the states differ.** A person approving a *modified* entity is asking which fields move, and three colours answer that before they read a word. A person approving a *new* one already knows: the answer is all of them. Repeating "New." down fourteen rows is a word that never varies pushed in front of the description that does.

**One table per entity, never one table for the whole model.** A twenty-row table whose Field column repeats `task_activities.` fourteen times is a schema dump: the prefix is the loudest thing in every cell, the reader cannot see where one entity ends, and no single set of columns fits five different tables. The entity name goes in the heading, once, and each table then picks **the columns that entity's change actually turns on** — key membership where identity moves, nullability where it is the point, the index where the query depends on it. Name and type are the only two that appear every time.

**An entity the change only reads gets a box in the diagram and no block of its own.** It is context, and the diagram is where context belongs. A field it holds that the change depends on goes in the note under the diagram — not in a table nobody needs to scan.

**The data model is the one place the page uses the real names.** Elsewhere it says "the App record"; here the heading is `github_apps` and the row is `app_key`, because a person approving a data model is approving the entity and the column, and "the record" does not say what changes.

**In Interfaces, a role is not a consumer.** "Anyone reading a conversation", "an operator", "the user" name nobody, so the reader cannot tell what breaks. Name the component — the `moat` CLI's message listing, the paired T3 client, the sync worker — and name the interface it actually holds: the payload type, the route, the event name, the RPC method, with `file:line`. A human operator is a consumer only where there is genuinely no code on the other side, and then the interface is the command or procedure by name. This does not reopen internal module boundaries: the test is whether something on the other side breaks when it changes, which is true of an in-repo payload the CLI renders and false of a helper one worker calls.

**Use the same labels the spec uses.** A data model on the page is described with the logical-model vocabulary — entity meaning, ownership, identity, lifecycle, invariants, relationships — not with labels invented for one spec.

**No two sections take the same form.** Five cards each holding five bullets are indistinguishable, and the reader loses which section they are in. Every section above has a different shape for that reason.

**One element per section — the data model is the exception, and the only one.** Five sections with a table *and* a diagram each is three screens of scroll; the page is capped by attention and the markdown is where the rest goes. The data model earns both because they answer different questions: the diagram answers "what relates to what", which no table can, and the per-entity tables answer "which fields move", which no diagram can.

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
