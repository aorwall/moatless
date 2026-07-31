---
name: asking-questions
description: The two shapes a question to the user can take — a ranked pick or a selection from a set — and what earns a place in the message beside the ask. Use when a turn asks the user to decide something, or when another skill reaches a decision point and hands the turn back.
---

# Asking Questions

This file owns the **shape of the question** and what may sit beside it. Everything else about the turn — the always-rules, what survives the renderer, the pre-send check — is `/writing-messages`, and it applies here too. **Run both.**

## Two shapes, and the second one is easy to miss

**A ranked pick** — right when the answer is genuinely one option out of a few: which language, where the seam sits, build it now or later. 2–4 options, one line of trade-off each, recommendation first with `← recommended` on it, each answerable with one character.

**A selection from a set** — right when the answer is "which of these real things survive, and where does each go". Show the set: every item, one line each, with what you'd do with it. Then ask which survive.

**If you are about to bundle a list of real things into 2–4 combinations, you want the second shape.** The user will regroup in ways you didn't offer. "Which of these four tiers run in CI" beats "db+sandbox / all / db only / sandbox+agent". "Here are the eight things only this file says, keep or move" beats "keep it / trim it / delete it". The tell: you find yourself writing an option that means "some of them".

**Neither shape is a fork in a paragraph.** "So the question becomes X or Y; I'd keep X" reads as settled to you and as unanswered to the reader — and it gets answered by retyping X, which is how you know no list was offered.

## What earns a place in the message

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
- **A fact that survives the strike test still gets one line, not its derivation.** How you established it — the probe, the trace, the three files you read — is not what makes it evidence. Where a page exists, the fact goes on it in full and the message carries the one line plus the link.

## An approval is not a question

The work is done and needs a yes. The artifact is the recommendation, and the one line asking for it is the whole ask — **no options, no ranked list**, because a numbered list under it asks the same thing a second time. **The test is whether anything is left for the user to choose between** — if nothing is, this file does not apply and `/writing-messages` is the whole of it.
