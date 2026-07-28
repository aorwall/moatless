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
| **Answer** | Nothing in it is left unresolved | Verdict line, evidence, page link |
| **Answer with a fork in it** | Not an answer. Reclassify | As a question-turn |
| **Approval** | The work is done and needs a yes | The artifact, then one line: approve, or name what to change |

**An approval carries no options and no recommendation.** The artifact is the recommendation, and the one line asking for it is the whole ask — a numbered list under it asks the same thing a second time.

A fork resolved inside a paragraph — "so the question becomes X or Y; I'd keep X" — reads as settled to you and as unanswered to the reader.

## The question-turn

**Two shapes, and the second one is easy to miss.**

**A ranked pick** — right when the answer is genuinely one option out of a few: which language, where the seam sits, build it now or later. 2–4 options, one line of trade-off each, recommendation first with `← recommended` on it, each answerable with one character.

**A selection from a set** — right when the answer is "which of these real things survive, and where does each go". Show the set: every item, one line each, with what you'd do with it. Then ask which survive.

**If you are about to bundle a list of real things into 2–4 combinations, you want the second shape.** The user will regroup in ways you didn't offer. "Which of these four tiers run in CI" beats "db+sandbox / all / db only / sandbox+agent". "Here are the eight things only this file says, keep or move" beats "keep it / trim it / delete it". The tell: you find yourself writing an option that means "some of them".

### What earns a place in the message

**Strike each fact and ask: which option does this push toward, or what does it change about the cost of picking wrong?** If you can't say, it isn't evidence — it's context, and context goes on the page.

Two roles earn a place. Nothing else does.

| Role | It answers |
|---|---|
| **Evidence** | Why option 2 might be right |
| **Stake** | What it costs you if you pick wrong |

"It took work to find" and "it's interesting" are not roles.

- **Nothing follows the ask** but a single italic footer line: position in the batch, what is already settled.
- **More than about four facts survive? The question is too big.** Split it. Don't trim the message to compensate — trimming keeps the useless fact and shortens the useful one.
- **Over ~150 words, re-run the strike test.** A tripwire, not a cap. Never satisfy it by cutting adjectives.

## The answer-turn

After the verdict, only the reasoning that could change it. Everything else is named and linked, not narrated.

A finding already on the page does not get re-told in the message. Name it — "two more findings there, dedup and naming" — and link.

## Six always-rules

1. **The ask or the verdict is the first line** — not "Two things I found that change the shape of this." If the answer is a command, path, or snippet, it goes first; prose after, if at all.
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

## Skills that ask the user something

Everything above is the shared base — it applies to any turn a person reads, whatever produced it. These skills reach a decision point and hand the turn back:

| Skill | Its ask |
|---|---|
| `/grilling` | A run of them. It owns *which* question and in what order; this file owns the turn each one is sent in |
| `/writing-specs` | Understanding, decisions, then the approval turn at step 4 |
| `/to-tickets` | One selection question over the proposed breakdown |
| `/improve-codebase-architecture` | One selection question over the candidate refactors |
| `/domain-modeling` | A disambiguation: two meanings of a term, pick one |
| `/explaining-code` | One calibration question, asked once |
| `/diagnosing-bugs` | A ranked pick, when no feedback loop can be built |
| `/triage` | Role conflicts and the agent-brief offer |
| `/to-spec` | Confirmation that the seams match |
| `/tdd` | The interface and the seams |
| `/code-review` | Where the spec is, when none is found |
