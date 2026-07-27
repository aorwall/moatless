# Design elements for specs

Two surfaces, two readers, and the elements below serve both — for different reasons.

The **page** is where a human agrees to the change: read a handful of times over one session, then never again. The **markdown** is what an agent reads twenty times during implementation — in a PR diff, in the task UI, in an editor, from a fresh checkout with no page anywhere.

**Anything that lives only on the page evaporates.** The page is not in the repo and does not outlive the sandbox, so every element it carries has to have a counterpart in the spec once the spec is written.

## Every element earns its place by doing one of four jobs

| Job | The problem it solves | Element |
|---|---|---|
| **Locate** | "Where am I, and what kind of thing is this?" | Fixed heading order · deep links |
| **Chunk** | A wall of text has no handholds — you can't enter or leave it | Tables · one instruction per line · short sections |
| **Signal state** | The reader can't hold what's settled, shipped, or blocked | Status line · progress bar · graph node colours · task lists |
| **Show shape** | Relationships prose can't carry | Mermaid graphs · before/after trees |

If an element does none of the four, it is decoration. Cut it. Decoration is not neutral — it competes for the same attention the real signals need.

There used to be a fifth — **hide**, via `<details>` — and it is gone from the markdown. It existed to cap a human's scroll while keeping the substance in the file. The human reads the page now, and hiding content from an agent buys nothing while costing it a section it has to know to open.

## No section is prose-only

The failure mode isn't length, it's flatness. A dense four-column table gets read to the bottom; three uniform paragraphs do not, and shortening them to two doesn't help. What stops reading is a stretch with nothing for the eye to land on.

So every `##` carries at least one table, list, diagram, or code block. A section that resists this is telling you something true: it's either not yet decided, or it's two sections.

The corollary bites hardest when tidying. Cutting an element to "clean up" removes a handhold, and the page gets harder to read while looking neater.

## The elements

### Status line with a progress bar

Progress that isn't visible doesn't register. Put it in the status line, in text, so it renders everywhere:

```md
> **Status:** in progress · ▓▓▓▓▓░░░░░ 3/7 tickets · **Updated:** 2026-07-25
```

Update it in the same edit that lands a ticket. A status line that lags is worse than none — once it's wrong, no line in the file is trusted.

### The decision table — four columns, never five

```md
| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-where-the-seam-sits) | Where the seam sits | Coordinator owns retry | Per-harness retry diverges silently |
| [2](#2-message-assembly) | Message assembly | Backend only | Sandbox-side coalescing duplicates work per harness |
```

The table is the highest-value element in the file, and the reason is the layout, not the content: the rejected alternative sits *beside* the chosen one, so the reader cannot take in the decision without also taking in what it cost. A bulleted list of decisions hides that; a table can't.

Four columns is the cap — past that, cells wrap and the table stops being scannable in a narrow diff view. The `#` links down to the `###` section carrying the full reasoning.

For a decision that is still open, add a fifth *row* rather than a fifth column: ranked options, one line each, recommendation first.

### The work graph, coloured by state

The same Mermaid graph does two jobs — it shows the dependency shape *and* it is the progress display. Colour the nodes by state and it stops being a plan and starts being a status board:

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

Where Mermaid isn't rendered, it degrades to a fenced block that still reads top-to-bottom. Don't let that stop you using it.

### Callouts, rationed

GitHub alert syntax, and only for things that would genuinely surprise:

```md
> [!WARNING]
> This contradicts ADR-0007 (teams grant no resource access). Reopening it is part of the ask — see Decision 3.
```

One or two per spec. Used for emphasis, they stop meaning anything; used for surprise, they're the only element that reliably interrupts a skim.

### Deep links from the decision table to its reasoning

Every decision row links to its `###` section. Every mention of another spec links to the file. The table is an index, and an index without links is a list — this is what lets an agent land on the paragraph that answers its question instead of the file that contains it.

### Before/after as trees

For structural change, two fenced trees under `**Today**` and `**After**` headings beat any prose description, and they diff cleanly when the plan changes:

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

Each of these looks like design and spends attention without returning any:

- **Emoji as decoration.** A status glyph in a fixed column is a signal; a rocket in a heading is noise.
- **Bold as emphasis-spray.** Bold is reserved for field labels and the leading term of a rule. Once a third of a paragraph is bold, none of it is.
- **`<details>`, and every other cap.** Truncated lists, "…and 12 more", collapsed sections. Nothing skims the spec markdown, and each of these costs an agent a section it has to know to open.
- **Bullets nested past two levels.** Three levels means the structure belongs in a table.
- **Tables past four columns.** They wrap in every narrow view and stop being scannable.
- **Screenshots.** They go stale silently and can't be diffed.
- **New headings.** The heading set is fixed. Adding one costs every future reader the map they'd already learned.
- **ASCII art** that isn't a directory tree or a box diagram.
