# Design elements for specs

The elements below serve both surfaces — the **page**, where the user agrees to the change, and the **markdown**, which the implementing agent reads.

**Anything that lives only on the page evaporates.** The page is not in the repo and does not outlive the sandbox, so every element it carries has to have a counterpart in the spec once the spec is written.

## Every element earns its place by doing one of five jobs

| Job | The problem it solves | Element |
|---|---|---|
| **Locate** | "Where am I, and what kind of thing is this?" | Fixed heading order · deep links |
| **Chunk** | A wall of text has no handholds — you can't enter or leave it | Tables · one instruction per line · short sections |
| **Signal state** | The reader can't hold what's settled, shipped, or blocked | Status line · progress bar · graph node colours · task lists |
| **Show shape** | Relationships prose can't carry | Mermaid graphs · before/after trees |
| **Ground** | "Is this a fact about the repo, or a proposal?" | Observed/proposed split · `file:line` evidence |

If an element does none of the five, it is decoration. Cut it.

## No section is prose-only

The failure mode isn't length, it's flatness — a stretch with nothing for the eye to land on.

Every `##` carries at least one table, list, diagram, or code block. A section that resists this is either not yet decided, or it's two sections.

**Don't cut elements while tidying.** Removing one takes out a handhold: the result looks neater and reads worse.

## The elements

### Status line with a progress bar

Put progress in the status line, in text, so it renders everywhere:

```md
> **Status:** in progress · ▓▓▓▓▓░░░░░ 3/7 tickets · **Updated:** 2026-07-25
```

Update it in the same edit that lands a ticket.

### The decision table — four columns, never five

```md
| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-where-the-seam-sits) | Where the seam sits | Coordinator owns retry | Per-harness retry diverges silently |
| [2](#2-message-assembly) | Message assembly | Backend only | Sandbox-side coalescing duplicates work per harness |
```

The rejected alternative sits *beside* the chosen one, in the same row — never as a separate list, which lets the two be read apart.

Four columns is the cap — past that, cells wrap and the table stops being scannable in a narrow diff view. The `#` links down to the `###` section carrying the full reasoning.

For a decision that is still open, add a fifth *row* rather than a fifth column: ranked options, one line each, recommendation first.

### Choosing a diagram form

| For | Use |
|---|---|
| Who did what to whom, in what order | `sequenceDiagram` — one participant per component, `alt` for the failure branch, and a `Note over` for the thing that deliberately does *not* happen |
| X depends on Y depends on Z | `flowchart` |
| Records, their fields, their lifetime and cardinality | `classDiagram`, with lifetime as a `<<stereotype>>` so it reads in words and not only in colour |
| Legal states and the moves between them | `stateDiagram-v2` |

**Structure and time do not share an axis.** Two diagrams beat one doing both jobs: if a class diagram has grown a paragraph underneath explaining *when* things happen, that paragraph is a sequence diagram.

**A diagram that is mostly text is a table.** Check the shape is the point before drawing — an information model whose value is the *reason* beside each field reads better as annotated facts than as boxes.

**A node with no edges costs a whole row of layout.** Something that relates to nothing belongs in a sentence under the diagram.

### Diagram hazards, and how to know

Three ways a diagram breaks with nothing reported. All three have been hit in this repo:

1. **The HTML parser eats the source.** Inside `<pre class="mermaid">` the diagram is markup first: `<<durable · new>>` reaches Mermaid as `<>` and errors, and `Registration<br/>proves it` reaches it as `Registrationproves it` — **silently**. On the page, escape every `<` as `&lt;`, or keep the source in a JS template literal and call `mermaid.render()`. Fenced blocks in the markdown are unaffected.
2. **`;` separates statements.** A semicolon inside a label splits the line — `BEGIN; INSERT; COMMIT` in a message is a parse error. Use commas or an em dash. This one applies to **both** surfaces.
3. **Page CSS leaks in.** `label`, `node`, `edge`, `cluster` and `title` are Mermaid's own class names. A page utility called `.label` matched 90 elements and rendered every diagram uppercase, letter-spaced and tiny, with CamelCase destroyed. Namespace page classes — `.kicker`, not `.label`.

**Validate at render level, not parse level.** `mermaid.parse()` accepts the broken `<>` in hazard 1, so a parse-only check reports success on a page displaying "Syntax error in text". Load the file in a browser, count `pre.mermaid svg`, count `[aria-roledescription='error']`, read `getComputedStyle` on rendered text — and look at the screenshot.

### The work graph, coloured by state

One Mermaid graph does two jobs — the dependency shape and the progress display. Colour the nodes by state:

````md
```mermaid
flowchart LR
  T1["01 · expand schema"]:::done --> T2["02 · migrate readers"]:::wip
  T1 --> T3["03 · migrate writers"]:::todo
  T2 --> T4["04 · contract"]:::blocked
  T3 --> T4
  classDef done fill:#d1fae5,stroke:#059669,color:#065f46
  classDef wip fill:#fef3c7,stroke:#d97706,color:#92400e
  classDef todo fill:#ffffff,stroke:#94a3b8,color:#475569
  classDef blocked fill:#f1f5f9,stroke:#cbd5e1,color:#94a3b8
```
````

Ticket names only — no acceptance criteria, no file paths. If the graph needs a legend to be read, it has too many states; four is the ceiling.

Use it even where Mermaid isn't rendered — it degrades to a fenced block that still reads top-to-bottom.

### Callouts, rationed

GitHub alert syntax, and only for things that would genuinely surprise:

```md
> [!WARNING]
> This contradicts ADR-0007 (teams grant no resource access). Reopening it is part of the ask — see Decision 3.
```

One or two per spec.

### Deep links from the decision table to its reasoning

Every decision row links to its `###` section. Every mention of another spec links to the file.

### Before/after as trees

For structural change, use two fenced trees under `**Today**` and `**After**` headings:

```
Today                          After
sandbox/src/                   sandbox/src/
├── services/          →       ├── features/
│   ├── file.service.ts        │   ├── files/
│   └── git.service.ts         │   └── git/
└── app.ts                     └── app.ts
```

Fine to draw them stacked rather than side by side when the paths are long.

## Ban list

Never use:

- **Emoji as decoration.** A status glyph in a fixed column is a signal; a rocket in a heading is noise.
- **Bold as emphasis-spray.** Bold is reserved for field labels and the leading term of a rule.
- **`<details>`, and every other cap.** Truncated lists, "…and 12 more", collapsed sections — each costs an agent a section it has to know to open.
- **Bullets nested past two levels.** Three levels means the structure belongs in a table.
- **Tables past four columns.** They wrap in every narrow view.
- **Screenshots.** They go stale silently and can't be diffed.
- **New headings.** The unconditional heading set is fixed, and the conditional sections have the names `/writing-specs` gives them. A section you want that has no heading is a sign the change belongs in the skill, not in one spec.
- **A conditional section stubbed with "n/a".** Omit it. An omitted section is a statement; an empty one is a gap.
- **ASCII art** that isn't a directory tree or a box diagram.
