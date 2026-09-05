---
name: explaining-code
description: Explain how existing code, a feature, or an architecture actually works — a trace along the execution path with file:line at every hop, not an inventory of components. Use when someone asks how something works, where something happens, why control does or doesn't reach a point, how two parts fit together, or for an HTML page or doc describing existing functionality; or when another skill needs the explanation shape.
---

# Explaining Code

Someone asking how a part of the system works is already pointed at it. They do not need motivating, and they do not need the tour. The failure here is not losing their attention — it is **answering a question next to the one they asked**, which makes them switch out and start over.

**Explain the mechanism, not the parts.** Naming components produces the feeling of understanding without the thing itself. The test of a good explanation is not "did they follow it" but **"can they now predict what happens if the input changes."**

## First, name which question it is

Four kinds, escalating. They take different answers, and answering at the wrong level is the most common way to be technically responsive and useless.

| The question | Example | The answer is |
|---|---|---|
| **Find a focus point** | "Which type represents a Turn?" | One line and a `file:line`. Nothing else |
| **Expand a focus point** | "What calls this?" | A ranked list. Most relevant first, not all callers |
| **Understand a subgraph** | "How does message delivery work?" | **A trace** — the rest of this skill |
| **Relate subgraphs** | "How do the sandbox and backend fit together?" | The seam, then a diagram of the two sides |

"How does X work" is almost always the third. Say which one you're answering in your first line — a wrong guess then costs one word to correct instead of a wasted page.

Answer the smaller ones at their own size. A pointer question deserves a pointer, and expanding it into a trace is its own kind of miss.

## Calibrate once, before answering

Explanation tuned for a novice actively degrades comprehension for someone who already knows the area — they have to reconcile your scaffolding against their own model. Prior knowledge is also the only lever that changes the intrinsic difficulty of the material, because it lets whole clusters collapse into one chunk.

So ask, once — a ranked pick, per `/asking-questions`, which puts it through `AskUserQuestion` where the runtime has it and in one line of prose where it doesn't:

> From the top, or do you already have the harness/coordinator split and want me to start at the signal boundary?

Skip the question when the answer is already visible — they used the vocabulary correctly, or they wrote the code. Never ask it twice in a conversation.

## The shape

Five slots, fixed order. Not all get used.

| Slot | Cap | Notes |
|---|---|---|
| **The answer** | 1–2 sentences | The mechanism in one breath. Must be specific enough to be wrong |
| **Vocabulary** | ≤ 4 terms | Only terms used below that they may not already chunk. Drop after calibration |
| **The state** | ≤ 5 entities | The slice of the data model this touches, and its named states. Before the trace |
| **The trace** | ≤ 7 hops | Execution order. One concrete named trigger. `file:line` every hop |
| **The picture** | 1 diagram | Only if the path branches, forks across processes, or loops |
| **Where it bites** | ≤ 3 bullets | Failure modes and surprises that the trace doesn't show |

Then stop and offer the two follow-ups that are actually likely: **trace a variant**, or **show who owns what**. Don't pre-empt them by answering both.

## The state

Most "how does this work" questions are really about **what is true before and what is true after**. Show that slice of the data model before the trace, not after it: naming the entities first lets the reader treat each one as a single chunk when the trace starts moving them around.

Two parts, and they must agree with each other.

**1. The slice.** Only what this path touches.

- **An entity that never appears in a hop never appears on the diagram.** This is the rule that keeps a data model slice from becoming a schema dump.
- **Only fields that change, or that gate a decision.** A column nobody reads on this path is noise, however important it is elsewhere.
- **Name the states; the encoding is the footnote.** "pending" is a state. `agent_session_id IS NULL` is how it happens to be stored. Lead with the name — named states are what the reader will reason with afterwards.
- **Show the guards, not just the writes.** A field in a `WHERE` clause is doing more work than one that is merely written; that is usually where the concurrency safety actually lives, and it is invisible in a schema.

**2. The deltas, inline at each hop** — not collected in a table at the end. The change belongs next to the thing that causes it:

```md
3. **`backend/src/message/send.rs:248`** — persists the row.
   `turn_number: ∅` · `agent_session_id: ∅` → **pending**
```

Every field in a delta appears in the slice, and every field in the slice appears in at least one delta or one guard. That check is self-enforcing: it stops the diagram growing and stops the trace hand-waving.

Which diagram:

| | Use |
|---|---|
| `erDiagram` | The question is how the entities relate |
| `stateDiagram-v2` | One entity moves through **3 or more** states — the workflow case |
| Neither | ≤ 2 entities and one field. A sentence beats a picture |

**Skip the slot entirely when nothing persists.** A pure computation has no state, and inventing a diagram for it is decoration.

## The trace

Start from a real trigger with a name — "a user sends a message to a running task", not "when a message arrives". Then one hop per step, in the order the machine does them:

```md
1. **`backend/src/routes/messages.rs:88`** — accepts the message, writes it pending, returns immediately.
2. **`crates/sandbox/src/agent/coordinator.rs:214`** — picks up pending input, submits it to the harness.
3. …
7. **outcome** — the observable thing the reader can check.
```

Each hop says **what happens and what it decides** — not what the module is responsible for. "Owns retry policy" is a job description; "retries here if the error class is transient, up to 3 times" is a mechanism.

- **`file:line` on every hop, always.** It is the landmark that lets a reader anchor to code they may already recognise, it means they stop holding locations in their head, and it makes every claim checkable — which is the only real defence against explaining something you have not actually verified.
- **≤ 7 hops.** Past that the trace has crossed a seam. Split it in two and name the seam; that is more useful than the eighth hop.
- **One trace, not three.** Two paths in one answer asks the reader to hold both. Pick the main path, name the variants in a line each, offer to trace one.
- **Read the code first.** Every hop is verified against the file, not recalled. An explanation that is confidently wrong at hop 4 costs more than no explanation.

## When a diagram earns its place

A diagram wins by placing information in space, so the reader finds things by looking instead of by searching. That pays only when there is a shape to place — branching, concurrency, two processes, a loop. A linear path is already a numbered list, and the list is better: it carries the `file:line` anchors.

So: Mermaid `sequenceDiagram` when the trace crosses a process boundary, `graph` when it branches or cycles, nothing when it's a straight line.

Tables are not a substitute here. They chunk, but they don't show shape.

## What not to do

- **No component inventory.** A list of modules and their responsibilities is the thing that feels like an explanation and isn't.
- **No backstory.** How it came to be this way is a different question. One line if a decision is load-bearing, with a link to the decision note.
- **No alternatives they didn't ask about.** That's the spec workflow's job, not this one.
- **No preamble.** The first line is the answer or the classification, never "Great question" or "Let me walk you through".
- **Don't explain what they can see.** If they've quoted the function, start below it.

## Where the answer lives

Two places, and the first is not optional.

**1. In chat. Always.** The answer stands on its own there, whether or not you build anything else.

**2. As a page in `scratchpad/`** when any of these hold:

- The trace crossed a process or service boundary and became two or more.
- More than one person will ask this.
- The user asked for a page.

See [HTML-REPORT.md](HTML-REPORT.md), on top of `/html-report`'s mechanics. A spine and a drawn boundary are exactly what chat handles worst, which is why a multi-seam trace earns one.

**The page is the artifact — there is no markdown behind it.** Don't write the explanation to `docs/` first and don't write it there afterwards; `docs/` is for things maintained alongside the code, and a traced explanation goes stale the moment the code moves. Write the page directly. **Answering in chat and stopping is the common failure — it feels finished, and for a multi-seam trace it isn't.**

The one case that stays chat-only: a single linear trace of about four hops. The numbered list already is the diagram.

**Done when:** the chat answer is sent, the page exists in `scratchpad/`, and its link is in the message.

**Give its path, don't paste it** — `scratchpad/<slug>-explained.html`, relative to the workspace root. Quoting the page into chat buries the answer you just gave.

If the user explicitly wants something committed and maintained, that's a normal file request — write it where they ask. It isn't this skill's default output.

Adding a term the codebase reuses? It belongs in `CONTEXT.md`, not in the explanation (`/domain-modeling`).

## Related

- `/codebase-design` — the vocabulary for *evaluating* a design (module, seam, depth, leverage). This skill describes what is there; that one judges it.
- `/spec` — for proposing a change rather than describing the current state.
