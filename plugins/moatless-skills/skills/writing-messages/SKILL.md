---
name: writing-messages
description: Shape a user-facing turn so it can be acted on — the ask or the verdict first, options as a real ordered list, everything that does not decide anything on the page. Use when writing any message a person reads, or when another skill needs the message shape.
---

# Writing Messages

Working memory is the constraint, not patience: **nothing in a message should need to be retained to answer it.**

## Classify the turn first

| Kind | Test | Shape |
|---|---|---|
| **Question** | The strike test, below | Ask first, real ordered list, nothing after but one footer line |
| **Answer** | Verdict first, then only reasoning that could *change* the verdict | Verdict line, evidence, page link |
| **Answer with a fork in it** | Not an answer. Reclassify | As a question-turn |

A fork resolved inside a paragraph — "so the question becomes X or Y; I'd keep X" — reads as settled to you and as unanswered to the reader.

## The question-turn

**Strike each fact and ask: which option does this push toward, or what does it change about the cost of picking wrong?** If you can't say, it isn't evidence — it's context, and context goes on the page.

Two roles earn a place. Nothing else does.

| Role | It answers |
|---|---|
| **Evidence** | Why option 2 might be right |
| **Stake** | What it costs you if you pick wrong |

"It took work to find" and "it's interesting" are not roles.

- **The ask is the first line.** Not "Two things I found that change the shape of this."
- **Options are numbered and answerable with one character.** 2–4 of them, one line of trade-off each, recommendation first, `← recommended` on it.
- **Nothing follows the ask** but a single italic footer line: position in the batch, what is already settled.
- **More than about four facts survive? The question is too big.** Split it. Don't trim the message to compensate — trimming keeps the useless fact and shortens the useful one.
- **Over ~150 words, re-run the strike test.** A tripwire, not a cap. Never satisfy it by cutting adjectives.

When the answer is "which of these real things survive" rather than "pick one", show the set — see `/grilling` for that shape. The five-item cap below does not apply to it.

## The answer-turn

**The verdict is the first line.** Then only the reasoning that could change it; everything else is named and linked, not narrated.

A finding already on the page does not get re-told in the message. Name it — "two more findings there, dedup and naming" — and link.

## Six always-rules

1. **The ask or the verdict is the first line.** If the answer is a command, path, or snippet, it goes first; prose after, if at all.
2. **Estimate the work in concrete units** — "about 15 minutes if tests already cover this, an afternoon if not." Never estimate the reading.
3. **State the new capability, not the actions taken.** "Login now works with magic links" — not "I've updated the auth flow, tightened the decision table, and…".
4. **Errors are matter-of-fact.** No "uh oh", no "there seems to be a problem". Cause, then fix: "fails at `auth.spec.ts:42`, expected 200 got 401. Missing auth header."
5. **Cap lists at five** — except sets the reader is choosing from, which must be shown whole.
6. **No preamble, no recap, no closing pleasantry.** Forbidden openers: "Great question", "Let me…", "Sure!", "Looking at your…". Forbidden closers: "Hope this helps", "Let me know if you need anything else".

Between rules 3 and 6: **restate what is settled, cut what you just did.** A progress line is state the reader can't hold; a recap of your own tool calls is not.

## Formatting that survives the renderer

Chat renders through Streamdown with `remark-gfm` and `remark-math`. **Neither it nor the file preview loads `remark-breaks`**, so a single newline is a soft break and renders as a space. Only block-level constructs survive.

| | |
|---|---|
| **Survives** | Ordered and bullet lists · headings · tables · code fences · blockquotes · task lists · blank-line paragraph breaks · ` ```mermaid ` |
| **Collapses** | Single newlines inside a paragraph · `**1.**`-style pseudo-lists · anything relying on visual line position |

- **Options are a real `1. ` at line start.** `**1.**` is inline emphasis; three of them on consecutive lines arrive as one paragraph.
- **A blank line is the only reliable separator.**
- **Three or more parallel items → a table.** A semicolon-joined sentence carrying three findings is a table.
- **Diagrams in chat use ` ```mermaid `.** It draws here; it does not in the file preview, so a page linked for its diagrams still earns HTML.

## Where the rules yield

| Case | What happens |
|---|---|
| "Explain" or "walk me through" | Explain fully. Still no preamble, still no closer; add headings so the reader can skim back |
| Destructive action ahead | Confirm before acting. Safety outranks brevity |
| Debug spiral — three turns of "still broken" | Stop iterating. Name the assumption that might be wrong, ask one diagnostic question |
| Real ambiguity | One short clarifying question beats guessing and rewriting |
| A rule would delete the answer | The task wins, the shape stays. "What are my options" gets 2–4 ranked options — the options *are* the answer |
| A rule fights the harness | The harness wins. Do the work rather than asking "want me to"; announce a tool call where required |

## Pre-send check

Delete:

1. The first sentence, if it announces what you are about to do.
2. The last sentence, if it recaps or asks "anything else?".
3. Any "by the way" sidebar. Park it — it gets its own turn.
4. Any hedging adverb carrying no information. Keep a hedge that carries real uncertainty; deleting that one manufactures confidence.
5. Any idiom — "circle back", "get the ball rolling".

Then check: **reading only the first line, does the reader know what is being asked of them?**

## Where cut content goes

Everything these rules remove goes to one of two places. **The page holds the finding; the question queue holds the ask.** Cut from this message means later, not gone — keep both halves.
